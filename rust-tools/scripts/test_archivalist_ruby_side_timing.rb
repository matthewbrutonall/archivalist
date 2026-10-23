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

module FakeJsonModelHttp
  def self.do_http_request(_url, _req)
    sleep 0.002
    :backend_response
  end
end

class FakePipelineIndexer
  attr_reader :solr_calls

  def initialize
    @solr_calls = []
  end

  def do_http_request(url, _req)
    @solr_calls << url
    sleep 0.001
    :solr_response
  end

  def send_commit
    do_http_request('commit', nil)
  end

  def delete_records(records)
    records.each { |r| do_http_request("delete #{r}", nil) }
  end

  def index_batch(_batch)
    do_http_request('update', nil)
  end

  def index_records(records)
    FakeJsonModelHttp.do_http_request('fetch', nil)
    index_batch(records)
  end

  def run_index_round
    index_records([1, 2])
    send_commit
    handle_deletes
  end

  def handle_deletes
    delete_records(%w[a b])
  end

  def fetch_records(*); end
end

class FakePipelinePui < FakePipelineIndexer
  def index_round_complete(*); end
  def add_infscroll_docs(*); end
end

class FakeLargeTree
  def add_largetree_docs(*); end
end

class FakeIndexBatchForTiming
  def <<(_doc); end
end

PIPELINE_HOOKED = ArchivalistRubySideTiming.install_native_pipeline!(
  indexer_common: FakePipelineIndexer, periodic_indexer: FakePipelineIndexer, pui_indexer: FakePipelinePui,
  large_tree: FakeLargeTree, index_batch: FakeIndexBatchForTiming, jsonmodel_http: FakeJsonModelHttp
)

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
    assert_empty ArchivalistRubySideTiming::STATE[:tallies]
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

  def test_threads_keep_separate_stacks_and_tallies_merge
    ArchivalistRubySideTiming.start!
    threads = 3.times.map do
      Thread.new do
        ArchivalistRubySideTiming.measure('outer') do
          sleep 0.01
          ArchivalistRubySideTiming.measure('inner') { sleep 0.01 }
        end
      end
    end
    threads.each(&:join)
    raw = ArchivalistRubySideTiming.stop!
    assert_equal 3, raw['threads']
    assert_equal 3, raw['counts']['outer']
    assert_equal 3, raw['counts']['inner']
    # Exclusive per thread: each outer excludes its own inner (~10 ms each, ~30 ms summed).
    assert_operator raw['totals_ms']['outer'], :>=, 25.0
    assert_operator raw['totals_ms']['outer'], :<, 60.0
    assert_operator raw['totals_ms']['inner'], :>=, 25.0
    assert_equal false, ArchivalistRubySideTiming::STATE[:active]
  end

  def test_native_pipeline_classifies_backend_and_solr_requests
    assert_equal ['do_http_request'], PIPELINE_HOOKED['backend_http']
    assert_equal ['run_index_round'], PIPELINE_HOOKED['index_round_main_thread']
    indexer = FakePipelineIndexer.new
    ArchivalistRubySideTiming.start!
    indexer.run_index_round
    raw = ArchivalistRubySideTiming.stop!
    counts = raw['counts']
    assert_equal 1, counts['backend_http']
    assert_equal 1, counts['solr_update_http']
    assert_equal 1, counts['solr_commit_http']
    assert_equal 2, counts['solr_delete_http']
    assert_equal 1, counts['index_round_main_thread']
    assert_operator raw['totals_ms']['backend_http'], :>=, 2.0
    assert_equal ['update', 'commit', 'delete a', 'delete b'], indexer.solr_calls
  end

  def test_multi_thread_summary_separates_main_thread_from_workers
    ArchivalistRubySideTiming.start!
    started = ArchivalistRubySideTiming.now
    ArchivalistRubySideTiming.measure('coordinate') do
      workers = 2.times.map { Thread.new { ArchivalistRubySideTiming.measure('work') { sleep 0.03 } } }
      workers.each(&:join)
    end
    wall_ms = (ArchivalistRubySideTiming.now - started) * 1000.0
    summary = ArchivalistRubySideTiming.summarize(ArchivalistRubySideTiming.stop!, wall_ms)

    assert_equal 2, summary['worker_threads']
    assert_nil summary['unattributed_ms'], 'no single wall remainder when threads overlap'
    assert_equal 'summed thread time across threads', summary['category_ms_kind']
    assert_equal ['coordinate'], summary['main_thread_category_ms'].keys
    assert_equal ['work'], summary['worker_category_ms'].keys
    assert_operator summary['worker_summed_thread_ms'], :>=, 55.0 # ~2 x 30 ms
    assert_operator summary['main_thread_unattributed_ms'], :>=, 0.0
    assert_in_delta wall_ms, summary['main_thread_category_ms']['coordinate'] + summary['main_thread_unattributed_ms'], 0.01
    # The old single number would have gone negative: summed thread time exceeds wall time.
    assert_operator summary['attributed_ms'], :>, wall_ms

    med = ArchivalistRubySideTiming.median_summary([summary, summary])
    assert_nil med['unattributed_ms']
    assert_equal summary['main_thread_unattributed_ms'], med['main_thread_unattributed_ms']
    assert_equal 2, med['worker_threads']
  end

  def test_single_thread_summary_keeps_wall_unattributed
    ArchivalistRubySideTiming.start!
    ArchivalistRubySideTiming.measure('only') { sleep 0.005 }
    summary = ArchivalistRubySideTiming.summarize(ArchivalistRubySideTiming.stop!, 20.0)
    assert_equal 0, summary['worker_threads']
    refute_nil summary['unattributed_ms']
    refute summary.key?('main_thread_category_ms')
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
