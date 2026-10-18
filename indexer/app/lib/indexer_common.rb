require 'ashttp'
require 'uri'
require 'json'
require 'fileutils'
require 'aspace_i18n'
require 'set'

require 'asutils'
require 'jsonmodel'
require 'jsonmodel_client'
require 'config/config-distribution'
require 'record_inheritance'

require_relative 'index_batch'
require_relative 'indexer_common_config'
require_relative 'indexer_timing'
require_relative 'fake_solr_timeout_response'
require_relative 'rust_index_document_builder'

class IndexerCommon

  include JSONModel

  @@record_types = IndexerCommonConfig.record_types

  @@global_types = IndexerCommonConfig.global_types

  @@records_with_children = []
  @@init_hooks = []

  @@resolved_attributes = IndexerCommonConfig.resolved_attributes

  @@paused_until = Time.now

  # Solr stores deletes and applies them at commit time.  If you go a long time
  # between commits, this can cause OOM issues and/or long commit times as
  # thousands of deletes are finally applied.
  #
  # To avoid this, we'll force a Solr commit when we hit a certain number of deletes.
  #
  # Fun fact: our solrconfig.xml has a <maxPendingDeletes> set, but it doesn't
  # appear to do anything anymore.  There's a JIRA suggesting the feature did
  # exist at some point: https://issues.apache.org/jira/browse/SOLR-310.
  #
  # So we're essentially reimplementing that feature here.
  MAX_PENDING_DELETES = AppConfig.has_key?(:indexer_max_pending_deletes) ? AppConfig[:indexer_max_pending_deletes] : 10000

  # Our best estimate of the number of deletes we have queued up.  It's an
  # estimate because sometimes we delete by query and don't know exactly how
  # many hits there will be.
  @@pending_delete_estimate = java.util.concurrent.atomic.AtomicLong.new(0)

  def self.add_indexer_initialize_hook(&block)
    @@init_hooks << block
  end

  def self.add_attribute_to_resolve(attr)
    @@resolved_attributes.push(attr) unless @@resolved_attributes.include?(attr)
  end

  def resolved_attributes
    @@resolved_attributes
  end

  def record_types
    @@record_types
  end

  # This is to pause the indexer.
  # Duration is given in seconds.
  def self.pause(duration = 900 )
    @@paused_until = Time.now + duration
  end

  def self.paused?
    @@paused_until > Time.now
  end


  def initialize(backend_url)
    @backend_url = backend_url
    @document_prepare_hooks = []
    @extra_documents_hooks = []
    @delete_hooks = []
    @batch_hooks = []
    @current_session = nil

    while true
      begin
        JSONModel::init(:client_mode => true, :url => @backend_url)
        break
      rescue
        Log.error("Connection to backend failed (#{$!}).  Retrying...")
        sleep(5)
      end
    end

    # Force load up front
    self.enum_fields

    configure_doc_rules

    @@init_hooks.each do |hook|
      hook.call(self)
    end

    final_doc_rules
  end

  def self.generate_years_for_date_range(begin_date, end_date)
    return [] unless begin_date

    end_date ||= begin_date

    b = begin_date.scan(/\A[0-9]{1,4}/).first
    e = end_date.scan(/\A[0-9]{1,4}/).first

    if b && e
      (b..e).to_a
    else
      []
    end
  end


  def self.generate_permutations_for_identifier(identifer)
    return [] if identifer.nil?

    [
      identifer,
      identifer.gsub(/[[:punct:]]+/, " "),
      identifer.gsub(/[[:punct:] ]+/, ""),
      identifer.scan(/([0-9]+|[^0-9]+)/).flatten(1).join(" ")
    ].uniq
  end


  # Isolate leading alpha and numeric values to create a sortable string
  def self.generate_sort_string_for_identifier(identifier, size = 255)
    letters, numbers, rest = identifier.scan(/([^0-9]*)([0-9]*)(.*)/)[0]
    letters.strip.ljust(size).gsub(' ', '#') + numbers.strip.rjust(size).gsub(' ', '0') + rest.strip.ljust(size)
  end


  def self.extract_string_values(doc, *opts)
    return doc, doc if doc.is_a?(String)

    if doc.is_a?(Array)
      published_queue = doc.flatten
    else
      published_queue = [doc]
    end

    extract_unpublished = !opts.include?(:published_only)

    unpublished_queue = []
    published_strings = []
    unpublished_strings = []
    published_done = false

    [
      [published_queue, published_strings],
      [unpublished_queue, unpublished_strings]
    ].each do |queue, strings|

      while !queue.empty?
        doc = queue.pop

        if (!published_done && doc.has_key?("publish") && !doc["publish"])
          if extract_unpublished
            unpublished_queue.push(doc)
          end
          next
        end

        doc.each do |key, val|
          if IndexerCommonConfig.fullrecord_excludes.include?(key) || key =~ /_enum_s$/
            next # ignored
          elsif val.is_a?(String)
            strings.push(val)
          elsif val.is_a?(Hash)
            queue.push(val)
          elsif val.is_a?(Array)
            val.flatten.each do |v|
              if v.is_a?(String)
                strings.push(v)
              elsif v.is_a?(Hash)
                queue.push(v)
              end
            end
          end
        end
      end

      if extract_unpublished
        published_done = true
      else
        break
      end

    end

    if extract_unpublished
      return published_strings, unpublished_strings
    else
      return published_strings
    end
  end


  def build_fullrecord(doc, record)
    # 'fullrecord' only contains unpublished text at this stage, but 'fullrecord_published'
    # will be merged into it by Solr using copyField
    doc['fullrecord_published'], doc['fullrecord'] = IndexerCommon.extract_string_values(record['record'])
  end

  def add_agents(doc, record)
    if record['record']['linked_agents']
      # index all linked agents first
      doc['agents'] = record['record']['linked_agents'].collect {|link| link['_resolved']['display_name']['sort_name']}
      doc['agent_uris'] = record['record']['linked_agents'].collect {|link| link['ref']}

      # only published agents
      doc['published_agents'] = []
      doc['published_agent_uris'] = []
      record['record']['linked_agents'].each do |link|
        if link['_resolved']['publish']
          doc['published_agents'] << link['_resolved']['display_name']['sort_name']
          doc['published_agent_uris'] << link['ref']
        end
      end

      # index the creators only
      creators = record['record']['linked_agents'].select {|link| link['role'] === 'creator'}
      doc['creators'] = creators.collect {|link| link['_resolved']['display_name']['sort_name']} if not creators.empty?
    end
  end

  def add_subjects(doc, record)
    if record['record']['subjects']
      doc['subjects'] = record['record']['subjects'].map {|s| s['_resolved']['title']}.compact
      doc['subject_uris'] = record['record']['subjects'].collect {|link| link['ref']}
    end
  end


  def add_subjects_subrecord(doc, record, subrecord, type = 'subjects')
    doc['subjects'] ||= []
    doc['subject_uris'] ||= []

    if record['record'][subrecord]
      record['record'][subrecord].each do |sr|
        next unless sr[type]

        doc['subjects'].concat(sr[type].map {|s| s['_resolved']['title']}.compact)
        doc['subject_uris'].concat(sr[type].collect {|link| link['ref']})
      end
    end
  end


  def add_audit_info(doc, record)
    ['created_by', 'last_modified_by', 'user_mtime', 'system_mtime', 'create_time'].each do |f|
      doc[f] = record['record'][f] if record['record'].has_key?(f)
    end
  end


  def add_notes(doc, record)
    if record['record']['notes']
      # 'notes' only contains unpublished notes at this stage, but 'notes_published'
      # will be merged into it by Solr using copyField
      doc['notes_published'], doc['notes'] = IndexerCommon.extract_string_values(record['record']['notes'])
    end
  end


  def add_years(doc, record)
    if record['record']['dates']
      doc['years'] = []
      record['record']['dates'].each do |date|
        doc['years'] += IndexerCommon.generate_years_for_date_range(date['begin'], date['end'])
      end
      unless doc['years'].empty?
        doc['years'] = doc['years'].sort.uniq
        doc['year_sort'] = doc['years'].first.rjust(4, '0') + doc['years'].last.rjust(4, '0')
      end
      dates = record['record']['dates']
      display_dates = dates.select {|date| date['date_type'] == 'inclusive'}
      display_dates = dates if display_dates.empty?
      doc['dates'] = []
      display_dates.each do |date|
        if date['expression']
          doc['dates'] << date['expression']
        elsif date['date_type'] === "single"
          doc['dates'] << date['begin']
        elsif date['date_type']
          doc['dates'] << "#{date['begin']} - #{date['end']}"
        end
      end
    end
  end


  def add_level(doc, record)
    if record['record'].has_key? 'level'
      doc['level'] = (record['record']['level'] === 'otherlevel') ? record['record']['other_level'] : record['record']['level']
    end
  end


  def add_summary(doc, record)
    if record['record'].has_key?('notes') && record['record']['notes'].is_a?(Array)
      notes = record['record']['notes']
      abstract = notes.find {|note| note['type'] == 'abstract'}
      if abstract
        doc['summary'] = abstract['content'].join("\n")
      else
        scopecontent = notes.find {|note| note['type'] == 'scopecontent'}
        if scopecontent && scopecontent.has_key?('subnotes')
          doc['summary'] = scopecontent['subnotes'].map {|sn| sn['content']}.join("\n")
        end
      end
    end
  end


  def add_extents(doc, record)
    if record['record']['extents']
      extents = record['record']['extents']
      display_extents = extents.select {|extent| extent['portion'] == 'whole'}
      display_extents = extents if display_extents.empty?
      doc['extents'] = []
      display_extents.each do |extent|
        doc['extents'] << "#{extent['number']} --- #{extent['extent_type']}"
      end
    end
  end


  def enum_fields
    return @enum_fields if @enum_fields

    enum_fields = []
    queue = JSONModel.models.map {|_, model| model.schema['properties']}.flatten.uniq

    while !queue.empty?
      elt = queue.shift

      if elt.is_a?(Hash)
        elt.each do |k, v|
          if v.is_a?(Hash)
            enum_fields.push(k) if v['dynamic_enum'] || v.dig('items', 'dynamic_enum')
          end
          queue << v
        end
      elsif elt.is_a?(Array)
        queue.concat(elt)
      end
    end

    enum_fields.delete('items') # not an enum, creeps in through dynamic enum lists
    @enum_fields = enum_fields.uniq
  end

  def trim_ark_value(s)
    s.gsub(/\A.*ark:/, 'ark:')
  end

  def add_arks(doc, record)
    return unless AppConfig[:arks_enabled]

    if arks = record['record']['ark_name']
      doc['ark_name'] = ([arks.fetch('current', nil)] + arks.fetch('previous')).compact.map {|s| trim_ark_value(s)}
    end
  end

  def clean_dynamic_enum_fields!(doc)
    doc.delete_if { |k, _| k.is_a?(String) && k.end_with?('_enum_s') }
  end

  def strict_json_equal?(expected, actual)
    return false unless expected.class == actual.class

    case expected
    when Hash
      return false unless expected.length == actual.length
      expected_keys = expected.keys.map(&:to_s).sort
      actual_keys = actual.keys.map(&:to_s).sort
      return false unless expected_keys == actual_keys

      expected.all? do |k, v|
        act_v = actual.fetch(k) { actual.fetch(k.to_s, nil) }
        strict_json_equal?(v, act_v)
      end
    when Array
      return false unless expected.length == actual.length
      expected.each_with_index.all? do |exp_val, idx|
        strict_json_equal?(exp_val, actual[idx])
      end
    else
      expected == actual
    end
  end

  def valid_dynamic_enum_and_ark_projection?(doc, record)
    return false unless doc.is_a?(Hash)
    record_data = (record.is_a?(Hash) && record['record'].is_a?(Hash)) ? record['record'] : (record.is_a?(Hash) ? record : nil)
    return false unless record_data.is_a?(Hash)

    # 1. Validate dynamic enums
    expected_enums = compute_expected_dynamic_enums(record_data, enum_fields)
    doc_enum_keys = doc.keys.select { |k| k.is_a?(String) && k.end_with?('_enum_s') }

    return false unless doc_enum_keys.sort == expected_enums.keys.sort

    expected_enums.each do |key, expected_vals|
      actual_vals = doc[key]
      return false unless actual_vals.is_a?(Array)
      return false unless strict_json_equal?(expected_vals, actual_vals)
    end

    # 2. Validate ARKs
    arks_enabled = !!AppConfig[:arks_enabled]
    if !arks_enabled
      return false if doc.has_key?('ark_name')
    else
      raw_arks = record_data['ark_name']
      if raw_arks.nil?
        return false if doc.has_key?('ark_name')
      else
        return false unless raw_arks.is_a?(Hash)
        current = raw_arks['current']
        previous = raw_arks['previous']
        return false unless previous.is_a?(Array)
        return false if current && !current.is_a?(String)
        return false if previous.any? { |p| !p.is_a?(String) }

        expected_arks = ([current] + previous).compact.map { |s| trim_ark_value(s) }
        actual_arks = doc['ark_name']
        return false unless actual_arks.is_a?(Array)
        return false unless strict_json_equal?(expected_arks, actual_arks)
      end
    end

    true
  end

  def valid_accession_date_projection?(doc, record)
    return false unless doc.is_a?(Hash)
    record_data = (record.is_a?(Hash) && record['record'].is_a?(Hash)) ? record['record'] : (record.is_a?(Hash) ? record : nil)
    return false unless record_data.is_a?(Hash)

    raw_date = record_data['accession_date']
    return false unless raw_date.is_a?(String)

    parsed_year = begin
      Date.parse(raw_date).year
    rescue ArgumentError, TypeError
      nil
    end
    return false if parsed_year.nil?

    actual_date = doc['accession_date']
    actual_year = doc['accession_date_year']
    return false unless actual_date.is_a?(String)
    return false unless actual_year.is_a?(Integer)
    return false unless actual_year == parsed_year

    expected_unknown = I18n.t('accession.accession_date_unknown')
    return false unless expected_unknown.is_a?(String) && !expected_unknown.empty?

    if raw_date == '9999-12-31'
      return false unless actual_date == expected_unknown
    else
      return false unless actual_date == raw_date
    end

    expected_published, expected_unpublished = IndexerCommon.extract_string_values(record_data)
    if raw_date == '9999-12-31'
      expected_suffix = "#{expected_unknown} "
      expected_unpublished = expected_unpublished + [expected_suffix]
    end

    return false unless doc['fullrecord_published'] == expected_published
    return false unless doc['fullrecord'] == expected_unpublished

    true
  end

  def compute_expected_dynamic_enums(record_data, fields = enum_fields)
    return {} unless record_data.is_a?(Hash)

    expected = {}
    found_keys = Set.new

    ASUtils.search_nested(record_data, fields, ['_resolved']) do |field, field_value|
      key = "#{field}_enum_s"
      expected[key] ||= Set.new
      expected[key] << field_value
      found_keys << key
    end

    ASUtils.search_nested(record_data, ['items'], ['_resolved']) do |_field, field_value|
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


  def configure_doc_rules
    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if rust_fields.include?('dynamic_enum_and_ark_metadata') && valid_dynamic_enum_and_ark_projection?(doc, record)
        # Accelerated by Rust sidecar: skip native dynamic enum traversal
      else
        doc['_archivalist_rust_fields'].delete('dynamic_enum_and_ark_metadata') if doc['_archivalist_rust_fields'].is_a?(Array)
        clean_dynamic_enum_fields!(doc)
        found_keys = Set.new

        ASUtils.search_nested(record["record"], enum_fields, ['_resolved']) do |field, field_value|
          key = "#{field}_enum_s"

          doc[key] ||= Set.new
          doc[key] << field_value

          found_keys << key
        end

        ASUtils.search_nested(record["record"], ['items'], ['_resolved']) do |field, field_value|
          if field_value.is_a?(Hash) && field_value.key?('type')
            doc['type_enum_s'] ||= Set.new
            doc['type_enum_s'] << field_value.fetch('type')
            found_keys << 'type_enum_s'
          end
        end

        # Turn our sets back into regular arrays so they serialize out to JSON correctly
        found_keys.each do |key|
          doc[key] = doc[key].to_a.flatten
        end
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if doc['primary_type'] == 'archival_object' && !rust_fields.include?('resource_archival_object_metadata')
        doc['resource'] = record['record']['resource']['ref'] if record['record']['resource']
        doc['title'] = record['record']['display_string']
        doc['identifier'] = record['record']['component_id']
        doc['component_id'] = record['record']['component_id']
        doc['ref_id'] = record['record']['ref_id']
        doc['slug'] = record['record']['slug']
        doc['is_slug_auto'] = record['record']['is_slug_auto']
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      unless rust_fields.include?('common_metadata')
        add_subjects(doc, record)
        add_agents(doc, record)
        add_audit_info(doc, record)
        add_notes(doc, record)
        add_years(doc, record)
        add_level(doc, record)
        add_summary(doc, record)
        add_extents(doc, record)
      end
      if rust_fields.include?('dynamic_enum_and_ark_metadata') && valid_dynamic_enum_and_ark_projection?(doc, record)
        # Accelerated by Rust sidecar: skip native add_arks
      else
        doc.delete('ark_name')
        add_arks(doc, record)
      end
    }

    add_document_prepare_hook {|doc, record|
      if doc['primary_type'] == 'accession'
        rust_fields = Array(doc['_archivalist_rust_fields'])
        if rust_fields.include?('accession_date_metadata') && valid_accession_date_projection?(doc, record)
          # Accelerated by Rust sidecar: skip native accession date handling
        else
          doc['_archivalist_rust_fields'].delete('accession_date_metadata') if doc['_archivalist_rust_fields'].is_a?(Array)
          build_fullrecord(doc, record)
          date = record['record']['accession_date']
          if date == '9999-12-31'
            unknown = I18n.t('accession.accession_date_unknown')
            doc['accession_date'] = unknown
            doc['fullrecord'] ||= ''
            doc['fullrecord'] << unknown + ' '
          else
            doc['accession_date'] = date
          end
          doc['accession_date_year'] = Date.parse(date).year
        end
        unless rust_fields.include?('accession_digital_object_metadata')
          doc['identifier'] = (0...4).map {|i| record['record']["id_#{i}"]}.compact.join("-")
          doc['title'] = record['record']['display_string']

          doc['acquisition_type'] = record['record']['acquisition_type']
          doc['resource_type'] = record['record']['resource_type']
          doc['restrictions_apply'] = record['record']['restrictions_apply']
          doc['access_restrictions'] = record['record']['access_restrictions']
          doc['use_restrictions'] = record['record']['use_restrictions']
          doc['related_resource_uris'] = record['record']['related_resources'].
                                            collect { |resource| resource["ref"] }.
                                            compact.uniq

          doc['related_accession_uris'] = record['record']['related_accessions'].
                                             collect { |accession| accession["ref"] }.
                                             compact.uniq

          doc['slug'] = record['record']['slug']
          doc['is_slug_auto'] = record['record']['is_slug_auto']
          if cm = record['record']['collection_management']
            doc['processing_priority'] = cm['processing_priority']
            doc['processors'] = cm['processors']
          end
        end
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if doc['primary_type'] == 'subject' && !rust_fields.include?('agent_subject_metadata')
        doc['source'] = record['record']['source']
        doc['first_term_type'] = record['record']['terms'][0]['term_type']
        doc['publish'] = record['record']['publish'] && record['record']['is_linked_to_published_record']
        doc['slug'] = record['record']['slug']
        doc['is_slug_auto'] = record['record']['is_slug_auto']
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if record['record'].has_key?('used_within_repositories') &&
         !rust_fields.include?('agent_subject_metadata')
        doc['used_within_repository'] = record['record']['used_within_repositories']
        doc['used_within_published_repository'] = record['record']['used_within_published_repositories']
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if doc['primary_type'] == 'repository' &&
         !rust_fields.include?('repository_event_location_metadata')
        doc['repository'] = doc["id"]
        doc['title'] = record['record']['repo_code']
        doc['title_sort'] = clean_for_sort(record['record']['display_string'])
        doc['repo_sort'] = record['record']['display_string']
        doc['slug'] = record['record']['slug']
        doc['is_slug_auto'] = record['record']['is_slug_auto']
        doc['position_int_sort'] = record['record']['position']
      end
    }

    add_document_prepare_hook {|doc, record|
      if doc['primary_type'] == 'location'
        rust_fields = Array(doc['_archivalist_rust_fields'])
        unless rust_fields.include?('repository_event_location_metadata')
          if record['record'].has_key? 'temporary'
            doc['temporary'] = record['record']['temporary']
          end
          doc['building'] = record['record']['building']
          doc['floor'] = record['record']['floor']
          doc['room'] = record['record']['room']
          doc['area'] = record['record']['area']
        end
        if record['record']['owner_repo'] &&
           (!rust_fields.include?('repository_event_location_metadata') ||
            !doc.has_key?('owner_repo_display_string_u_ssort'))
          repo = JSONModel::HTTP.get_json(record['record']['owner_repo']['ref'])
          doc['owner_repo_uri_u_sstr'] = record['record']['owner_repo']['ref']
          doc['owner_repo_display_string_u_ssort'] = repo["repo_code"]
        end
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if doc['primary_type'] == 'digital_object_component' &&
         !rust_fields.include?('component_classification_metadata')
        doc['digital_object'] = record['record']['digital_object']['ref']
        doc['digital_object_id'] = record['record']['component_id']
        doc['identifier'] = record['record']['component_id']
        doc['title'] = record['record']['display_string']
        doc['slug'] = record['record']['slug']
        doc['is_slug_auto'] = record['record']['is_slug_auto']
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if doc['primary_type'] == 'resource' && !rust_fields.include?('resource_archival_object_metadata')
        doc['finding_aid_title'] = record['record']['finding_aid_title']
        doc['finding_aid_filing_title'] = record['record']['finding_aid_filing_title']
        doc['identifier'] = (0...4).map {|i| record['record']["id_#{i}"]}.compact.join("-")
        doc['resource_type'] = record['record']['resource_type']
        doc['level'] = record['record']['level']
        doc['restrictions'] = record['record']['restrictions']
        doc['ead_id'] = record['record']['ead_id']
        doc['finding_aid_status'] = record['record']['finding_aid_status']
        doc['related_accession_uris'] = record['record']['related_accessions'].
                                           collect {|accession| accession["ref"]}.
                                           compact.uniq
        doc['slug'] = record['record']['slug']
        doc['is_slug_auto'] = record['record']['is_slug_auto']
        if cm = record['record']['collection_management']
          doc['processing_priority'] = cm['processing_priority']
          doc['processors'] = cm['processors']
        end
      end

      if doc['primary_type'] == 'digital_object' && !rust_fields.include?('accession_digital_object_metadata')
        doc['digital_object_type'] = record['record']['digital_object_type']

        doc['digital_object_id'] = record['record']['digital_object_id']
        doc['identifier'] = record['record']['digital_object_id']
        doc['level'] = record['record']['level']
        doc['restrictions'] = record['record']['restrictions']
        doc['slug'] = record['record']['slug']
        doc['is_slug_auto'] = record['record']['is_slug_auto']

        doc['collection_uri_u_sstr'] = record['record']['collection'].map {|collection| collection['ref']}
        doc['linked_instance_uris'] = record['record']['linked_instances'].
                                         collect {|instance| instance["ref"]}.
                                         compact.uniq
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if doc['primary_type'] == 'repository' &&
         !rust_fields.include?('repository_event_location_metadata')
        doc['repository'] = doc["id"]
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if doc['primary_type'] == 'event' &&
         !rust_fields.include?('repository_event_location_metadata')
        doc['event_type'] = record['record']['event_type']
        doc['title'] = record['record']['event_type'] # adding this for emedded searches
        doc['outcome'] = record['record']['outcome']
        doc['linked_record_uris'] = record['record']['linked_records'].map { |c| c['ref'] }

        # ANW-1635: index linked record titles/display names so they are available in CSV output
        doc['linked_record_titles'] = record['record']['linked_records'].map do |rec|
          if    rec['_resolved']['jsonmodel_type'] == "agent_person"
            rec['_resolved']['display_name']['sort_name']
          elsif rec['_resolved']['jsonmodel_type'] == "agent_family"
            rec['_resolved']['display_name']['sort_name']
          elsif rec['_resolved']['jsonmodel_type'] == "agent_corporate_entity"
            rec['_resolved']['display_name']['sort_name']
          elsif rec['_resolved']['jsonmodel_type'] == "agent_software"
            rec['_resolved']['display_name']['sort_name']
          elsif rec['_resolved']['jsonmodel_type'] == "accession"
            rec['_resolved']['title']
          elsif rec['_resolved']['jsonmodel_type'] == "resource"
            rec['_resolved']['title']
          elsif rec['_resolved']['jsonmodel_type'] == "digital_object"
            rec['_resolved']['title']
          elsif rec['_resolved']['jsonmodel_type'] == "digital_object_component"
            rec['_resolved']['title']
          elsif rec['_resolved']['jsonmodel_type'] == "archival_object"
            rec['_resolved']['display_string']
          else
            "not_found"
          end
        end
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if ['agent_person', 'agent_family', 'agent_software', 'agent_corporate_entity'].include?(doc['primary_type']) &&
         !rust_fields.include?('agent_subject_metadata')
        record['record'].reject! { |rec| rec === 'agent_contacts' }
        doc['title'] = record['record']['display_name']['sort_name']

        authorized_name = record['record']['names'].find {|name| name['authorized']}

        has_conventions_dec = !record['record']['agent_conventions_declarations'].empty?
        has_agent_record_id = !record['record']['agent_record_identifiers'].empty?

        if has_agent_record_id
          primary_record_id = record['record']['agent_record_identifiers'].select do |ari|
            ari['primary_identifier'] == true
          end

          primary_record_id = primary_record_id.first
        else
          primary_record_id = nil
        end

        if has_conventions_dec
          conventions_dec = record['record']['agent_conventions_declarations'].first
        else
          conventions_dec = nil
        end

        if primary_record_id
          doc['authority_id'] = primary_record_id['record_identifier']
          doc['source'] = primary_record_id['source']
        elsif authorized_name
          doc['authority_id'] = authorized_name['authority_id']
          doc['source'] = authorized_name['source']
        end

        if conventions_dec && conventions_dec['name_rule']
          doc['rules'] = conventions_dec['name_rule']
        elsif authorized_name
          doc['rules'] = authorized_name['rules']
        end

        doc['linked_agent_roles'] = record['record']['linked_agent_roles']

        doc['related_agent_uris'] = ASUtils.wrap(record['record']['related_agents']).collect {|ra| ra['ref']}
        doc['slug'] = record['record']['slug']
        doc['is_slug_auto'] = record['record']['is_slug_auto']

        if record['record']['is_user']
          doc['is_user'] = true
          doc['types'] << 'agent_with_user'
        else
          doc['is_user'] = false
        end

        add_subjects_subrecord(doc, record, 'agent_functions')
        add_subjects_subrecord(doc, record, 'agent_occupations')
        add_subjects_subrecord(doc, record, 'agent_places')
        add_subjects_subrecord(doc, record, 'agent_topics')

        add_subjects_subrecord(doc, record, 'agent_functions', 'places')
        add_subjects_subrecord(doc, record, 'agent_occupations', 'places')
        add_subjects_subrecord(doc, record, 'agent_resources', 'places')
        add_subjects_subrecord(doc, record, 'agent_topics', 'places')

        # Assign the additional type of 'agent'
        doc['types'] << 'agent'
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      unless rust_fields.include?('resource_archival_object_metadata') ||
             rust_fields.include?('accession_digital_object_metadata') ||
             rust_fields.include?('agent_subject_metadata') ||
             rust_fields.include?('component_classification_metadata') ||
             rust_fields.include?('repository_event_location_metadata') ||
             rust_fields.include?('profile_assessment_metadata') ||
             rust_fields.include?('top_container_metadata') ||
             rust_fields.include?('job_metadata')
        doc['external_id'] = Array(record['record']['external_ids']).map do |eid|
          eid['external_id']
        end
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if ['classification', 'classification_term'].include?(doc['primary_type']) &&
         !rust_fields.include?('component_classification_metadata')
        doc['classification_path'] = ASUtils.to_json(record['record']['path_from_root'])
        doc['agent_uris'] = ASUtils.wrap(record['record']['creator']).collect {|agent| agent['ref']}
        doc['published_agent_uris'] = []
        if !record.dig(:record, :creator, :_resolved).nil?
          if record['record']['creator']['_resolved']['publish'] && !record['record']['creator']['ref'].nil?
            doc['published_agent_uris'] << record['record']['creator']['ref']
          end
        end
        doc['agents'] = ASUtils.wrap(record['record']['creator']).collect {|link| link['_resolved']['display_name']['sort_name']}
        doc['identifier_sort'] = IndexerCommon.generate_sort_string_for_identifier(record['record']['identifier'])
        doc['repo_sort'] = record['record']['repository']['_resolved']['display_string']
        doc['has_classification_terms'] = record['record']['has_classification_terms']
        doc['slug'] = record['record']['slug']
        doc['is_slug_auto'] = record['record']['is_slug_auto']
        doc['identifier'] = record['record']['identifier']
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if doc['primary_type'] == 'classification_term' &&
         !rust_fields.include?('component_classification_metadata')
        doc['classification'] = record['record']['classification']['ref']
      end
    }

    add_document_prepare_hook {|doc, record|
      if doc['primary_type'] == 'job'
        rust_fields = Array(doc['_archivalist_rust_fields'])
        report_type = record['record']['job']['report_type']
        doc['title'] = (report_type ? I18n.t("reports.#{report_type}.title", :default => report_type) :
          I18n.t("job.types.#{record['record']['job_type']}"))
        unless rust_fields.include?('job_metadata')
          doc['types'] << record['record']['job_type']
          doc['types'] << report_type
          doc['job_type'] = record['record']['job_type']
          doc['report_type'] = report_type
          doc['job_report_type'] = report_type || doc['job_type']
          doc['status'] = record['record']['status']
          doc['owner'] = record['record']['owner']
        end
        doc['time_submitted'] = Time.parse(record['record']['time_submitted']).getlocal if record['record']['time_submitted']
        doc['time_started'] = Time.parse(record['record']['time_started']).getlocal if record['record']['time_started']
        doc['time_finished'] = Time.parse(record['record']['time_finished']).getlocal if record['record']['time_finished']

        filenames = record['record']['job']['filenames'] || [record['record']['job']['filename']].compact
        doc['files'] = []
        doc['job_data'] = []
        files = JSONModel::HTTP::get_json("#{record['record']['uri']}/output_files")
        files.each do |file|
          job_id = record['record']['uri'].split('/').last
          link = "/jobs/#{job_id}/file/#{file}"
          doc['files'] << link
          filename = filenames.shift
          doc['job_data'] << (filename ? "input_file --- #{filename}" : "output_file --- #{link}")
        end
        unless record['record']['job'].is_a? String
          record['record']['job'].reject { |k, _v| ['jsonmodel_type', 'filenames', 'report_type'].include? k }.each do |k, v|
            doc['job_data'] << "#{k} --- #{v}"
          end
        end
        doc['queue_position'] = record['record']['queue_position'] unless rust_fields.include?('job_metadata')
      end
    }


    add_document_prepare_hook {|doc, record|
      records_with_classifications = ['resource', 'accession', 'digital_object']
      rust_fields = Array(doc['_archivalist_rust_fields'])

      if records_with_classifications.include?(doc['primary_type']) &&
         !rust_fields.include?('resource_archival_object_metadata') &&
         !rust_fields.include?('accession_digital_object_metadata') &&
         record['record']['classifications'].length > 0
        doc['classification_paths'] = record['record']['classifications'].map { |c| ASUtils.to_json(c['_resolved']['path_from_root']) }
        doc['classification_uris'] = record['record']['classifications'].map { |c| c['ref'] }
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if ['resource', 'archival_object', 'accession'].include?(doc['primary_type']) &&
         !rust_fields.include?('resource_archival_object_metadata') &&
         !rust_fields.include?('accession_digital_object_metadata') &&
         record['record']['instances'] && record['record']['instances'].length > 0
        doc['location_uris'] = record['record']['instances'].
                                  collect {|instance| instance["sub_container"]}.compact.
                                  collect {|sub_container| sub_container["top_container"]["_resolved"]}.compact.
                                  collect {|top_container| top_container["container_locations"]}.flatten.
                                  collect {|container_location| container_location["ref"]}.uniq
        doc['digital_object_uris'] = record['record']['instances'].
                                        collect {|instance| instance["digital_object"]}.compact.
                                        collect {|digital_object_instance| digital_object_instance["ref"]}.
                                        flatten.uniq
      end
    }


    # Index four-part IDs separately
    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      unless rust_fields.include?('resource_archival_object_metadata') ||
             rust_fields.include?('accession_digital_object_metadata') ||
             rust_fields.include?('agent_subject_metadata') ||
             rust_fields.include?('component_classification_metadata') ||
             rust_fields.include?('repository_event_location_metadata') ||
             rust_fields.include?('profile_assessment_metadata') ||
             rust_fields.include?('top_container_metadata') ||
             rust_fields.include?('job_metadata')
        four_part_id = (0..3).map {|n| record['record']["id_#{n}"]}.compact.join(" ")

        unless four_part_id.empty?
          doc['four_part_id'] = four_part_id
        end
      end
    }


    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if record['record']['jsonmodel_type'] == 'top_container' &&
         !rust_fields.include?('top_container_metadata')
        doc['title'] = record['record']['long_display_string']
        doc['display_string'] = record['record']['display_string']
        doc['type_u_ssort'] = record['record']['type']
        doc['notes'] = record['record']['internal_note']

        if record['record']['series']
          doc['series_uri_u_sstr'] = record['record']['series'].map {|series| series['ref']}
          doc['series_title_u_sstr'] = record['record']['series'].map {|series| series['display_string']}
          doc['series_level_u_sstr'] = record['record']['series'].map {|series| series['level_display_string']}
          doc['series_identifier_stored_u_sstr'] = record['record']['series'].map {|series| series['identifier']}
          doc['series_identifier_u_stext'] = record['record']['series'].map {|series|
            IndexerCommon.generate_permutations_for_identifier(series['identifier'])
          }.flatten

          record['record']['series'].select {|series| series['publish']}.each do |series|
            doc['published_series_uri_u_sstr'] ||= []
            doc['published_series_uri_u_sstr'] << series['ref']
            doc['published_series_title_u_sstr'] ||= []
            doc['published_series_title_u_sstr'] << series['display_string']
          end
        end

        if record['record']['collection']
          doc['collection_uri_u_sstr'] = record['record']['collection'].map {|collection| collection['ref']}
          doc['collection_display_string_u_sstr'] = record['record']['collection'].map {|collection| collection['display_string']}
          doc['collection_identifier_stored_u_sstr'] = record['record']['collection'].map {|collection| collection['identifier']}
          doc['collection_identifier_u_stext'] = record['record']['collection'].map {|collection|
            IndexerCommon.generate_permutations_for_identifier(collection['identifier'])
          }.flatten
        end

        if record['record']['container_profile']
          doc['container_profile_uri_u_sstr'] = record['record']['container_profile']['ref']
          doc['container_profile_display_string_u_sstr'] = record['record']['container_profile']['_resolved']['display_string']
        end

        if record['record']['container_locations'].length > 0
          doc['has_location_u_sbool'] = true
          doc['location_uri_u_sstr'] = []
          doc['location_uris'] = []
          doc['location_display_string_u_sstr'] = []
          doc['location_profile_display_string_u_sstr'] = []
          record['record']['container_locations'].each do |container_location|
            if container_location['status'] == 'current'
              doc['location_uri_u_sstr'] << container_location['ref']
              doc['location_uris'] << container_location['ref']
              doc['location_display_string_u_sstr'] << container_location['_resolved']['title']
              if container_location['_resolved']['location_profile']
                lp_resolved = container_location['_resolved']['location_profile']['_resolved']
                doc['location_profile_display_string_u_sstr'] << lp_resolved['display_string'] if lp_resolved
              end
            end
          end
          doc['location_profile_display_string_u_sstr'].uniq!
        else
          doc['has_location_u_sbool'] = false
        end
        doc['exported_u_sbool'] = record['record'].has_key?('exported_to_ils')
        doc['empty_u_sbool'] = record['record']['collection'].empty?

        if record['record']['indicator']
          doc['indicator_u_icusort'] = record['record']['indicator']
        end

        doc['top_container_u_typeahead_utext'] = record['record']['display_string'].gsub(/[^0-9A-Za-z]/, '').downcase
        doc['top_container_u_icusort'] = record['record']['display_string']
        doc['barcode_u_sstr'] = record['record']['barcode']
        doc['barcode_u_icusort'] = record['record']['barcode']
        doc['ils_holding_id_u_sstr'] = record['record']['ils_holding_id']

        doc['subcontainer_barcodes_u_sstr'] = record["record"]["subcontainer_barcodes"]
        doc['created_for_collection_u_sstr'] = record['record']['created_for_collection']
      end
    }


    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if ['resource', 'archival_object', 'accession'].include?(doc['primary_type']) &&
         !rust_fields.include?('resource_archival_object_metadata') &&
         !rust_fields.include?('accession_digital_object_metadata')
        # we no longer want the contents of containers to be indexed at the container's location
        doc.delete('location_uris')

        # index the top_container's linked via a sub_container
        ASUtils.wrap(record['record']['instances']).each {|instance|
          if instance['sub_container'] && instance['sub_container']['top_container']
            doc['top_container_uri_u_sstr'] ||= []
            doc['top_container_uri_u_sstr'] << instance['sub_container']['top_container']['ref']
            if instance['sub_container']['type_2']
              doc['child_container_u_sstr'] ||= []
              doc['child_container_u_sstr'] << "#{instance['sub_container']['type_2']} #{instance['sub_container']['indicator_2']} #{instance['sub_container']['barcode_2']}"
            end
            if instance['sub_container']['type_3']
              doc['grand_child_container_u_sstr'] ||= []
              doc['grand_child_container_u_sstr'] << "#{instance['sub_container']['type_3']} #{instance['sub_container']['indicator_3']}"
            end
          end
        }
      end
    }


    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if doc['primary_type'] == 'container_profile' &&
         !rust_fields.include?('profile_assessment_metadata')
        doc['title'] = record['record']['display_string']
        doc['title_sort'] = clean_for_sort(record['record']['display_string'])
        doc['display_string'] = record['record']['display_string']
        doc['note'] = record['record']['note']

        ['width', 'height', 'depth'].each do |property|
          doc["container_profile_#{property}_u_sstr"] = record['record'][property]
        end

        doc["container_profile_dimension_units_u_sstr"] = record['record']['dimension_units']

        doc['typeahead_sort_key_u_sort'] = record['record']['display_string']
      end
    }


    add_document_prepare_hook { |doc, record|
      if !self.instance_of?(PUIIndexer)
        # The PUI indexer makes its own call to build_fullrecord, so only call it here for realtime and periodic
        rust_fields = Array(doc['_archivalist_rust_fields'])
        build_fullrecord(doc, record) unless rust_fields.include?('fullrecord')
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if doc['primary_type'] == 'location_profile' &&
         !rust_fields.include?('profile_assessment_metadata')
        doc['title'] = record['record']['display_string']
        doc['title_sort'] = clean_for_sort(record['record']['display_string'])
        doc['display_string'] = record['record']['display_string']

        ['width', 'height', 'depth'].each do |property|
          doc["location_profile_#{property}_u_sstr"] = record['record'][property]
        end

        doc["location_profile_dimension_units_u_sstr"] = record['record']['dimension_units']

        doc['typeahead_sort_key_u_sort'] = record['record']['display_string']
      end

      if record['record']['location_profile'] &&
         !rust_fields.include?('repository_event_location_metadata')
        doc['location_profile_uri_u_sstr'] = record['record']['location_profile']['ref']
        doc['location_profile_display_string_u_ssort'] = record['record']['location_profile']['_resolved']['display_string']
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      unless rust_fields.include?('resource_archival_object_metadata') ||
             rust_fields.include?('accession_digital_object_metadata') ||
             rust_fields.include?('agent_subject_metadata') ||
             rust_fields.include?('component_classification_metadata') ||
             rust_fields.include?('repository_event_location_metadata') ||
             rust_fields.include?('profile_assessment_metadata') ||
             rust_fields.include?('top_container_metadata') ||
             rust_fields.include?('job_metadata')
        doc['ancestors'] = ASUtils.wrap(record['record']['ancestors']).map {|ancestor|
          ancestor.fetch('ref')
        }
      end
    }

    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      unless rust_fields.include?('resource_archival_object_metadata') ||
             rust_fields.include?('accession_digital_object_metadata') ||
             rust_fields.include?('agent_subject_metadata') ||
             rust_fields.include?('component_classification_metadata') ||
             rust_fields.include?('repository_event_location_metadata') ||
             rust_fields.include?('profile_assessment_metadata') ||
             rust_fields.include?('top_container_metadata') ||
             rust_fields.include?('job_metadata')
        ASUtils.wrap(record['record']['rights_statements']).each do |rights_statement|
          ASUtils.wrap(rights_statement['linked_agents']).each do |agent_link|
            doc['rights_statement_agent_uris'] ||= []
            doc['rights_statement_agent_uris'] << agent_link['ref']
          end
        end
      end
    }

    record_has_children('collection_management')
    @collection_management_extra_hook = proc {|record|
      docs = []

      cm = record['record']['collection_management']
      if cm
        parent_type = JSONModel.parse_reference(record['uri'])[:type]
        title = record['record']['title'] || record['record']['display_string']
        docs << {
          'id' => cm['uri'],
          'uri' => cm['uri'],
          'parent_id' => record['uri'],
          'parent_title' => title,
          'parent_type' => parent_type,
          'title' => title,
          'title_sort' => clean_for_sort(title),
          'types' => ['collection_management'],
          'primary_type' => 'collection_management',
          'json' => cm.to_json(:max_nesting => false),
          'processing_priority' => cm['processing_priority'],
          'processing_status' => cm['processing_status'],
          'processing_hours_total' => cm['processing_hours_total'],
          'processing_funding_source' => cm['processing_funding_source'],
          'processors' => cm['processors'],
          'suppressed' => record['record']['suppressed'],
          'repository' => get_record_scope(record['uri']),
          'created_by' => cm['created_by'],
          'last_modified_by' => cm['last_modified_by'],
          'system_mtime' => cm['system_mtime'],
          'user_mtime' => cm['user_mtime'],
          'create_time' => cm['create_time'],
        }
      end

      docs
    }
    add_extra_documents_hook(&@collection_management_extra_hook)


    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      if doc['primary_type'] == 'assessment' &&
         !rust_fields.include?('profile_assessment_metadata')
        doc['assessment_id'] = JSONModel.parse_reference(record['record']['uri']).fetch(:id)
        doc['title'] = record['record']['display_string']
        doc['display_string'] = record['record']['display_string']

        doc['assessment_record_uris'] = ASUtils.wrap(record['record']['records']).map {|r| r['ref']}
        doc['assessment_records'] = ASUtils.wrap(record['record']['records']).map {|r| r['_resolved']['display_string'] || r['_resolved']['title']}
        doc['assessment_record_types'] = ASUtils.wrap(record['record']['records']).map {|r| r['_resolved']['jsonmodel_type']}.uniq.sort
        doc['assessment_surveyor_uris'] = ASUtils.wrap(record['record']['surveyed_by']).map {|r| r['ref']}
        doc['assessment_surveyors'] = ASUtils.wrap(record['record']['surveyed_by']).map {|r| r['_resolved']['title']}
        doc['assessment_survey_begin'] = "#{record['record']['survey_begin']}T00:00:00Z"
        doc['assessment_survey_end'] = "#{record['record']['survey_end']}T00:00:00Z" if record['record']['survey_end']
        doc['assessment_review_required'] = record['record']['review_required'] || false
        doc['assessment_sensitive_material'] = record['record']['sensitive_material'] || false
        if (ASUtils.wrap(record['record']['reviewer']).length > 0)
          doc['assessment_reviewer_uris'] = ASUtils.wrap(record['record']['reviewer']).map {|r| r['ref']}
          doc['assessment_reviewers'] = ASUtils.wrap(record['record']['reviewer']).map {|r| r['_resolved']['title']}
        end
        doc['assessment_inactive'] = record['record']['inactive'] || false

        doc['assessment_survey_year'] = IndexerCommon.generate_years_for_date_range(record['record']['survey_begin'], record['record']['survey_end'])

        doc['assessment_collection_uris'] = ASUtils.wrap(record['record']['collections']).map {|r| r['ref']}
        doc['assessment_collections'] = ASUtils.wrap(record['record']['collections']).map {|r| r['_resolved']['display_string'] || r['_resolved']['title']}

        doc['assessment_completed'] = !record['record']['survey_end'].nil?

        doc['assessment_formats'] = record['record']['formats'].select {|r| r.has_key?('value')}.map {|r| r['label']}
        doc['assessment_ratings'] = record['record']['ratings'].select {|r| r.has_key?('value') || r.has_key?('note')}.map {|r| r['label']}
        doc['assessment_conservation_issues'] = record['record']['conservation_issues'].select {|r| r.has_key?('value')}.map {|r| r['label']}

        doc['title_sort'] = doc['assessment_id'].to_s.rjust(10, '0')
      end
    }


    add_document_prepare_hook {|doc, record|
      rust_fields = Array(doc['_archivalist_rust_fields'])
      unless rust_fields.include?('resource_archival_object_metadata') ||
             rust_fields.include?('accession_digital_object_metadata') ||
             rust_fields.include?('agent_subject_metadata') ||
             rust_fields.include?('component_classification_metadata') ||
             rust_fields.include?('repository_event_location_metadata') ||
             rust_fields.include?('profile_assessment_metadata') ||
             rust_fields.include?('top_container_metadata') ||
             rust_fields.include?('job_metadata')
        doc['langcode'] ||= []
        if record['record'].has_key?('lang_materials') and record['record']['lang_materials'].is_a?(Array)
          record['record']['lang_materials'].each { |langmaterial|
            if langmaterial.has_key?('language_and_script')
              doc['langcode'].push(langmaterial['language_and_script']['language'])
            end
          }
          doc['langcode'].uniq!
        end
      end
    }

    # Internal handoff metadata must never reach Solr or plugin hooks.
    add_document_prepare_hook {|doc, _record|
      doc.delete_if {|k, _| k.start_with?('_archivalist_')}
    }
  end


  def add_document_prepare_hook(&block)
    @document_prepare_hooks << block
  end


  def record_has_children(record_type)
    @@records_with_children << record_type.to_s
  end


  def records_with_children
    @@records_with_children || []
  end


  def add_extra_documents_hook(&block)
    @extra_documents_hooks << block
  end


  def add_batch_hook(&block)
    @batch_hooks << block
  end


  def add_delete_hook(&block)
    @delete_hooks << block
  end


  def solr_url
    URI.parse(AppConfig[:solr_url])
  end


  def do_http_request(url, req)
    req['X-ArchivesSpace-Session'] = @current_session

    opts = {
      :read_timeout => AppConfig[:indexer_solr_timeout_seconds].to_i
    }

    ASHTTP.start_uri(url, opts) do |http|
      http.request(req)
    end
  rescue Timeout::Error
    FakeSolrTimeoutResponse.new(req)
  end


  def reset_session
    @current_session = nil
  end


  def login
    if @current_session
      return @current_session
    end

    username = AppConfig[:search_username]
    password = AppConfig[:search_user_secret]

    url = URI.parse(@backend_url + "/users/#{username}/login")

    request = Net::HTTP::Post.new(url.request_uri)
    request.set_form_data("expiring" => "false",
                          "password" => password)

    response = do_http_request(url, request)

    if response.code == '200'
      auth = ASUtils.json_parse(response.body)

      @current_session = auth['session']
      JSONModel::HTTP.current_backend_session = auth['session']

    else
      raise "Authentication to backend failed: #{response.body}"
    end
  end


  def get_record_scope(uri)
    JSONModel.parse_reference(uri)[:repository] || "global"
  end


  def is_repository_unpublished?(uri, values)
    repo_id = get_record_scope(uri)

    return false if (repo_id == "global")
    return false if values['repository'].nil? || values['repository']['_resolved'].nil?

    values['repository']['_resolved']['publish'] == false
  end


  def delete_records(records, opts = {})
    return if records.empty?

    req = Net::HTTP::Post.new("#{solr_url.path}/update")
    req['Content-Type'] = 'application/json'

    # Delete the ID plus any documents that were the child of that ID
    delete_request = {:delete => records.map {|id|
        [{"id" => id},
         {'query' => opts.fetch(:parent_id_field, 'parent_id') + ":\"#{id}\""}]}.flatten(1)
    }

    @delete_hooks.each do |hook|
      hook.call(records, delete_request)
    end

    @@pending_delete_estimate.addAndGet(delete_request.fetch(:delete).length)

    # In delete_request, we have an array of individual requests like:
    #
    #   {:delete=>
    #     [{"id"=>"/some/uri/123"},
    #      {"query"=>"parent_id:\"/some/uri/123\""},
    #      {"id"=>"/some/uri/456"},
    #      {"query"=>"parent_id:\"/some/uri/456\""},
    #      ...]}
    #
    # And since delete hooks assume this format, I didn't want to change it.
    # But...
    #
    # Solr processes each delete one at a time, at commit time, on a single
    # thread.  The id deletes are quite cheap, but the query-based deletes take
    # substantially longer (20ms+) because each one looks up the schema, gets an
    # index reader, opens a searcher, parses a query, etc..  Solr performs no
    # batching internally.
    #
    # So, we group them into boolean clauses here to amortize the cost of all of
    # that.  Note that the size of each query group must be lower than Solr's
    # <maxBooleanClauses> setting.

    id_deletes = delete_request.fetch(:delete).select {|r| r['id']}
    grouped_queries = delete_request
                        .fetch(:delete)
                        .map {|r| r['query']}
                        .compact
                        .each_slice(512)
                        .map do |clause_group|
      {'query' => clause_group.join(" OR ")}
    end

    delete_request = {:delete => id_deletes + grouped_queries}

    req.body = delete_request.to_json

    response = do_http_request(solr_url, req)

    # If there are lots of pending deletes, fire a commit to clear them out
    # before they get too numerous.  See the comment on MAX_PENDING_DELETES for
    # more detail.
    pending_deletes = @@pending_delete_estimate.get
    if pending_deletes >= MAX_PENDING_DELETES && @@pending_delete_estimate.compareAndSet(pending_deletes, 0)
      Log.info "Sending soft commit to apply deletes to Solr"
      send_commit(:soft)
    end

    if response.code == '200'
      Log.info "Deleted #{records.length} documents: #{response}"
    else
      Log.error "SolrIndexerError when deleting records: #{response.body}"
    end
  end


  # When applying a batch of updates, keep only the most recent version of each record
  def dedupe_by_uri(records)
    result = []
    seen = {}

    records.reverse.each do |record|
      if !seen[record['uri']]
        result << record
        seen[record['uri']] = true
      end
    end

    result.reverse
  end


  def clean_whitespace(doc)
    if doc.is_a?(String) && !doc.frozen?
      doc.strip!
    elsif doc.is_a?(Hash)
      doc.values.each {|v| clean_whitespace(v)}
    elsif doc.is_a?(Array)
      doc.each {|v| clean_whitespace(v)}
    end

    doc
  end


  def clean_for_sort(value)
    return nil if value.nil?
    out = value.gsub(/<[^>]+>/, '')
    out.gsub!(/-/, ' ')
    out.gsub!(/[^\p{L}\d\s_]/, '')
    out.gsub!(/\s+/, ' ')
    out.strip
  end

  # ANW-1065
  # iterate through the do_not_index list and scrub out that part of the JSON tree
  def sanitize_json(json)
    IndexerCommonConfig.do_not_index.each do |k, v|
      if json["jsonmodel_type"] == k
        # subrec is a reference used to navigate inside of the JSON as specified by the v[:location] to find the part of the tree to sanitize
        subrec = json

        v[:location].each do |l|
          unless subrec.nil?
            subrec = subrec[l]
          end
        end

        unless subrec.nil?
          subrec[v[:to_clean]] = []
        end
      end
    end

    return json
  end

  def native_index_document(uri, record_type, values)
    doc = {}
    doc['id'] = uri
    doc['uri'] = uri
    doc['title'] = values['title']
    doc['primary_type'] = record_type
    doc['types'] = [record_type]
    doc['json'] = ASUtils.to_json(values)
    doc['suppressed'] = values.has_key?('suppressed') && values['suppressed']
    if doc['suppressed']
      doc['publish'] = false
    elsif is_repository_unpublished?(uri, values)
      doc['publish'] = false
    elsif values['has_unpublished_ancestor']
      doc['publish'] = false
    else
      doc['publish'] = values.has_key?('publish') && values['publish']
    end
    doc['system_generated'] = values.has_key?('system_generated') ? values['system_generated'].to_s : 'false'
    doc['repository'] = get_record_scope(uri)
    doc
  end

  def rust_index_document_builder_enabled?
    AppConfig.has_key?(:rust_index_document_builder_enabled) &&
      AppConfig[:rust_index_document_builder_enabled]
  end

  def rust_index_document_enrichment_enabled?
    AppConfig.has_key?(:rust_index_document_enrichment_enabled) &&
      AppConfig[:rust_index_document_enrichment_enabled] &&
      self.class.name != 'PUIIndexer'
  end

  def rust_pui_document_preparation_enabled?
    AppConfig.has_key?(:rust_pui_document_preparation_enabled) &&
      AppConfig[:rust_pui_document_preparation_enabled] &&
      self.class.name == 'PUIIndexer'
  end

  def deep_copy_record_candidate(val)
    case val
    when Hash
      val.each_with_object({}) { |(k, v), h| h[k.to_s] = deep_copy_record_candidate(v) }
    when Array
      val.map { |v| deep_copy_record_candidate(v) }
    when String
      val.dup
    else
      val
    end
  end

  def rust_index_documents(prepared_records)
    return nil unless rust_index_document_builder_enabled?

    maximum = (AppConfig[:rust_index_document_builder_max_batch] || 1000).to_i
    if prepared_records.length > maximum
      Log.warn("Rust index document builder skipped batch of #{prepared_records.length}; maximum is #{maximum}")
      return nil
    end

    inputs = prepared_records.map do |prepared|
      prepared[:values] = sanitize_json(prepared[:values])
      prepared[:sanitized] = true
      input = {
        'uri' => prepared[:uri],
        'record_type' => prepared[:record_type],
        'repository' => get_record_scope(prepared[:uri]),
        'record' => prepared[:values],
        'enrich' => rust_index_document_enrichment_enabled?,
      }
      if rust_pui_document_preparation_enabled? && RecordInheritance.has_type?(prepared[:record_type])
        candidate = deep_copy_record_candidate(prepared[:values])
        candidate = RecordInheritance.merge(candidate)
        input['prepare_pui_final'] = true
        input['pui_candidate_record'] = candidate
      end
      if rust_index_document_enrichment_enabled?
        input['dynamic_enum_fields'] = enum_fields
        input['arks_enabled'] = !!AppConfig[:arks_enabled]
        if prepared[:record_type] == 'accession'
          input['accession_date_unknown_label'] = I18n.t('accession.accession_date_unknown')
          if prepared[:values].is_a?(Hash) && prepared[:values]['accession_date'].is_a?(String)
            begin
              input['parsed_accession_date_year'] = Date.parse(prepared[:values]['accession_date']).year
            rescue ArgumentError, TypeError
              # unparseable date
            end
          end
        end
      end
      input
    end
    builder = RustIndexDocumentBuilder.new(
      AppConfig[:rust_index_document_builder_url] || 'http://127.0.0.1:4570',
      AppConfig[:rust_index_document_builder_timeout_seconds] || 2
    )
    builder.build(inputs)
  rescue => error
    Log.warn("Rust index document builder unavailable; using native indexer: #{error.class}: #{error.message}")
    nil
  end

  def index_records(records, timing = IndexerTiming.new)
    batch = IndexBatch.new
    @pui_final_projections = {}
    @pui_routing_projections = {}
    @pui_notes_projections = {}

    records = dedupe_by_uri(records)

    timing.time_block(:conversion_ms) do
      prepared_records = []
      records.each do |record|
        values = record['record']
        uri = record['uri']

        reference = JSONModel.parse_reference(uri)
        record_type = reference && reference[:type]

        if !record_type || skip_index_record?(record) || (record_type != 'repository' && !record_types.include?(record_type.intern))
          next
        end

        prepared_records << {
          :record => record,
          :values => values,
          :uri => uri,
          :record_type => record_type,
        }
      end

      rust_documents = rust_index_documents(prepared_records)

      prepared_records.each do |prepared|
        record = prepared[:record]
        values = prepared[:values]
        values = sanitize_json(values) unless prepared[:sanitized]
        uri = prepared[:uri]
        record_type = prepared[:record_type]
        doc = rust_documents && rust_documents[uri]
        doc ||= native_index_document(uri, record_type, values)

        rust_cm_docs = doc.delete('_archivalist_collection_management_documents')
        if rust_cm_docs
          rust_cm_docs.each {|child| child.delete_if {|k, _| k.start_with?('_archivalist_')}}
        end

        if doc.has_key?('_archivalist_pui_final')
          pui_final = doc.delete('_archivalist_pui_final')
          @pui_final_projections[uri] = pui_final if pui_final
        end
        if doc.has_key?('_archivalist_pui_routing')
          pui_routing = doc.delete('_archivalist_pui_routing')
          @pui_routing_projections[uri] = pui_routing if pui_routing
        end
        if doc.has_key?('_archivalist_pui_notes')
          pui_notes = doc.delete('_archivalist_pui_notes')
          @pui_notes_projections[uri] = pui_notes if pui_notes
        end
        if doc['_archivalist_rust_fields'].is_a?(Array)
          doc['_archivalist_rust_fields'].delete('pui_final_document_metadata')
          doc['_archivalist_rust_fields'].delete('pui_routing_metadata')
          doc['_archivalist_rust_fields'].delete('pui_notes_metadata')
          doc.delete('_archivalist_rust_fields') if doc['_archivalist_rust_fields'].empty?
        elsif doc['_archivalist_rust_fields'] == ['pui_final_document_metadata']
          doc.delete('_archivalist_rust_fields')
        end

        @document_prepare_hooks.each do |hook|
          hook.call(doc, record)
        end

        if ( !values["finding_aid_filing_title"].nil? && values["finding_aid_filing_title"].length > 0 )
          doc['title_sort'] ||= clean_for_sort(values["finding_aid_filing_title"])
        else
          doc['title_sort'] ||= clean_for_sort(values['title'])
        end

        # do this last of all so we know for certain the doc is published
        apply_pui_fields(doc, record)

        next if skip_index_doc?(doc)

        doc.delete_if {|k, _| k.to_s.start_with?('_archivalist_')}
        batch << clean_whitespace(doc)

        # Allow a single record to spawn multiple Solr documents if desired
        @extra_documents_hooks.each do |hook|
          if hook.equal?(@collection_management_extra_hook) && rust_cm_docs && !rust_cm_docs.empty?
            batch.concat(rust_cm_docs)
          else
            batch.concat(hook.call(record))
          end
        end
      end
      @pui_final_projections = nil
      @pui_routing_projections = nil
      @pui_notes_projections = nil
    end

    index_batch(batch, timing)

    timing
  end


  def index_batch(batch, timing = IndexerTiming.new, opts = {})
    timing ||= IndexerTiming.new

    timing.time_block(:batch_hooks_ms) do
      # Allow hooks to operate on the entire batch if desired
      @batch_hooks.each_with_index do |hook|
        hook.call(batch)
      end
    end

    if !batch.empty?
      # For any record we're updating, delete any child records first (where applicable)
      records_with_children = self.records_with_children.map {|record_type|
        batch.record_info_for_type(record_type).map {|info| '"%s"' % [info[:id]]}
      }.flatten

      if !records_with_children.empty?
        req = Net::HTTP::Post.new("#{solr_url.path}/update")
        req['Content-Type'] = 'application/json'
        req.body = {:delete => {'query' => opts.fetch(:parent_id_field, 'parent_id') + ":(" + records_with_children.join(" OR ") + ")"}}.to_json
        response = do_http_request(solr_url, req)
      end

      # Now apply the updates
      req = Net::HTTP::Post.new("#{solr_url.path}/update")
      req['Content-Type'] = 'application/json'

      # Note: We call to_json_stream before asking for the count because this
      # writes out the closing array and newline.
      stream = batch.to_json_stream
      req['Content-Length'] = batch.byte_count

      req.body_stream = stream

      timing.time_block(:solr_add_ms) do
        response = do_http_request(solr_url, req)

        stream.close
        batch.destroy

        if response.code != '200'
          Log.error "SolrIndexerError when indexing records: #{response.body}"
        end
      end
    end
  end


  def send_commit(type = :hard)
    req = Net::HTTP::Post.new("#{solr_url.path}/update")
    req['Content-Type'] = 'application/json'
    req.body = {:commit => {"softCommit" => (type == :soft) }}.to_json

    response = do_http_request(solr_url, req)

    if response.code != '200'
      if response.body =~ /exceeded limit of maxWarmingSearchers/
        Log.info "INFO: #{response.body}"
      else
        Log.error "SolrIndexerError when committing: #{response.body}"
      end
    end
  end

  def paused?
    self.singleton_class.class_variable_get(:@@paused_until) > Time.now
  end

  def skip_index_record?(record)
    false
  end

  def skip_index_doc?(doc)
    false
  end

  def apply_pui_fields(doc, record)
    # only add pui types if the record is published
    if doc['publish']
      object_record_types = ['accession', 'digital_object', 'digital_object_component']

      if object_record_types.include?(doc['primary_type'])
        doc['types'] << 'pui_record'
      end

      if ['agent_person', 'agent_corporate_entity'].include?(doc['primary_type'])
        doc['types'] << 'pui_agent'
      end

      unless RecordInheritance.has_type?(doc['primary_type'])
        # All record types are available to PUI except archival objects, since
        # our pui_indexer indexes a specially formatted version of those.
        if ['resource'].include?(doc['primary_type'])
          doc['types'] << 'pui_collection'
        elsif ['classification'].include?(doc['primary_type'])
          doc['types'] << 'pui_record_group'
        elsif ['agent_person'].include?(doc['primary_type'])
          doc['types'] << 'pui_person'
        else
          doc['types'] << 'pui_' + doc['primary_type']
        end

        doc['types'] << 'pui'
      end
    end

    # index all top containers for pui
    if doc['primary_type'] == 'top_container'
      doc['publish'] = record['record']['is_linked_to_published_record']
      if doc['publish']
        doc['types'] << 'pui_container'
        doc['types'] << 'pui'
      end
    end
  end
end

def final_doc_rules
  # give subclasses a place to hang custom behavior.
  # runs after all the hooks have been added
end

ASUtils.find_local_directories('indexer').each do |dir|
  Dir.glob(File.join(dir, "*.rb")).sort.each do |file|
    require file
  end
end
