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

module ArchivalistRubySideTiming
  STATE = { active: false, stack: [], totals: Hash.new(0.0), counts: Hash.new(0) }
  BUILDER_CONTEXTS = %w[builder_build].freeze
  BUILDER_VALIDATION_PREFIXES = %w[validate_ compute_expected_ reconstruct_expected_].freeze
  CALIBRATION = { per_call_ms: nil }

  def self.now
    Process.clock_gettime(Process::CLOCK_MONOTONIC)
  end

  def self.enabled?(env = ENV)
    %w[1 true yes on].include?(env['ARCHIVALIST_RUBY_SIDE_TIMING'].to_s.strip.downcase)
  end

  def self.measure(category)
    return yield unless STATE[:active]

    stack = STATE[:stack]
    frame = [category, 0.0]
    stack.push(frame)
    started = now
    begin
      yield
    ensure
      elapsed = now - started
      stack.pop
      STATE[:totals][category] += elapsed - frame[1]
      STATE[:counts][category] += 1
      stack.last[1] += elapsed unless stack.empty?
    end
  end

  def self.inside_builder?
    STATE[:stack].any? { |frame| BUILDER_CONTEXTS.include?(frame[0]) }
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

  # Estimates the cost of one measure call (two clock reads plus bookkeeping).
  def self.calibrate!(iterations = 20_000)
    saved = { active: STATE[:active], stack: STATE[:stack], totals: STATE[:totals], counts: STATE[:counts] }
    STATE.merge!(active: true, stack: [], totals: Hash.new(0.0), counts: Hash.new(0))
    started = now
    iterations.times { measure('calibration') { nil } }
    CALIBRATION[:per_call_ms] = (now - started) * 1000.0 / iterations
  ensure
    STATE.merge!(saved)
  end

  def self.start!
    STATE.merge!(active: true, stack: [], totals: Hash.new(0.0), counts: Hash.new(0))
  end

  # Stops collection and returns { totals_ms, counts } for the pass.
  def self.stop!
    result = { 'totals_ms' => STATE[:totals].transform_values { |s| s * 1000.0 }, 'counts' => STATE[:counts].dup }
    STATE.merge!(active: false, stack: [], totals: Hash.new(0.0), counts: Hash.new(0))
    result
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
