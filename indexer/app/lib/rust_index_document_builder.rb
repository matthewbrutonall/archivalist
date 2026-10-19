require 'net/http'
require 'uri'
require 'json'
require 'set'
require 'date'
require_relative 'indexer_common_config'

class RustIndexDocumentBuilder
  EXPECTED_SERVER = 'archivalist-rust-index-document-builder'

  COLLECTION_MANAGEMENT_CHILD_FIELDS = [
    'id',
    'uri',
    'parent_id',
    'parent_title',
    'parent_type',
    'title',
    'title_sort',
    'types',
    'primary_type',
    'json',
    'processing_priority',
    'processing_status',
    'processing_hours_total',
    'processing_funding_source',
    'processors',
    'suppressed',
    'repository',
    'created_by',
    'last_modified_by',
    'system_mtime',
    'user_mtime',
    'create_time',
  ].freeze

  PUI_ROUTING_FIELDS = [
    'marker',
    'id',
    'pui_parent_id',
    'types',
  ].freeze

  PUI_NOTES_FIELDS = [
    'marker',
    'notes_published',
  ].freeze

  PUI_AVAILABILITY_FIELDS = [
    'marker',
    'uri',
    'primary_type',
    'record_inheritance_has_type',
    'conditional_additions',
    'top_container_override',
  ].freeze

  TOP_CONTAINER_OVERRIDE_FIELDS = [
    'publish',
    'additions',
  ].freeze

  PUI_AUXILIARY_FIELDS = [
    'id',
    'uri',
    'pui_parent_id',
    'publish',
    'primary_type',
    'types',
    'json',
  ].freeze

  PUI_AUXILIARY_META_FIELDS = [
    'marker',
    'auxiliary_type',
  ].freeze

  PUI_AUXILIARY_TYPES = [
    'resource_ordered_records',
    'tree_root',
    'tree_waypoint',
    'tree_node',
    'tree_node_from_root',
  ].freeze

  def self.compute_pui_conditional_additions(primary_type, record_inheritance_has_type)
    additions = []
    if ['accession', 'digital_object', 'digital_object_component'].include?(primary_type)
      additions << 'pui_record'
    end
    if ['agent_person', 'agent_corporate_entity'].include?(primary_type)
      additions << 'pui_agent'
    end
    unless record_inheritance_has_type
      if ['resource'].include?(primary_type)
        additions << 'pui_collection'
      elsif ['classification'].include?(primary_type)
        additions << 'pui_record_group'
      elsif ['agent_person'].include?(primary_type)
        additions << 'pui_person'
      else
        additions << 'pui_' + primary_type.to_s
      end
      additions << 'pui'
    end
    additions
  end

  def initialize(base_url, timeout_seconds)
    @url = URI.parse("#{base_url.to_s.chomp('/')}/index/documents")
    @timeout_seconds = timeout_seconds.to_f
  end

  def build_pui_auxiliary(inputs)
    return [] if inputs.empty?

    raise "inputs must be an Array" unless inputs.is_a?(Array)
    maximum = 1000
    if inputs.length > maximum
      raise "inputs exceeds maximum batch size of #{maximum}"
    end

    inputs.each do |input|
      raise "input must be a Hash" unless input.is_a?(Hash)
      raise "input.prepare_pui_auxiliary must be explicitly true" unless input['prepare_pui_auxiliary'] == true
      uri = input['uri']
      raise "input.uri must be a non-empty string" unless uri.is_a?(String) && !uri.strip.empty?
      record_type = input['record_type']
      raise "invalid record_type #{record_type.inspect}" unless PUI_AUXILIARY_TYPES.include?(record_type)
      parent_id = input['pui_parent_id']
      raise "pui_parent_id must be a non-empty string" unless parent_id.is_a?(String) && !parent_id.strip.empty?
      raise "input.record must not be nil" if input['record'].nil?
      raise "input.record must be a Hash or Array" unless input['record'].is_a?(Hash) || input['record'].is_a?(Array)
      aux_json = input['pui_auxiliary_json']
      raise "pui_auxiliary_json must be a non-empty string" unless aux_json.is_a?(String) && !aux_json.strip.empty?
    end

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

    result = []
    documents.zip(inputs).each_with_index do |(doc, input), idx|
      raise "response document must be a Hash at index #{idx}" unless doc.is_a?(Hash)

      uri = input['uri']
      parent_id = input['pui_parent_id']
      record_type = input['record_type']
      record = input['record']

      if doc.keys.any? { |k| !k.is_a?(String) }
        raise "non-string key found in response document at index #{idx}"
      end

      expected_keys = PUI_AUXILIARY_FIELDS + ['_archivalist_pui_auxiliary']
      missing_keys = expected_keys - doc.keys
      unless missing_keys.empty?
        raise "pui auxiliary missing required fields for #{uri}: #{missing_keys.sort.join(', ')}"
      end
      extra_keys = doc.keys - expected_keys
      unless extra_keys.empty?
        raise "pui auxiliary contains unexpected fields for #{uri}: #{extra_keys.sort.join(', ')}"
      end

      aux_meta = doc['_archivalist_pui_auxiliary']
      raise "auxiliary metadata must be a Hash for #{uri}" unless aux_meta.is_a?(Hash)
      if aux_meta.keys.any? { |k| !k.is_a?(String) }
        raise "non-string key found in auxiliary metadata for #{uri}"
      end
      missing_meta = PUI_AUXILIARY_META_FIELDS - aux_meta.keys
      unless missing_meta.empty?
        raise "auxiliary metadata missing fields for #{uri}: #{missing_meta.sort.join(', ')}"
      end
      extra_meta = aux_meta.keys - PUI_AUXILIARY_META_FIELDS
      unless extra_meta.empty?
        raise "auxiliary metadata contains unexpected fields for #{uri}: #{extra_meta.sort.join(', ')}"
      end
      unless aux_meta['marker'] == 'pui_auxiliary_document_metadata'
        raise "auxiliary metadata marker mismatch for #{uri}: expected 'pui_auxiliary_document_metadata', got #{aux_meta['marker'].inspect}"
      end
      unless aux_meta['auxiliary_type'] == record_type
        raise "auxiliary metadata type mismatch for #{uri}: expected #{record_type.inspect}, got #{aux_meta['auxiliary_type'].inspect}"
      end

      # Strip marker
      doc = doc.dup
      doc.delete('_archivalist_pui_auxiliary')

      # Ensure remaining keys match exact native keys
      unless doc.keys.sort == PUI_AUXILIARY_FIELDS.sort
        raise "doc keys mismatch after stripping marker for #{uri}"
      end

      # Strict type and value checks
      unless doc['id'].is_a?(String) && doc['id'] == uri
        raise "id mismatch for #{uri}: expected #{uri.inspect}, got #{doc['id'].inspect}"
      end
      unless doc['uri'].is_a?(String) && doc['uri'] == uri
        raise "uri mismatch for #{uri}: expected #{uri.inspect}, got #{doc['uri'].inspect}"
      end
      unless doc['pui_parent_id'].is_a?(String) && doc['pui_parent_id'] == parent_id
        raise "pui_parent_id mismatch for #{uri}: expected #{parent_id.inspect}, got #{doc['pui_parent_id'].inspect}"
      end
      unless doc['primary_type'].is_a?(String) && doc['primary_type'] == record_type
        raise "primary_type mismatch for #{uri}: expected #{record_type.inspect}, got #{doc['primary_type'].inspect}"
      end
      unless doc['publish'].is_a?(String) && doc['publish'] == 'true'
        raise "publish mismatch for #{uri}: expected string 'true', got #{doc['publish'].inspect} (#{doc['publish'].class})"
      end
      unless doc['types'].is_a?(Array) && doc['types'] == ['pui']
        raise "types mismatch for #{uri}: expected ['pui'], got #{doc['types'].inspect}"
      end
      unless doc['json'].is_a?(String)
        raise "json must be a String for #{uri}"
      end

      expected_json = ASUtils.to_json(record)
      unless doc['json'] == expected_json
        raise "json serialization mismatch for #{uri}"
      end

      parsed_json = ASUtils.json_parse(doc['json'])
      validate_json_equal!(record, parsed_json, "pui auxiliary json for #{uri}")

      # Assert no marker or internal field leakage
      if doc.keys.any? { |k| k.start_with?('_archivalist') }
        raise "internal marker leaked into candidate document for #{uri}"
      end

      result << doc
    end

    result
  end

  def build(inputs)
    return {} if inputs.empty?

    inputs.each do |input|
      uri = input['uri']
      record = input['record']
      record_type = input['record_type']

      if input['prepare_pui_final'] && input['prepare_pui_availability']
        raise "cannot combine prepare_pui_final and prepare_pui_availability in same input for #{uri}"
      end

      if input['prepare_pui_availability']
        unless record.is_a?(Hash)
          raise "record must be a Hash for #{uri}"
        end
        unless record.has_key?('uri') && record['uri'].is_a?(String) && !record['uri'].strip.empty?
          raise "record.uri must be a non-empty string for #{uri}"
        end
        unless record['uri'] == uri
          raise "record uri mismatch for #{uri}"
        end
        unless record.has_key?('jsonmodel_type') && record['jsonmodel_type'].is_a?(String) && !record['jsonmodel_type'].strip.empty?
          raise "record.jsonmodel_type must be a non-empty string for #{uri}"
        end
        unless record['jsonmodel_type'] == record_type
          raise "record jsonmodel_type mismatch for #{uri}"
        end
        unless input.has_key?('record_inheritance_has_type') && (input['record_inheritance_has_type'] == true || input['record_inheritance_has_type'] == false)
          raise "record_inheritance_has_type must be explicitly boolean for #{uri}"
        end
      end
    end

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
      if input['prepare_pui_final'] && input['prepare_pui_availability']
        raise "cannot combine prepare_pui_final and prepare_pui_availability in same input for #{uri}"
      end
      if input['prepare_pui_availability']
        unless record.is_a?(Hash)
          raise "record must be a Hash for #{uri}"
        end
        unless record.has_key?('uri') && record['uri'].is_a?(String) && !record['uri'].strip.empty?
          raise "record.uri must be a non-empty string for #{uri}"
        end
        unless record['uri'] == uri
          raise "record uri mismatch for #{uri}"
        end
        unless record.has_key?('jsonmodel_type') && record['jsonmodel_type'].is_a?(String) && !record['jsonmodel_type'].strip.empty?
          raise "record.jsonmodel_type must be a non-empty string for #{uri}"
        end
        unless record['jsonmodel_type'] == record_type
          raise "record jsonmodel_type mismatch for #{uri}"
        end
        unless input.has_key?('record_inheritance_has_type') && (input['record_inheritance_has_type'] == true || input['record_inheritance_has_type'] == false)
          raise "record_inheritance_has_type must be explicitly boolean for #{uri}"
        end
      end
      agent_types = ['agent_person', 'agent_family', 'agent_software', 'agent_corporate_entity']
      raise "response id mismatch for #{uri}" unless document['id'] == uri
      expected_title = if input['prepare_pui_final']
                         record['title']
                       elsif ['archival_object', 'accession'].include?(record_type)
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
          expected_fields << 'accession_date_metadata' if record_type == 'accession'
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
        expected_fields << 'dynamic_enum_and_ark_metadata'
        if input['prepare_pui_availability']
          expected_fields << 'pui_availability_metadata'
        end
        unless document['_archivalist_rust_fields'] == expected_fields
          raise "response enrichment marker mismatch for #{uri}"
        end
        validate_dynamic_enum_and_ark_projection!(document, input)
        validate_accession_date_projection!(document, input) if record_type == 'accession'
        validate_pui_availability_projection!(document, input) if input['prepare_pui_availability']
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
                            'is_slug_auto', 'external_id', 'ancestors', 'langcode',
                            'accession_date', 'accession_date_year'
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
      elsif input['prepare_pui_final']
        validate_pui_final_projection!(document, input)
        validate_pui_routing_projection!(document, input)
        validate_pui_notes_projection!(document, input)
      elsif input['prepare_pui_availability']
        expected_fields = ['pui_availability_metadata']
        unless document['_archivalist_rust_fields'] == expected_fields
          raise "response enrichment marker mismatch for #{uri}"
        end
        validate_pui_availability_projection!(document, input)
      elsif document.has_key?('_archivalist_rust_fields')
        raise "unexpected response enrichment marker for #{uri}"
      end

      if !input['prepare_pui_final'] && (document.has_key?('_archivalist_pui_final') ||
                                         document.has_key?('_archivalist_pui_routing') ||
                                         document.has_key?('_archivalist_pui_notes'))
        raise "unexpected response pui projection marker for #{uri}"
      end

      if !input['prepare_pui_availability'] && document.has_key?('_archivalist_pui_availability')
        raise "unexpected response pui availability projection marker for #{uri}"
      end

      cm = record.is_a?(Hash) && record['collection_management']
      has_cm = cm.is_a?(Hash)
      valid_cm_parent = ['resource', 'accession', 'digital_object'].include?(record_type)

      if document.has_key?('_archivalist_collection_management_documents')
        unless input['enrich'] && has_cm && valid_cm_parent
          raise "unexpected response collection management documents for #{uri}"
        end

        cm_docs = document['_archivalist_collection_management_documents']
        raise "collection management documents must be an array for #{uri}" unless cm_docs.is_a?(Array)
        raise "collection management documents count mismatch for #{uri}" unless cm_docs.length == 1

        cm_doc = cm_docs.first
        raise "collection management document must be an object for #{uri}" unless cm_doc.is_a?(Hash)
        raise "child document contains private handoff field" if cm_doc.keys.any? {|k| k.start_with?('_archivalist_')}

        missing_keys = COLLECTION_MANAGEMENT_CHILD_FIELDS - cm_doc.keys
        unless missing_keys.empty?
          raise "child document missing required fields for #{uri}: #{missing_keys.sort.join(', ')}"
        end

        extra_keys = cm_doc.keys - COLLECTION_MANAGEMENT_CHILD_FIELDS
        unless extra_keys.empty?
          raise "child document contains unexpected fields for #{uri}: #{extra_keys.sort.join(', ')}"
        end

        raise "child id mismatch for #{uri}" unless cm_doc['id'] == cm['uri']
        raise "child uri mismatch for #{uri}" unless cm_doc['uri'] == cm['uri']
        raise "child parent_id mismatch for #{uri}" unless cm_doc['parent_id'] == uri
        raise "child parent_type mismatch for #{uri}" unless cm_doc['parent_type'] == record_type

        raw_title = record['title'] || record['display_string']
        expected_cm_title = raw_title.is_a?(String) ? raw_title.strip : raw_title
        raise "child parent_title mismatch for #{uri}" unless cm_doc['parent_title'] == expected_cm_title
        raise "child title mismatch for #{uri}" unless cm_doc['title'] == expected_cm_title

        expected_title_sort = expected_cm_title ? clean_for_sort(expected_cm_title) : nil
        raise "child title_sort mismatch for #{uri}" unless cm_doc['title_sort'] == expected_title_sort

        raise "child primary_type mismatch for #{uri}" unless cm_doc['primary_type'] == 'collection_management'
        raise "child types mismatch for #{uri}" unless cm_doc['types'] == ['collection_management']

        expected_child_repo = record_type == 'repository' && input['enrich'] ? uri : input['repository']
        raise "child repository mismatch for #{uri}" unless cm_doc['repository'] == expected_child_repo
        raise "child suppression mismatch for #{uri}" unless cm_doc['suppressed'] == record['suppressed']

        raise "child json missing for #{uri}" unless cm_doc['json'].is_a?(String)
        parsed_json = ASUtils.json_parse(cm_doc['json'])
        raise "child json must be an object for #{uri}" unless parsed_json.is_a?(Hash)

        expected_cm_json = reconstruct_expected_collection_management_json(cm)
        validate_json_equal!(expected_cm_json, parsed_json, "child json for #{uri}")

        expected_json_string = ASUtils.to_json(expected_cm_json)
        raise "child json serialization mismatch for #{uri}" unless cm_doc['json'] == expected_json_string

        ['processing_priority', 'processing_status', 'processing_hours_total', 'processing_funding_source', 'processors'].each do |field|
          expected_val = cm[field]
          expected_val = expected_val.strip if expected_val.is_a?(String)
          raise "child #{field} mismatch for #{uri}" unless cm_doc[field] == expected_val
        end

        ['created_by', 'last_modified_by', 'system_mtime', 'user_mtime', 'create_time'].each do |field|
          expected_val = cm[field]
          raise "child #{field} mismatch for #{uri}" unless cm_doc[field] == expected_val
        end
      end

      result[uri] = document
    end
  end

  private

  def reconstruct_expected_collection_management_json(cm)
    expected = deep_copy_json_value(cm)
    strip_fullrecord_object(expected)
    expected
  end

  def deep_copy_json_value(val)
    case val
    when Hash
      val.each_with_object({}) { |(k, v), h| h[k.to_s] = deep_copy_json_value(v) }
    when Array
      val.map { |v| deep_copy_json_value(v) }
    when String
      val.dup
    else
      val
    end
  end

  def strip_fullrecord_object(hash)
    hash.each do |key, val|
      key_str = key.to_s
      next if IndexerCommonConfig.fullrecord_excludes.include?(key_str) || key_str =~ /_enum_s$/
      case val
      when String
        hash[key] = val.strip
      when Hash
        strip_fullrecord_object(val)
      when Array
        strip_fullrecord_array(val)
      end
    end
  end

  def strip_fullrecord_array(arr)
    arr.each_with_index do |val, idx|
      case val
      when String
        arr[idx] = val.strip
      when Hash
        strip_fullrecord_object(val)
      when Array
        strip_fullrecord_array(val)
      end
    end
  end

  def validate_json_equal!(expected, actual, path = 'child json')
    if expected.is_a?(TrueClass) || expected.is_a?(FalseClass)
      unless actual.is_a?(TrueClass) || actual.is_a?(FalseClass)
        raise "#{path} type mismatch: expected Boolean, got #{actual.class}"
      end
    else
      unless expected.class == actual.class
        raise "#{path} type mismatch: expected #{expected.class}, got #{actual.class}"
      end
    end

    case expected
    when Hash
      expected_keys = expected.keys.map(&:to_s).sort
      actual_keys = actual.keys.map(&:to_s).sort
      unless expected_keys == actual_keys
        missing = expected_keys - actual_keys
        extra = actual_keys - expected_keys
        raise "#{path} keys mismatch (missing: #{missing.inspect}, extra: #{extra.inspect})"
      end
      expected_keys.each do |key|
        exp_val = expected.has_key?(key) ? expected[key] : (expected.has_key?(key.to_sym) ? expected[key.to_sym] : nil)
        act_val = actual.has_key?(key) ? actual[key] : (actual.has_key?(key.to_sym) ? actual[key.to_sym] : nil)
        validate_json_equal!(exp_val, act_val, "#{path}.#{key}")
      end
    when Array
      unless expected.length == actual.length
        raise "#{path} array length mismatch: expected #{expected.length}, got #{actual.length}"
      end
      expected.each_with_index do |exp_val, idx|
        validate_json_equal!(exp_val, actual[idx], "#{path}[#{idx}]")
      end
    else
      unless expected == actual
        raise "#{path} value mismatch: expected #{expected.inspect}, got #{actual.inspect}"
      end
    end
  end

  def clean_for_sort(value)
    return nil if value.nil?
    out = value.gsub(/<[^>]+>/, '')
    out.gsub!(/-/, ' ')
    out.gsub!(/[^\p{L}\d\s_]/, '')
    out.gsub!(/\s+/, ' ')
    out.strip
  end

  def expected_publish(record_type, record, suppressed)
    if record_type == 'subject'
      return !!record['publish'] && !!record['is_linked_to_published_record']
    end
    return false if suppressed
    return false if record.dig('repository', '_resolved', 'publish') == false
    return false if record['has_unpublished_ancestor']
    record.has_key?('publish') && record['publish']
  end

  def validate_dynamic_enum_and_ark_projection!(document, input)
    uri = input['uri']
    record = input['record']
    dynamic_enum_fields = Array(input['dynamic_enum_fields'])
    arks_enabled = !!input['arks_enabled']

    # 1. Dynamic Enums validation
    expected_enums = compute_expected_dynamic_enums(record, dynamic_enum_fields)
    doc_enum_keys = document.keys.select { |k| k.is_a?(String) && k.end_with?('_enum_s') }

    unexpected_keys = doc_enum_keys - expected_enums.keys
    unless unexpected_keys.empty?
      raise "unexpected dynamic enum keys projected for #{uri}: #{unexpected_keys.sort.join(', ')}"
    end

    missing_keys = expected_enums.keys - doc_enum_keys
    unless missing_keys.empty?
      raise "missing dynamic enum keys for #{uri}: #{missing_keys.sort.join(', ')}"
    end

    expected_enums.each do |key, expected_vals|
      actual_vals = document[key]
      unless actual_vals.is_a?(Array)
        raise "dynamic enum field #{key} must be an array for #{uri}"
      end
      begin
        validate_json_equal!(expected_vals, actual_vals, "dynamic enum #{key} for #{uri}")
      rescue => e
        raise "dynamic enum values mismatch for #{uri} on #{key}: #{e.message}"
      end
    end

    # 2. ARK validation
    if !arks_enabled
      if document.has_key?('ark_name')
        raise "unexpected ark_name projected when ARKs are disabled for #{uri}"
      end
    else
      raw_arks = record.is_a?(Hash) ? record['ark_name'] : nil
      if raw_arks.nil?
        if document.has_key?('ark_name')
          raise "unexpected ark_name projected when absent in record for #{uri}"
        end
      else
        unless raw_arks.is_a?(Hash)
          raise "record ark_name must be a hash for #{uri}"
        end
        current = raw_arks['current']
        previous = raw_arks['previous']
        unless previous.is_a?(Array)
          raise "record ark_name.previous must be an array for #{uri}"
        end
        if current && !current.is_a?(String)
          raise "record ark_name.current must be a string for #{uri}"
        end
        if previous.any? { |p| !p.is_a?(String) }
          raise "record ark_name.previous elements must be strings for #{uri}"
        end
        expected_arks = ([current] + previous).compact.map { |s| s.gsub(/\A.*ark:/, 'ark:') }
        actual_arks = document['ark_name']
        unless actual_arks.is_a?(Array)
          raise "response ark_name must be an array for #{uri}"
        end
        begin
          validate_json_equal!(expected_arks, actual_arks, "response ark_name for #{uri}")
        rescue => e
          raise "response ark_name mismatch for #{uri}: #{e.message}"
        end
      end
    end
  end

  def validate_accession_date_projection!(document, input)
    uri = input['uri']
    record = input['record']
    unless record.is_a?(Hash)
      raise "record must be a hash for #{uri}"
    end

    raw_date = record['accession_date']
    unless raw_date.is_a?(String)
      raise "record accession_date missing or invalid for #{uri}"
    end

    parsed_year = begin
      Date.parse(raw_date).year
    rescue ArgumentError, TypeError => e
      raise "record accession_date unparseable for #{uri}: #{e.message}"
    end

    actual_date = document['accession_date']
    unless actual_date.is_a?(String)
      raise "response accession_date must be a string for #{uri}"
    end

    actual_year = document['accession_date_year']
    unless actual_year.is_a?(Integer)
      raise "response accession_date_year must be an integer for #{uri}"
    end

    unless actual_year == parsed_year
      raise "response accession_date_year mismatch for #{uri}: expected #{parsed_year.inspect}, got #{actual_year.inspect}"
    end

    if raw_date == '9999-12-31'
      expected_unknown = input['accession_date_unknown_label']
      unless expected_unknown.is_a?(String) && !expected_unknown.empty?
        raise "accession_date_unknown_label missing or empty in request for #{uri}"
      end

      unless actual_date == expected_unknown
        raise "response accession_date mismatch for sentinel on #{uri}: expected #{expected_unknown.inspect}, got #{actual_date.inspect}"
      end

      fullrecord = document['fullrecord']
      expected_suffix = "#{expected_unknown} "
      if fullrecord.is_a?(Array)
        unless fullrecord.last == expected_suffix
          raise "response fullrecord array does not end with sentinel label for #{uri}"
        end
      elsif fullrecord.is_a?(String)
        unless fullrecord.end_with?(expected_suffix)
          raise "response fullrecord string does not end with sentinel label for #{uri}"
        end
      else
        raise "response fullrecord must be array or string for #{uri}"
      end
    else
      unless actual_date == raw_date
        raise "response accession_date mismatch for #{uri}: expected #{raw_date.inspect}, got #{actual_date.inspect}"
      end
    end
  end

  def compute_expected_dynamic_enums(record_hash, fields)
    return {} unless record_hash.is_a?(Hash)

    expected = {}
    found_keys = Set.new

    ASUtils.search_nested(record_hash, fields, ['_resolved']) do |field, field_value|
      key = "#{field}_enum_s"
      expected[key] ||= Set.new
      expected[key] << field_value
      found_keys << key
    end

    ASUtils.search_nested(record_hash, ['items'], ['_resolved']) do |_field, field_value|
      if field_value.is_a?(Hash) && field_value.key?('type')
        expected['type_enum_s'] ||= Set.new
        expected['type_enum_s'] << field_value.fetch('type')
        found_keys << 'type_enum_s'
      end
    end

    result = {}
    found_keys.each do |key|
      result[key] = expected[key].to_a.flatten
    end
    result
  end

  def validate_pui_final_projection!(document, input)
    uri = input['uri']
    unless document['_archivalist_rust_fields'] == ['pui_final_document_metadata', 'pui_routing_metadata', 'pui_notes_metadata']
      raise "response pui final marker mismatch for #{uri}"
    end
    unless document.has_key?('_archivalist_pui_final') && document['_archivalist_pui_final'].is_a?(Hash)
      raise "missing _archivalist_pui_final for #{uri}"
    end
    pui_final = document['_archivalist_pui_final']
    unless pui_final['marker'] == 'pui_final_document_metadata'
      raise "pui final payload marker mismatch for #{uri}"
    end
    unless pui_final['json'].is_a?(String)
      raise "pui final json must be a String for #{uri}"
    end
    unless pui_final['fullrecord_published'].is_a?(Array)
      raise "pui final fullrecord_published must be an Array for #{uri}"
    end
    if document.has_key?('fullrecord')
      raise "unexpected unpublished fullrecord field in pui final document for #{uri}"
    end

    record = input['record']
    raise "missing record for #{uri}" unless record.is_a?(Hash)
    rec_uri = record['uri']
    unless rec_uri.is_a?(String) && !rec_uri.empty?
      raise "record.uri must be a non-empty string for #{uri}"
    end
    unless rec_uri == input['uri']
      raise "record uri mismatch for #{uri}: expected #{input['uri']}, got #{rec_uri}"
    end
    rec_type = record['jsonmodel_type']
    unless rec_type.is_a?(String) && !rec_type.empty?
      raise "record.jsonmodel_type must be a non-empty string for #{uri}"
    end
    unless rec_type == input['record_type']
      raise "record jsonmodel_type mismatch for #{uri}: expected #{input['record_type']}, got #{rec_type}"
    end

    candidate = input['pui_candidate_record']
    raise "missing pui_candidate_record for #{uri}" unless candidate.is_a?(Hash)
    cand_uri = candidate['uri']
    unless cand_uri.is_a?(String) && !cand_uri.empty?
      raise "pui_candidate_record.uri must be a non-empty string for #{uri}"
    end
    unless cand_uri == input['uri']
      raise "pui_candidate_record uri mismatch for #{uri}: expected #{input['uri']}, got #{cand_uri}"
    end
    cand_type = candidate['jsonmodel_type']
    unless cand_type.is_a?(String) && !cand_type.empty?
      raise "pui_candidate_record.jsonmodel_type must be a non-empty string for #{uri}"
    end
    unless cand_type == input['record_type']
      raise "pui_candidate_record jsonmodel_type mismatch for #{uri}: expected #{input['record_type']}, got #{cand_type}"
    end

    parsed_json = ASUtils.json_parse(pui_final['json'])
    raise "pui final json must parse to a Hash for #{uri}" unless parsed_json.is_a?(Hash)
    parsed_uri = parsed_json['uri']
    unless parsed_uri.is_a?(String) && !parsed_uri.empty?
      raise "pui final json uri must be a non-empty string for #{uri}"
    end
    unless parsed_uri == input['uri']
      raise "pui final json uri mismatch for #{uri}: expected #{input['uri']}, got #{parsed_uri}"
    end
    parsed_type = parsed_json['jsonmodel_type']
    unless parsed_type.is_a?(String) && !parsed_type.empty?
      raise "pui final json jsonmodel_type must be a non-empty string for #{uri}"
    end
    unless parsed_type == input['record_type']
      raise "pui final json jsonmodel_type mismatch for #{uri}: expected #{input['record_type']}, got #{parsed_type}"
    end

    if parsed_json.has_key?('ancestors')
      raise "pui final json must not contain ancestors for #{uri}"
    end
    if parsed_json['instances'].is_a?(Array)
      parsed_json['instances'].each do |instance|
        next unless instance.is_a?(Hash)
        sub_container = instance['sub_container']
        if sub_container.is_a?(Hash) && sub_container['top_container'].is_a?(Hash)
          resolved_tc = sub_container['top_container']['_resolved']
          if resolved_tc.is_a?(Hash)
            if resolved_tc.has_key?('internal_note')
              raise "pui final json contains top_container internal_note for #{uri}"
            end
            resolved_cp = resolved_tc.dig('container_profile', '_resolved')
            if resolved_cp.is_a?(Hash) && resolved_cp.has_key?('notes')
              raise "pui final json contains container_profile notes for #{uri}"
            end
          end
        end
      end
    end

    expected_clean = reconstruct_expected_pui_final_json(candidate)
    validate_json_equal!(expected_clean, parsed_json, "pui final json for #{uri}")

    expected_json_string = ASUtils.to_json(expected_clean)
    unless pui_final['json'] == expected_json_string
      raise "pui final json serialization mismatch for #{uri}"
    end

    expected_fullrecord = extract_published_strings(expected_clean)
    unless pui_final['fullrecord_published'] == expected_fullrecord
      raise "pui final fullrecord_published mismatch for #{uri}"
    end
  end

  def validate_pui_routing_projection!(document, input)
    uri = input['uri']
    unless document.has_key?('_archivalist_pui_routing') && document['_archivalist_pui_routing'].is_a?(Hash)
      raise "missing _archivalist_pui_routing for #{uri}"
    end
    routing = document['_archivalist_pui_routing']
    if routing.keys.any? { |k| !k.is_a?(String) }
      raise "pui routing contains non-string keys for #{uri}: #{routing.keys.inspect}"
    end
    missing_keys = PUI_ROUTING_FIELDS - routing.keys
    unless missing_keys.empty?
      raise "pui routing missing required fields for #{uri}: #{missing_keys.sort.join(', ')}"
    end
    extra_keys = routing.keys - PUI_ROUTING_FIELDS
    unless extra_keys.empty?
      raise "pui routing contains unexpected fields for #{uri}: #{extra_keys.sort.join(', ')}"
    end
    unless routing['marker'] == 'pui_routing_metadata'
      raise "pui routing marker mismatch for #{uri}"
    end
    expected_id = "#{uri}#pui"
    unless routing['id'] == expected_id
      raise "pui routing id mismatch for #{uri}: expected #{expected_id}, got #{routing['id']}"
    end
    unless routing['pui_parent_id'] == uri
      raise "pui routing pui_parent_id mismatch for #{uri}: expected #{uri}, got #{routing['pui_parent_id']}"
    end
    expected_types = ['pui', "pui_#{input['record_type']}", 'pui_record', 'pui_only']
    unless routing['types'] == expected_types
      raise "pui routing types mismatch for #{uri}: expected #{expected_types.inspect}, got #{routing['types'].inspect}"
    end
  end

  def validate_pui_notes_projection!(document, input)
    uri = input['uri']
    unless document.has_key?('_archivalist_pui_notes') && document['_archivalist_pui_notes'].is_a?(Hash)
      raise "missing _archivalist_pui_notes for #{uri}"
    end
    notes_proj = document['_archivalist_pui_notes']
    if notes_proj.keys.any? { |k| !k.is_a?(String) }
      raise "pui notes contains non-string keys for #{uri}: #{notes_proj.keys.inspect}"
    end
    missing_keys = PUI_NOTES_FIELDS - notes_proj.keys
    unless missing_keys.empty?
      raise "pui notes missing required fields for #{uri}: #{missing_keys.sort.join(', ')}"
    end
    extra_keys = notes_proj.keys - PUI_NOTES_FIELDS
    unless extra_keys.empty?
      raise "pui notes contains unexpected fields for #{uri}: #{extra_keys.sort.join(', ')}"
    end
    unless notes_proj['marker'] == 'pui_notes_metadata'
      raise "pui notes marker mismatch for #{uri}"
    end
    record = input['record']
    raw_notes = record.is_a?(Hash) ? record['notes'] : nil
    if raw_notes.nil? || raw_notes == false
      unless notes_proj.key?('notes_published') && notes_proj['notes_published'].nil?
        raise "pui notes_published must be nil when record notes is absent or false for #{uri}"
      end
    else
      unless notes_proj['notes_published'].is_a?(Array)
        raise "pui notes_published must be an array when record notes is present for #{uri}"
      end
      expected_notes = extract_published_strings(raw_notes)
      unless notes_proj['notes_published'] == expected_notes
        raise "pui notes_published mismatch for #{uri}: expected #{expected_notes.inspect}, got #{notes_proj['notes_published'].inspect}"
      end
    end
  end

  def validate_pui_availability_projection!(document, input)
    uri = input['uri']
    unless document.has_key?('_archivalist_pui_availability') && document['_archivalist_pui_availability'].is_a?(Hash)
      raise "missing _archivalist_pui_availability for #{uri}"
    end
    proj = document['_archivalist_pui_availability']
    if proj.keys.any? { |k| !k.is_a?(String) }
      raise "pui availability contains non-string keys for #{uri}: #{proj.keys.inspect}"
    end
    missing_keys = PUI_AVAILABILITY_FIELDS - proj.keys
    unless missing_keys.empty?
      raise "pui availability missing required fields for #{uri}: #{missing_keys.sort.join(', ')}"
    end
    extra_keys = proj.keys - PUI_AVAILABILITY_FIELDS
    unless extra_keys.empty?
      raise "pui availability contains unexpected fields for #{uri}: #{extra_keys.sort.join(', ')}"
    end
    unless proj['marker'] == 'pui_availability_metadata'
      raise "pui availability marker mismatch for #{uri}"
    end
    unless proj['uri'] == uri
      raise "pui availability uri mismatch for #{uri}: expected #{uri}, got #{proj['uri']}"
    end
    record_type = input['record_type']
    unless proj['primary_type'] == record_type
      raise "pui availability primary_type mismatch for #{uri}: expected #{record_type}, got #{proj['primary_type']}"
    end
    unless input.has_key?('record_inheritance_has_type') && (input['record_inheritance_has_type'] == true || input['record_inheritance_has_type'] == false)
      raise "record_inheritance_has_type must be explicitly boolean for #{uri}"
    end
    expected_inheritance = input['record_inheritance_has_type']
    unless proj['record_inheritance_has_type'] == expected_inheritance
      raise "pui availability record_inheritance_has_type mismatch for #{uri}: expected #{expected_inheritance}, got #{proj['record_inheritance_has_type']}"
    end

    expected_additions = self.class.compute_pui_conditional_additions(record_type, expected_inheritance)
    unless proj['conditional_additions'] == expected_additions
      raise "pui availability conditional_additions mismatch for #{uri}: expected #{expected_additions.inspect}, got #{proj['conditional_additions'].inspect}"
    end

    record = input['record']
    unless record.is_a?(Hash)
      raise "record must be a Hash for #{uri}"
    end
    if record_type == 'top_container'
      tc_override = proj['top_container_override']
      unless tc_override.is_a?(Hash)
        raise "pui availability top_container_override must be a hash for #{uri}"
      end
      if tc_override.keys.any? { |k| !k.is_a?(String) }
        raise "pui availability top_container_override contains non-string keys for #{uri}: #{tc_override.keys.inspect}"
      end
      missing_tc = TOP_CONTAINER_OVERRIDE_FIELDS - tc_override.keys
      unless missing_tc.empty?
        raise "pui availability top_container_override missing required fields for #{uri}: #{missing_tc.sort.join(', ')}"
      end
      extra_tc = tc_override.keys - TOP_CONTAINER_OVERRIDE_FIELDS
      unless extra_tc.empty?
        raise "pui availability top_container_override contains unexpected fields for #{uri}: #{extra_tc.sort.join(', ')}"
      end
      expected_publish = record['is_linked_to_published_record']
      unless tc_override['publish'] == expected_publish
        raise "pui availability top_container_override publish mismatch for #{uri}: expected #{expected_publish.inspect}, got #{tc_override['publish'].inspect}"
      end
      unless tc_override['additions'] == ['pui_container', 'pui']
        raise "pui availability top_container_override additions mismatch for #{uri}: expected ['pui_container', 'pui'], got #{tc_override['additions'].inspect}"
      end
    else
      unless proj['top_container_override'].nil?
        raise "pui availability top_container_override must be nil for non-top_container #{uri}"
      end
    end
  end

  def reconstruct_expected_pui_final_json(candidate)
    expected = deep_copy_json_value(candidate)
    expected.delete('ancestors')
    if expected['instances'].is_a?(Array)
      expected['instances'].each do |instance|
        next unless instance.is_a?(Hash)
        sub_container = instance['sub_container']
        if sub_container.is_a?(Hash) && sub_container['top_container'].is_a?(Hash)
          top_container = sub_container['top_container']
          if top_container['_resolved'].is_a?(Hash)
            top_container['_resolved'].delete('internal_note')
            if top_container['_resolved']['container_profile'].is_a?(Hash) &&
               top_container['_resolved']['container_profile']['_resolved'].is_a?(Hash)
              top_container['_resolved']['container_profile']['_resolved'].delete('notes')
            end
          end
        end
      end
    end
    expected
  end

  def extract_published_strings(doc)
    if defined?(IndexerCommon) && IndexerCommon.respond_to?(:extract_string_values)
      IndexerCommon.extract_string_values(doc, :published_only)
    else
      extract_string_values_published_only(doc)
    end
  end

  def extract_string_values_published_only(doc)
    return [doc] if doc.is_a?(String)
    published_queue = doc.is_a?(Array) ? doc.flatten : [doc]
    published_strings = []

    while !published_queue.empty?
      curr = published_queue.pop

      if curr.is_a?(Hash) && curr.has_key?('publish') && !curr['publish']
        next
      end

      if curr.is_a?(Hash)
        curr.each do |key, val|
          if IndexerCommonConfig.fullrecord_excludes.include?(key.to_s) || key.to_s =~ /_enum_s$/
            next
          elsif val.is_a?(String)
            published_strings.push(val)
          elsif val.is_a?(Hash)
            published_queue.push(val)
          elsif val.is_a?(Array)
            val.flatten.each do |v|
              if v.is_a?(String)
                published_strings.push(v)
              elsif v.is_a?(Hash)
                published_queue.push(v)
              end
            end
          end
        end
      end
    end

    published_strings
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
