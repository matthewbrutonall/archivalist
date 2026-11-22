#!/usr/bin/env ruby
# frozen_string_literal: true

# Archivalist Bounded Live PUI Concurrency Safety Gate
#
# Purpose:
#   Exercises real PUIIndexer worker batches (PeriodicIndexer#start_worker_thread ->
#   PUIIndexer#fetch_records -> IndexerCommon#index_records -> document prepare hooks ->
#   IndexerCommon#index_batch) against Repository 3 only on a verified disposable profiling
#   stack, once with PUI_THREADS=1 and once with PUI_THREADS=4, with the Rust PUI document
#   and projection flags enabled that exercise the per-thread projection maps fixed in
#   commit 545b5c809.
#
# Guardrails:
#   - Verifies backend and Solr URLs against ArchivalistHarnessSafety allowlists.
#   - Verifies disposable profiling stack database nonce and Solr identity core via
#     ArchivalistProfilingStack.verify! before any login, core creation, or sidecar start.
#   - Bounds execution strictly to Repository 3 (refuses identity repo 2 and medium repo 4).
#   - Enables only the Rust PUI document/projection flags required to exercise the per-thread
#     maps (:rust_index_document_builder_enabled, :rust_pui_document_preparation_enabled,
#     :rust_pui_record_inheritance_candidate_enabled, :rust_pui_common_metadata_enabled);
#     keeps other Rust flags (:rust_index_document_enrichment_enabled,
#     :rust_pui_availability_preparation_enabled, :rust_pui_auxiliary_documents_enabled) false
#     and leaves default pui_indexer_records_per_thread (25) unchanged.
#   - Collects mutex-synchronized acceptance/rejection counters across worker threads and
#     fails closed on any rejection, silent native fallback, worker error, or thread-local state leak.
#   - Compares all stored documents across the two stage cores via complete cursorMark pagination
#     with recursive type-strict equality (ignoring only Solr _version_ and score).
#   - Always stops the stage-owned sidecar in ensure.

require 'rubygems'
if defined?(JRUBY_VERSION) && File.directory?('/archivesspace/build/gems/jruby/3.1.0')
  ENV['GEM_HOME'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  ENV['GEM_PATH'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  Gem.paths = {
    'GEM_HOME' => '/archivesspace/build/gems/jruby/3.1.0',
    'GEM_PATH' => '/archivesspace/build/gems/jruby/3.1.0'
  }
end

require 'json'
require 'net/http'
require 'uri'
require 'fileutils'
require 'time'
require 'digest'
require 'securerandom'

require_relative 'integration_stage_harness'
require_relative 'archivalist_profiling_stack'

module ArchivalistPuiConcurrencyGate
  ALLOWED_REPO_ID = 3
  FORBIDDEN_REPO_IDS = [2, 4].freeze
  GATE_THREAD_COUNTS = [1, 4].freeze
  IGNORED_SOLR_FIELDS = ['_version_', 'score'].freeze
  COMMON_HELPERS = %w[subjects agents audit_info years level summary extents].freeze

  ENABLED_RUST_FLAGS = %i[
    rust_index_document_builder_enabled
    rust_pui_document_preparation_enabled
    rust_pui_record_inheritance_candidate_enabled
    rust_pui_common_metadata_enabled
  ].freeze

  DISABLED_RUST_FLAGS = %i[
    rust_index_document_enrichment_enabled
    rust_pui_availability_preparation_enabled
    rust_pui_auxiliary_documents_enabled
  ].freeze

  ALL_RUST_FLAGS = (ENABLED_RUST_FLAGS + DISABLED_RUST_FLAGS).freeze

  def self.validate_repo_id!(repo_id_raw, identity_repo_id: 2)
    unless repo_id_raw.to_s =~ /\A[1-9]\d*\z/
      raise "Safety violation: REPO_ID must be a positive integer (got #{repo_id_raw.inspect})"
    end

    repo_id = repo_id_raw.to_i
    if repo_id == identity_repo_id.to_i || FORBIDDEN_REPO_IDS.include?(repo_id)
      raise "Safety violation: REPO_ID=#{repo_id} is forbidden in this bounded gate (repo 2 is stack identity; repo 4 is the 24,200-AO medium repo)"
    end
    unless repo_id == ALLOWED_REPO_ID
      raise "Safety violation: bounded concurrency safety gate is restricted to REPO_ID=#{ALLOWED_REPO_ID} (got #{repo_id})"
    end

    repo_id
  end

  def self.configure_gate_flags!(app_config, sidecar_url:, threads:)
    unless threads.is_a?(Integer) && threads >= 1
      raise "Safety violation: invalid thread count #{threads.inspect}"
    end

    app_config[:pui_indexer_thread_count] = threads
    app_config[:rust_index_document_builder_url] = sidecar_url
    ENABLED_RUST_FLAGS.each { |flag| app_config[flag] = true }
    DISABLED_RUST_FLAGS.each { |flag| app_config[flag] = false }

    verify_gate_flags!(app_config, sidecar_url: sidecar_url, threads: threads)
  end

  def self.verify_gate_flags!(app_config, sidecar_url:, threads:)
    unless app_config[:pui_indexer_thread_count] == threads
      raise "Safety violation: expected pui_indexer_thread_count=#{threads}, got #{app_config[:pui_indexer_thread_count].inspect}"
    end
    unless app_config[:rust_index_document_builder_url] == sidecar_url
      raise "Safety violation: unexpected rust_index_document_builder_url #{app_config[:rust_index_document_builder_url].inspect}"
    end

    ENABLED_RUST_FLAGS.each do |flag|
      raise "Safety violation: required Rust PUI flag #{flag} is not enabled" unless app_config[flag] == true
    end
    DISABLED_RUST_FLAGS.each do |flag|
      raise "Safety violation: non-target Rust flag #{flag} must remain false" unless app_config[flag] == false
    end
    true
  end

  def self.reset_gate_flags!(app_config, default_pui_threads: 1)
    app_config[:pui_indexer_thread_count] = default_pui_threads
    ALL_RUST_FLAGS.each { |flag| app_config[flag] = false }
  end

  def self.new_instrumentation(threads:, records_per_thread:)
    {
      'threads_configured' => threads,
      'records_per_thread' => records_per_thread,
      'worker_batches_dispatched' => 0,
      'fetch_batches' => 0,
      'records_fetched' => 0,
      'index_batches' => 0,
      'builder_calls' => 0,
      'builder_inputs_sent' => 0,
      'builder_docs_received' => 0,
      'native_fallbacks' => 0,
      'accepted_base_docs' => 0,
      'accepted_routing' => 0,
      'accepted_notes' => 0,
      'accepted_final' => 0,
      'accepted_candidate' => 0,
      'accepted_common_helpers' => COMMON_HELPERS.to_h { |h| [h, 0] },
      'rejections' => 0,
      'thread_leak_violations' => 0,
      'worker_statuses' => {},
      'fetch_thread_ids' => [],
      'index_thread_ids' => [],
      'builder_thread_ids' => [],
      'snapshot_digests' => {}
    }
  end

  def self.validate_acceptance!(instrumentation, expected_records:, expected_batches:, expected_threads:, sidecar_requests_delta:)
    errors = []

    statuses = (instrumentation['worker_statuses'] || {}).values.flatten
    if statuses.empty?
      errors << 'no worker statuses recorded'
    elsif statuses.include?(2) # PeriodicIndexer::WORKER_STATUS_INDEX_ERROR
      errors << "worker thread reported WORKER_STATUS_INDEX_ERROR: #{instrumentation['worker_statuses'].inspect}"
    elsif !statuses.all? { |s| s == 1 }
      errors << "expected all active worker threads to return WORKER_STATUS_INDEX_SUCCESS (1), got #{instrumentation['worker_statuses'].inspect}"
    end

    expected_active_threads = [expected_threads, expected_batches].min
    fetch_threads = Array(instrumentation['fetch_thread_ids']).uniq.length
    index_threads = Array(instrumentation['index_thread_ids']).uniq.length
    builder_threads = Array(instrumentation['builder_thread_ids']).uniq.length

    if fetch_threads != expected_active_threads
      errors << "expected #{expected_active_threads} distinct worker threads in fetch_records, observed #{fetch_threads}"
    end
    if index_threads != expected_active_threads
      errors << "expected #{expected_active_threads} distinct worker threads in index_records, observed #{index_threads}"
    end
    if builder_threads != expected_active_threads
      errors << "expected #{expected_active_threads} distinct worker threads in RustIndexDocumentBuilder#build, observed #{builder_threads}"
    end

    if instrumentation['worker_batches_dispatched'] != expected_batches
      errors << "worker_batches_dispatched (#{instrumentation['worker_batches_dispatched']}) != expected_batches (#{expected_batches})"
    end
    if instrumentation['fetch_batches'] != expected_batches
      errors << "fetch_batches (#{instrumentation['fetch_batches']}) != expected_batches (#{expected_batches})"
    end
    if instrumentation['records_fetched'] != expected_records
      errors << "records_fetched (#{instrumentation['records_fetched']}) != expected_records (#{expected_records})"
    end
    if instrumentation['index_batches'] != expected_batches
      errors << "index_batches (#{instrumentation['index_batches']}) != expected_batches (#{expected_batches})"
    end
    if instrumentation['builder_calls'] != expected_batches
      errors << "builder_calls (#{instrumentation['builder_calls']}) != expected_batches (#{expected_batches})"
    end
    if instrumentation['builder_inputs_sent'] != expected_records
      errors << "builder_inputs_sent (#{instrumentation['builder_inputs_sent']}) != expected_records (#{expected_records})"
    end
    if instrumentation['builder_docs_received'] != expected_records
      errors << "builder_docs_received (#{instrumentation['builder_docs_received']}) != expected_records (#{expected_records})"
    end
    if sidecar_requests_delta != expected_batches
      errors << "sidecar_requests_delta (#{sidecar_requests_delta}) != expected_batches (#{expected_batches})"
    end
    if instrumentation['native_fallbacks'] != 0
      errors << "silent native_fallbacks detected (#{instrumentation['native_fallbacks']})"
    end
    if instrumentation['accepted_base_docs'] != expected_records
      errors << "accepted_base_docs (#{instrumentation['accepted_base_docs']}) != expected_records (#{expected_records})"
    end
    if instrumentation['accepted_routing'] != expected_records
      errors << "accepted_routing (#{instrumentation['accepted_routing']}) != expected_records (#{expected_records})"
    end
    if instrumentation['accepted_notes'] != expected_records
      errors << "accepted_notes (#{instrumentation['accepted_notes']}) != expected_records (#{expected_records})"
    end
    if instrumentation['accepted_final'] != expected_records
      errors << "accepted_final (#{instrumentation['accepted_final']}) != expected_records (#{expected_records})"
    end
    if instrumentation['accepted_candidate'] != expected_records
      errors << "accepted_candidate (#{instrumentation['accepted_candidate']}) != expected_records (#{expected_records})"
    end

    helpers = instrumentation['accepted_common_helpers'] || {}
    COMMON_HELPERS.each do |helper_name|
      if helpers[helper_name] != expected_records
        errors << "accepted_common_helpers[#{helper_name}] (#{helpers[helper_name].inspect}) != expected_records (#{expected_records})"
      end
    end

    if instrumentation['rejections'] != 0
      errors << "projection rejections detected (#{instrumentation['rejections']})"
    end
    if instrumentation['thread_leak_violations'] != 0
      errors << "thread-local or instance projection leak detected (#{instrumentation['thread_leak_violations']})"
    end

    unless errors.empty?
      raise "Rust PUI acceptance verification failed (threads=#{expected_threads}): #{errors.join('; ')}"
    end

    true
  end

  def self.verify_snapshot_equivalence!(pass1_digests, pass2_digests, expected_records:)
    unless pass1_digests.is_a?(Hash) && pass2_digests.is_a?(Hash)
      raise 'Snapshot equivalence failure: snapshot_digests must be Hashes'
    end
    if pass1_digests.length != expected_records || pass2_digests.length != expected_records
      raise "Snapshot equivalence failure: expected #{expected_records} URIs, got pass1=#{pass1_digests.length}, pass2=#{pass2_digests.length}"
    end
    unless pass1_digests.keys.sort == pass2_digests.keys.sort
      missing = pass1_digests.keys - pass2_digests.keys
      extra = pass2_digests.keys - pass1_digests.keys
      raise "Snapshot equivalence failure: URI set mismatch (missing=#{missing.first(5).inspect}, extra=#{extra.first(5).inspect})"
    end

    mismatched_uris = pass1_digests.keys.select { |uri| pass1_digests[uri] != pass2_digests[uri] }
    unless mismatched_uris.empty?
      raise "Snapshot equivalence failure: record content changed between passes for #{mismatched_uris.length} URI(s): #{mismatched_uris.first(5).inspect}"
    end

    true
  end

  def self.fetch_cursor_paginated_docs(solr_base, core, page_size: 250, http_get_json: nil)
    getter = http_get_json || lambda do |url|
      res = Net::HTTP.get_response(URI(url))
      raise "Solr GET #{url} failed: HTTP #{res.code}" unless res.is_a?(Net::HTTPSuccess)

      JSON.parse(res.body)
    end

    cursor = '*'
    seen_cursors = {}
    expected_num_found = nil
    previous_id = nil
    docs = []

    loop do
      raise "Cursor cycle detected in #{core} at cursorMark=#{cursor}" if seen_cursors.key?(cursor)

      seen_cursors[cursor] = true
      query = URI.encode_www_form(
        'q' => '*:*',
        'sort' => 'id asc',
        'cursorMark' => cursor,
        'rows' => page_size,
        'wt' => 'json'
      )
      data = getter.call("#{solr_base.chomp('/')}/#{core}/select?#{query}")
      response = data.fetch('response')
      num_found = response.fetch('numFound')
      unless num_found.is_a?(Integer) && (expected_num_found.nil? || num_found == expected_num_found)
        raise "Changing or invalid numFound in #{core}: #{num_found.inspect} (expected #{expected_num_found.inspect})"
      end

      expected_num_found = num_found
      page_docs = response.fetch('docs')
      page_docs.each do |doc|
        doc_id = doc['id']
        unless doc_id.is_a?(String) && !doc_id.empty? && (previous_id.nil? || doc_id > previous_id)
          raise "Missing, repeated, or unordered id in #{core}: #{doc_id.inspect} after #{previous_id.inspect}"
        end

        previous_id = doc_id
        docs << doc.reject { |k, _| IGNORED_SOLR_FIELDS.include?(k) }
      end

      next_cursor = data.fetch('nextCursorMark')
      break if next_cursor == cursor

      raise "Empty non-final page in #{core} at cursorMark=#{cursor}" if page_docs.empty?

      cursor = next_cursor
    end

    if docs.length != expected_num_found
      raise "Incomplete cursor pagination in #{core}: yielded #{docs.length} of #{expected_num_found} documents"
    end

    { 'numFound' => expected_num_found, 'docs' => docs, 'pages' => seen_cursors.length }
  end

  def self.first_difference(left, right, path = 'doc')
    if left.class != right.class
      return "#{path}: #{left.class} != #{right.class}"
    end

    case left
    when Hash
      if left.keys.sort != right.keys.sort
        symmetric_diff = (left.keys - right.keys) | (right.keys - left.keys)
        return "#{path}: field sets differ (#{symmetric_diff.sort.inspect})"
      end
      left.keys.sort.each do |key|
        diff = first_difference(left[key], right[key], "#{path}.#{key}")
        return diff if diff
      end
    when Array
      if left.length != right.length
        return "#{path}: lengths #{left.length} != #{right.length}"
      end
      left.each_with_index do |item, idx|
        diff = first_difference(item, right[idx], "#{path}[#{idx}]")
        return diff if diff
      end
    else
      return "#{path}: values differ (#{left.inspect} != #{right.inspect})" unless left == right
    end
    nil
  end

  def self.compare_cores_cursor_paginated!(solr_base, baseline_core, candidate_core, expected_count:, page_size: 250, http_get_json: nil)
    left_res = fetch_cursor_paginated_docs(solr_base, baseline_core, page_size: page_size, http_get_json: http_get_json)
    right_res = fetch_cursor_paginated_docs(solr_base, candidate_core, page_size: page_size, http_get_json: http_get_json)

    if left_res['numFound'] != right_res['numFound']
      raise "numFound mismatch: #{baseline_core}=#{left_res['numFound']} vs #{candidate_core}=#{right_res['numFound']}"
    end
    if expected_count && left_res['numFound'] != expected_count
      raise "numFound (#{left_res['numFound']}) != expected_count (#{expected_count})"
    end

    left_docs = left_res['docs']
    right_docs = right_res['docs']
    max_len = [left_docs.length, right_docs.length].max
    mismatches = []

    (0...max_len).each do |idx|
      l_doc = left_docs[idx]
      r_doc = right_docs[idx]
      diff = if l_doc.nil? || r_doc.nil?
               "doc[#{idx}]: one core ended before the other"
             elsif l_doc['id'] != r_doc['id']
               "doc[#{idx}]: id mismatch (#{l_doc['id']} != #{r_doc['id']})"
             else
               first_difference(l_doc, r_doc, "doc[#{l_doc['id']}]")
             end
      mismatches << diff if diff
    end

    # Also verify privacy/internal-marker invariants on all stored docs
    [baseline_core, candidate_core].zip([left_docs, right_docs]).each do |core_name, docs|
      docs.each do |doc|
        id = doc['id']
        raise "Privacy leak in #{core_name} #{id}: root ancestors present" if doc.key?('ancestors')
        if doc['json'].is_a?(String)
          parsed_json = JSON.parse(doc['json'])
          raise "Privacy leak in #{core_name} #{id}: json ancestors present" if parsed_json.is_a?(Hash) && parsed_json.key?('ancestors')
        end
        doc.each_key do |k|
          raise "Internal marker leak in #{core_name} #{id}: #{k}" if k.to_s.start_with?('_archivalist_')
        end
      end
    end

    unless mismatches.empty?
      first_ten = mismatches.first(10)
      raise "Document parity failed (#{mismatches.length} mismatch(es) across #{max_len} docs); first #{first_ten.length}: #{first_ten.join(' | ')}"
    end

    {
      'baseline_core' => baseline_core,
      'candidate_core' => candidate_core,
      'num_found' => left_res['numFound'],
      'compared_docs' => max_len,
      'mismatches' => 0,
      'baseline_pages' => left_res['pages'],
      'candidate_pages' => right_res['pages']
    }
  end

  DEFAULT_WORKER_OFFER_TIMEOUT_MS = 5000
  DEFAULT_WORKER_JOIN_TIMEOUT_S = 30

  def self.prepare_isolated_data_dir!(base_dir:, run_id:, threads:, primary_data_dir: nil)
    raise 'Safety violation: base_dir must be a non-empty String' if base_dir.to_s.strip.empty?
    raise 'Safety violation: run_id must be a non-empty String' if run_id.to_s.strip.empty?
    unless threads.is_a?(Integer) && threads >= 1
      raise "Safety violation: invalid thread count #{threads.inspect}"
    end

    expanded_base = File.expand_path(base_dir)
    if primary_data_dir && !primary_data_dir.to_s.strip.empty?
      expanded_primary = File.expand_path(primary_data_dir)
      if expanded_base == expanded_primary ||
         expanded_base.start_with?("#{expanded_primary}/") ||
         expanded_primary.start_with?("#{expanded_base}/")
        raise "Safety violation: isolated index-state base_dir (#{expanded_base}) overlaps primary data_directory (#{expanded_primary})"
      end
    end

    dir_name = "pui_gate_index_state_#{run_id}_t#{threads}_#{SecureRandom.hex(4)}"
    data_dir = File.join(expanded_base, dir_name)
    if File.exist?(data_dir)
      raise "Safety violation: isolated index-state directory already exists: #{data_dir}"
    end

    FileUtils.mkdir_p(File.join(data_dir, 'indexer_pui_state'))
    data_dir
  end

  # Configures per-pass AppConfig state (isolated preserved data_dir, stage core solr_url,
  # Rust PUI gate flags) and the RustIndexDocumentBuilder#build instrumentation wrapper, yields
  # the preserved data_dir to the pass body, and restores AppConfig and RustIndexDocumentBuilder#build
  # in ensure without ever deleting the created data_dir.
  def self.with_pass_harness_state(
    app_config:,
    builder_class:,
    sidecar_url:,
    threads:,
    default_pui_threads:,
    target_solr_url:,
    index_state_base_dir:,
    run_id:,
    mutex:,
    instrumentation:
  )
    orig_data_dir = app_config[:data_directory]
    orig_solr_url = app_config[:solr_url]
    orig_builder_build = builder_class.instance_method(:build)

    data_dir = prepare_isolated_data_dir!(
      base_dir: index_state_base_dir,
      run_id: run_id,
      threads: threads,
      primary_data_dir: orig_data_dir
    )
    mutex.synchronize { instrumentation['index_state_data_dir'] = data_dir }

    app_config[:data_directory] = data_dir
    app_config[:solr_url] = target_solr_url
    configure_gate_flags!(app_config, sidecar_url: sidecar_url, threads: threads)

    builder_class.define_method(:build) do |inputs|
      res = orig_builder_build.bind(self).call(inputs)
      mutex.synchronize do
        instrumentation['builder_calls'] += 1
        instrumentation['builder_inputs_sent'] += inputs.length if inputs.is_a?(Array)
        instrumentation['builder_docs_received'] += res.length if res.is_a?(Hash)
        instrumentation['builder_thread_ids'] << Thread.current.object_id
      end
      res
    end

    yield data_dir
  ensure
    builder_class.define_method(:build, orig_builder_build) if orig_builder_build
    reset_gate_flags!(app_config, default_pui_threads: default_pui_threads)
    app_config[:data_directory] = orig_data_dir
    app_config[:solr_url] = orig_solr_url
  end

  # Runs the production PeriodicIndexer worker queue loop (start_worker_thread) for the given
  # record_types -> id_sets map on the provided indexer instance.
  # Guarantees on ALL exits (normal return or any exception):
  #   1. Enqueues :finished tokens for all gate-spawned worker threads.
  #   2. Joins all gate-spawned worker threads with a bounded timeout (join_timeout_s).
  #   3. If any gate-spawned worker thread remains alive on timeout/failure, terminates ONLY
  #      those gate-owned worker threads and fails closed (never returns success).
  def self.run_worker_batches_for_repo(
    indexer,
    id_sets,
    threads:,
    records_per_thread:,
    mutex:,
    instrumentation:,
    queue_factory: nil,
    offer_timeout_ms: DEFAULT_WORKER_OFFER_TIMEOUT_MS,
    join_timeout_s: DEFAULT_WORKER_JOIN_TIMEOUT_S
  )
    id_sets.each do |type, id_set|
      next if id_set.empty?

      time_unit = defined?(JRUBY_VERSION) ? java.util.concurrent.TimeUnit::MILLISECONDS : :milliseconds
      work_queue = if queue_factory
                     queue_factory.call(threads)
                   else
                     java.util.concurrent.LinkedBlockingQueue.new(threads)
                   end

      workers = []
      worker_statuses = []
      primary_error = nil
      cleanup_errors = []

      begin
        (0...threads).each do
          workers << indexer.start_worker_thread(work_queue, type)
        end

        id_set.each_slice(records_per_thread) do |id_subset|
          mutex.synchronize { instrumentation['worker_batches_dispatched'] += 1 }
          until work_queue.offer(id_subset, offer_timeout_ms, time_unit)
            workers.each do |thread|
              thread.value if thread.status.nil?
            end
          end
        end
      rescue Exception => e
        primary_error = e
      ensure
        # If an early error interrupted batch queueing while the bounded queue was full,
        # clear unconsumed batch slices so :finished tokens can be offered immediately.
        if primary_error && work_queue.respond_to?(:clear)
          begin
            work_queue.clear
          rescue Exception => e
            cleanup_errors << e
          end
        end

        workers.length.times do
          begin
            offered = work_queue.offer(:finished, offer_timeout_ms, time_unit)
            unless offered
              if work_queue.respond_to?(:clear)
                work_queue.clear
                offered = work_queue.offer(:finished, offer_timeout_ms, time_unit)
              end
              cleanup_errors << RuntimeError.new("Failed to enqueue :finished token for #{type} worker") unless offered
            end
          rescue Exception => e
            cleanup_errors << e
          end
        end

        still_alive = []
        workers.each do |thread|
          next unless thread

          begin
            joined = thread.join(join_timeout_s)
            if joined.nil?
              still_alive << thread
            else
              worker_statuses << thread.value
            end
          rescue Exception => e
            cleanup_errors << e unless e.equal?(primary_error)
          end
        end

        unless still_alive.empty?
          still_alive.each do |thread|
            begin
              thread.kill if thread.alive?
            rescue Exception => e
              cleanup_errors << e
            end
          end
          still_alive.each do |thread|
            begin
              thread.join(join_timeout_s)
            rescue Exception => e
              cleanup_errors << e
            end
          end
          mutex.synchronize do
            instrumentation['terminated_worker_threads'] =
              (instrumentation['terminated_worker_threads'] || 0) + still_alive.length
          end
          alive_after_kill = still_alive.count(&:alive?)
          cleanup_errors << RuntimeError.new(
            "Terminated #{still_alive.length} gate-owned worker thread(s) for #{type} that remained alive " \
            "after #{join_timeout_s}s join timeout (alive_after_kill=#{alive_after_kill})"
          )
        end

        mutex.synchronize do
          instrumentation['worker_statuses'][type.to_s] = worker_statuses
        end
      end

      if primary_error
        if cleanup_errors.empty?
          raise primary_error
        else
          cleanup_msg = cleanup_errors.map { |err| "#{err.class}: #{err.message}" }.join('; ')
          raise RuntimeError, "#{primary_error.class}: #{primary_error.message} [worker cleanup: #{cleanup_msg}]", primary_error.backtrace
        end
      elsif !cleanup_errors.empty?
        first_err = cleanup_errors.first
        if cleanup_errors.length == 1
          raise first_err
        else
          extra_msg = cleanup_errors[1..].map { |err| "#{err.class}: #{err.message}" }.join('; ')
          raise RuntimeError, "#{first_err.class}: #{first_err.message} [additional cleanup errors: #{extra_msg}]", first_err.backtrace
        end
      end

      unless worker_statuses.all? { |status| status == 0 }
        indexer.send_commit
      end
    end
  end
end

if __FILE__ == $PROGRAM_NAME
  SOLR_BASE = ENV['SOLR_BASE_URL'] || 'http://archivalist-profiling-solr:8983/solr'
  BACKEND_BASE = ENV['BACKEND_URL'] || 'http://archivalist-profiling-backend:4567'

  unless ArchivalistHarnessSafety.valid_backend_url?(BACKEND_BASE)
    raise "Safety violation: Invalid or unapproved backend URL '#{BACKEND_BASE}'"
  end
  unless ArchivalistHarnessSafety.valid_solr_url?(SOLR_BASE)
    raise "Safety violation: Invalid or unapproved Solr URL '#{SOLR_BASE}'"
  end

  STACK_IDENTITY = ArchivalistProfilingStack.verify!(env: ENV, backend_url: BACKEND_BASE, solr_url: SOLR_BASE)
  REPO_ID = ArchivalistPuiConcurrencyGate.validate_repo_id!(
    ENV['REPO_ID'] || '3',
    identity_repo_id: STACK_IDENTITY['identity_repo_id']
  )

  SIDECAR_BINARY = ArchivalistHarnessSafety.resolve_sidecar_binary(ENV)
  SOURCE_PROVENANCE = ArchivalistHarnessSafety.source_provenance(ENV)

  BENCHMARK_DIR = File.expand_path('../benchmarks', __dir__)
  FileUtils.mkdir_p(BENCHMARK_DIR)

  GATE_RUN_ID = ArchivalistHarnessSafety.generate_run_id
  RUN_ID_T1 = "#{GATE_RUN_ID}a1"
  RUN_ID_T4 = "#{GATE_RUN_ID}b4"
  CORE_PUI_T1 = ArchivalistHarnessSafety.stage_core_name(RUN_ID_T1, 'candidate', 'pui')
  CORE_PUI_T4 = ArchivalistHarnessSafety.stage_core_name(RUN_ID_T4, 'candidate', 'pui')

  SIDECAR_PORT = if ENV['SIDECAR_PORT']
                   ENV['SIDECAR_PORT'].to_i
                 else
                   ArchivalistHarnessSafety.find_unused_port(start_port: 4590, end_port: 4690)
                 end
  SIDECAR_URL = "http://127.0.0.1:#{SIDECAR_PORT}"

  puts '=' * 80
  puts 'Archivalist Bounded Live PUI Concurrency Safety Gate'
  puts "Gate Run ID:   #{GATE_RUN_ID}"
  puts "Runtime:       #{RUBY_DESCRIPTION}"
  puts "Backend URL:   #{BACKEND_BASE}"
  puts "Solr Base:     #{SOLR_BASE}"
  puts "Stack ID:      #{STACK_IDENTITY['stack_id']} (DB nonce & Solr identity core #{STACK_IDENTITY['identity_core']} verified)"
  puts "Target Repo:   /repositories/#{REPO_ID} (bounded; repo 2 identity and repo 4 medium excluded)"
  puts "Stage Cores:   T1=#{CORE_PUI_T1}  T4=#{CORE_PUI_T4}"
  puts "Sidecar Bin:   #{SIDECAR_BINARY['path']} (#{SIDECAR_BINARY['build_mode']}, sha256=#{SIDECAR_BINARY['sha256']})"
  puts "Git Commit:    #{SOURCE_PROVENANCE['git_commit']} (dirty=#{SOURCE_PROVENANCE['git_dirty']})"
  puts '=' * 80

  $LOAD_PATH.unshift('/archivesspace/common')
  $LOAD_PATH.unshift('/archivesspace/indexer')
  $LOAD_PATH.unshift('/archivesspace/indexer/app/lib')

  require 'active_support/core_ext/string/inflections'
  require 'config/config-distribution'
  AppConfig.reload
  DEFAULT_PUI_THREADS = AppConfig[:pui_indexer_thread_count].to_i
  DEFAULT_PUI_RECORDS_PER_THREAD = AppConfig[:pui_indexer_records_per_thread].to_i
  unless DEFAULT_PUI_THREADS == 1 && DEFAULT_PUI_RECORDS_PER_THREAD == 25
    raise "Unexpected default PUI thread config: threads=#{DEFAULT_PUI_THREADS}, records_per_thread=#{DEFAULT_PUI_RECORDS_PER_THREAD}"
  end
  ArchivalistPuiConcurrencyGate::ALL_RUST_FLAGS.each do |flag|
    raise "Default Rust flag #{flag} must be false, got #{AppConfig[flag].inspect}" unless AppConfig[flag] == false
  end

  AppConfig[:backend_url] = BACKEND_BASE
  AppConfig[:solr_url] = SOLR_BASE
  AppConfig[:search_user_secret] = ENV['SEARCH_USER_SECRET'] || 'devserver'

  require 'index_state'
  require 'asutils'
  require 'aspace_i18n'
  require 'ashttp'
  require 'record_inheritance'
  require 'rust_index_document_builder'
  require 'periodic_indexer'
  require 'pui_indexer'
  require 'index_batch'

  include JSONModel

  def get_sidecar_metrics(sidecar_url)
    metrics_uri = URI("#{sidecar_url}/metrics")
    metrics_res = Net::HTTP.get_response(metrics_uri)
    return {} unless metrics_res.code == '200'

    out = {}
    metrics_res.body.each_line do |line|
      next if line.start_with?('#')

      parts = line.strip.split(/\s+/)
      out[parts[0]] = parts[1].to_f if parts.length == 2
    end
    out
  rescue StandardError
    {}
  end

  # Pre-flight repository & ID discovery (nil state avoids touching primary data_directory)
  discovery_pui = PUIIndexer.new(BACKEND_BASE, nil, 'Gate Discovery PUI')
  discovery_pui.login
  repo_record = JSONModel(:repository).find(REPO_ID)
  raise "Repository #{REPO_ID} not found" if repo_record.nil?
  raise "Repository #{REPO_ID} is not published" unless repo_record.publish == true

  JSONModel.set_repository(REPO_ID)
  pui_record_types = discovery_pui.record_types
  id_sets = {}
  present_counts = {}
  absent_types = []

  pui_record_types.each do |type|
    ids = JSONModel::HTTP.get_json(JSONModel(type).uri_for, :all_ids => true)
    unless ids.is_a?(Array) && ids.all? { |id| id.is_a?(Integer) && id > 0 } && ids.uniq.length == ids.length
      raise "Invalid ID enumeration for #{type} in repo #{REPO_ID}: #{ids.inspect}"
    end

    if ids.empty?
      absent_types << type.to_s
    else
      frozen_ids = ids.sort.freeze
      id_sets[type] = frozen_ids
      present_counts[type.to_s] = frozen_ids.length
    end
  end

  total_expected_records = present_counts.values.sum
  raise "No PUI worker-batch records found in repo #{REPO_ID}" unless total_expected_records > 0

  expected_batches = id_sets.values.sum { |ids| (ids.length.to_f / DEFAULT_PUI_RECORDS_PER_THREAD).ceil }
  puts "Source Repository: #{repo_record.uri} (#{repo_record.repo_code}, publish=#{repo_record.publish})"
  puts "PUIIndexer#record_types: #{pui_record_types.map(&:to_s).inspect}"
  puts "Present PUI worker-batch record counts: #{present_counts.inspect} (total=#{total_expected_records}, batches=#{expected_batches} @ #{DEFAULT_PUI_RECORDS_PER_THREAD}/batch)"

  # Create both fresh isolated stage cores before starting passes
  [
    [CORE_PUI_T1, RUN_ID_T1],
    [CORE_PUI_T4, RUN_ID_T4]
  ].each do |core_name, run_id|
    ArchivalistHarnessSafety.ensure_stage_core(core_name, run_id, solr_base: SOLR_BASE)
    puts "Created isolated stage core: #{core_name}"
  end

  INDEX_STATE_BASE_DIR = File.join(BENCHMARK_DIR, 'pui_concurrency_gate_index_state')
  FileUtils.mkdir_p(INDEX_STATE_BASE_DIR)

  stage_state_file = nil
  begin
    stage_state_file = ArchivalistHarnessSafety.start_stage_sidecar(
      port: SIDECAR_PORT,
      solr_url: SOLR_BASE,
      collection: CORE_PUI_T1,
      run_id: GATE_RUN_ID,
      binary_path: SIDECAR_BINARY['path'],
      expected_sha256: SIDECAR_BINARY['sha256']
    )
    SIDECAR_BINARY.merge!(JSON.parse(File.read(stage_state_file))['launched_binary'])
    puts "Spawned & verified stage sidecar: port=#{SIDECAR_PORT} exe=#{SIDECAR_BINARY['launched_exe']} sha256=#{SIDECAR_BINARY['launched_sha256']}"

    run_pass = lambda do |threads, target_core|
      mutex = Mutex.new
      inst = ArchivalistPuiConcurrencyGate.new_instrumentation(
        threads: threads,
        records_per_thread: DEFAULT_PUI_RECORDS_PER_THREAD
      )
      core_solr_url = "#{SOLR_BASE}/#{target_core}"
      before_reqs = (get_sidecar_metrics(SIDECAR_URL)['archivalist_sidecar_requests_total'] || 0).to_i
      puts "\n--- Running PUIIndexer Worker Batches: PUI_THREADS=#{threads} -> #{target_core} ---"
      started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      data_dir = nil

      ArchivalistPuiConcurrencyGate.with_pass_harness_state(
        app_config: AppConfig,
        builder_class: RustIndexDocumentBuilder,
        sidecar_url: SIDECAR_URL,
        threads: threads,
        default_pui_threads: DEFAULT_PUI_THREADS,
        target_solr_url: core_solr_url,
        index_state_base_dir: INDEX_STATE_BASE_DIR,
        run_id: GATE_RUN_ID,
        mutex: mutex,
        instrumentation: inst
      ) do |isolated_data_dir|
        data_dir = isolated_data_dir
        puts "  index_state_data_dir=#{data_dir} (preserved)"

        indexer = PUIIndexer.new(BACKEND_BASE, IndexState.new('indexer_pui_state'), "PUIIndexer-Gate-T#{threads}")
        indexer.instance_variable_set(:@solr_url, URI.parse(core_solr_url))
        indexer.login
        JSONModel.set_repository(REPO_ID)

        orig_fetch = indexer.method(:fetch_records)
        indexer.define_singleton_method(:fetch_records) do |type, ids, resolve|
          recs = orig_fetch.call(type, ids, resolve)
          digests = recs.map do |r|
            [r.uri, Digest::SHA256.hexdigest(ASUtils.to_json(r.to_hash(:trusted)))]
          end
          mutex.synchronize do
            inst['fetch_batches'] += 1
            inst['records_fetched'] += recs.length
            inst['fetch_thread_ids'] << Thread.current.object_id
            digests.each { |u, d| inst['snapshot_digests'][u] = d }
          end
          recs
        end

        orig_index_records = indexer.method(:index_records)
        indexer.define_singleton_method(:index_records) do |records, timing = IndexerTiming.new|
          mutex.synchronize do
            inst['index_batches'] += 1
            inst['index_thread_ids'] << Thread.current.object_id
          end
          begin
            orig_index_records.call(records, timing)
          ensure
            leaked_thread = !Thread.current[:archivalist_pui_projections].nil?
            leaked_ivars = %i[
              @pui_final_projections @pui_routing_projections @pui_notes_projections
              @pui_availability_projections @pui_candidate_projections @pui_common_projections
            ].any? { |iv| !instance_variable_get(iv).nil? }
            if leaked_thread || leaked_ivars
              mutex.synchronize { inst['thread_leak_violations'] += 1 }
            end
          end
        end

        orig_native_doc = indexer.method(:native_index_document)
        indexer.define_singleton_method(:native_index_document) do |uri, record_type, values|
          mutex.synchronize { inst['native_fallbacks'] += 1 }
          orig_native_doc.call(uri, record_type, values)
        end

        orig_routing = indexer.method(:apply_rust_pui_routing_projection)
        indexer.define_singleton_method(:apply_rust_pui_routing_projection) do |doc, record|
          res = orig_routing.call(doc, record)
          mutex.synchronize do
            if res
              inst['accepted_routing'] += 1
            else
              inst['rejections'] += 1
            end
          end
          res
        end

        orig_notes = indexer.method(:apply_rust_pui_notes_projection)
        indexer.define_singleton_method(:apply_rust_pui_notes_projection) do |doc, record|
          res = orig_notes.call(doc, record)
          mutex.synchronize do
            if res
              inst['accepted_notes'] += 1
            else
              inst['rejections'] += 1
            end
          end
          res
        end

        orig_final = indexer.method(:apply_rust_pui_final_projection)
        indexer.define_singleton_method(:apply_rust_pui_final_projection) do |doc, record|
          uri = record.is_a?(Hash) && record['record'].is_a?(Hash) ? record['record']['uri'] : nil
          cand_present = uri && pui_candidate_projections.is_a?(Hash) && pui_candidate_projections[uri].is_a?(Hash)
          res = orig_final.call(doc, record)
          mutex.synchronize do
            if res
              inst['accepted_final'] += 1
              inst['accepted_candidate'] += 1 if cand_present
            else
              inst['rejections'] += 1
            end
          end
          res
        end

        orig_helper = indexer.method(:apply_rust_pui_common_helper)
        indexer.define_singleton_method(:apply_rust_pui_common_helper) do |helper_name, doc, record, &block|
          res = orig_helper.call(helper_name, doc, record, &block)
          mutex.synchronize do
            if res
              inst['accepted_common_helpers'][helper_name] += 1
            else
              inst['rejections'] += 1
            end
          end
          res
        end

        ArchivalistPuiConcurrencyGate.run_worker_batches_for_repo(
          indexer,
          id_sets,
          threads: threads,
          records_per_thread: DEFAULT_PUI_RECORDS_PER_THREAD,
          mutex: mutex,
          instrumentation: inst
        )
      end

      wall_ms = ((Process.clock_gettime(Process::CLOCK_MONOTONIC) - started) * 1000.0).round(2)
      after_reqs = (get_sidecar_metrics(SIDECAR_URL)['archivalist_sidecar_requests_total'] || 0).to_i
      # after_reqs includes the trailing GET /metrics request itself
      sidecar_http_delta = after_reqs - before_reqs
      sidecar_index_delta = sidecar_http_delta - 1

      inst['accepted_base_docs'] = inst['builder_inputs_sent'] - inst['native_fallbacks']
      inst['fetch_thread_ids'] = inst['fetch_thread_ids'].uniq
      inst['index_thread_ids'] = inst['index_thread_ids'].uniq
      inst['builder_thread_ids'] = inst['builder_thread_ids'].uniq

      ArchivalistPuiConcurrencyGate.validate_acceptance!(
        inst,
        expected_records: total_expected_records,
        expected_batches: expected_batches,
        expected_threads: threads,
        sidecar_requests_delta: sidecar_index_delta
      )

      puts "  raw_wall_ms=#{wall_ms} (not a benchmark) sidecar_index_requests_delta=+#{sidecar_index_delta} (http_total_delta=+#{sidecar_http_delta})"
      puts "  worker_threads_observed: fetch=#{inst['fetch_thread_ids'].length} index=#{inst['index_thread_ids'].length} builder=#{inst['builder_thread_ids'].length}"
      puts "  accepted: base=#{inst['accepted_base_docs']} routing=#{inst['accepted_routing']} notes=#{inst['accepted_notes']} " \
           "final=#{inst['accepted_final']} candidate=#{inst['accepted_candidate']} rejections=#{inst['rejections']} fallbacks=#{inst['native_fallbacks']}"
      puts "  accepted_common_helpers: #{inst['accepted_common_helpers'].inspect}"

      {
        'threads' => threads,
        'core' => target_core,
        'index_state_data_dir' => data_dir,
        'raw_wall_ms' => wall_ms,
        'sidecar_index_requests_delta' => sidecar_index_delta,
        'sidecar_http_requests_delta' => sidecar_http_delta,
        'instrumentation' => inst
      }
    end

    pass_t1 = run_pass.call(1, CORE_PUI_T1)
    pass_t4 = run_pass.call(4, CORE_PUI_T4)

    puts "\n--- Verifying Source Snapshot Equivalence Across T1 and T4 ---"
    ArchivalistPuiConcurrencyGate.verify_snapshot_equivalence!(
      pass_t1['instrumentation']['snapshot_digests'],
      pass_t4['instrumentation']['snapshot_digests'],
      expected_records: total_expected_records
    )
    puts "  Snapshot equivalence: PASS (#{total_expected_records} identical record SHA-256 digests)"

    puts "\n--- Comparing All Stored Documents (Cursor-Paginated, Type-Strict) ---"
    parity = ArchivalistPuiConcurrencyGate.compare_cores_cursor_paginated!(
      SOLR_BASE,
      CORE_PUI_T1,
      CORE_PUI_T4,
      expected_count: total_expected_records
    )
    puts "  Parity: PASS (numFound=#{parity['num_found']}, compared=#{parity['compared_docs']}, mismatches=#{parity['mismatches']})"

    # Strip per-URI digest map from summary JSON to keep report compact while recording aggregate hash
    snapshot_aggregate_sha256 = Digest::SHA256.hexdigest(
      pass_t1['instrumentation']['snapshot_digests'].sort.map { |u, h| "#{u}:#{h}" }.join("\n")
    )
    pass_t1['instrumentation'].delete('snapshot_digests')
    pass_t4['instrumentation'].delete('snapshot_digests')

    report = {
      'gate_run_id' => GATE_RUN_ID,
      'timestamp' => Time.now.utc.iso8601,
      'runtime' => RUBY_DESCRIPTION,
      'profiling_stack' => STACK_IDENTITY,
      'sidecar_binary' => SIDECAR_BINARY,
      'source_provenance' => SOURCE_PROVENANCE,
      'repository' => {
        'id' => REPO_ID,
        'uri' => repo_record.uri,
        'repo_code' => repo_record.repo_code,
        'publish' => repo_record.publish,
        'pui_record_types' => pui_record_types.map(&:to_s),
        'present_counts' => present_counts,
        'absent_types' => absent_types,
        'total_records' => total_expected_records,
        'batches_per_pass' => expected_batches,
        'records_per_thread' => DEFAULT_PUI_RECORDS_PER_THREAD,
        'snapshot_aggregate_sha256' => snapshot_aggregate_sha256
      },
      'flags' => {
        'enabled' => ArchivalistPuiConcurrencyGate::ENABLED_RUST_FLAGS.map(&:to_s),
        'disabled' => ArchivalistPuiConcurrencyGate::DISABLED_RUST_FLAGS.map(&:to_s)
      },
      'passes' => {
        'pui_threads_1' => pass_t1,
        'pui_threads_4' => pass_t4
      },
      'preserved_index_state_dirs' => [
        pass_t1['index_state_data_dir'],
        pass_t4['index_state_data_dir']
      ],
      'parity' => parity,
      'notes' => [
        'Bounded live concurrency safety gate on repository 3 only; raw wall times are recorded for transparency and are NOT a benchmark or speedup claim.'
      ]
    }

    report_path = File.join(BENCHMARK_DIR, "pui_concurrency_gate_#{GATE_RUN_ID}.json")
    File.write(report_path, JSON.pretty_generate(report))
    puts "\nGate Report: #{report_path}"
  ensure
    if stage_state_file && File.exist?(stage_state_file)
      begin
        ArchivalistHarnessSafety.stop_stage_process(stage_state_file)
        puts "Stopped stage-owned sidecar (#{stage_state_file})"
      rescue StandardError => e
        warn "WARN: Failed stopping stage sidecar (#{stage_state_file}): #{e.class}: #{e.message}"
      end
    end
  end
end
