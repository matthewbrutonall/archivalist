# frozen_string_literal: true

# Opt-in aggregate profiler for the real Ruby -> HTTP -> Rust -> validation -> Solr indexing path.
#
# Design:
# - Never loaded by production code. Only the stage harness requires it, and only the harness
#   installs wrappers (when ARCHIVALIST_PROFILE=1).
# - Aggregates only: per label call count, bytes, inclusive and EXCLUSIVE monotonic time.
#   No record content, URIs, queries or credentials are ever stored.
# - Exclusive time subtracts time spent in nested profiled calls, so summing exclusive times of
#   all labels inside a region plus the region's own exclusive time equals the region wall time
#   (no double counting). Inclusive time of a label counts only its outermost activation.
# - Wrappers are prepended anonymous modules. A prepended module cannot be removed from Ruby's
#   ancestor chain, so "restoration" = `uninstall_all!`, after which every wrapper is a pure
#   pass-through to `super` (no timing, no recording). Arguments, keyword arguments, blocks,
#   return values, exceptions and method visibility (public/protected/private) are forwarded
#   unchanged.
# - Labels not wrapped are NOT measured: their time lands in the exclusive time of the enclosing
#   region/label and is reported as "unattributed" rather than guessed.

require 'json'

module ArchivalistProfiler
  Stat = Struct.new(:calls, :bytes, :inclusive, :exclusive, :errors)

  # Category used when summarizing exclusive time. Anything not listed here is reported under
  # its own label name.
  DEFAULT_CATEGORIES = {
    'region' => 'region_unwrapped_remainder'
  }.freeze

  class Aggregator
    attr_reader :clock

    def initialize(clock: -> { Process.clock_gettime(Process::CLOCK_MONOTONIC) })
      @clock = clock
      @mutex = Mutex.new
      reset!
    end

    def reset!
      @mutex.synchronize do
        @stats = {}
        @stacks = {}
      end
    end

    # Returns a token if the activation is recorded, nil if skipped (nested re-entry of an
    # outermost-only label, or recording disabled by caller).
    def enter(label, outermost_only: false)
      stack = stack_for_thread
      return nil if outermost_only && stack.any? { |f| f[:label] == label }

      active_before = stack.count { |f| f[:label] == label }
      frame = { label: label, start: @clock.call, child: 0.0, nested_same: active_before.positive? }
      stack.push(frame)
      frame
    end

    def leave(frame, bytes: nil, error: false)
      return if frame.nil?

      stack = stack_for_thread
      finish = @clock.call
      popped = stack.pop
      raise 'profiler stack corrupted' unless popped.equal?(frame)

      elapsed = finish - frame[:start]
      exclusive = elapsed - frame[:child]
      exclusive = 0.0 if exclusive.negative?
      stack.last[:child] += elapsed unless stack.empty?

      @mutex.synchronize do
        stat = (@stats[frame[:label]] ||= Stat.new(0, 0, 0.0, 0.0, 0))
        stat.calls += 1
        stat.bytes += bytes.to_i
        stat.exclusive += exclusive
        stat.inclusive += elapsed unless frame[:nested_same]
        stat.errors += 1 if error
      end
    end

    def measure(label, outermost_only: false, bytes: nil)
      frame = enter(label, outermost_only: outermost_only)
      failed = true
      begin
        result = yield
        failed = false
        result
      ensure
        leave(frame, bytes: bytes.respond_to?(:call) ? nil : bytes, error: failed)
      end
    end

    def add_bytes(label, bytes)
      @mutex.synchronize do
        stat = (@stats[label] ||= Stat.new(0, 0, 0.0, 0.0, 0))
        stat.bytes += bytes.to_i
      end
    end

    # Milliseconds, deterministic key order (by label).
    def snapshot
      @mutex.synchronize do
        @stats.keys.sort.each_with_object({}) do |label, out|
          s = @stats[label]
          out[label] = {
            'calls' => s.calls,
            'bytes' => s.bytes,
            'inclusive_ms' => (s.inclusive * 1000.0).round(3),
            'exclusive_ms' => (s.exclusive * 1000.0).round(3),
            'errors' => s.errors
          }
        end
      end
    end

    def open_frames
      stack_for_thread.length
    end

    private

    def stack_for_thread
      @mutex.synchronize { @stacks[Thread.current.object_id] ||= [] }
    end
  end

  @aggregator = Aggregator.new
  @installed = []
  @active = false

  class << self
    attr_accessor :aggregator
    attr_reader :installed

    def active?
      @active
    end

    def activate!
      @active = true
    end

    def deactivate!
      @active = false
    end

    # Pure pass-through afterwards; the prepended modules remain but record nothing.
    def uninstall_all!
      deactivate!
    end

    # Run block with profiling active inside a root region label. Returns [block_result, snapshot].
    def region(label)
      aggregator.reset!
      activate!
      begin
        result = aggregator.measure(label) { yield }
      ensure
        deactivate!
      end
      [result, aggregator.snapshot]
    end

    # Wrap an instance (or singleton, when singleton: true) method of target.
    # bytes: optional lambda(args, result) -> Integer, evaluated after the call.
    def wrap(target, method_name, label, singleton: false, outermost_only: false, bytes: nil)
      owner = singleton ? target.singleton_class : target
      visibility = if owner.private_method_defined?(method_name, true)
                     :private
                   elsif owner.protected_method_defined?(method_name, true)
                     :protected
                   elsif owner.public_method_defined?(method_name, true)
                     :public
                   else
                     raise ArgumentError, "#{target}##{method_name} is not defined"
                   end

      profiler = self
      wrapper = Module.new
      wrapper.send(:define_method, method_name) do |*args, **kwargs, &block|
        unless profiler.active?
          next super(*args, **kwargs, &block)
        end

        agg = profiler.aggregator
        frame = agg.enter(label, outermost_only: outermost_only)
        failed = true
        result = nil
        begin
          result = super(*args, **kwargs, &block)
          failed = false
          result
        ensure
          measured = nil
          if bytes && !failed
            begin
              measured = bytes.call(args, result)
            rescue StandardError
              measured = nil
            end
          end
          agg.leave(frame, bytes: measured, error: failed)
        end
      end
      wrapper.send(visibility, method_name)
      owner.prepend(wrapper)
      @installed << { target: target.to_s, method: method_name.to_s, label: label, singleton: singleton, visibility: visibility }
      wrapper
    end

    # Summaries: exclusive time per label, per category, plus the region's remainder.
    def summarize(snapshot, root_label, categories: {})
      root = snapshot[root_label]
      return nil unless root

      by_category = Hash.new(0.0)
      snapshot.each do |label, stat|
        category = label == root_label ? 'unwrapped_remainder_in_region' : (categories[label] || label)
        by_category[category] += stat['exclusive_ms']
      end
      total = root['inclusive_ms']
      {
        'region_ms' => total,
        'exclusive_sum_ms' => by_category.values.sum.round(3),
        'by_category_exclusive_ms' => by_category.sort.to_h.transform_values { |v| v.round(3) },
        'by_category_share' => by_category.sort.to_h.transform_values { |v| total.positive? ? (v / total).round(4) : 0.0 }
      }
    end
  end
end
