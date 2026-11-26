#!/usr/bin/env ruby
# frozen_string_literal: true

# Deterministic offline tests for archivalist_sidecar_timing.rb (fake builder and responses).
# Run with MRI or JRuby: ruby rust-tools/scripts/test_archivalist_sidecar_timing.rb

require 'minitest/autorun'
require_relative 'archivalist_sidecar_timing'

class TimingFakeRequest
  attr_reader :body

  def initialize(body)
    @body = body
  end
end

class TimingFakeResponse
  attr_reader :body

  def initialize(body, server_timing)
    @body = body
    @headers = { 'Server-Timing' => server_timing }
  end

  def [](name)
    @headers[name]
  end
end

class TimingFakeBuilder
  attr_accessor :responses

  def build(inputs)
    perform_request(TimingFakeRequest.new("{\"records\":#{inputs.length}}"))
  end

  def perform_request(_request)
    responses.shift
  end
end

ArchivalistSidecarTiming.install!(TimingFakeBuilder)

# Mirrors the stage harness, which re-defines build via instance_method; must not recurse.
ORIGINAL_TIMING_FAKE_BUILD = TimingFakeBuilder.instance_method(:build)
TimingFakeBuilder.define_method(:build) { |inputs| ORIGINAL_TIMING_FAKE_BUILD.bind(self).call(inputs) }

class TestArchivalistSidecarTiming < Minitest::Test
  HEADER = 'receive;dur=1.500, parse;dur=2.000, build;dur=10.000, serialize;dur=0.500, total;dur=14.250'

  def test_parse_server_timing
    assert_equal({ 'receive' => 1.5, 'parse' => 2.0, 'build' => 10.0, 'serialize' => 0.5, 'total' => 14.25 },
                 ArchivalistSidecarTiming.parse_server_timing(HEADER))
    assert_equal({}, ArchivalistSidecarTiming.parse_server_timing(nil))
    assert_equal({}, ArchivalistSidecarTiming.parse_server_timing('cache;desc=hit'))
  end

  def test_enabled_accepts_boolish_values
    assert ArchivalistSidecarTiming.enabled?('ARCHIVALIST_SIDECAR_INDEX_TIMING' => '1')
    assert ArchivalistSidecarTiming.enabled?('ARCHIVALIST_SIDECAR_INDEX_TIMING' => 'true')
    refute ArchivalistSidecarTiming.enabled?('ARCHIVALIST_SIDECAR_INDEX_TIMING' => '0')
    refute ArchivalistSidecarTiming.enabled?({})
  end

  def test_inactive_hook_records_nothing
    builder = TimingFakeBuilder.new
    builder.responses = [TimingFakeResponse.new('{}', HEADER)]
    builder.build([1])
    assert_equal [], ArchivalistSidecarTiming::STATE[:calls]
  end

  def test_capture_and_summarize
    builder = TimingFakeBuilder.new
    builder.responses = [TimingFakeResponse.new('{"a":1}', HEADER), TimingFakeResponse.new('{"b":22}', HEADER)]
    result, calls = ArchivalistSidecarTiming.capture do
      builder.build([1, 2])
      builder.build([3])
      :done
    end
    assert_equal :done, result
    assert_equal false, ArchivalistSidecarTiming::STATE[:active]
    assert_equal 2, calls.length

    summary = ArchivalistSidecarTiming.summarize(calls, 100.0)
    assert_equal 2, summary['calls']
    assert_equal 2, summary['calls_with_timing']
    assert_equal 28.5, summary['rust_server_ms']
    assert_equal 20.0, summary['server_phase_ms']['build']
    assert_equal '{"records":2}'.bytesize + '{"records":1}'.bytesize, summary['request_bytes']
    assert_equal 15, summary['response_bytes']
    assert_in_delta summary['round_trip_ms'], summary['rust_server_ms'] + summary['transport_and_http_ms'], 0.002
    assert_in_delta 100.0, summary['outside_sidecar_ms'] + summary['round_trip_ms'], 0.002
  end

  def test_summarize_fails_closed_when_header_missing
    calls = [{ 'round_trip_ms' => 5.0, 'server_timing' => {}, 'request_bytes' => 1, 'response_bytes' => 1 }]
    err = assert_raises(RuntimeError) { ArchivalistSidecarTiming.summarize(calls, 10.0) }
    assert_match(/no Server-Timing header/, err.message)
  end

  def test_summarize_with_no_calls
    summary = ArchivalistSidecarTiming.summarize([], 42.0)
    assert_equal 0, summary['calls']
    assert_equal 0.0, summary['round_trip_ms']
    assert_equal 42.0, summary['outside_sidecar_ms']
  end

  def test_median_summary
    a = { 'calls' => 2, 'rust_server_ms' => 10.0, 'server_phase_ms' => { 'build' => 4.0 } }
    b = { 'calls' => 2, 'rust_server_ms' => 30.0, 'server_phase_ms' => { 'build' => 8.0 } }
    c = { 'calls' => 2, 'rust_server_ms' => 20.0, 'server_phase_ms' => { 'build' => 6.0 } }
    assert_equal({ 'calls' => 2, 'rust_server_ms' => 20.0, 'server_phase_ms' => { 'build' => 6.0 } },
                 ArchivalistSidecarTiming.median_summary([a, b, c]))
    assert_equal 15.0, ArchivalistSidecarTiming.median_summary([a, c])['rust_server_ms']
    assert_nil ArchivalistSidecarTiming.median_summary([])
  end
end
