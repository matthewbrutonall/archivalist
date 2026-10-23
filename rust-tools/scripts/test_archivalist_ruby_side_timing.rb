#!/usr/bin/env ruby
# frozen_string_literal: true

# Deterministic offline tests for archivalist_ruby_side_timing.rb (fake classes only).
# Run with MRI or JRuby: ruby rust-tools/scripts/test_archivalist_ruby_side_timing.rb

require 'minitest/autorun'
require 'json'
require_relative 'archivalist_ruby_side_timing'

module FakeASUtils
  def self.to_json(obj, _opts = {})
    JSON.generate(obj)
  end

  def self.json_parse(str)
    JSON.parse(str)
  end
end

class FakeRecordInheritance
  def self.merge(json, opts = {})
    sleep 0.002
    json.merge('merged' => true, 'opts' => opts)
  end

  def self.build_plan(_json, _config = nil, _opts = {})
    { 'rules' => [] }
  end
end

class FakeTimingBuilder
  def build(inputs)
    request_body = FakeASUtils.to_json('records' => inputs)
    response = perform_request(request_body)
    docs = FakeASUtils.json_parse(response)['docs']
    docs.each { |doc| validate_thing!(doc) }
    docs
  end

  def perform_request(body)
    sleep 0.003
    body.sub('records', 'docs')
  end

  def validate_thing!(doc)
    FakeASUtils.to_json(doc)
    deep_copy_json_value(doc)
    sleep 0.001
  end

  # Recursive helper: must not be wrapped (counted inside validate_thing!).
  def deep_copy_json_value(val)
    val.is_a?(Hash) ? val.to_h { |k, v| [k, deep_copy_json_value(v)] } : val
  end

  def unrelated
    :ok
  end
end

class FakeIndexerCommon
  def rust_index_documents(records)
    copy = deep_copy_record_candidate(records)
    FakeRecordInheritance.merge({ 'a' => 1 }, remove_ancestors: true)
    sanitize_json(copy)
    FakeTimingBuilder.new.build(copy)
  end

  # Recursive like the real helper; must not be wrapped (its time belongs to the caller).
  def deep_copy_record_candidate(val)
    case val
    when Hash then val.to_h { |k, v| [k, deep_copy_record_candidate(v)] }
    when Array then val.map { |v| deep_copy_record_candidate(v) }
    else val
    end
  end

  def sanitize_json(json)
    json
  end
end

class FakePuiIndexer < FakeIndexerCommon
  def apply_rust_pui_final_projection(doc, _record)
    FakeASUtils.to_json(doc)
    true
  end
end

HOOKED = ArchivalistRubySideTiming.install!(
  builder_class: FakeTimingBuilder, indexer_common: FakeIndexerCommon, pui_indexer: FakePuiIndexer,
  record_inheritance: FakeRecordInheritance, asutils: FakeASUtils
)

# Mirrors the stage harness: build is re-defined via instance_method and timed as builder_build.
ORIGINAL_FAKE_TIMING_BUILD = FakeTimingBuilder.instance_method(:build)
FakeTimingBuilder.define_method(:build) do |inputs|
  ArchivalistRubySideTiming.measure('builder_build') { ORIGINAL_FAKE_TIMING_BUILD.bind(self).call(inputs) }
end

class TestArchivalistRubySideTiming < Minitest::Test
  def test_install_reports_hooked_methods
    assert_equal ['validate_thing!'], HOOKED['builder_validation']
    assert_equal ['perform_request'], HOOKED['sidecar_http_round_trip']
    assert_equal ['apply_rust_pui_final_projection'], HOOKED['pui_projection_apply']
    assert_equal ['merge'], HOOKED['record_inheritance_merge']
    refute HOOKED.key?('candidate_deep_copy'), 'recursive deep copy must not be wrapped'
    refute_nil ArchivalistRubySideTiming::CALIBRATION[:per_call_ms]
  end

  def test_inactive_records_nothing_and_preserves_results
    result = FakePuiIndexer.new.rust_index_documents([{ 'x' => 1 }])
    assert_equal [{ 'x' => 1 }], result
    assert_empty ArchivalistRubySideTiming::STATE[:totals]
    assert_equal({ 'a' => 1, 'merged' => true, 'opts' => { remove_ancestors: true } },
                 FakeRecordInheritance.merge({ 'a' => 1 }, remove_ancestors: true))
  end

  def test_exclusive_categories_sum_to_wall_time
    indexer = FakePuiIndexer.new
    ArchivalistRubySideTiming.start!
    started = ArchivalistRubySideTiming.now
    indexer.rust_index_documents([{ 'x' => 1 }, { 'y' => 2 }])
    indexer.apply_rust_pui_final_projection({ 'd' => 1 }, nil)
    FakeASUtils.to_json({ 'outside' => true })
    wall_ms = (ArchivalistRubySideTiming.now - started) * 1000.0
    raw = ArchivalistRubySideTiming.stop!
    summary = ArchivalistRubySideTiming.summarize(raw, wall_ms)
    cats = summary['category_ms']

    %w[rust_input_preparation record_inheritance_merge sanitize_json builder_build
       builder_request_json_encode sidecar_http_round_trip builder_json_decode builder_validation
       builder_validation_json_encode pui_projection_apply other_json_encode].each do |cat|
      assert cats.key?(cat), "missing #{cat}"
    end
    assert_equal 1, summary['category_calls']['builder_request_json_encode']
    assert_equal 2, summary['category_calls']['builder_validation'] # validate_thing! only, not its recursive helper
    assert_equal 1, summary['category_calls']['rust_input_preparation'] # deep copy recursion not timed
    assert_equal 2, summary['category_calls']['builder_validation_json_encode']
    assert_equal 2, summary['category_calls']['other_json_encode'] # apply_rust_pui + outside
    assert_operator cats['sidecar_http_round_trip'], :>=, 3.0
    assert_operator cats['record_inheritance_merge'], :>=, 2.0
    assert_operator cats['builder_validation'], :>=, 2.0
    # Exclusive accounting: sidecar time is not also counted in builder_build.
    assert_operator cats['builder_build'], :<, cats['sidecar_http_round_trip']
    assert_in_delta wall_ms, summary['attributed_ms'] + summary['unattributed_ms'], 0.01
    assert_operator summary['unattributed_ms'], :>=, 0.0
    assert_equal false, ArchivalistRubySideTiming::STATE[:active]
  end

  def test_median_summary_handles_missing_categories
    a = { 'category_ms' => { 'x' => 10.0 }, 'category_calls' => { 'x' => 1 }, 'attributed_ms' => 10.0, 'unattributed_ms' => 5.0, 'timer_calls' => 1, 'estimated_timer_overhead_ms' => 0.1 }
    b = { 'category_ms' => { 'x' => 30.0, 'y' => 4.0 }, 'category_calls' => { 'x' => 1, 'y' => 2 }, 'attributed_ms' => 34.0, 'unattributed_ms' => 7.0, 'timer_calls' => 3, 'estimated_timer_overhead_ms' => 0.3 }
    m = ArchivalistRubySideTiming.median_summary([a, b])
    assert_equal 20.0, m['category_ms']['x']
    assert_equal 2.0, m['category_ms']['y']
    assert_equal 22.0, m['attributed_ms']
    assert_nil ArchivalistRubySideTiming.median_summary([])
  end
end
