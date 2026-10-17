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
      agent_types = ['agent_person', 'agent_family', 'agent_software', 'agent_corporate_entity']
      raise "response id mismatch for #{uri}" unless document['id'] == uri
      expected_title = if ['archival_object', 'accession'].include?(record_type)
                         record['display_string']
                       elsif record_type == 'digital_object_component'
                         record['display_string']
                       elsif agent_types.include?(record_type)
                         record.dig('display_name', 'sort_name')
                       elsif record_type == 'repository'
                         input['enrich'] ? record['repo_code'] : record['title']
                       elsif record_type == 'event'
                         input['enrich'] ? record['event_type'] : record['title']
                       elsif ['container_profile', 'location_profile', 'assessment'].include?(record_type)
                         input['enrich'] ? record['display_string'] : record['title']
                       elsif record_type == 'top_container'
                         input['enrich'] ? record['long_display_string'] : record['title']
                       else
                         record['title']
                       end
      raise "response title mismatch for #{uri}" unless document['title'] == expected_title
      raise "response record type mismatch for #{uri}" unless document['primary_type'] == record_type
      expected_types = [record_type]
      expected_types << 'agent_with_user' if agent_types.include?(record_type) && record['is_user']
      expected_types << 'agent' if agent_types.include?(record_type)
      if record_type == 'job' && input['enrich']
        expected_types << record['job_type']
        expected_types << (record['job'] && record['job']['report_type'])
      end
      raise "response types mismatch for #{uri}" unless document['types'] == expected_types
      expected_repository = record_type == 'repository' && input['enrich'] ? uri : input['repository']
      raise "response repository mismatch for #{uri}" unless document['repository'] == expected_repository
      raise "response JSON missing for #{uri}" unless document['json'].is_a?(String)
      raise "response JSON mismatch for #{uri}" unless ASUtils.json_parse(document['json']) == record
      expected_suppressed = record.has_key?('suppressed') && record['suppressed']
      raise "response suppression mismatch for #{uri}" unless document['suppressed'] == expected_suppressed
      expected_publish = expected_publish(record_type, record, expected_suppressed)
      raise "response publication mismatch for #{uri}" unless document['publish'] == expected_publish
      expected_generated = record.has_key?('system_generated') ? record['system_generated'].to_s : 'false'
      raise "response system-generated mismatch for #{uri}" unless document['system_generated'] == expected_generated
      if input['enrich']
        expected_fields = ['common_metadata', 'fullrecord']
        if ['resource', 'archival_object'].include?(record_type)
          expected_fields << 'resource_archival_object_metadata'
        elsif ['accession', 'digital_object'].include?(record_type)
          expected_fields << 'accession_digital_object_metadata'
        elsif record_type == 'subject' || agent_types.include?(record_type)
          expected_fields << 'agent_subject_metadata'
        elsif ['digital_object_component', 'classification', 'classification_term'].include?(record_type)
          expected_fields << 'component_classification_metadata'
        elsif ['repository', 'event', 'location'].include?(record_type)
          expected_fields << 'repository_event_location_metadata'
        elsif ['container_profile', 'location_profile', 'assessment'].include?(record_type)
          expected_fields << 'profile_assessment_metadata'
        elsif record_type == 'top_container'
          expected_fields << 'top_container_metadata'
        elsif record_type == 'job'
          expected_fields << 'job_metadata'
        end
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
        if record_type == 'resource'
          required.concat([
                            'finding_aid_title', 'finding_aid_filing_title', 'identifier',
                            'resource_type', 'level', 'restrictions', 'ead_id',
                            'finding_aid_status', 'related_accession_uris', 'slug',
                            'is_slug_auto', 'external_id', 'ancestors', 'langcode'
                          ])
          if record['collection_management'].is_a?(Hash)
            required.concat(['processing_priority', 'processors'])
          end
          if record['classifications'].is_a?(Array) && !record['classifications'].empty?
            required.concat(['classification_paths', 'classification_uris'])
          end
        elsif record_type == 'archival_object'
          required.concat([
                            'title', 'identifier', 'component_id', 'ref_id', 'slug',
                            'is_slug_auto', 'external_id', 'ancestors', 'langcode'
                          ])
          required << 'resource' if record['resource'].is_a?(Hash)
        elsif record_type == 'accession'
          required.concat([
                            'title', 'identifier', 'acquisition_type', 'resource_type',
                            'restrictions_apply', 'access_restrictions', 'use_restrictions',
                            'related_resource_uris', 'related_accession_uris', 'slug',
                            'is_slug_auto', 'external_id', 'ancestors', 'langcode'
                          ])
          if record['collection_management'].is_a?(Hash)
            required.concat(['processing_priority', 'processors'])
          end
        elsif record_type == 'digital_object'
          required.concat([
                            'digital_object_type', 'digital_object_id', 'identifier', 'level',
                            'restrictions', 'slug', 'is_slug_auto', 'collection_uri_u_sstr',
                            'linked_instance_uris', 'external_id', 'ancestors', 'langcode'
                          ])
        elsif record_type == 'subject'
          required.concat([
                            'source', 'first_term_type', 'slug', 'is_slug_auto',
                            'external_id', 'ancestors', 'langcode'
                          ])
        elsif agent_types.include?(record_type)
          required.concat([
                            'title', 'linked_agent_roles', 'related_agent_uris', 'slug',
                            'is_slug_auto', 'is_user', 'subjects', 'subject_uris',
                            'external_id', 'ancestors', 'langcode'
                          ])
          authorized_name = Array(record['names']).find {|name| name['authorized'] }
          primary_identifier = Array(record['agent_record_identifiers']).find do |identifier|
            identifier['primary_identifier'] == true
          end
          required.concat(['authority_id', 'source']) if primary_identifier || authorized_name
          first_convention = Array(record['agent_conventions_declarations']).first
          required << 'rules' if first_convention&.[]('name_rule') || authorized_name
        elsif record_type == 'digital_object_component'
          required.concat([
                            'digital_object', 'digital_object_id', 'identifier', 'title',
                            'slug', 'is_slug_auto', 'external_id', 'ancestors', 'langcode'
                          ])
        elsif ['classification', 'classification_term'].include?(record_type)
          required.concat([
                            'classification_path', 'agent_uris', 'published_agent_uris',
                            'agents', 'identifier_sort', 'repo_sort',
                            'has_classification_terms', 'slug', 'is_slug_auto',
                            'identifier', 'external_id', 'ancestors', 'langcode'
                          ])
          required << 'classification' if record_type == 'classification_term'
        elsif record_type == 'repository'
          required.concat([
                            'repository', 'title', 'title_sort', 'repo_sort',
                            'slug', 'is_slug_auto', 'position_int_sort',
                            'external_id', 'ancestors', 'langcode'
                          ])
        elsif record_type == 'event'
          required.concat([
                            'event_type', 'title', 'outcome',
                            'linked_record_uris', 'linked_record_titles',
                            'external_id', 'ancestors', 'langcode'
                          ])
        elsif record_type == 'location'
          required.concat([
                            'building', 'floor', 'room', 'area',
                            'external_id', 'ancestors', 'langcode'
                          ])
          required << 'temporary' if record.has_key?('temporary')
          if record['owner_repo'].is_a?(Hash)
            required << 'owner_repo_uri_u_sstr'
            if record.dig('owner_repo', '_resolved', 'repo_code') || record.dig('owner_repo', 'repo_code')
              required << 'owner_repo_display_string_u_ssort'
            end
          end
          if record['location_profile'].is_a?(Hash)
            required.concat([
                              'location_profile_uri_u_sstr',
                              'location_profile_display_string_u_ssort'
                            ])
          end
        elsif record_type == 'container_profile'
          required.concat([
                            'title', 'title_sort', 'display_string', 'note',
                            'container_profile_width_u_sstr',
                            'container_profile_height_u_sstr',
                            'container_profile_depth_u_sstr',
                            'container_profile_dimension_units_u_sstr',
                            'typeahead_sort_key_u_sort',
                            'external_id', 'ancestors', 'langcode'
                          ])
        elsif record_type == 'location_profile'
          required.concat([
                            'title', 'title_sort', 'display_string',
                            'location_profile_width_u_sstr',
                            'location_profile_height_u_sstr',
                            'location_profile_depth_u_sstr',
                            'location_profile_dimension_units_u_sstr',
                            'typeahead_sort_key_u_sort',
                            'external_id', 'ancestors', 'langcode'
                          ])
        elsif record_type == 'assessment'
          required.concat([
                            'assessment_id', 'title', 'display_string',
                            'assessment_record_uris', 'assessment_records',
                            'assessment_record_types',
                            'assessment_surveyor_uris', 'assessment_surveyors',
                            'assessment_survey_begin',
                            'assessment_review_required',
                            'assessment_sensitive_material',
                            'assessment_inactive',
                            'assessment_survey_year',
                            'assessment_collection_uris', 'assessment_collections',
                            'assessment_completed',
                            'assessment_formats', 'assessment_ratings',
                            'assessment_conservation_issues',
                            'title_sort',
                            'external_id', 'ancestors', 'langcode'
                          ])
          required << 'assessment_survey_end' if record['survey_end']
          reviewers = record['reviewer'].is_a?(Array) ? record['reviewer'] : [record['reviewer']].compact
          unless reviewers.empty?
            required.concat(['assessment_reviewer_uris', 'assessment_reviewers'])
          end
        elsif record_type == 'top_container'
          required.concat([
                            'title', 'display_string', 'type_u_ssort', 'notes',
                            'has_location_u_sbool', 'exported_u_sbool', 'empty_u_sbool',
                            'top_container_u_typeahead_utext', 'top_container_u_icusort',
                            'barcode_u_sstr', 'barcode_u_icusort', 'ils_holding_id_u_sstr',
                            'subcontainer_barcodes_u_sstr', 'created_for_collection_u_sstr',
                            'external_id', 'ancestors', 'langcode'
                          ])
          if record['series']
            required.concat([
                              'series_uri_u_sstr', 'series_title_u_sstr',
                              'series_level_u_sstr', 'series_identifier_stored_u_sstr',
                              'series_identifier_u_stext'
                            ])
            if Array(record['series']).any? {|series| series.is_a?(Hash) && series['publish']}
              required.concat([
                                'published_series_uri_u_sstr',
                                'published_series_title_u_sstr'
                              ])
            end
          end
          if record['collection']
            required.concat([
                              'collection_uri_u_sstr',
                              'collection_display_string_u_sstr',
                              'collection_identifier_stored_u_sstr',
                              'collection_identifier_u_stext'
                            ])
          end
          if record['container_profile']
            required.concat([
                              'container_profile_uri_u_sstr',
                              'container_profile_display_string_u_sstr'
                            ])
          end
          unless Array(record['container_locations']).empty?
            required.concat([
                              'location_uri_u_sstr', 'location_uris',
                              'location_display_string_u_sstr',
                              'location_profile_display_string_u_sstr'
                            ])
          end
          required << 'indicator_u_icusort' if record['indicator']
        elsif record_type == 'job'
          required.concat([
                            'job_type', 'report_type', 'job_report_type',
                            'status', 'owner', 'queue_position',
                            'external_id', 'ancestors', 'langcode'
                          ])
        end
        if record_type == 'subject' || agent_types.include?(record_type)
          if record.has_key?('used_within_repositories')
            required.concat(['used_within_repository', 'used_within_published_repository'])
          end
        end
        if ['resource', 'archival_object', 'accession', 'digital_object'].include?(record_type)
          if ['resource', 'accession', 'digital_object'].include?(record_type) &&
             record['classifications'].is_a?(Array) && !record['classifications'].empty?
            required.concat(['classification_paths', 'classification_uris'])
          end
          required << 'four_part_id' if (0..3).any? {|index| !record["id_#{index}"].nil? }
        end
        if ['resource', 'archival_object', 'accession'].include?(record_type)
          instances = Array(record['instances'])
          required << 'digital_object_uris' if instances.any? {|instance| instance['digital_object'] }
          if instances.any? {|instance| instance.dig('sub_container', 'top_container') }
            required << 'top_container_uri_u_sstr'
          end
          required << 'child_container_u_sstr' if instances.any? {|instance| !instance.dig('sub_container', 'type_2').nil? }
          required << 'grand_child_container_u_sstr' if instances.any? {|instance| !instance.dig('sub_container', 'type_3').nil? }
          rights_statements = Array(record['rights_statements'])
          if rights_statements.any? {|statement| !Array(statement['linked_agents']).empty? }
            required << 'rights_statement_agent_uris'
          end
        end
        if record_type == 'digital_object'
          rights_statements = Array(record['rights_statements'])
          if rights_statements.any? {|statement| !Array(statement['linked_agents']).empty? }
            required << 'rights_statement_agent_uris'
          end
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

  def expected_publish(record_type, record, suppressed)
    if record_type == 'subject'
      return !!record['publish'] && !!record['is_linked_to_published_record']
    end
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
