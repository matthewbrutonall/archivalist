#!/usr/bin/env ruby
# frozen_string_literal: true

# Deterministic offline tests for archivalist_profiler.rb (fake clock, synthetic classes only).
# Run with MRI or JRuby: ruby rust-tools/scripts/test_archivalist_profiler.rb

require 'minitest/autorun'
require_relative 'archivalist_profiler'

class FakeClock
  attr_accessor :now

  def initialize
    @now = 0.0
  end

  def call
    @now
  end
end

class ArchivalistProfilerTest < Minitest::Test
  def setup
    @clock = FakeClock.new
    @agg = ArchivalistProfiler::Aggregator.new(clock: @clock)
    ArchivalistProfiler.aggregator = @agg
    ArchivalistProfiler.deactivate!
  end

  def teardown
    ArchivalistProfiler.deactivate!
  end

  def advance(seconds)
    @clock.now += seconds
  end

  def test_exclusive_time_subtracts_children_and_sums_to_region
    @agg.measure('root') do
      advance(0.010)
      @agg.measure('a') do
        advance(0.020)
        @agg.measure('b') { advance(0.030) }
        advance(0.005)
      end
      advance(0.001)
    end
    snap = @agg.snapshot
    assert_in_delta 66.0, snap['root']['inclusive_ms'], 0.001
    assert_in_delta 11.0, snap['root']['exclusive_ms'], 0.001
    assert_in_delta 55.0, snap['a']['inclusive_ms'], 0.001
    assert_in_delta 25.0, snap['a']['exclusive_ms'], 0.001
    assert_in_delta 30.0, snap['b']['exclusive_ms'], 0.001
    total_excl = snap.values.sum { |s| s['exclusive_ms'] }
    assert_in_delta snap['root']['inclusive_ms'], total_excl, 0.001
    assert_equal 0, @agg.open_frames
  end

  def test_recursive_label_inclusive_counted_once_exclusive_always
    @agg.measure('r') do
      advance(0.010)
      @agg.measure('r') do
        advance(0.020)
      end
    end
    s = @agg.snapshot['r']
    assert_equal 2, s['calls']
    assert_in_delta 30.0, s['inclusive_ms'], 0.001 # outer only, not 50
    assert_in_delta 30.0, s['exclusive_ms'], 0.001
  end

  def test_outermost_only_skips_nested_activation
    @agg.measure('o', outermost_only: true) do
      advance(0.010)
      @agg.measure('o', outermost_only: true) { advance(0.010) }
    end
    s = @agg.snapshot['o']
    assert_equal 1, s['calls']
    assert_in_delta 20.0, s['exclusive_ms'], 0.001
  end

  def test_exception_records_error_and_balances_stack
    assert_raises(RuntimeError) do
      @agg.measure('boom') do
        advance(0.004)
        raise 'x'
      end
    end
    s = @agg.snapshot['boom']
    assert_equal 1, s['errors']
    assert_in_delta 4.0, s['exclusive_ms'], 0.001
    assert_equal 0, @agg.open_frames
  end

  def test_snapshot_contains_only_aggregates
    @agg.measure('lab', bytes: 12) { advance(0.001) }
    snap = @agg.snapshot
    assert_equal %w[bytes calls errors exclusive_ms inclusive_ms], snap['lab'].keys.sort
    assert_equal 12, snap['lab']['bytes']
  end

  class Target
    attr_reader :seen

    def pub(a, b = 2, *rest, k: 0, &blk)
      @seen = [a, b, rest, k, blk ? blk.call : nil]
    end

    def fails
      raise ArgumentError, 'nope'
    end

    def self.cls(x)
      x * 2
    end

    private

    def priv(x)
      x + 1
    end

    protected

    def prot
      :p
    end
  end

  class OtherTarget
    def work(x)
      x
    end
  end

  def call_priv(t, x)
    t.send(:priv, x)
  end

  def test_wrapper_forwards_args_kwargs_block_and_return
    t = Target.new
    ArchivalistProfiler.wrap(Target, :pub, 'pub')
    ArchivalistProfiler.activate!
    ret = t.pub(1, 3, 4, 5, k: 9) { :blk }
    assert_equal [1, 3, [4, 5], 9, :blk], ret
    assert_equal 1, @agg.snapshot['pub']['calls']
  end

  def test_visibility_preserved
    ArchivalistProfiler.wrap(Target, :priv, 'priv')
    ArchivalistProfiler.wrap(Target, :prot, 'prot')
    t = Target.new
    assert_raises(NoMethodError) { t.priv(1) }
    assert_raises(NoMethodError) { t.prot }
    assert Target.private_method_defined?(:priv)
    assert Target.protected_method_defined?(:prot)
    ArchivalistProfiler.activate!
    assert_equal 2, call_priv(t, 1)
    assert_equal 1, @agg.snapshot['priv']['calls']
  end

  def test_singleton_wrap_and_bytes
    ArchivalistProfiler.wrap(Target, :cls, 'cls', singleton: true, bytes: ->(args, result) { args[0] + result })
    ArchivalistProfiler.activate!
    assert_equal 6, Target.cls(3)
    assert_equal 9, @agg.snapshot['cls']['bytes']
  end

  def test_exceptions_propagate_and_are_counted
    ArchivalistProfiler.wrap(Target, :fails, 'fails')
    ArchivalistProfiler.activate!
    assert_raises(ArgumentError) { Target.new.fails }
    assert_equal 1, @agg.snapshot['fails']['errors']
    assert_equal 0, @agg.open_frames
  end

  def test_inactive_is_pure_passthrough_and_records_nothing
    ArchivalistProfiler.wrap(OtherTarget, :work, 'work')
    assert_equal 5, OtherTarget.new.work(5)
    assert_empty @agg.snapshot
    ArchivalistProfiler.activate!
    OtherTarget.new.work(1)
    ArchivalistProfiler.uninstall_all!
    OtherTarget.new.work(2)
    assert_equal 1, @agg.snapshot['work']['calls']
    assert_equal 2, OtherTarget.new.work(2)
  end

  def test_wrap_undefined_method_raises
    assert_raises(ArgumentError) { ArchivalistProfiler.wrap(Target, :missing, 'm') }
  end

  def test_region_returns_result_and_summary_has_no_double_counting
    result, snap = ArchivalistProfiler.region('pass') do
      @agg.measure('x') { advance(0.010) }
      advance(0.005)
      :done
    end
    assert_equal :done, result
    refute ArchivalistProfiler.active?
    summary = ArchivalistProfiler.summarize(snap, 'pass', categories: { 'x' => 'cat' })
    assert_in_delta 15.0, summary['region_ms'], 0.001
    assert_in_delta 15.0, summary['exclusive_sum_ms'], 0.001
    assert_in_delta 10.0, summary['by_category_exclusive_ms']['cat'], 0.001
    assert_in_delta 5.0, summary['by_category_exclusive_ms']['unwrapped_remainder_in_region'], 0.001
  end
end
