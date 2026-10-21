require_relative 'rust_index_document_builder' unless defined?(RustIndexDocumentBuilder)

class LargeTreeDocIndexer

  attr_reader :batch, :deletes

  def initialize(batch, builder = nil)
    # We'll track the nodes we find as we need to index their path from root
    # in a relatively efficient way
    @node_uris = []

    @batch = batch
    @deletes = []
    @builder = builder
    @call_depth = 0
    @pending_auxiliary_inputs = []
    @max_batch_size = (AppConfig[:rust_index_document_builder_max_batch] || 128).to_i
  end

  def rust_pui_auxiliary_documents_enabled?
    AppConfig.has_key?(:rust_pui_auxiliary_documents_enabled) &&
      AppConfig[:rust_pui_auxiliary_documents_enabled]
  end

  def rust_builder
    @builder ||= RustIndexDocumentBuilder.new(
      AppConfig[:rust_index_document_builder_url] || 'http://127.0.0.1:4570',
      AppConfig[:rust_index_document_builder_timeout_seconds] || 2
    )
  end

  def emit_auxiliary_doc(uri, record_type, pui_parent_id, payload)
    if rust_pui_auxiliary_documents_enabled?
      authoritative_json = ASUtils.to_json(payload)
      @pending_auxiliary_inputs << {
        'uri' => uri,
        'record_type' => record_type,
        'pui_parent_id' => pui_parent_id,
        'repository' => 'global',
        'record' => payload,
        'pui_auxiliary_json' => authoritative_json,
        'prepare_pui_auxiliary' => true,
      }
      if @pending_auxiliary_inputs.length >= @max_batch_size
        flush_auxiliary_batch!
      end
    else
      batch << {
        'id' => uri,
        'uri' => uri,
        'pui_parent_id' => pui_parent_id,
        'publish' => "true",
        'primary_type' => record_type,
        'types' => ['pui'],
        'json' => ASUtils.to_json(payload)
      }
    end
  end

  def flush_auxiliary_batch!
    return if @pending_auxiliary_inputs.empty?
    inputs = @pending_auxiliary_inputs
    @pending_auxiliary_inputs = []

    begin
      docs = rust_builder.build_pui_auxiliary(inputs)
      docs.each { |doc| batch << doc }
    rescue => e
      Log.warn("Rust PUI auxiliary document builder failed; using native fallback: #{e.class}: #{e.message}") if defined?(Log) && Log.respond_to?(:warn)
      inputs.each do |input|
        batch << {
          'id' => input['uri'],
          'uri' => input['uri'],
          'pui_parent_id' => input['pui_parent_id'],
          'publish' => "true",
          'primary_type' => input['record_type'],
          'types' => ['pui'],
          'json' => input['pui_auxiliary_json'] || ASUtils.to_json(input['record'])
        }
      end
    end
  end

  def add_largetree_docs(root_record_uris)
    @call_depth += 1
    begin
      root_record_uris.each do |node_uri|
        @node_uris.clear

        json = JSONModel::HTTP.get_json(node_uri + '/tree/root',
                                        :published_only => true)

        emit_auxiliary_doc(
          "#{node_uri}/tree/root",
          "tree_root",
          node_uri,
          json
        )

        add_waypoints(json, node_uri, nil)

        flush_auxiliary_batch!

        index_paths_to_root(node_uri, @node_uris)
      end
    ensure
      @call_depth -= 1
      flush_auxiliary_batch! if @call_depth == 0
    end
  end

  def add_waypoints(json, root_record_uri, parent_uri)
    @call_depth += 1
    begin
      json.fetch('waypoints').times do |waypoint_number|
        waypoint_json = JSONModel::HTTP.get_json(root_record_uri + '/tree/waypoint',
                                                :offset => waypoint_number,
                                                :parent_node => parent_uri,
                                                :published_only => true)

        emit_auxiliary_doc(
          "#{root_record_uri}/tree/waypoint_#{parent_uri}_#{waypoint_number}",
          "tree_waypoint",
          (parent_uri || root_record_uri),
          waypoint_json
        )

        waypoint_json.each do |waypoint_record|
          add_nodes(root_record_uri, waypoint_record)
        end
      end
    ensure
      @call_depth -= 1
      flush_auxiliary_batch! if @call_depth == 0
    end
  end

  def add_nodes(root_record_uri, waypoint_record)
    @call_depth += 1
    begin
      record_uri = waypoint_record.fetch('uri')

      @node_uris << record_uri

      # Index the node itself if it has children
      if waypoint_record.fetch('child_count') > 0
        node_json = JSONModel::HTTP.get_json(root_record_uri + '/tree/node',
                                            :node_uri => record_uri,
                                            :published_only => true)

        # We might bomb out if a record was deleted out from under us.
        return if node_json.nil?

        emit_auxiliary_doc(
          "#{root_record_uri}/tree/node_#{node_json.fetch('uri')}",
          "tree_node",
          node_json.fetch('uri'),
          node_json
        )

        # Finally, walk the node's waypoints and index those too.
        add_waypoints(node_json, root_record_uri, node_json.fetch('uri'))
      else
        # Fixing #ANW-731
        # This node has no published children but it might have previously
        # so we need to remember its node doc so our caller can delete it
        @deletes.push("#{root_record_uri}/tree/node_#{record_uri}")
      end
    ensure
      @call_depth -= 1
      flush_auxiliary_batch! if @call_depth == 0
    end
  end

  def index_paths_to_root(root_uri, node_uris)
    node_uris.each_slice(128) do |slice_node_uris|

      node_id_to_uri = Hash[slice_node_uris.map {|uri| [JSONModel.parse_reference(uri).fetch(:id), uri]}]
      node_paths = JSONModel::HTTP.get_json(root_uri + '/tree/node_from_root',
                                            'node_ids[]' => node_id_to_uri.keys,
                                            :published_only => true)

      if rust_pui_auxiliary_documents_enabled?
        inputs = []
        node_paths.each do |node_id, path|
          parent_id = node_id_to_uri.fetch(Integer(node_id))
          payload = { node_id => path }
          authoritative_json = ASUtils.to_json(payload)
          inputs << {
            'uri' => "#{root_uri}/tree/node_from_root_#{node_id}",
            'record_type' => "tree_node_from_root",
            'pui_parent_id' => parent_id,
            'repository' => 'global',
            'record' => payload,
            'pui_auxiliary_json' => authoritative_json,
            'prepare_pui_auxiliary' => true,
          }
        end
        if !inputs.empty?
          begin
            docs = rust_builder.build_pui_auxiliary(inputs)
            docs.each { |doc| batch << doc }
          rescue => e
            Log.warn("Rust PUI auxiliary document builder failed in index_paths_to_root; using native fallback: #{e.class}: #{e.message}") if defined?(Log) && Log.respond_to?(:warn)
            inputs.each do |input|
              batch << {
                'id' => input['uri'],
                'uri' => input['uri'],
                'pui_parent_id' => input['pui_parent_id'],
                'publish' => "true",
                'primary_type' => "tree_node_from_root",
                'types' => ['pui'],
                'json' => input['pui_auxiliary_json'] || ASUtils.to_json(input['record'])
              }
            end
          end
        end
      else
        node_paths.each do |node_id, path|
          batch << {
            'id' => "#{root_uri}/tree/node_from_root_#{node_id}",
            'uri' => "#{root_uri}/tree/node_from_root_#{node_id}",
            'pui_parent_id' => node_id_to_uri.fetch(Integer(node_id)),
            'publish' => "true",
            'primary_type' => "tree_node_from_root",
            'types' => ['pui'],
            'json' => ASUtils.to_json({node_id => path})
          }
        end
      end
    end
  end

end
