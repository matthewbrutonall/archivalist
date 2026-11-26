# frozen_string_literal: true

# Opt-in byte accounting for sidecar /index/documents traffic (ARCHIVALIST_PAYLOAD_ACCOUNTING=1).
#
# Counts bytes and calls, never time, so it is not subject to the timing distortion of the
# profiler. It re-parses every request and response body, which is expensive, so the stage
# harness only activates it on a dedicated extra pass whose wall time is reported as NOT
# comparable and is never used in a median.
#
# Only aggregates are kept: byte totals per field name, call counts, size maxima and
# duplicate counts. No record content, URIs, identifiers or digests are stored.
#
# Field bytes are re-serialized with JSON.generate and are therefore approximate; body byte
# totals are exact (bytesize of the HTTP bodies actually sent and received).

require 'json'
require 'digest'

module ArchivalistPayloadAccounting
  STATE = { active: false, calls: [] }

  KIND_BUILD = 'build'
  KIND_AUXILIARY = 'build_pui_auxiliary'

  # Only perform_request is prepended. build/build_pui_auxiliary are deliberately NOT: the stage
  # harness captures `instance_method(:build)` and re-defines it, and a prepended wrapper would
  # then call itself recursively. The call kind is derived from the request content instead.
  module BuilderHook
    def perform_request(request)
      response = super
      if ArchivalistPayloadAccounting::STATE[:active]
        ArchivalistPayloadAccounting::STATE[:calls] << {
          'request_body' => request.body.to_s,
          'response_body' => response.body.to_s
        }
      end
      response
    end
  end

  def self.install!(builder_class)
    builder_class.prepend(BuilderHook) unless builder_class.ancestors.include?(BuilderHook)
  end

  # Auxiliary (tree/ordered-records) inputs are the only ones marked prepare_pui_auxiliary.
  def self.call_kind(request)
    records = Array(request['records'])
    records.any? { |input| input.is_a?(Hash) && input['prepare_pui_auxiliary'] == true } ? KIND_AUXILIARY : KIND_BUILD
  end

  # Runs the block with capture on and returns [block_result, summary]. Bodies are held only
  # until the pass ends and are summarized after the block, outside the captured work.
  def self.capture
    STATE[:calls] = []
    STATE[:active] = true
    result = yield
    STATE[:active] = false
    [result, summarize(STATE[:calls])]
  ensure
    STATE[:active] = false
    STATE[:calls] = []
  end

  def self.json_bytes(value)
    JSON.generate(value).bytesize
  end

  def self.add_field_bytes(target, hash)
    hash.each { |key, value| target[key] += json_bytes(value) }
  end

  def self.sorted_desc(counter)
    counter.sort_by { |key, bytes| [-bytes, key] }.to_h
  end

  # Summarizes captured calls into aggregates only.
  def self.summarize(calls)
    by_kind = Hash.new { |h, k| h[k] = { 'calls' => 0, 'request_bytes' => 0, 'response_bytes' => 0, 'max_request_bytes' => 0, 'max_response_bytes' => 0, 'inputs' => 0, 'documents' => 0 } }
    request_fields = Hash.new(0)
    request_nested_fields = Hash.new { |h, k| h[k] = Hash.new(0) }
    response_fields = Hash.new(0)
    response_projection_fields = Hash.new { |h, k| h[k] = Hash.new(0) }
    uri_occurrences = Hash.new(0)
    record_digests = {}
    redundant_record_bytes = 0
    ancestor_entries = 0
    ancestor_bytes = 0
    ancestor_digests = {}
    redundant_ancestor_bytes = 0

    calls.each do |call|
      request = JSON.parse(call['request_body'])
      stats = by_kind[call_kind(request)]
      req_bytes = call['request_body'].bytesize
      resp_bytes = call['response_body'].bytesize
      stats['calls'] += 1
      stats['request_bytes'] += req_bytes
      stats['response_bytes'] += resp_bytes
      stats['max_request_bytes'] = [stats['max_request_bytes'], req_bytes].max
      stats['max_response_bytes'] = [stats['max_response_bytes'], resp_bytes].max

      Array(request['records']).each do |input|
        next unless input.is_a?(Hash)

        stats['inputs'] += 1
        add_field_bytes(request_fields, input)
        uri_occurrences[input['uri']] += 1 if input['uri'].is_a?(String)

        input.each do |key, value|
          next unless value.is_a?(Hash)

          add_field_bytes(request_nested_fields[key], value)
          Array(value['ancestors']).each do |ancestor|
            bytes = json_bytes(ancestor)
            ancestor_entries += 1
            ancestor_bytes += bytes
            digest = Digest::SHA256.digest(JSON.generate(ancestor))
            if ancestor_digests[digest]
              redundant_ancestor_bytes += bytes
            else
              ancestor_digests[digest] = true
            end
          end
        end

        next unless input['record'].is_a?(Hash)

        record_json = JSON.generate(input['record'])
        digest = Digest::SHA256.digest(record_json)
        if record_digests[digest]
          redundant_record_bytes += record_json.bytesize
        else
          record_digests[digest] = true
        end
      end

      response = JSON.parse(call['response_body'])
      Array(response['documents']).each do |document|
        next unless document.is_a?(Hash)

        stats['documents'] += 1
        add_field_bytes(response_fields, document)
        document.each do |key, value|
          add_field_bytes(response_projection_fields[key], value) if key.start_with?('_archivalist') && value.is_a?(Hash)
        end
      end
    end

    occurrence_histogram = Hash.new(0)
    uri_occurrences.each_value { |count| occurrence_histogram[count] += 1 }
    totals = by_kind.values

    {
      'note' => 'Byte counts only; no timings. Body bytes are exact, field bytes are re-serialized approximations.',
      'calls' => totals.sum { |s| s['calls'] },
      'request_bytes' => totals.sum { |s| s['request_bytes'] },
      'response_bytes' => totals.sum { |s| s['response_bytes'] },
      'by_kind' => by_kind.to_h,
      'request_field_bytes' => sorted_desc(request_fields),
      'request_nested_field_bytes' => request_nested_fields.transform_values { |v| sorted_desc(v) },
      'response_field_bytes' => sorted_desc(response_fields),
      'response_projection_field_bytes' => response_projection_fields.transform_values { |v| sorted_desc(v) },
      'duplicates' => {
        'inputs_with_uri' => uri_occurrences.values.sum,
        'distinct_uris' => uri_occurrences.length,
        'max_sends_per_uri' => uri_occurrences.values.max || 0,
        'uri_send_count_histogram' => occurrence_histogram.sort.to_h.transform_keys(&:to_s),
        'redundant_identical_record_bytes' => redundant_record_bytes,
        'ancestor_entries' => ancestor_entries,
        'ancestor_bytes' => ancestor_bytes,
        'distinct_ancestor_entries' => ancestor_digests.length,
        'redundant_identical_ancestor_bytes' => redundant_ancestor_bytes
      }
    }
  end
end
