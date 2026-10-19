require 'record_inheritance'

require_relative 'periodic_indexer'
require_relative 'large_tree_doc_indexer'

require 'set'

class PUIIndexer < PeriodicIndexer

  PUI_RESOLVES = AppConfig[:record_inheritance_resolves]

  PUI_ROUTING_FIELDS = ['marker', 'id', 'pui_parent_id', 'types'].freeze
  PUI_NOTES_FIELDS = ['marker', 'notes_published'].freeze

  def initialize(backend = nil, state = nil, name)
    state_class = AppConfig[:index_state_class].constantize
    index_state = state || state_class.new("indexer_pui_state")

    super(backend, index_state, name)

    # Set up our JSON schemas now that we know the JSONModels have been loaded
    RecordInheritance.prepare_schemas

    @time_to_sleep = AppConfig[:pui_indexing_frequency_seconds].to_i
    @thread_count = AppConfig[:pui_indexer_thread_count].to_i
    @records_per_thread = AppConfig[:pui_indexer_records_per_thread].to_i

    @unpublished_records = java.util.Collections.synchronizedList(java.util.ArrayList.new)
  end

  def fetch_records(type, ids, resolve)
    records = JSONModel(type).all(:id_set => ids.join(","), 'resolve[]' => resolve)
    if RecordInheritance.has_type?(type)
      RecordInheritance.merge(records, :direct_only => true)
    else
      records
    end
  end

  def self.get_indexer(state = nil, name = "PUI Indexer")
    indexer = self.new(state, name)
  end

  def resolved_attributes
    super + PUI_RESOLVES
  end

  def record_types
    # We only want to index the record types we're going to make separate
    # PUI-specific versions of...
    (super.select {|type| RecordInheritance.has_type?(type)} + [:archival_object]).uniq
  end

  def configure_doc_rules
    super

    record_has_children('resource')
    record_has_children('archival_object')
    record_has_children('digital_object')
    record_has_children('digital_object_component')
    record_has_children('classification')
    record_has_children('classification_term')


    add_document_prepare_hook {|doc, record|

      if RecordInheritance.has_type?(doc['primary_type'])
        if rust_pui_document_preparation_enabled? && apply_rust_pui_routing_projection(doc, record)
          next
        end

        parent_id = doc['id']
        doc['id'] = "#{parent_id}#pui"
        doc['pui_parent_id'] = parent_id
        doc['types'] ||= []
        doc['types'] << 'pui'
        doc['types'] << "pui_#{doc['primary_type']}"
        doc['types'] << 'pui_record'
        doc['types'] << 'pui_only'
      end
    }
  end
  
  # Run the final doc rules after all the hooks have been added
  # This allows plugins to access ancestor data in PUI records before it is removed
  def final_doc_rules
    # this runs after the hooks in indexer_common, so we can overwrite with confidence
    add_document_prepare_hook {|doc, record|
      if RecordInheritance.has_type?(doc['primary_type'])
        if rust_pui_document_preparation_enabled? && apply_rust_pui_final_projection(doc, record)
          next
        end

        # special handling for json because we need to include indirectly inherited
        # fields too - the json sent to indexer_common only has directly inherited
        # fields because only they should be indexed.
        # so we remerge without the :direct_only flag, and we remove the ancestors
        doc['json'] = ASUtils.to_json(RecordInheritance.merge(record['record'],
                                                              :remove_ancestors => true))

        # special handling for title because it is populated from display_string
        # in indexer_common and display_string is not changed in the merge process
        doc['title'] = record['record']['title'] if record['record']['title']

        # special handling for fullrecord because we don't want the ancestors indexed.
        # we're now done with the ancestors, so we can just delete them from the record
        record['record'].delete('ancestors')
        # we don't want container_profile or top_container notes indexed for the public either
        if record['record']['instances']
          record['record']['instances'].each do |instance|
            if instance['sub_container'] && instance['sub_container']['top_container']
              top_container = instance['sub_container']['top_container']
              if top_container['_resolved']
                top_container['_resolved'].delete('internal_note')
                if top_container['_resolved']['container_profile'] && top_container['_resolved']['container_profile']['_resolved']
                  top_container['_resolved']['container_profile']['_resolved'].delete('notes')
                end
              end
            end
          end
        end
        build_fullrecord(doc, record)
      end
    }
  end

  def apply_rust_pui_final_projection(doc, record)
    return false unless record.is_a?(Hash) && record['record'].is_a?(Hash)
    record_obj = record['record']
    rec_uri = record_obj['uri']
    return false unless rec_uri.is_a?(String) && !rec_uri.empty?
    rec_type = record_obj['jsonmodel_type']
    return false unless rec_type.is_a?(String) && rec_type == doc['primary_type']

    uri = rec_uri
    projection = @pui_final_projections && @pui_final_projections[uri]
    return false unless projection.is_a?(Hash)

    # 1. Capability marker & structure validation
    return false unless projection['marker'] == 'pui_final_document_metadata'
    return false unless projection['json'].is_a?(String)
    return false unless projection['fullrecord_published'].is_a?(Array)
    return false if doc.has_key?('fullrecord')

    # 2. Parse projection JSON
    parsed_json = begin
                    ASUtils.json_parse(projection['json'])
                  rescue StandardError
                    nil
                  end
    return false unless parsed_json.is_a?(Hash)
    return false unless parsed_json['uri'].is_a?(String) && parsed_json['uri'] == uri
    return false unless parsed_json['jsonmodel_type'].is_a?(String) && parsed_json['jsonmodel_type'] == doc['primary_type']

    # 3. Privacy assertions on parsed_json
    return false if parsed_json.has_key?('ancestors')
    if parsed_json['instances'].is_a?(Array)
      parsed_json['instances'].each do |instance|
        next unless instance.is_a?(Hash)
        sub_c = instance['sub_container']
        if sub_c.is_a?(Hash) && sub_c['top_container'].is_a?(Hash)
          resolved_tc = sub_c['top_container']['_resolved']
          if resolved_tc.is_a?(Hash)
            return false if resolved_tc.has_key?('internal_note')
            resolved_cp = resolved_tc.dig('container_profile', '_resolved')
            return false if resolved_cp.is_a?(Hash) && resolved_cp.has_key?('notes')
          end
        end
      end
    end

    # 4. Authoritative native independent computation:
    # Build expected clean fully inherited record using exact native merge & clean logic
    expected_merged = RecordInheritance.merge(deep_copy_record_candidate(record['record']))
    expected_clean = deep_copy_record_candidate(expected_merged)
    expected_clean.delete('ancestors')
    if expected_clean['instances'].is_a?(Array)
      expected_clean['instances'].each do |instance|
        next unless instance.is_a?(Hash)
        sub_c = instance['sub_container']
        if sub_c.is_a?(Hash) && sub_c['top_container'].is_a?(Hash)
          resolved_tc = sub_c['top_container']['_resolved']
          if resolved_tc.is_a?(Hash)
            resolved_tc.delete('internal_note')
            resolved_cp = resolved_tc.dig('container_profile', '_resolved')
            resolved_cp.delete('notes') if resolved_cp.is_a?(Hash)
          end
        end
      end
    end

    # Exact value and serialization equality
    return false unless parsed_json == expected_clean
    return false unless projection['json'] == ASUtils.to_json(expected_clean)

    # Published-only fullrecord check
    expected_fullrecord = IndexerCommon.extract_string_values(expected_clean, :published_only)
    return false unless projection['fullrecord_published'] == expected_fullrecord

    # 5. Apply projection safely
    doc['json'] = projection['json']
    doc['title'] = record['record']['title'] if record['record']['title']

    # Perform required native mutation on record['record']
    record['record'].delete('ancestors')
    if record['record']['instances'].is_a?(Array)
      record['record']['instances'].each do |instance|
        next unless instance.is_a?(Hash)
        sub_c = instance['sub_container']
        if sub_c.is_a?(Hash) && sub_c['top_container'].is_a?(Hash)
          resolved_tc = sub_c['top_container']['_resolved']
          if resolved_tc.is_a?(Hash)
            resolved_tc.delete('internal_note')
            resolved_cp = resolved_tc.dig('container_profile', '_resolved')
            resolved_cp.delete('notes') if resolved_cp.is_a?(Hash)
          end
        end
      end
    end

    doc['fullrecord_published'] = projection['fullrecord_published']
    doc.delete('fullrecord')
    doc.delete('ancestors')
    doc.delete_if { |k, _| k.to_s.start_with?('_archivalist_') }
    @pui_final_projections.delete(uri)
    true
  rescue StandardError => e
    Log.warn("PUI final projection validation failed for #{uri}: #{e.class}: #{e.message}; using native fallback")
    @pui_final_projections.delete(uri) if @pui_final_projections
    false
  end

  def discard_pui_routing(uri)
    @pui_routing_projections.delete(uri) if @pui_routing_projections && uri
    false
  end

  def discard_pui_notes(uri)
    @pui_notes_projections.delete(uri) if @pui_notes_projections && uri
    false
  end

  def apply_rust_pui_routing_projection(doc, record)
    return false unless record.is_a?(Hash) && record['record'].is_a?(Hash)
    record_obj = record['record']
    rec_uri = record_obj['uri']
    return false unless rec_uri.is_a?(String) && !rec_uri.empty?
    uri = rec_uri

    rec_type = record_obj['jsonmodel_type']
    return discard_pui_routing(uri) unless rec_type.is_a?(String) && rec_type == doc['primary_type']
    return discard_pui_routing(uri) unless RecordInheritance.has_type?(doc['primary_type'])

    if @pui_routing_projections && @pui_routing_projections.key?(uri)
      projection = @pui_routing_projections[uri]
      return discard_pui_routing(uri) unless projection.is_a?(Hash)
    else
      return false
    end

    # 1. Exact key set validation: reject missing or extra keys, including string/symbol edge concerns
    return discard_pui_routing(uri) if projection.keys.any? { |k| !k.is_a?(String) }
    return discard_pui_routing(uri) unless (PUI_ROUTING_FIELDS - projection.keys).empty? && (projection.keys - PUI_ROUTING_FIELDS).empty?

    # 2. Capability marker & structure validation
    return discard_pui_routing(uri) unless projection['marker'] == 'pui_routing_metadata'
    return discard_pui_routing(uri) unless projection['id'].is_a?(String)
    return discard_pui_routing(uri) unless projection['pui_parent_id'].is_a?(String)
    return discard_pui_routing(uri) unless projection['types'].is_a?(Array)

    # 3. Authoritative native independent computation:
    # doc['id'] prior to this hook equals the native id / parent_id (which is rec_uri)
    parent_id = doc['id']
    return discard_pui_routing(uri) unless parent_id == uri

    expected_id = "#{parent_id}#pui"
    expected_parent_id = parent_id
    expected_type_additions = ['pui', "pui_#{doc['primary_type']}", 'pui_record', 'pui_only']

    # Exact value equality against native independent computation
    return discard_pui_routing(uri) unless projection['id'] == expected_id
    return discard_pui_routing(uri) unless projection['pui_parent_id'] == expected_parent_id
    return discard_pui_routing(uri) unless projection['types'] == expected_type_additions

    # 4. Apply projection safely with exact native append semantics
    doc['id'] = projection['id']
    doc['pui_parent_id'] = projection['pui_parent_id']
    doc['types'] ||= []
    doc['types'].concat(projection['types'])

    @pui_routing_projections.delete(uri)
    true
  rescue StandardError => e
    Log.warn("PUI routing projection validation failed for #{uri}: #{e.class}: #{e.message}; using native fallback")
    @pui_routing_projections.delete(uri) if @pui_routing_projections
    false
  end

  def apply_rust_pui_notes_projection(doc, record)
    return false unless record.is_a?(Hash) && record['record'].is_a?(Hash)
    record_obj = record['record']
    rec_uri = record_obj['uri']
    return false unless rec_uri.is_a?(String) && !rec_uri.empty?
    uri = rec_uri

    rec_type = record_obj['jsonmodel_type']
    return discard_pui_notes(uri) unless rec_type.is_a?(String) && rec_type == doc['primary_type']
    return discard_pui_notes(uri) unless RecordInheritance.has_type?(doc['primary_type'])

    if @pui_notes_projections && @pui_notes_projections.key?(uri)
      projection = @pui_notes_projections[uri]
      return discard_pui_notes(uri) unless projection.is_a?(Hash)
    else
      return false
    end

    # 1. Exact key set validation: reject missing or extra keys, including string/symbol edge concerns
    return discard_pui_notes(uri) if projection.keys.any? { |k| !k.is_a?(String) }
    return discard_pui_notes(uri) unless (PUI_NOTES_FIELDS - projection.keys).empty? && (projection.keys - PUI_NOTES_FIELDS).empty?

    # 2. Capability marker & structure validation
    return discard_pui_notes(uri) unless projection['marker'] == 'pui_notes_metadata'

    # 3. Check notes presence / absence
    raw_notes = record_obj['notes']
    has_notes = !raw_notes.nil? && raw_notes != false

    if !has_notes
      # Preserve absence behavior exactly when notes is absent/nil/false
      return discard_pui_notes(uri) unless projection['notes_published'].nil?
      # Do not set doc['notes_published']
      @pui_notes_projections.delete(uri)
      return true
    end

    # When notes are present, notes_published must be an Array
    return discard_pui_notes(uri) unless projection['notes_published'].is_a?(Array)

    # 4. Authoritative native independent computation:
    expected_notes_published = IndexerCommon.extract_string_values(raw_notes, :published_only)

    # Exact equality against native independent computation
    return discard_pui_notes(uri) unless projection['notes_published'] == expected_notes_published

    # 5. Apply projection safely
    doc['notes_published'] = projection['notes_published']
    @pui_notes_projections.delete(uri)
    true
  rescue StandardError => e
    Log.warn("PUI notes projection validation failed for #{uri}: #{e.class}: #{e.message}; using native fallback")
    @pui_notes_projections.delete(uri) if @pui_notes_projections
    false
  end

  def build_fullrecord(doc, record)
    doc['fullrecord_published'] = IndexerCommon.extract_string_values(record['record'], :published_only)
  end

  def add_notes(doc, record)
    if rust_pui_document_preparation_enabled? && apply_rust_pui_notes_projection(doc, record)
      return
    end

    if record['record']['notes']
      doc['notes_published'] = IndexerCommon.extract_string_values(record['record']['notes'], :published_only)
    end
  end

  def add_infscroll_docs(resource_uris, batch)
    return if resource_uris.empty?

    if rust_pui_auxiliary_documents_enabled?
      fetched = []
      begin
        resource_uris.each do |resource_uri|
          json = JSONModel::HTTP.get_json(resource_uri + '/ordered_records')
          authoritative_json = ASUtils.to_json(json)
          fetched << [resource_uri, json, authoritative_json]
        end
      rescue => e
        # Preserve exact native observable behavior: on a fetch exception after earlier
        # successful fetches, ensure the documents for all earlier successful fetches
        # are appended in original order using native construction, then re-raise the
        # original exception. Do not invoke the sidecar after the fetch failure.
        fetched.each do |resource_uri, _json, authoritative_json|
          batch << {
            'id' => "#{resource_uri}/ordered_records",
            'uri' => "#{resource_uri}/ordered_records",
            'pui_parent_id' => resource_uri,
            'publish' => "true",
            'primary_type' => "resource_ordered_records",
            'types' => ['pui'],
            'json' => authoritative_json
          }
        end
        raise e
      end

      return if fetched.empty?

      inputs = fetched.map do |resource_uri, json, authoritative_json|
        {
          'uri' => "#{resource_uri}/ordered_records",
          'record_type' => "resource_ordered_records",
          'pui_parent_id' => resource_uri,
          'record' => json,
          'pui_auxiliary_json' => authoritative_json,
          'prepare_pui_auxiliary' => true,
        }
      end

      begin
        docs = rust_index_document_builder.build_pui_auxiliary(inputs)
        docs.each { |doc| batch << doc }
      rescue => e
        Log.warn("Rust PUI auxiliary document builder failed in add_infscroll_docs; using native fallback: #{e.class}: #{e.message}") if defined?(Log) && Log.respond_to?(:warn)
        fetched.each do |resource_uri, _json, authoritative_json|
          batch << {
            'id' => "#{resource_uri}/ordered_records",
            'uri' => "#{resource_uri}/ordered_records",
            'pui_parent_id' => resource_uri,
            'publish' => "true",
            'primary_type' => "resource_ordered_records",
            'types' => ['pui'],
            'json' => authoritative_json
          }
        end
      end
    else
      resource_uris.each do |resource_uri|
        json = JSONModel::HTTP.get_json(resource_uri + '/ordered_records')

        batch << {
          'id' => "#{resource_uri}/ordered_records",
          'uri' => "#{resource_uri}/ordered_records",
          'pui_parent_id' => resource_uri,
          'publish' => "true",
          'primary_type' => "resource_ordered_records",
          'types' => ['pui'],
          'json' => ASUtils.to_json(json)
        }
      end
    end
  end

  def skip_index_record?(record)
    published = record['record']['publish']

    stage_unpublished_for_deletion("#{record['record']['uri']}#pui") unless published

    !published
  end


  def skip_index_doc?(doc)
    published = doc['publish']

    stage_unpublished_for_deletion(doc['id']) unless published

    !published
  end

  def index_round_complete(repository)
    # Index any trees in `repository`
    tree_types = [[:resource, :archival_object],
                  [:digital_object, :digital_object_component],
                  [:classification, :classification_term]]

    start = Time.now
    checkpoints = []
    update_mtimes = false
    state_type = 'tree'

    tree_uris = []

    tree_types.each do |pair|
      root_type = pair.first
      node_type = pair.last

      checkpoints << [repository, root_type, start]
      checkpoints << [repository, node_type, start]

      last_root_node_mtime = [@state.get_last_mtime(repository.id, root_type, state_type) - @window_seconds, 0].max
      last_node_mtime = [@state.get_last_mtime(repository.id, node_type, state_type) - @window_seconds, 0].max

      root_node_ids = Set.new(JSONModel::HTTP.get_json(JSONModel(root_type).uri_for, :all_ids => true, :modified_since => last_root_node_mtime))
      node_ids = JSONModel::HTTP.get_json(JSONModel(node_type).uri_for, :all_ids => true, :modified_since => last_node_mtime)

      node_ids.each_slice(@records_per_thread) do |ids|
        node_records = JSONModel(node_type).all(:id_set => ids.join(","), 'resolve[]' => [])

        node_records.each do |record|
          root_node_ids << JSONModel.parse_reference(record[root_type.to_s]['ref']).fetch(:id)
        end
      end

      tree_uris.concat(root_node_ids.map {|id| JSONModel(root_type).uri_for(id) })
    end

    batch = IndexBatch.new

    add_infscroll_docs(tree_uris.select {|uri| JSONModel.parse_reference(uri).fetch(:type) == 'resource'},
                       batch)

    tree_indexer = LargeTreeDocIndexer.new(batch, rust_index_document_builder)
    tree_indexer.add_largetree_docs(tree_uris)

    if batch.length > 0
      log "Indexed #{batch.length} additional PUI records in repository #{repository.repo_code}"

      index_batch(batch, nil, :parent_id_field => 'pui_parent_id')
      send_commit
      update_mtimes = true
    end

    if tree_indexer.deletes.length > 0
      tree_indexer.deletes.each_slice(100) do |deletes|
        delete_records(deletes, :parent_id_field => 'pui_parent_id')
      end
    end

    handle_deletes(:parent_id_field => 'pui_parent_id')

    # Delete any unpublished records and decendents
    delete_records(@unpublished_records, :parent_id_field => 'pui_parent_id')
    @unpublished_records.clear()

    checkpoints.each do |repository, type, start|
      @state.set_last_mtime(repository.id, type, start, state_type) if update_mtimes
    end

  end

  def stage_unpublished_for_deletion(doc_id)
    @unpublished_records.add(doc_id) if doc_id =~ /#pui$/
  end

  def repositories_updated_action(updated_repositories)

    updated_repositories.each do |repository|

      if !repository['record']['publish']

        # Delete PUI-only Solr documents in case this is the first index run after the repository has been unpublished
        req = Net::HTTP::Post.new("#{solr_url.path}/update")
        req['Content-Type'] = 'application/json'
        delete_request = {:delete => {'query' => "repository:\"#{repository['uri']}\" AND types:pui_only"}}
        req.body = delete_request.to_json
        response = do_http_request(solr_url, req)
        if response.code == '200'
          Log.info "Deleted PUI-only documents in private repository #{repository['record']['repo_code']}: #{response}"
        else
          Log.error "SolrIndexerError when deleting PUI-only records in private repository #{repository['record']['repo_code']}: #{response.body}"
        end

      end

    end

  end
  
end
