#!/usr/bin/env ruby
# frozen_string_literal: true

require 'rubygems'
if defined?(JRUBY_VERSION) && File.directory?('/archivesspace/build/gems/jruby/3.1.0')
  ENV['GEM_HOME'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  ENV['GEM_PATH'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  Gem.paths = {
    'GEM_HOME' => '/archivesspace/build/gems/jruby/3.1.0',
    'GEM_PATH' => '/archivesspace/build/gems/jruby/3.1.0'
  }
end

require 'digest'
require 'fileutils'
require 'json'
require 'optparse'
require 'time'
require 'timeout'

require_relative 'staff_full_handoff_spike'

module Gate4IsolatedHandoffRound
  class Gate4NoChangeViolationError < StandardError; end
  class Gate4MutationViolationError < StandardError; end

  RUST_FLAGS = %i[
    rust_index_document_builder_enabled
    rust_index_document_enrichment_enabled
    rust_pui_availability_preparation_enabled
    rust_pui_document_preparation_enabled
    rust_pui_common_metadata_enabled
    rust_pui_record_inheritance_candidate_enabled
    rust_pui_auxiliary_documents_enabled
  ].freeze

  EXPECTED_DATA_DIRECTORY = '/archivalist-profiling-data'
  EXPECTED_SOLR_URL = 'http://archivalist-profiling-solr-844ef09c:8983/solr/archivesspace'
  EXPECTED_BACKEND_URL = 'http://localhost:4567'

  def self.expected_checkpoint_keys(repo_ids)
    keys = [['repositories', 'repositories'], ['_deletes', 'deletes']]
    repo_ids.each do |repo_id|
      IndexerCommonConfig.record_types.each do |type|
        keys << [repo_id, type]
      end
    end
    keys
  end

  def self.snapshot_state_path(path)
    unless File.exist?(path)
      return {
        'path' => path,
        'exists' => false,
        'entry_count' => 0,
        'files' => {},
        'tree_sha256' => nil
      }
    end

    files = {}
    Dir.glob(File.join(path, '**', '*'), File::FNM_DOTMATCH).sort.each do |entry|
      base = File.basename(entry)
      next if %w[. ..].include?(base)
      next unless File.file?(entry)

      rel = entry.sub(/\A#{Regexp.escape(path)}\/?/, '')
      files[rel] = Digest::SHA256.file(entry).hexdigest
    end
    canonical = files.sort.map { |rel, sha| "#{rel}:#{sha}" }.join("\n")
    {
      'path' => path,
      'exists' => true,
      'entry_count' => files.length,
      'files' => files,
      'tree_sha256' => Digest::SHA256.hexdigest(canonical)
    }
  end

  def self.verify_state_checkpoints!(state, state_dir_path:, repo_ids:, expected_epoch:)
    raise ArgumentError, "State directory does not exist: #{state_dir_path}" unless Dir.exist?(state_dir_path)

    expected_keys = expected_checkpoint_keys(repo_ids)
    entries = Dir.children(state_dir_path).sort
    tmp_files = entries.select { |e| e.end_with?('.tmp') }
    raise ArgumentError, "Unexpected .tmp files in #{state_dir_path}: #{tmp_files.inspect}" unless tmp_files.empty?

    expected_filenames = expected_keys.map { |scope, type| "#{scope}_#{type}.dat" }.sort
    if entries != expected_filenames
      missing = expected_filenames - entries
      extra = entries - expected_filenames
      raise ArgumentError,
            "Checkpoint file set mismatch in #{state_dir_path}: expected #{expected_filenames.length}, " \
            "got #{entries.length} (missing=#{missing.inspect}, extra=#{extra.inspect})"
    end

    file_values = {}
    expected_keys.each do |scope, type|
      api_val = state.get_last_mtime(scope, type)
      unless api_val == expected_epoch
        raise ArgumentError,
              "IndexState#get_last_mtime(#{scope.inspect}, #{type.inspect}) returned #{api_val.inspect}, " \
              "expected #{expected_epoch}"
      end

      filename = "#{scope}_#{type}.dat"
      full_path = File.join(state_dir_path, filename)
      raw_text = File.read(full_path).strip
      unless raw_text == expected_epoch.to_s
        raise ArgumentError,
              "Raw checkpoint file #{filename} contained #{raw_text.inspect}, expected #{expected_epoch.to_s.inspect}"
      end
      file_values[filename] = Integer(raw_text, 10)
    end

    {
      'state_dir_path' => state_dir_path,
      'file_count' => entries.length,
      'expected_file_count' => expected_keys.length,
      'expected_epoch' => expected_epoch,
      'all_match_expected_epoch' => true,
      'repositories_checkpoint' => file_values['repositories_repositories.dat'],
      'deletes_checkpoint' => file_values['_deletes_deletes.dat'],
      'per_repo_type_file_count' => entries.length - 2,
      'sample_checkpoints' => {
        'repositories_repositories.dat' => file_values['repositories_repositories.dat'],
        '_deletes_deletes.dat' => file_values['_deletes_deletes.dat'],
        '2_resource.dat' => file_values['2_resource.dat'],
        '2_archival_object.dat' => file_values['2_archival_object.dat'],
        '3_archival_object.dat' => file_values['3_archival_object.dat'],
        '4_archival_object.dat' => file_values['4_archival_object.dat'],
        '4_job.dat' => file_values['4_job.dat']
      },
      'all_files' => file_values
    }
  end

  def self.install_no_change_guard!(indexer)
    telemetry = {
      'index_records_calls' => 0,
      'index_records_nonempty_calls' => 0,
      'index_batch_calls' => 0,
      'index_batch_nonempty_calls' => 0,
      'delete_records_calls' => 0,
      'delete_records_nonempty_calls' => 0,
      'send_commit_calls' => 0,
      'solr_http_requests' => 0,
      'backend_login_requests' => 0
    }

    indexer.define_singleton_method(:gate4_guard_telemetry) { telemetry }

    indexer.define_singleton_method(:index_records) do |records, *args|
      telemetry['index_records_calls'] += 1
      count = records.respond_to?(:length) ? records.length : 0
      if count.positive?
        telemetry['index_records_nonempty_calls'] += 1
        raise Gate4NoChangeViolationError,
              "Aborting no-change handoff round: index_records called with #{count} record(s)"
      end
      raise Gate4NoChangeViolationError, 'Aborting no-change handoff round: unexpected index_records invocation'
    end

    indexer.define_singleton_method(:index_batch) do |batch, *args|
      telemetry['index_batch_calls'] += 1
      nonempty = batch.respond_to?(:empty?) ? !batch.empty? : true
      if nonempty
        telemetry['index_batch_nonempty_calls'] += 1
        raise Gate4NoChangeViolationError, 'Aborting no-change handoff round: non-empty index_batch invocation'
      end
      raise Gate4NoChangeViolationError, 'Aborting no-change handoff round: unexpected index_batch invocation'
    end

    indexer.define_singleton_method(:delete_records) do |records, *args|
      telemetry['delete_records_calls'] += 1
      count = records.respond_to?(:length) ? records.length : 0
      if count.positive?
        telemetry['delete_records_nonempty_calls'] += 1
        raise Gate4NoChangeViolationError,
              "Aborting no-change handoff round: delete_records called with #{count} deleted record URI(s)"
      end
      super(records, *args)
    end

    indexer.define_singleton_method(:send_commit) do |*args|
      telemetry['send_commit_calls'] += 1
      raise Gate4NoChangeViolationError, 'Aborting no-change handoff round: unexpected send_commit call'
    end

    indexer.define_singleton_method(:do_http_request) do |url, req|
      url_str = url.to_s
      req_path = req.respond_to?(:path) ? req.path.to_s : ''
      if url_str.include?('/solr') || req_path.include?('/update') || req_path.include?('/solr')
        telemetry['solr_http_requests'] += 1
        raise Gate4NoChangeViolationError,
              "Aborting no-change handoff round: unexpected Solr HTTP request (#{req.method} #{url_str} #{req_path})"
      end
      telemetry['backend_login_requests'] += 1
      super(url, req)
    end

    telemetry
  end

  def self.boot_aspace_jruby_environment!
    $LOAD_PATH.unshift('/archivesspace/common')
    $LOAD_PATH.unshift('/archivesspace/indexer')
    $LOAD_PATH.unshift('/archivesspace/indexer/app/lib')

    require 'active_support/core_ext/string/inflections'
    require 'config/config-distribution'
    AppConfig.reload

    unless AppConfig[:data_directory] == EXPECTED_DATA_DIRECTORY
      raise "Unexpected AppConfig[:data_directory]=#{AppConfig[:data_directory].inspect} " \
            "(expected #{EXPECTED_DATA_DIRECTORY.inspect})"
    end
    unless AppConfig[:solr_url] == EXPECTED_SOLR_URL
      raise "Unexpected AppConfig[:solr_url]=#{AppConfig[:solr_url].inspect} " \
            "(expected #{EXPECTED_SOLR_URL.inspect})"
    end
    unless AppConfig[:index_state_class] == 'IndexState'
      raise "Unexpected AppConfig[:index_state_class]=#{AppConfig[:index_state_class].inspect} (expected 'IndexState')"
    end

    s3_cfg = AppConfig.has_key?(:index_state_s3) ? AppConfig[:index_state_s3] : nil
    raise "Expected AppConfig[:index_state_s3] to be nil, got #{s3_cfg.inspect}" unless s3_cfg.nil?

    AppConfig[:backend_url] = EXPECTED_BACKEND_URL
    AppConfig[:search_user_secret] = ENV['SEARCH_USER_SECRET'] || 'devserver'
    RUST_FLAGS.each { |flag| AppConfig[flag] = false }

    require 'index_state'
    require 'asutils'
    require 'aspace_i18n'
    require 'ashttp'
    require 'record_inheritance'
    require 'rust_index_document_builder'
    require 'periodic_indexer'
    require 'pui_indexer'
    require 'large_tree_doc_indexer'
    require 'index_batch'

    extend JSONModel

    {
      'ruby_description' => RUBY_DESCRIPTION,
      'backend_url' => AppConfig[:backend_url],
      'solr_url' => AppConfig[:solr_url],
      'data_directory' => AppConfig[:data_directory],
      'index_state_class' => AppConfig[:index_state_class],
      'index_state_s3' => s3_cfg,
      'enable_indexer' => AppConfig[:enable_indexer],
      'pui_indexer_enabled' => AppConfig[:pui_indexer_enabled],
      'indexer_thread_count' => AppConfig[:indexer_thread_count].to_i,
      'indexer_records_per_thread' => AppConfig[:indexer_records_per_thread].to_i,
      'rust_flags' => RUST_FLAGS.to_h { |f| [f.to_s, AppConfig[f]] }
    }
  end

  def self.run_preflight!(modified_since:, expected_repo_ids:)
    runtime_cfg = boot_aspace_jruby_environment!

    default_staff_path = File.join(EXPECTED_DATA_DIRECTORY, 'indexer_state')
    default_pui_path = File.join(EXPECTED_DATA_DIRECTORY, 'indexer_pui_state')
    data_dir_entries = Dir.exist?(EXPECTED_DATA_DIRECTORY) ? Dir.children(EXPECTED_DATA_DIRECTORY).sort : []

    dummy_state = Object.new
    indexer = PeriodicIndexer.new(AppConfig[:backend_url], dummy_state, 'Gate4ReadOnlyPreflight', false)
    indexer.login

    repositories = JSONModel(:repository).all('resolve[]' => indexer.resolved_attributes)
    repo_summaries = repositories.map do |repo|
      mtime_epoch = Time.parse(repo['system_mtime']).to_i
      {
        'id' => repo.id.to_i,
        'uri' => repo.uri,
        'repo_code' => repo.repo_code,
        'system_mtime' => repo['system_mtime'],
        'system_mtime_epoch' => mtime_epoch,
        'updated_at_or_after_modified_since' => mtime_epoch >= modified_since
      }
    end

    actual_repo_ids = repo_summaries.map { |r| r['id'] }
    unless actual_repo_ids == expected_repo_ids
      raise "Repository ID mismatch: expected #{expected_repo_ids.inspect}, got #{actual_repo_ids.inspect}"
    end
    updated_repos = repo_summaries.select { |r| r['updated_at_or_after_modified_since'] }
    unless updated_repos.empty?
      raise "Preflight failed: repositories updated at/after #{modified_since}: #{updated_repos.inspect}"
    end

    deletes = JSONModel::HTTP.get_json(
      '/delete-feed',
      :modified_since => modified_since,
      :page => 1,
      :page_size => AppConfig[:indexer_records_per_thread].to_i
    )
    unless deletes.is_a?(Hash) && deletes['results'].is_a?(Array)
      raise "Preflight failed: malformed /delete-feed response: #{deletes.inspect}"
    end
    unless deletes['results'].empty?
      raise "Preflight failed: /delete-feed returned #{deletes['results'].length} deleted record(s) since #{modified_since}"
    end

    global_types = IndexerCommonConfig.global_types
    per_repo_feeds = {}
    total_feed_queries = 0
    nonempty_feeds = []

    expected_repo_ids.each_with_index do |repo_id, repo_idx|
      JSONModel.set_repository(repo_id)
      repo_type_counts = {}
      IndexerCommonConfig.record_types.each do |type|
        uri = JSONModel(type).uri_for
        ids = JSONModel::HTTP.get_json(uri, :all_ids => true, :modified_since => modified_since)
        unless ids.is_a?(Array)
          raise "Preflight failed: feed #{uri} (repo #{repo_id}, type #{type}) did not return an Array: #{ids.inspect}"
        end
        total_feed_queries += 1
        is_global = global_types.include?(type)
        queried_in_round = !(is_global && repo_idx.positive?)
        repo_type_counts[type.to_s] = {
          'uri' => uri,
          'count' => ids.length,
          'global_type' => is_global,
          'queried_by_periodic_indexer_in_this_repo' => queried_in_round
        }
        nonempty_feeds << { 'repo_id' => repo_id, 'type' => type.to_s, 'count' => ids.length } unless ids.empty?
      end
      per_repo_feeds[repo_id.to_s] = repo_type_counts
    end

    unless nonempty_feeds.empty?
      raise "Preflight failed: non-empty STAFF feeds since #{modified_since}: #{nonempty_feeds.inspect}"
    end

    {
      'mode' => 'preflight',
      'status' => 'ok',
      'modified_since' => modified_since,
      'runtime_config' => runtime_cfg,
      'data_directory_entries' => data_dir_entries,
      'default_staff_state_snapshot' => snapshot_state_path(default_staff_path),
      'default_pui_state_snapshot' => snapshot_state_path(default_pui_path),
      'repositories' => repo_summaries,
      'delete_feed' => {
        'modified_since' => modified_since,
        'results_count' => deletes['results'].length,
        'first_page' => deletes['first_page'],
        'last_page' => deletes['last_page']
      },
      'staff_type_feeds_checked' => total_feed_queries,
      'nonempty_feeds_count' => 0,
      'per_repo_type_feeds' => per_repo_feeds
    }
  end

  def self.run_apply_and_one_round!(state_dir_name:, epoch:, repo_ids:, promoted:, timeout_sec:)
    raise ArgumentError, 'promoted must be true' unless promoted == true
    unless state_dir_name =~ /\Aindexer_state_gate4_handoff_20261124_[0-9a-f]{8,32}\z/
      raise ArgumentError, "Invalid state_dir_name: #{state_dir_name.inspect}"
    end

    runtime_cfg = boot_aspace_jruby_environment!

    default_staff_path = File.join(EXPECTED_DATA_DIRECTORY, 'indexer_state')
    default_pui_path = File.join(EXPECTED_DATA_DIRECTORY, 'indexer_pui_state')
    handoff_state_path = File.join(EXPECTED_DATA_DIRECTORY, state_dir_name)

    if File.exist?(handoff_state_path)
      raise "Refusing to overwrite existing handoff state directory: #{handoff_state_path}"
    end

    staff_before = snapshot_state_path(default_staff_path)
    pui_before = snapshot_state_path(default_pui_path)

    # Pause IndexerCommon class state while applying handoff checkpoints, per Gate 4 protocol
    IndexerCommon.pause(900)
    paused_during_handoff = IndexerCommon.paused?
    raise 'IndexerCommon.pause did not mark IndexerCommon as paused' unless paused_during_handoff

    state = IndexState.new(state_dir_name)
    summary = {
      'committed' => true,
      'full_reindex_started_at_epoch_sec' => epoch,
      'repository_ids' => repo_ids
    }
    handoff_apply_result = StaffFullHandoffSpike.apply(summary, state: state, promoted: true)

    checkpoints_before_round = verify_state_checkpoints!(
      state,
      state_dir_path: handoff_state_path,
      repo_ids: repo_ids,
      expected_epoch: epoch
    )

    # Unpause IndexerCommon before calling run_index_round directly (and verify run_index_round executes)
    IndexerCommon.pause(-1)

    indexer = PeriodicIndexer.new(AppConfig[:backend_url], state, 'Gate4HandoffOneShotStaff', false)
    guard_telemetry = install_no_change_guard!(indexer)

    observed_http_get_calls = []
    jsonmodel_http_singleton = JSONModel::HTTP.singleton_class
    orig_get_json = JSONModel::HTTP.method(:get_json)
    jsonmodel_http_singleton.send(:define_method, :get_json) do |uri, params = {}|
      res = orig_get_json.call(uri, params)
      count = if res.is_a?(Array)
                res.length
              elsif res.is_a?(Hash) && res['results'].is_a?(Array)
                res['results'].length
              end
      observed_http_get_calls << {
        'uri' => uri.to_s,
        'params' => params.transform_keys(&:to_s),
        'result_count' => count
      }
      if count && count.positive?
        raise Gate4NoChangeViolationError,
              "Aborting no-change handoff round: feed #{uri} with params #{params.inspect} returned #{count} record(s)"
      end
      res
    end

    started_mono = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    begin
      Timeout.timeout(timeout_sec) do
        indexer.run_index_round
      end
    ensure
      jsonmodel_http_singleton.send(:define_method, :get_json, orig_get_json)
    end
    elapsed_sec = (Process.clock_gettime(Process::CLOCK_MONOTONIC) - started_mono).round(4)

    unless guard_telemetry['solr_http_requests'].zero? &&
           guard_telemetry['index_records_calls'].zero? &&
           guard_telemetry['index_batch_calls'].zero? &&
           guard_telemetry['send_commit_calls'].zero? &&
           guard_telemetry['delete_records_nonempty_calls'].zero?
      raise "Unexpected indexing/Solr activity recorded in guard telemetry: #{guard_telemetry.inspect}"
    end

    fresh_state_reader = IndexState.new(state_dir_name)
    checkpoints_after_round = verify_state_checkpoints!(
      fresh_state_reader,
      state_dir_path: handoff_state_path,
      repo_ids: repo_ids,
      expected_epoch: epoch
    )

    staff_after = snapshot_state_path(default_staff_path)
    pui_after = snapshot_state_path(default_pui_path)

    unless staff_before == staff_after
      raise "Default STAFF state path changed unexpectedly: before=#{staff_before.inspect}, after=#{staff_after.inspect}"
    end
    unless pui_before == pui_after
      raise "Default PUI state path changed unexpectedly: before=#{pui_before.inspect}, after=#{pui_after.inspect}"
    end

    {
      'mode' => 'apply-and-run',
      'status' => 'ok',
      'runtime_config' => runtime_cfg,
      'state_dir_name' => state_dir_name,
      'handoff_state_path' => handoff_state_path,
      'paused_during_checkpoint_write' => paused_during_handoff,
      'handoff_apply_result' => handoff_apply_result,
      'checkpoints_before_round' => checkpoints_before_round,
      'round_execution' => {
        'method' => 'PeriodicIndexer#run_index_round',
        'timeout_sec' => timeout_sec,
        'wall_elapsed_sec' => elapsed_sec,
        'guard_telemetry' => guard_telemetry,
        'observed_feed_get_json_calls_count' => observed_http_get_calls.length,
        'observed_feed_get_json_calls' => observed_http_get_calls
      },
      'checkpoints_after_round' => checkpoints_after_round,
      'default_staff_state_before' => staff_before,
      'default_staff_state_after' => staff_after,
      'default_staff_state_unchanged' => staff_before == staff_after,
      'default_pui_state_before' => pui_before,
      'default_pui_state_after' => pui_after,
      'default_pui_state_unchanged' => pui_before == pui_after
    }
  end

  def self.verify_state_checkpoints_after_mutation_round!(
    state,
    state_dir_path:,
    repo_ids:,
    expected_epoch:,
    expected_advanced_keys:,
    min_advanced_epoch:
  )
    raise ArgumentError, "State directory does not exist: #{state_dir_path}" unless Dir.exist?(state_dir_path)

    expected_keys = expected_checkpoint_keys(repo_ids)
    entries = Dir.children(state_dir_path).sort
    tmp_files = entries.select { |e| e.end_with?('.tmp') }
    raise ArgumentError, "Unexpected .tmp files in #{state_dir_path}: #{tmp_files.inspect}" unless tmp_files.empty?

    expected_filenames = expected_keys.map { |scope, type| "#{scope}_#{type}.dat" }.sort
    if entries != expected_filenames
      missing = expected_filenames - entries
      extra = entries - expected_filenames
      raise ArgumentError,
            "Checkpoint file set mismatch in #{state_dir_path}: expected #{expected_filenames.length}, " \
            "got #{entries.length} (missing=#{missing.inspect}, extra=#{extra.inspect})"
    end

    advanced_key_set = expected_advanced_keys.map { |scope, type| "#{scope}_#{type}.dat" }
    file_values = {}
    advanced_files = {}
    unchanged_files = {}

    expected_keys.each do |scope, type|
      filename = "#{scope}_#{type}.dat"
      full_path = File.join(state_dir_path, filename)
      raw_text = File.read(full_path).strip
      val = Integer(raw_text, 10)
      api_val = state.get_last_mtime(scope, type)
      unless api_val == val
        raise ArgumentError,
              "IndexState#get_last_mtime(#{scope.inspect}, #{type.inspect}) returned #{api_val.inspect}, " \
              "but file #{filename} contained #{val.inspect}"
      end
      file_values[filename] = val

      if advanced_key_set.include?(filename)
        unless val > expected_epoch && val >= min_advanced_epoch
          raise ArgumentError,
                "Expected checkpoint #{filename} to advance past #{expected_epoch} (>= #{min_advanced_epoch}), " \
                "got #{val}"
        end
        advanced_files[filename] = val
      else
        unless val == expected_epoch
          raise ArgumentError,
                "Expected untouched checkpoint #{filename} to remain at #{expected_epoch}, got #{val}"
        end
        unchanged_files[filename] = val
      end
    end

    {
      'state_dir_path' => state_dir_path,
      'file_count' => entries.length,
      'expected_file_count' => expected_keys.length,
      'baseline_epoch' => expected_epoch,
      'min_advanced_epoch' => min_advanced_epoch,
      'advanced_file_count' => advanced_files.length,
      'unchanged_file_count' => unchanged_files.length,
      'advanced_files' => advanced_files,
      'all_other_files_remained_at_baseline_epoch' => (unchanged_files.length == entries.length - advanced_files.length),
      'all_files' => file_values
    }
  end

  def self.install_expected_mutation_guard!(
    indexer,
    allowed_index_uris:,
    expected_deleted_uris:
  )
    allowed_set = allowed_index_uris.to_h { |u| [u.to_s, true] }
    expected_del_sorted = expected_deleted_uris.map(&:to_s).sort
    mutex = Mutex.new
    telemetry = {
      'index_records_calls' => 0,
      'index_records_nonempty_calls' => 0,
      'indexed_record_uris' => [],
      'index_batch_calls' => 0,
      'index_batch_nonempty_calls' => 0,
      'delete_records_calls' => 0,
      'delete_records_nonempty_calls' => 0,
      'deleted_record_uris' => [],
      'send_commit_calls' => 0,
      'solr_http_requests' => 0,
      'backend_http_requests' => 0
    }

    indexer.define_singleton_method(:gate4_mutation_telemetry) { telemetry }

    indexer.define_singleton_method(:index_records) do |records, *args|
      uris = Array(records).map { |r| r.is_a?(Hash) ? r['uri'].to_s : r.to_s }
      unexpected = uris.reject { |u| allowed_set.key?(u) }
      mutex.synchronize do
        telemetry['index_records_calls'] += 1
        telemetry['index_records_nonempty_calls'] += 1 unless uris.empty?
        telemetry['indexed_record_uris'].concat(uris)
      end
      if uris.empty?
        raise Gate4MutationViolationError, 'Aborting mutation handoff round: empty index_records call'
      end
      unless unexpected.empty?
        raise Gate4MutationViolationError,
              "Aborting mutation handoff round: unexpected record URI(s) in index_records: #{unexpected.inspect} " \
              "(allowed: #{allowed_set.keys.sort.inspect})"
      end
      super(records, *args)
    end

    indexer.define_singleton_method(:index_batch) do |batch, *args|
      nonempty = batch.respond_to?(:empty?) ? !batch.empty? : true
      mutex.synchronize do
        telemetry['index_batch_calls'] += 1
        telemetry['index_batch_nonempty_calls'] += 1 if nonempty
      end
      super(batch, *args)
    end

    indexer.define_singleton_method(:delete_records) do |records, *args|
      uris = Array(records).map(&:to_s)
      mutex.synchronize do
        telemetry['delete_records_calls'] += 1
        if uris.any?
          telemetry['delete_records_nonempty_calls'] += 1
          telemetry['deleted_record_uris'].concat(uris)
        end
      end
      if uris.any? && uris.sort != expected_del_sorted
        raise Gate4MutationViolationError,
              "Aborting mutation handoff round: unexpected delete_records URI(s) #{uris.inspect} " \
              "(expected #{expected_del_sorted.inspect})"
      end
      super(records, *args)
    end

    indexer.define_singleton_method(:send_commit) do |*args|
      mutex.synchronize { telemetry['send_commit_calls'] += 1 }
      super(*args)
    end

    indexer.define_singleton_method(:do_http_request) do |url, req|
      url_str = url.to_s
      req_path = req.respond_to?(:path) ? req.path.to_s : ''
      mutex.synchronize do
        if url_str.include?('/solr') || req_path.include?('/update') || req_path.include?('/solr')
          telemetry['solr_http_requests'] += 1
        else
          telemetry['backend_http_requests'] += 1
        end
      end
      super(url, req)
    end

    telemetry
  end

  def self.run_preflight_expected_mutations!(
    modified_since:,
    expected_repo_ids:,
    target_repo_id:,
    expected_updated_ao_ids:,
    expected_updated_resource_ids:,
    expected_deleted_uris:
  )
    runtime_cfg = boot_aspace_jruby_environment!

    default_staff_path = File.join(EXPECTED_DATA_DIRECTORY, 'indexer_state')
    default_pui_path = File.join(EXPECTED_DATA_DIRECTORY, 'indexer_pui_state')
    data_dir_entries = Dir.exist?(EXPECTED_DATA_DIRECTORY) ? Dir.children(EXPECTED_DATA_DIRECTORY).sort : []

    dummy_state = Object.new
    indexer = PeriodicIndexer.new(AppConfig[:backend_url], dummy_state, 'Gate4MutationReadOnlyPreflight', false)
    indexer.login

    repositories = JSONModel(:repository).all('resolve[]' => indexer.resolved_attributes)
    repo_summaries = repositories.map do |repo|
      mtime_epoch = Time.parse(repo['system_mtime']).to_i
      {
        'id' => repo.id.to_i,
        'uri' => repo.uri,
        'repo_code' => repo.repo_code,
        'system_mtime' => repo['system_mtime'],
        'system_mtime_epoch' => mtime_epoch,
        'updated_at_or_after_modified_since' => mtime_epoch >= modified_since
      }
    end

    actual_repo_ids = repo_summaries.map { |r| r['id'] }
    unless actual_repo_ids == expected_repo_ids
      raise "Repository ID mismatch: expected #{expected_repo_ids.inspect}, got #{actual_repo_ids.inspect}"
    end
    updated_repos = repo_summaries.select { |r| r['updated_at_or_after_modified_since'] }
    unless updated_repos.empty?
      raise "Preflight failed: repositories updated at/after #{modified_since}: #{updated_repos.inspect}"
    end

    deletes = JSONModel::HTTP.get_json(
      '/delete-feed',
      :modified_since => modified_since,
      :page => 1,
      :page_size => AppConfig[:indexer_records_per_thread].to_i
    )
    unless deletes.is_a?(Hash) && deletes['results'].is_a?(Array)
      raise "Preflight failed: malformed /delete-feed response: #{deletes.inspect}"
    end
    actual_deleted_uris = deletes['results'].map(&:to_s).sort
    expected_deleted_sorted = expected_deleted_uris.map(&:to_s).sort
    unless actual_deleted_uris == expected_deleted_sorted
      raise "Preflight failed: /delete-feed returned #{actual_deleted_uris.inspect}, " \
            "expected #{expected_deleted_sorted.inspect}"
    end

    global_types = IndexerCommonConfig.global_types
    per_repo_feeds = {}
    total_feed_queries = 0
    nonempty_feeds = []

    expected_ao_sorted = expected_updated_ao_ids.map(&:to_i).sort
    expected_res_sorted = expected_updated_resource_ids.map(&:to_i).sort

    expected_repo_ids.each_with_index do |repo_id, repo_idx|
      JSONModel.set_repository(repo_id)
      repo_type_counts = {}
      IndexerCommonConfig.record_types.each do |type|
        uri = JSONModel(type).uri_for
        ids = JSONModel::HTTP.get_json(uri, :all_ids => true, :modified_since => modified_since)
        unless ids.is_a?(Array)
          raise "Preflight failed: feed #{uri} (repo #{repo_id}, type #{type}) did not return an Array: #{ids.inspect}"
        end
        sorted_ids = ids.map(&:to_i).sort
        total_feed_queries += 1
        is_global = global_types.include?(type)
        queried_in_round = !(is_global && repo_idx.positive?)
        repo_type_counts[type.to_s] = {
          'uri' => uri,
          'count' => sorted_ids.length,
          'ids' => sorted_ids,
          'global_type' => is_global,
          'queried_by_periodic_indexer_in_this_repo' => queried_in_round
        }
        unless sorted_ids.empty?
          nonempty_feeds << {
            'repo_id' => repo_id,
            'type' => type.to_s,
            'count' => sorted_ids.length,
            'ids' => sorted_ids
          }
        end

        if repo_id == target_repo_id && type == :archival_object
          unless sorted_ids == expected_ao_sorted
            raise "Preflight failed: repo #{repo_id} archival_object feed returned #{sorted_ids.inspect}, " \
                  "expected #{expected_ao_sorted.inspect}"
          end
        elsif repo_id == target_repo_id && type == :resource
          unless sorted_ids == expected_res_sorted
            raise "Preflight failed: repo #{repo_id} resource feed returned #{sorted_ids.inspect}, " \
                  "expected #{expected_res_sorted.inspect}"
          end
        elsif !sorted_ids.empty?
          raise "Preflight failed: unexpected non-empty feed in repo #{repo_id} type #{type}: #{sorted_ids.inspect}"
        end
      end
      per_repo_feeds[repo_id.to_s] = repo_type_counts
    end

    {
      'mode' => 'preflight-expected-mutations',
      'status' => 'ok',
      'modified_since' => modified_since,
      'runtime_config' => runtime_cfg,
      'data_directory_entries' => data_dir_entries,
      'default_staff_state_snapshot' => snapshot_state_path(default_staff_path),
      'default_pui_state_snapshot' => snapshot_state_path(default_pui_path),
      'repositories' => repo_summaries,
      'delete_feed' => {
        'modified_since' => modified_since,
        'results_count' => actual_deleted_uris.length,
        'results' => actual_deleted_uris,
        'first_page' => deletes['first_page'],
        'last_page' => deletes['last_page']
      },
      'staff_type_feeds_checked' => total_feed_queries,
      'nonempty_feeds_count' => nonempty_feeds.length,
      'nonempty_feeds' => nonempty_feeds,
      'per_repo_type_feeds' => per_repo_feeds
    }
  end

  def self.run_apply_and_one_mutation_round!(
    state_dir_name:,
    epoch:,
    repo_ids:,
    promoted:,
    timeout_sec:,
    target_repo_id:,
    expected_updated_ao_ids:,
    expected_updated_resource_ids:,
    expected_deleted_uris:
  )
    raise ArgumentError, 'promoted must be true' unless promoted == true
    unless state_dir_name =~ /\Aindexer_state_gate4_(?:mutation_)?handoff_20261124_[0-9a-f]{8,32}\z/
      raise ArgumentError, "Invalid state_dir_name: #{state_dir_name.inspect}"
    end

    runtime_cfg = boot_aspace_jruby_environment!

    default_staff_path = File.join(EXPECTED_DATA_DIRECTORY, 'indexer_state')
    default_pui_path = File.join(EXPECTED_DATA_DIRECTORY, 'indexer_pui_state')
    handoff_state_path = File.join(EXPECTED_DATA_DIRECTORY, state_dir_name)

    if File.exist?(handoff_state_path)
      raise "Refusing to overwrite existing handoff state directory: #{handoff_state_path}"
    end

    staff_before = snapshot_state_path(default_staff_path)
    pui_before = snapshot_state_path(default_pui_path)

    IndexerCommon.pause(900)
    paused_during_handoff = IndexerCommon.paused?
    raise 'IndexerCommon.pause did not mark IndexerCommon as paused' unless paused_during_handoff

    state = IndexState.new(state_dir_name)
    summary = {
      'committed' => true,
      'full_reindex_started_at_epoch_sec' => epoch,
      'repository_ids' => repo_ids
    }
    handoff_apply_result = StaffFullHandoffSpike.apply(summary, state: state, promoted: true)

    checkpoints_before_round = verify_state_checkpoints!(
      state,
      state_dir_path: handoff_state_path,
      repo_ids: repo_ids,
      expected_epoch: epoch
    )

    IndexerCommon.pause(-1)

    allowed_ao_uris = expected_updated_ao_ids.map { |id| "/repositories/#{target_repo_id}/archival_objects/#{id.to_i}" }
    allowed_res_uris = expected_updated_resource_ids.map { |id| "/repositories/#{target_repo_id}/resources/#{id.to_i}" }
    allowed_index_uris = (allowed_ao_uris + allowed_res_uris).sort
    expected_deleted_sorted = expected_deleted_uris.map(&:to_s).sort

    indexer = PeriodicIndexer.new(AppConfig[:backend_url], state, 'Gate4MutationHandoffOneShotStaff', false)
    guard_telemetry = install_expected_mutation_guard!(
      indexer,
      allowed_index_uris: allowed_index_uris,
      expected_deleted_uris: expected_deleted_sorted
    )

    observed_http_get_calls = []
    get_mutex = Mutex.new
    jsonmodel_http_singleton = JSONModel::HTTP.singleton_class
    orig_get_json = JSONModel::HTTP.method(:get_json)
    jsonmodel_http_singleton.send(:define_method, :get_json) do |uri, params = {}|
      res = orig_get_json.call(uri, params)
      count = if res.is_a?(Array)
                res.length
              elsif res.is_a?(Hash) && res['results'].is_a?(Array)
                res['results'].length
              end
      get_mutex.synchronize do
        observed_http_get_calls << {
          'uri' => uri.to_s,
          'params' => params.transform_keys(&:to_s),
          'result_count' => count
        }
      end
      res
    end

    round_start_epoch = Time.now.to_i
    started_mono = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    begin
      Timeout.timeout(timeout_sec) do
        indexer.run_index_round
      end
    ensure
      jsonmodel_http_singleton.send(:define_method, :get_json, orig_get_json)
    end
    elapsed_sec = (Process.clock_gettime(Process::CLOCK_MONOTONIC) - started_mono).round(4)

    actual_indexed_sorted = guard_telemetry['indexed_record_uris'].sort
    unless actual_indexed_sorted == allowed_index_uris
      raise Gate4MutationViolationError,
            "Indexed record URIs mismatch: expected #{allowed_index_uris.inspect}, got #{actual_indexed_sorted.inspect}"
    end

    actual_deleted_sorted = guard_telemetry['deleted_record_uris'].sort
    unless actual_deleted_sorted == expected_deleted_sorted
      raise Gate4MutationViolationError,
            "Deleted record URIs mismatch: expected #{expected_deleted_sorted.inspect}, got #{actual_deleted_sorted.inspect}"
    end

    expected_advanced_keys = [['_deletes', 'deletes'], [target_repo_id, :archival_object]]
    expected_advanced_keys << [target_repo_id, :resource] unless expected_updated_resource_ids.empty?

    fresh_state_reader = IndexState.new(state_dir_name)
    checkpoints_after_round = verify_state_checkpoints_after_mutation_round!(
      fresh_state_reader,
      state_dir_path: handoff_state_path,
      repo_ids: repo_ids,
      expected_epoch: epoch,
      expected_advanced_keys: expected_advanced_keys,
      min_advanced_epoch: round_start_epoch
    )

    staff_after = snapshot_state_path(default_staff_path)
    pui_after = snapshot_state_path(default_pui_path)

    unless staff_before == staff_after
      raise "Default STAFF state path changed unexpectedly: before=#{staff_before.inspect}, after=#{staff_after.inspect}"
    end
    unless pui_before == pui_after
      raise "Default PUI state path changed unexpectedly: before=#{pui_before.inspect}, after=#{pui_after.inspect}"
    end

    {
      'mode' => 'apply-and-run-expected-mutations',
      'status' => 'ok',
      'runtime_config' => runtime_cfg,
      'state_dir_name' => state_dir_name,
      'handoff_state_path' => handoff_state_path,
      'paused_during_checkpoint_write' => paused_during_handoff,
      'handoff_apply_result' => handoff_apply_result,
      'checkpoints_before_round' => checkpoints_before_round,
      'round_execution' => {
        'method' => 'PeriodicIndexer#run_index_round',
        'timeout_sec' => timeout_sec,
        'wall_elapsed_sec' => elapsed_sec,
        'round_start_epoch' => round_start_epoch,
        'allowed_index_uris' => allowed_index_uris,
        'expected_deleted_uris' => expected_deleted_sorted,
        'guard_telemetry' => guard_telemetry,
        'observed_feed_get_json_calls_count' => observed_http_get_calls.length,
        'observed_feed_get_json_calls' => observed_http_get_calls
      },
      'checkpoints_after_round' => checkpoints_after_round,
      'default_staff_state_before' => staff_before,
      'default_staff_state_after' => staff_after,
      'default_staff_state_unchanged' => staff_before == staff_after,
      'default_pui_state_before' => pui_before,
      'default_pui_state_after' => pui_after,
      'default_pui_state_unchanged' => pui_before == pui_after
    }
  end
end

if __FILE__ == $PROGRAM_NAME
  options = {
    mode: nil,
    modified_since: nil,
    state_dir_name: nil,
    epoch: nil,
    repo_ids: [2, 3, 4],
    target_repo_id: 3,
    expected_updated_ao_ids: [],
    expected_updated_resource_ids: [],
    expected_deleted_uris: [],
    promoted: false,
    timeout_sec: 120
  }

  OptionParser.new do |opts|
    opts.on('--mode MODE', String) { |v| options[:mode] = v }
    opts.on('--modified-since SEC', Integer) { |v| options[:modified_since] = v }
    opts.on('--state-dir-name NAME', String) { |v| options[:state_dir_name] = v }
    opts.on('--epoch SEC', Integer) { |v| options[:epoch] = v }
    opts.on('--repo-ids IDS', String) { |v| options[:repo_ids] = v.split(',').map { |s| Integer(s.strip, 10) } }
    opts.on('--target-repo-id ID', Integer) { |v| options[:target_repo_id] = v }
    opts.on('--expected-updated-ao-ids IDS', String) do |v|
      options[:expected_updated_ao_ids] = v.empty? ? [] : v.split(',').map { |s| Integer(s.strip, 10) }
    end
    opts.on('--expected-updated-resource-ids IDS', String) do |v|
      options[:expected_updated_resource_ids] = v.empty? ? [] : v.split(',').map { |s| Integer(s.strip, 10) }
    end
    opts.on('--expected-deleted-uris URIS', String) do |v|
      options[:expected_deleted_uris] = v.empty? ? [] : v.split(',').map(&:strip)
    end
    opts.on('--promoted BOOL', String) { |v| options[:promoted] = (v == 'true') }
    opts.on('--timeout-sec SEC', Integer) { |v| options[:timeout_sec] = v }
  end.parse!

  case options[:mode]
  when 'preflight'
    raise ArgumentError, '--modified-since is required for preflight' unless options[:modified_since]
    result = Gate4IsolatedHandoffRound.run_preflight!(
      modified_since: options[:modified_since],
      expected_repo_ids: options[:repo_ids]
    )
    puts JSON.generate(result)
  when 'apply-and-run'
    raise ArgumentError, '--state-dir-name is required' unless options[:state_dir_name]
    raise ArgumentError, '--epoch is required' unless options[:epoch]
    result = Gate4IsolatedHandoffRound.run_apply_and_one_round!(
      state_dir_name: options[:state_dir_name],
      epoch: options[:epoch],
      repo_ids: options[:repo_ids],
      promoted: options[:promoted],
      timeout_sec: options[:timeout_sec]
    )
    puts JSON.generate(result)
  when 'preflight-expected-mutations'
    raise ArgumentError, '--modified-since is required for preflight-expected-mutations' unless options[:modified_since]
    result = Gate4IsolatedHandoffRound.run_preflight_expected_mutations!(
      modified_since: options[:modified_since],
      expected_repo_ids: options[:repo_ids],
      target_repo_id: options[:target_repo_id],
      expected_updated_ao_ids: options[:expected_updated_ao_ids],
      expected_updated_resource_ids: options[:expected_updated_resource_ids],
      expected_deleted_uris: options[:expected_deleted_uris]
    )
    puts JSON.generate(result)
  when 'apply-and-run-expected-mutations'
    raise ArgumentError, '--state-dir-name is required' unless options[:state_dir_name]
    raise ArgumentError, '--epoch is required' unless options[:epoch]
    result = Gate4IsolatedHandoffRound.run_apply_and_one_mutation_round!(
      state_dir_name: options[:state_dir_name],
      epoch: options[:epoch],
      repo_ids: options[:repo_ids],
      promoted: options[:promoted],
      timeout_sec: options[:timeout_sec],
      target_repo_id: options[:target_repo_id],
      expected_updated_ao_ids: options[:expected_updated_ao_ids],
      expected_updated_resource_ids: options[:expected_updated_resource_ids],
      expected_deleted_uris: options[:expected_deleted_uris]
    )
    puts JSON.generate(result)
  else
    raise ArgumentError,
          "Unsupported --mode #{options[:mode].inspect} " \
          "(expected 'preflight', 'apply-and-run', 'preflight-expected-mutations', or 'apply-and-run-expected-mutations')"
  end
end
