# frozen_string_literal: true

# Opt-in coarse Ruby-side timers for the stage harness (ARCHIVALIST_RUBY_SIDE_TIMING=1).
#
# Attributes the Ruby-side cost of the Rust document path to a few categories using EXCLUSIVE
# wall time (time in nested categories is subtracted from the enclosing one), so the categories
# never double count and their sum plus `unattributed_ms` equals the pass wall time:
#
#   rust_input_preparation        IndexerCommon#rust_index_documents itself: input building,
#                                 including the recursive deep_copy_record_candidate copies
#   sanitize_json                 IndexerCommon#sanitize_json (native and Rust paths)
#   record_inheritance_build_plan RecordInheritance.build_plan
#   record_inheritance_merge      RecordInheritance.merge (native merges and validation merges)
#   builder_build                 RustIndexDocumentBuilder#build / #build_pui_auxiliary inline
#                                 checks (timed from the harness's existing build wrapper)
#   builder_request_json_encode   ASUtils.to_json of the {'records' => ...} request body
#   builder_validation_json_encode other ASUtils.to_json calls made inside the builder
#   builder_json_decode           ASUtils.json_parse inside the builder (response decoding)
#   sidecar_http_round_trip       RustIndexDocumentBuilder#perform_request
#   builder_validation            the builder's validate_*/compute_expected_*/reconstruct_expected_*
#                                 methods (Ruby re-computation and checks), including the recursive
#                                 copy/strip/extract helpers they call
#   pui_projection_apply          PUIIndexer#apply_rust_pui_* and apply_rust_pui_availability_plan
#   other_json_encode / other_json_decode   ASUtils JSON calls outside the builder
#
# Everything else (backend/Solr HTTP, native document construction, batching) is
# `unattributed_ms`. Recursive helpers are deliberately NOT wrapped (one timer per JSON node
# cost seconds per pass); only their non-recursive callers are, so their time lands there. Hooks are prepended and only `build` is left alone, because the harness
# redefines it via instance_method; the harness times it with ArchivalistRubySideTiming.measure.
# Overhead is estimated from a calibration loop and reported, never subtracted.
#
# Thread-safe: each thread keeps its own stack and tallies (registered under a mutex and merged
# by stop!). With several threads the categories add up thread time, so their sum can exceed
# the wall time; summarize reports the number of threads seen.

module ArchivalistRubySideTiming
  STATE = { active: false, generation: 0, tallies: [] }
  LOCK = Mutex.new
  BUILDER_CONTEXTS = %w[builder_build].freeze
  BUILDER_VALIDATION_PREFIXES = %w[validate_ compute_expected_ reconstruct_expected_].freeze
  CALIBRATION = { per_call_ms: nil }

  def self.now
    Process.clock_gettime(Process::CLOCK_MONOTONIC)
  end

  def self.enabled?(env = ENV)
    %w[1 true yes on].include?(env['ARCHIVALIST_RUBY_SIDE_TIMING'].to_s.strip.downcase)
  end

  # This thread's tally for the current collection generation (created and registered on demand).
  def self.tally
    current = Thread.current[:archivalist_ruby_side_timing]
    return current if current && current[:generation] == STATE[:generation]

    fresh = { generation: STATE[:generation], stack: [], totals: Hash.new(0.0), counts: Hash.new(0) }
    LOCK.synchronize { STATE[:tallies] << fresh }
    Thread.current[:archivalist_ruby_side_timing] = fresh
  end

  def self.measure(category)
    return yield unless STATE[:active]

    current = tally
    stack = current[:stack]
    frame = [category, 0.0]
    stack.push(frame)
    started = now
    begin
      yield
    ensure
      elapsed = now - started
      stack.pop
      current[:totals][category] += elapsed - frame[1]
      current[:counts][category] += 1
      stack.last[1] += elapsed unless stack.empty?
    end
  end

  def self.inside?(categories)
    tally[:stack].any? { |frame| categories.include?(frame[0]) }
  end

  def self.inside_builder?
    inside?(BUILDER_CONTEXTS)
  end

  def self.wrap_methods(owner, names, category)
    names = names.select { |n| owner.method_defined?(n) || owner.private_method_defined?(n) }
    return [] if names.empty?

    hook = Module.new do
      names.each do |name|
        define_method(name) do |*args, **kwargs, &blk|
          ArchivalistRubySideTiming.measure(category) { super(*args, **kwargs, &blk) }
        end
      end
    end
    owner.prepend(hook)
    names
  end

  module ASUtilsHook
    def to_json(obj, *rest)
      return super unless ArchivalistRubySideTiming::STATE[:active]

      category = if ArchivalistRubySideTiming.inside_builder?
                   obj.is_a?(Hash) && obj.keys == ['records'] ? 'builder_request_json_encode' : 'builder_validation_json_encode'
                 else
                   'other_json_encode'
                 end
      ArchivalistRubySideTiming.measure(category) { super }
    end

    def json_parse(str)
      return super unless ArchivalistRubySideTiming::STATE[:active]

      category = ArchivalistRubySideTiming.inside_builder? ? 'builder_json_decode' : 'other_json_decode'
      ArchivalistRubySideTiming.measure(category) { super }
    end
  end

  # Installs the hooks; returns the list of hooked methods for the report.
  def self.install!(builder_class:, indexer_common:, pui_indexer:, record_inheritance:, asutils:)
    hooked = {}
    asutils.singleton_class.prepend(ASUtilsHook) unless asutils.singleton_class.ancestors.include?(ASUtilsHook)
    hooked['ASUtils'] = %w[to_json json_parse]

    validation = (builder_class.instance_methods(false) + builder_class.private_instance_methods(false)).map(&:to_s)
                 .select { |n| BUILDER_VALIDATION_PREFIXES.any? { |p| n.start_with?(p) } }
    hooked['builder_validation'] = wrap_methods(builder_class, validation, 'builder_validation')
    hooked['sidecar_http_round_trip'] = wrap_methods(builder_class, %w[perform_request], 'sidecar_http_round_trip')
    hooked['rust_input_preparation'] = wrap_methods(indexer_common, %w[rust_index_documents], 'rust_input_preparation')
    hooked['sanitize_json'] = wrap_methods(indexer_common, %w[sanitize_json], 'sanitize_json')
    hooked['pui_projection_apply'] =
      wrap_methods(pui_indexer, %w[apply_rust_pui_final_projection apply_rust_pui_routing_projection
                                   apply_rust_pui_notes_projection apply_rust_pui_common_helper], 'pui_projection_apply') +
      wrap_methods(indexer_common, %w[apply_rust_pui_availability_plan], 'pui_projection_apply')
    hooked['record_inheritance_merge'] = wrap_methods(record_inheritance.singleton_class, %w[merge], 'record_inheritance_merge')
    hooked['record_inheritance_build_plan'] = wrap_methods(record_inheritance.singleton_class, %w[build_plan], 'record_inheritance_build_plan')
    calibrate!
    hooked
  end

  # Solr requests from IndexerCommon#do_http_request, classified by the enclosing operation.
  module SolrHttpHook
    def do_http_request(url, req)
      return super unless ArchivalistRubySideTiming::STATE[:active]

      category = if ArchivalistRubySideTiming.inside?(%w[solr_commit])
                   'solr_commit_http'
                 elsif ArchivalistRubySideTiming.inside?(%w[solr_delete_records])
                   'solr_delete_http'
                 else
                   'solr_update_http'
                 end
      ArchivalistRubySideTiming.measure(category) { super }
    end
  end

  # Native indexing pipeline categories for full index rounds (see native_indexing_profile.rb):
  #
  #   index_round_main_thread   run_index_round on the coordinating thread, INCLUDING time spent
  #                             waiting for worker threads (queue offers and joins)
  #   backend_http              JSONModel::HTTP.do_http_request (all backend API calls)
  #   fetch_records             PeriodicIndexer/PUIIndexer#fetch_records excluding HTTP and JSON
  #                             decoding (JSONModel object construction, PUI direct-only merge
  #                             appears separately as record_inheritance_merge)
  #   document_preparation      IndexerCommon#index_records excluding nested categories
  #   batch_append              IndexBatch#<< (temp-file write; the JSON encode is other_json_encode)
  #   solr_index_batch          IndexerCommon#index_batch setup excluding its Solr HTTP
  #   solr_update_http / solr_commit_http / solr_delete_http   Solr requests by operation
  #   solr_commit / solr_delete_records                         request building around them
  #   handle_deletes            PeriodicIndexer#handle_deletes excluding nested categories
  #   pui_tree_round            PUIIndexer#index_round_complete (tree id discovery) exclusive
  #   pui_infscroll / large_tree  PUI ordered-records and LargeTree document generation
  def self.install_native_pipeline!(indexer_common:, periodic_indexer:, pui_indexer:, large_tree:, index_batch:, jsonmodel_http:)
    hooked = {}
    hooked['backend_http'] = wrap_methods(jsonmodel_http.singleton_class, %w[do_http_request], 'backend_http')
    indexer_common.prepend(SolrHttpHook) unless indexer_common.ancestors.include?(SolrHttpHook)
    hooked['solr_http'] = %w[do_http_request]
    hooked['solr_commit'] = wrap_methods(indexer_common, %w[send_commit], 'solr_commit')
    hooked['solr_delete_records'] = wrap_methods(indexer_common, %w[delete_records], 'solr_delete_records')
    hooked['solr_index_batch'] = wrap_methods(indexer_common, %w[index_batch], 'solr_index_batch')
    hooked['document_preparation'] = wrap_methods(indexer_common, %w[index_records], 'document_preparation')
    hooked['batch_append'] = wrap_methods(index_batch, %w[<<], 'batch_append')
    hooked['fetch_records'] = wrap_methods(periodic_indexer, %w[fetch_records], 'fetch_records') +
                              wrap_methods(pui_indexer, %w[fetch_records], 'fetch_records')
    hooked['index_round_main_thread'] = wrap_methods(periodic_indexer, %w[run_index_round], 'index_round_main_thread')
    hooked['handle_deletes'] = wrap_methods(periodic_indexer, %w[handle_deletes], 'handle_deletes')
    hooked['pui_tree_round'] = wrap_methods(pui_indexer, %w[index_round_complete], 'pui_tree_round')
    hooked['pui_infscroll'] = wrap_methods(pui_indexer, %w[add_infscroll_docs], 'pui_infscroll')
    hooked['large_tree'] = wrap_methods(large_tree, %w[add_largetree_docs], 'large_tree')
    hooked
  end

  # Estimates the cost of one measure call (two clock reads plus bookkeeping).
  def self.calibrate!(iterations = 20_000)
    start!
    started = now
    iterations.times { measure('calibration') { nil } }
    CALIBRATION[:per_call_ms] = (now - started) * 1000.0 / iterations
  ensure
    stop!
  end

  def self.start!
    LOCK.synchronize do
      STATE[:generation] += 1
      STATE[:tallies] = []
      STATE[:active] = true
    end
  end

  # Stops collection and returns { totals_ms, counts, threads } merged across threads.
  def self.stop!
    tallies = LOCK.synchronize do
      STATE[:active] = false
      STATE[:generation] += 1
      taken = STATE[:tallies]
      STATE[:tallies] = []
      taken
    end
    totals = Hash.new(0.0)
    counts = Hash.new(0)
    tallies.each do |t|
      t[:totals].each { |k, v| totals[k] += v }
      t[:counts].each { |k, v| counts[k] += v }
    end
    { 'totals_ms' => totals.transform_values { |v| v * 1000.0 }, 'counts' => counts, 'threads' => tallies.length }
  end

  def self.summarize(raw, pass_wall_ms)
    totals = raw['totals_ms'].transform_values { |v| v.round(3) }.sort_by { |k, v| [-v, k] }.to_h
    attributed = raw['totals_ms'].values.sum
    calls = raw['counts'].values.sum
    {
      'category_ms' => totals,
      'category_calls' => raw['counts'].sort.to_h,
      'attributed_ms' => attributed.round(3),
      'unattributed_ms' => (pass_wall_ms - attributed).round(3),
      'timer_calls' => calls,
      'threads' => raw.fetch('threads', 1),
      'estimated_timer_overhead_ms' => CALIBRATION[:per_call_ms] ? (calls * CALIBRATION[:per_call_ms]).round(3) : nil
    }
  end

  # Field-wise medians across passes; categories missing from a pass count as 0.
  def self.median_summary(summaries)
    return nil if summaries.empty?

    median = lambda do |values|
      sorted = values.sort
      mid = sorted.length / 2
      sorted.length.odd? ? sorted[mid] : ((sorted[mid - 1] + sorted[mid]) / 2.0).round(3)
    end
    categories = summaries.flat_map { |s| s['category_ms'].keys }.uniq
    category_ms = categories.to_h { |c| [c, median.call(summaries.map { |s| s['category_ms'].fetch(c, 0.0) })] }
    {
      'category_ms' => category_ms.sort_by { |k, v| [-v, k] }.to_h,
      'category_calls' => categories.to_h { |c| [c, median.call(summaries.map { |s| s['category_calls'].fetch(c, 0) })] },
      'attributed_ms' => median.call(summaries.map { |s| s['attributed_ms'] }),
      'unattributed_ms' => median.call(summaries.map { |s| s['unattributed_ms'] }),
      'timer_calls' => median.call(summaries.map { |s| s['timer_calls'] }),
      'estimated_timer_overhead_ms' => summaries.first['estimated_timer_overhead_ms'] && median.call(summaries.map { |s| s['estimated_timer_overhead_ms'] })
    }
  end
end
