#!/usr/bin/env ruby
# frozen_string_literal: true

# Deterministic offline tests for archivalist_payload_accounting.rb (synthetic bodies only).
# Run with MRI or JRuby: ruby rust-tools/scripts/test_archivalist_payload_accounting.rb

require 'minitest/autorun'
require_relative 'archivalist_payload_accounting'

class FakeRequest
  attr_reader :body

  def initialize(body)
    @body = body
  end
end

FakeResponse = Struct.new(:body)

class FakeBuilder
  attr_accessor :responses

  def build(inputs)
    perform_request(FakeRequest.new(JSON.generate('records' => inputs)))
  end

  def build_pui_auxiliary(inputs)
    perform_request(FakeRequest.new(JSON.generate('records' => inputs)))
  end

  def perform_request(_request)
    FakeResponse.new(responses.shift)
  end
end

ArchivalistPayloadAccounting.install!(FakeBuilder)

# Mirrors the stage harness, which captures instance_method(:build) after the hook is installed
# and re-defines build; this must not recurse.
ORIGINAL_FAKE_BUILD = FakeBuilder.instance_method(:build)
FakeBuilder.define_method(:build) { |inputs| ORIGINAL_FAKE_BUILD.bind(self).call(inputs) }

class TestArchivalistPayloadAccounting < Minitest::Test
  ANCESTOR = { 'ref' => '/repositories/2/resources/1', 'level' => 'collection' }.freeze

  def input(id, ancestors: [ANCESTOR])
    record = { 'uri' => "/repositories/2/archival_objects/#{id}", 'title' => "t#{id}" }
    {
      'uri' => record['uri'],
      'record_type' => 'archival_object',
      'record' => record,
      'pui_candidate_record' => record.merge('ancestors' => ancestors)
    }
  end

  def response(count)
    JSON.generate('documents' => Array.new(count) { |i| { 'id' => "d#{i}", '_archivalist_pui_final' => { 'json' => 'x' * 10, 'fullrecord' => 'y' } } }, 'served_by' => 'test')
  end

  def test_inactive_hook_records_nothing
    builder = FakeBuilder.new
    builder.responses = [response(1)]
    builder.build([input(1)])
    assert_equal [], ArchivalistPayloadAccounting::STATE[:calls]
  end

  def test_capture_summarizes_bytes_kinds_fields_and_duplicates
    builder = FakeBuilder.new
    builder.responses = [response(2), response(1), response(1)]
    req1 = JSON.generate('records' => [input(1), input(2)])

    result, summary = ArchivalistPayloadAccounting.capture do
      builder.build([input(1), input(2)])
      builder.build([input(1)])
      builder.build_pui_auxiliary([input(3, ancestors: []).merge('prepare_pui_auxiliary' => true)])
      :done
    end

    assert_equal :done, result
    assert_equal false, ArchivalistPayloadAccounting::STATE[:active]
    assert_equal [], ArchivalistPayloadAccounting::STATE[:calls]

    assert_equal 3, summary['calls']
    assert_equal 2, summary['by_kind']['build']['calls']
    assert_equal 1, summary['by_kind']['build_pui_auxiliary']['calls']
    assert_equal 3, summary['by_kind']['build']['inputs']
    assert_equal 3, summary['by_kind']['build']['documents']
    assert_equal req1.bytesize, summary['by_kind']['build']['max_request_bytes']
    assert_equal response(2).bytesize, summary['by_kind']['build']['max_response_bytes']
    assert_equal summary['by_kind'].values.sum { |s| s['request_bytes'] }, summary['request_bytes']

    assert_equal %w[prepare_pui_auxiliary pui_candidate_record record uri record_type].sort, summary['request_field_bytes'].keys.sort
    assert_operator summary['request_field_bytes']['pui_candidate_record'], :>, summary['request_field_bytes']['record']
    assert_equal 3 * JSON.generate([ANCESTOR]).bytesize + JSON.generate([]).bytesize, summary['request_nested_field_bytes']['pui_candidate_record']['ancestors']
    assert_includes summary['response_projection_field_bytes']['_archivalist_pui_final'].keys, 'json'

    dup = summary['duplicates']
    assert_equal 4, dup['inputs_with_uri']
    assert_equal 3, dup['distinct_uris']
    assert_equal 2, dup['max_sends_per_uri']
    assert_equal({ '1' => 2, '2' => 1 }, dup['uri_send_count_histogram'])
    assert_equal JSON.generate(input(1)['record']).bytesize, dup['redundant_identical_record_bytes']
    assert_equal 3, dup['ancestor_entries']
    assert_equal 1, dup['distinct_ancestor_entries']
    assert_equal 2 * JSON.generate(ANCESTOR).bytesize, dup['redundant_identical_ancestor_bytes']
  end

  def test_summary_contains_no_uris_or_record_content
    builder = FakeBuilder.new
    builder.responses = [response(1)]
    _, summary = ArchivalistPayloadAccounting.capture { builder.build([input(42)]) }
    text = JSON.generate(summary)
    refute_includes text, '/repositories/'
    refute_includes text, 't42'
    refute_includes text, 'xxxxxxxxxx'
  end

  def test_capture_resets_state_when_block_raises
    builder = FakeBuilder.new
    builder.responses = [response(1)]
    assert_raises(RuntimeError) do
      ArchivalistPayloadAccounting.capture do
        builder.build([input(1)])
        raise 'boom'
      end
    end
    assert_equal false, ArchivalistPayloadAccounting::STATE[:active]
    assert_equal [], ArchivalistPayloadAccounting::STATE[:calls]
  end
end
