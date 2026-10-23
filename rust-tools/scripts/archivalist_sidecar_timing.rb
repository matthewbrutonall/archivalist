# frozen_string_literal: true

# Opt-in per-pass aggregation of sidecar /index/documents timings (ARCHIVALIST_SIDECAR_INDEX_TIMING=1).
#
# The sidecar started with that variable adds a Server-Timing header (receive, parse, build,
# serialize, total in milliseconds) to successful /index/documents responses. This collector
# sums those per pass and also times each HTTP round trip from the Ruby side (two monotonic
# clock reads around perform_request), so a pass can be split into:
#
#   rust_server_ms       sum of server-side `total` (read request .. just before writing reply)
#   round_trip_ms        Ruby-side wall time of the HTTP calls (includes rust_server_ms, the
#                        response write/read, connection setup and Net::HTTP overhead)
#   outside_sidecar_ms   pass wall time minus round_trip_ms (Ruby preparation, validation,
#                        backend/Solr HTTP and everything else in the pass)
#
# It never re-parses bodies; per call it adds one header parse and two clock reads. Only
# aggregates are kept. The timing header excludes the server's response write, which therefore
# appears in round_trip_ms rather than rust_server_ms.

module ArchivalistSidecarTiming
  PHASES = %w[receive parse build serialize total].freeze
  STATE = { active: false, calls: [] }

  # Only perform_request is prepended. build/build_pui_auxiliary are deliberately NOT: the stage
  # harness captures `instance_method(:build)` and re-defines it, and a prepended wrapper would
  # then call itself recursively.
  module BuilderHook
    def perform_request(request)
      return super unless ArchivalistSidecarTiming::STATE[:active]

      started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      response = super
      round_trip_ms = (Process.clock_gettime(Process::CLOCK_MONOTONIC) - started) * 1000.0
      ArchivalistSidecarTiming::STATE[:calls] << {
        'round_trip_ms' => round_trip_ms,
        'server_timing' => ArchivalistSidecarTiming.parse_server_timing(response['Server-Timing']),
        'request_bytes' => request.body.to_s.bytesize,
        'response_bytes' => response.body.to_s.bytesize
      }
      response
    end
  end

  def self.install!(builder_class)
    builder_class.prepend(BuilderHook) unless builder_class.ancestors.include?(BuilderHook)
  end

  def self.enabled?(env = ENV)
    %w[1 true yes on].include?(env['ARCHIVALIST_SIDECAR_INDEX_TIMING'].to_s.strip.downcase)
  end

  # "receive;dur=1.5, parse;dur=0.25" => { 'receive' => 1.5, 'parse' => 0.25 }; nil or a header
  # without durations gives {}.
  def self.parse_server_timing(header)
    header.to_s.split(',').each_with_object({}) do |metric, out|
      name, *params = metric.split(';').map(&:strip)
      dur = params.find { |p| p.start_with?('dur=') }
      out[name] = Float(dur.delete_prefix('dur=')) if name && !name.empty? && dur
    end
  end

  def self.start!
    STATE[:calls] = []
    STATE[:active] = true
  end

  # Stops collection and returns the calls recorded since start!.
  def self.stop!
    calls = STATE[:calls]
    STATE[:active] = false
    STATE[:calls] = []
    calls
  end

  # Runs the block with collection on and returns [block_result, calls].
  def self.capture
    start!
    result = yield
    [result, stop!]
  ensure
    stop! if STATE[:active]
  end

  # Aggregates one pass. Raises when calls were made but none carried the header, which means
  # the sidecar was not started with timing enabled (or is a binary without the feature).
  def self.summarize(calls, pass_wall_ms)
    timed = calls.reject { |c| c['server_timing'].empty? }
    if !calls.empty? && timed.empty?
      raise 'Sidecar timing requested but no Server-Timing header was returned; was the sidecar started with ARCHIVALIST_SIDECAR_INDEX_TIMING=1?'
    end

    phase_ms = PHASES.to_h { |phase| [phase, timed.sum { |c| c['server_timing'].fetch(phase, 0.0) }.round(3)] }
    round_trip_ms = calls.sum { |c| c['round_trip_ms'] }.round(3)
    {
      'calls' => calls.length,
      'calls_with_timing' => timed.length,
      'server_phase_ms' => phase_ms,
      'rust_server_ms' => phase_ms['total'],
      'round_trip_ms' => round_trip_ms,
      'transport_and_http_ms' => (round_trip_ms - phase_ms['total']).round(3),
      'outside_sidecar_ms' => (pass_wall_ms - round_trip_ms).round(3),
      'max_call_round_trip_ms' => (calls.map { |c| c['round_trip_ms'] }.max || 0.0).round(3),
      'request_bytes' => calls.sum { |c| c['request_bytes'] },
      'response_bytes' => calls.sum { |c| c['response_bytes'] }
    }
  end

  # Field-wise medians across passes (nested server_phase_ms included).
  def self.median_summary(summaries)
    return nil if summaries.empty?

    median = lambda do |values|
      sorted = values.sort
      mid = sorted.length / 2
      sorted.length.odd? ? sorted[mid] : ((sorted[mid - 1] + sorted[mid]) / 2.0).round(3)
    end
    out = {}
    summaries.first.each_key do |key|
      out[key] = if summaries.first[key].is_a?(Hash)
                   summaries.first[key].keys.to_h { |k| [k, median.call(summaries.map { |s| s[key][k] })] }
                 else
                   median.call(summaries.map { |s| s[key] })
                 end
    end
    out
  end
end
