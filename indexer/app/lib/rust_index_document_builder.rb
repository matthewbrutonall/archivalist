require 'net/http'
require 'uri'
require 'json'

class RustIndexDocumentBuilder
  EXPECTED_SERVER = 'archivalist-rust-index-document-builder'

  def initialize(base_url, timeout_seconds)
    @url = URI.parse("#{base_url.to_s.chomp('/')}/index/documents")
    @timeout_seconds = timeout_seconds.to_f
  end

  def build(inputs)
    return {} if inputs.empty?

    request = Net::HTTP::Post.new(@url.request_uri)
    request['Content-Type'] = 'application/json'
    request.body = ASUtils.to_json('records' => inputs)
    response = perform_request(request)
    raise "HTTP #{response.code}" unless response.code == '200'

    payload = ASUtils.json_parse(response.body)
    raise 'unexpected response identity' unless payload['served_by'] == EXPECTED_SERVER
    documents = payload['documents']
    raise 'response documents must be an array' unless documents.is_a?(Array)
    raise 'response document count mismatch' unless documents.length == inputs.length

    expected = inputs.each_with_object({}) {|input, result| result[input['uri']] = input}
    documents.each_with_object({}) do |document, result|
      raise 'response document must be an object' unless document.is_a?(Hash)
      uri = document['uri']
      raise "unexpected response uri #{uri.inspect}" unless expected.has_key?(uri)
      raise "duplicate response uri #{uri.inspect}" if result.has_key?(uri)
      input = expected[uri]
      record_type = input['record_type']
      record = input['record']
      raise "response id mismatch for #{uri}" unless document['id'] == uri
      raise "response title mismatch for #{uri}" unless document['title'] == record['title']
      raise "response record type mismatch for #{uri}" unless document['primary_type'] == record_type
      raise "response types mismatch for #{uri}" unless document['types'] == [record_type]
      raise "response repository mismatch for #{uri}" unless document['repository'] == input['repository']
      raise "response JSON missing for #{uri}" unless document['json'].is_a?(String)
      raise "response JSON mismatch for #{uri}" unless ASUtils.json_parse(document['json']) == record
      expected_suppressed = record.has_key?('suppressed') && record['suppressed']
      raise "response suppression mismatch for #{uri}" unless document['suppressed'] == expected_suppressed
      expected_publish = expected_publish(record, expected_suppressed)
      raise "response publication mismatch for #{uri}" unless document['publish'] == expected_publish
      expected_generated = record.has_key?('system_generated') ? record['system_generated'].to_s : 'false'
      raise "response system-generated mismatch for #{uri}" unless document['system_generated'] == expected_generated
      if input['enrich']
        expected_fields = ['common_metadata', 'fullrecord']
        unless document['_archivalist_rust_fields'] == expected_fields
          raise "response enrichment marker mismatch for #{uri}"
        end
        required = ['fullrecord_published', 'fullrecord']
        required.concat(['subjects', 'subject_uris']) if record.has_key?('subjects')
        if record.has_key?('linked_agents')
          required.concat(['agents', 'agent_uris', 'published_agents', 'published_agent_uris'])
        end
        required.concat(['notes_published', 'notes']) if record.has_key?('notes')
        required.concat(['dates']) if record.has_key?('dates')
        required.concat(['level']) if record.has_key?('level')
        required.concat(['extents']) if record.has_key?('extents')
        ['created_by', 'last_modified_by', 'user_mtime', 'system_mtime', 'create_time'].each do |field|
          required << field if record.has_key?(field)
        end
        missing = required.uniq.reject {|field| document.has_key?(field)}
        raise "response enrichment fields missing for #{uri}: #{missing.join(', ')}" unless missing.empty?
      elsif document.has_key?('_archivalist_rust_fields')
        raise "unexpected response enrichment marker for #{uri}"
      end
      result[uri] = document
    end
  end

  private

  def expected_publish(record, suppressed)
    return false if suppressed
    return false if record.dig('repository', '_resolved', 'publish') == false
    return false if record['has_unpublished_ancestor']
    record.has_key?('publish') && record['publish']
  end

  def perform_request(request)
    http = Net::HTTP.new(@url.host, @url.port)
    http.open_timeout = @timeout_seconds
    http.read_timeout = @timeout_seconds
    http.write_timeout = @timeout_seconds if http.respond_to?(:write_timeout=)
    http.use_ssl = @url.scheme == 'https'
    http.start {|connection| connection.request(request)}
  end
end
