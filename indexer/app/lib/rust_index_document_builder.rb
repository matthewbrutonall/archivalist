require 'net/http'
require 'uri'
require 'json'
require 'set'
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
        expected_fields << 'dynamic_enum_and_ark_metadata'
        unless document['_archivalist_rust_fields'] == expected_fields
          raise "response enrichment marker mismatch for #{uri}"
        end
        validate_dynamic_enum_and_ark_projection!(document, input)
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
    unless expected.class == actual.class
      raise "#{path} type mismatch: expected #{expected.class}, got #{actual.class}"
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
        validate_json_equal!(expected[key], actual[key], "#{path}.#{key}")
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

  def perform_request(request)
    http = Net::HTTP.new(@url.host, @url.port)
    http.open_timeout = @timeout_seconds
    http.read_timeout = @timeout_seconds
    http.write_timeout = @timeout_seconds if http.respond_to?(:write_timeout=)
    http.use_ssl = @url.scheme == 'https'
    http.start {|connection| connection.request(request)}
  end
end
