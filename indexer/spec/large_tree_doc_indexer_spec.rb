require 'spec_helper'
require_relative '../app/lib/large_tree_doc_indexer'
require_relative '../app/lib/rust_index_document_builder'

describe LargeTreeDocIndexer do
  let(:root_uri) { '/repositories/2/resources/16' }
  let(:batch) { [] }

  describe '#index_paths_to_root' do
    it 'requests and indexes paths in batches of 128 nodes with native fallback when disabled' do
      node_uris = (1..129).map { |id| "/repositories/2/archival_objects/#{id}" }
      first_paths = (1..128).to_h { |id| [id.to_s, ["path-#{id}"]] }
      final_paths = {'129' => ['path-129']}

      expect(JSONModel::HTTP).to receive(:get_json).with(
        root_uri + '/tree/node_from_root',
        'node_ids[]' => (1..128).to_a,
        :published_only => true
      ).ordered.and_return(first_paths)
      expect(JSONModel::HTTP).to receive(:get_json).with(
        root_uri + '/tree/node_from_root',
        'node_ids[]' => [129],
        :published_only => true
      ).ordered.and_return(final_paths)

      described_class.new(batch).index_paths_to_root(root_uri, node_uris)

      expect(batch.length).to eq(129)
      expect(batch.first).to include(
        'id' => root_uri + '/tree/node_from_root_1',
        'uri' => root_uri + '/tree/node_from_root_1',
        'pui_parent_id' => node_uris.first,
        'publish' => 'true',
        'primary_type' => 'tree_node_from_root',
        'types' => ['pui']
      )
      expect(JSON.parse(batch.first.fetch('json'))).to eq('1' => ['path-1'])
      expect(batch.last.fetch('pui_parent_id')).to eq(node_uris.last)
      expect(JSON.parse(batch.last.fetch('json'))).to eq('129' => ['path-129'])
    end

    it 'uses rust builder for index_paths_to_root when flag is enabled' do
      begin
        AppConfig[:rust_pui_auxiliary_documents_enabled] = true
        node_uris = ["/repositories/2/archival_objects/1", "/repositories/2/archival_objects/2"]
        paths = {'1' => ['path-1'], '2' => ['path-2']}

        expect(JSONModel::HTTP).to receive(:get_json).with(
          root_uri + '/tree/node_from_root',
          'node_ids[]' => [1, 2],
          :published_only => true
        ).and_return(paths)

        builder = instance_double('RustIndexDocumentBuilder')
        expected_inputs = [
          {
            'uri' => "#{root_uri}/tree/node_from_root_1",
            'record_type' => 'tree_node_from_root',
            'pui_parent_id' => node_uris[0],
            'record' => {'1' => ['path-1']},
            'pui_auxiliary_json' => ASUtils.to_json({'1' => ['path-1']}),
            'prepare_pui_auxiliary' => true,
          },
          {
            'uri' => "#{root_uri}/tree/node_from_root_2",
            'record_type' => 'tree_node_from_root',
            'pui_parent_id' => node_uris[1],
            'record' => {'2' => ['path-2']},
            'pui_auxiliary_json' => ASUtils.to_json({'2' => ['path-2']}),
            'prepare_pui_auxiliary' => true,
          },
        ]
        mock_docs = expected_inputs.map do |inp|
          {
            'id' => inp['uri'],
            'uri' => inp['uri'],
            'pui_parent_id' => inp['pui_parent_id'],
            'publish' => 'true',
            'primary_type' => inp['record_type'],
            'types' => ['pui'],
            'json' => ASUtils.to_json(inp['record']),
          }
        end
        expect(builder).to receive(:build_pui_auxiliary).with(expected_inputs).and_return(mock_docs)

        indexer = described_class.new(batch, builder)
        indexer.index_paths_to_root(root_uri, node_uris)

        expect(batch.length).to eq(2)
        expect(batch[0]['publish']).to eq('true')
        expect(batch[0]['types']).to eq(['pui'])
        expect(batch[0]['pui_parent_id']).to eq(node_uris[0])
        expect(JSON.parse(batch[0]['json'])).to eq('1' => ['path-1'])
        expect(batch.none? { |d| d.keys.any? { |k| k.start_with?('_archivalist') } }).to be(true)
      ensure
        AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      end
    end

    it 'falls back to native document construction on builder failure in index_paths_to_root' do
      begin
        AppConfig[:rust_pui_auxiliary_documents_enabled] = true
        node_uris = ["/repositories/2/archival_objects/1"]
        paths = {'1' => ['path-1']}

        expect(JSONModel::HTTP).to receive(:get_json).and_return(paths)

        builder = instance_double('RustIndexDocumentBuilder')
        expect(builder).to receive(:build_pui_auxiliary).and_raise(RuntimeError, "sidecar unreachable")

        indexer = described_class.new(batch, builder)
        indexer.index_paths_to_root(root_uri, node_uris)

        expect(batch.length).to eq(1)
        expect(batch.first).to include(
          'id' => "#{root_uri}/tree/node_from_root_1",
          'uri' => "#{root_uri}/tree/node_from_root_1",
          'pui_parent_id' => node_uris.first,
          'publish' => 'true',
          'primary_type' => 'tree_node_from_root',
          'types' => ['pui']
        )
      ensure
        AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      end
    end

    it 'preserves native exception timing when node_id cannot be converted to Integer' do
      node_uris = ["/repositories/2/archival_objects/1"]
      paths = {'invalid_id' => ['path-1']}
      expect(JSONModel::HTTP).to receive(:get_json).and_return(paths)

      expect {
        described_class.new(batch).index_paths_to_root(root_uri, node_uris)
      }.to raise_error(ArgumentError)
    end
  end

  describe '#add_largetree_docs' do
    let(:root_json) do
      {
        'title' => 'Sample Collection',
        'waypoints' => 1,
      }
    end
    let(:waypoint_json) do
      [
        {
          'uri' => '/repositories/2/archival_objects/101',
          'child_count' => 1,
        },
        {
          'uri' => '/repositories/2/archival_objects/102',
          'child_count' => 0,
        },
      ]
    end
    let(:node_json) do
      {
        'uri' => '/repositories/2/archival_objects/101',
        'title' => 'Subseries 1',
        'waypoints' => 0,
      }
    end

    it 'builds tree_root, tree_waypoint, tree_node, and tree_node_from_root via Rust with batching' do
      begin
        AppConfig[:rust_pui_auxiliary_documents_enabled] = true

        expect(JSONModel::HTTP).to receive(:get_json).with(
          root_uri + '/tree/root',
          :published_only => true
        ).and_return(root_json)

        expect(JSONModel::HTTP).to receive(:get_json).with(
          root_uri + '/tree/waypoint',
          :offset => 0,
          :parent_node => nil,
          :published_only => true
        ).and_return(waypoint_json)

        expect(JSONModel::HTTP).to receive(:get_json).with(
          root_uri + '/tree/node',
          :node_uri => '/repositories/2/archival_objects/101',
          :published_only => true
        ).and_return(node_json)

        expect(JSONModel::HTTP).to receive(:get_json).with(
          root_uri + '/tree/node_from_root',
          'node_ids[]' => [101, 102],
          :published_only => true
        ).and_return({'101' => ['path-101'], '102' => ['path-102']})

        builder = instance_double('RustIndexDocumentBuilder')
        # Expect batched call for the tree walk (root, waypoint, node)
        expect(builder).to receive(:build_pui_auxiliary) do |inputs|
          expect(inputs.length).to eq(3)
          expect(inputs[0]['record_type']).to eq('tree_root')
          expect(inputs[1]['record_type']).to eq('tree_waypoint')
          expect(inputs[2]['record_type']).to eq('tree_node')
          inputs.map do |inp|
            {
              'id' => inp['uri'],
              'uri' => inp['uri'],
              'pui_parent_id' => inp['pui_parent_id'],
              'publish' => 'true',
              'primary_type' => inp['record_type'],
              'types' => ['pui'],
              'json' => ASUtils.to_json(inp['record']),
            }
          end
        end.ordered

        # Expect batched call for index_paths_to_root (2 items)
        expect(builder).to receive(:build_pui_auxiliary) do |inputs|
          expect(inputs.length).to eq(2)
          expect(inputs[0]['record_type']).to eq('tree_node_from_root')
          expect(inputs[1]['record_type']).to eq('tree_node_from_root')
          inputs.map do |inp|
            {
              'id' => inp['uri'],
              'uri' => inp['uri'],
              'pui_parent_id' => inp['pui_parent_id'],
              'publish' => 'true',
              'primary_type' => inp['record_type'],
              'types' => ['pui'],
              'json' => ASUtils.to_json(inp['record']),
            }
          end
        end.ordered

        indexer = described_class.new(batch, builder)
        indexer.add_largetree_docs([root_uri])

        expect(batch.length).to eq(5)
        # 1. tree_root
        expect(batch[0]['primary_type']).to eq('tree_root')
        expect(batch[0]['publish']).to eq('true')
        expect(batch[0]['pui_parent_id']).to eq(root_uri)
        # 2. tree_waypoint
        expect(batch[1]['primary_type']).to eq('tree_waypoint')
        expect(batch[1]['id']).to eq("#{root_uri}/tree/waypoint__0")
        # 3. tree_node
        expect(batch[2]['primary_type']).to eq('tree_node')
        expect(batch[2]['id']).to eq("#{root_uri}/tree/node_/repositories/2/archival_objects/101")
        # 4 & 5. paths to root
        expect(batch[3]['primary_type']).to eq('tree_node_from_root')
        expect(batch[4]['primary_type']).to eq('tree_node_from_root')

        # Childless node 102 was added to deletes
        expect(indexer.deletes).to eq(["#{root_uri}/tree/node_/repositories/2/archival_objects/102"])
        # No internal marker leakage
        expect(batch.none? { |d| d.keys.any? { |k| k.start_with?('_archivalist') } }).to be(true)
      ensure
        AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      end
    end

    it 'falls back to native document construction for tree walk when builder fails' do
      begin
        AppConfig[:rust_pui_auxiliary_documents_enabled] = true

        expect(JSONModel::HTTP).to receive(:get_json).with(
          root_uri + '/tree/root',
          :published_only => true
        ).and_return(root_json)

        expect(JSONModel::HTTP).to receive(:get_json).with(
          root_uri + '/tree/waypoint',
          :offset => 0,
          :parent_node => nil,
          :published_only => true
        ).and_return(waypoint_json)

        expect(JSONModel::HTTP).to receive(:get_json).with(
          root_uri + '/tree/node',
          :node_uri => '/repositories/2/archival_objects/101',
          :published_only => true
        ).and_return(node_json)

        expect(JSONModel::HTTP).to receive(:get_json).with(
          root_uri + '/tree/node_from_root',
          'node_ids[]' => [101, 102],
          :published_only => true
        ).and_return({})

        builder = instance_double('RustIndexDocumentBuilder')
        # Fail tree walk batch
        expect(builder).to receive(:build_pui_auxiliary).and_raise(RuntimeError, "sidecar timeout")

        indexer = described_class.new(batch, builder)
        indexer.add_largetree_docs([root_uri])

        # All 3 tree walk documents must be in batch via native fallback
        expect(batch.length).to eq(3)
        expect(batch[0]['primary_type']).to eq('tree_root')
        expect(batch[0]['publish']).to eq('true')
        expect(batch[1]['primary_type']).to eq('tree_waypoint')
        expect(batch[2]['primary_type']).to eq('tree_node')
        expect(indexer.deletes).to eq(["#{root_uri}/tree/node_/repositories/2/archival_objects/102"])
      ensure
        AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      end
    end

    it 'falls back to native document construction when builder fails due to noncanonical or mismatched JSON' do
      begin
        AppConfig[:rust_pui_auxiliary_documents_enabled] = true
        node_uris = ["/repositories/2/archival_objects/1"]
        paths = {'1' => ['path-1']}

        expect(JSONModel::HTTP).to receive(:get_json).and_return(paths)

        builder = instance_double('RustIndexDocumentBuilder')
        expect(builder).to receive(:build_pui_auxiliary).and_raise(RuntimeError, "json serialization mismatch for #{root_uri}/tree/node_from_root_1")

        indexer = described_class.new(batch, builder)
        indexer.index_paths_to_root(root_uri, node_uris)

        expect(batch.length).to eq(1)
        expect(batch.first['id']).to eq("#{root_uri}/tree/node_from_root_1")
        expect(batch.first['json']).to eq(ASUtils.to_json({'1' => ['path-1']}))
      ensure
        AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      end
    end
  end

  describe 'rust_pui_auxiliary_documents_enabled? gate consistency' do
    it 'supports general Ruby truthiness matching IndexerCommon flag semantics' do
      indexer = described_class.new([])

      # Unset / false
      AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be_falsey

      AppConfig[:rust_pui_auxiliary_documents_enabled] = nil
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be_falsey

      # Standard boolean true
      AppConfig[:rust_pui_auxiliary_documents_enabled] = true
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be_truthy

      # Non-boolean truthy values (strings, symbols, integers)
      AppConfig[:rust_pui_auxiliary_documents_enabled] = "true"
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be_truthy

      AppConfig[:rust_pui_auxiliary_documents_enabled] = :enabled
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be_truthy

      AppConfig[:rust_pui_auxiliary_documents_enabled] = 1
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be_truthy
    ensure
      AppConfig[:rust_pui_auxiliary_documents_enabled] = false
    end

    it 'enables tree acceleration consistently whenever ordered-records acceleration is enabled' do
      indexer = described_class.new([])
      pui_indexer = PUIIndexer.new

      # When set to a truthy string, both indexers enable acceleration
      AppConfig[:rust_pui_auxiliary_documents_enabled] = "true"
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be_truthy
      expect(pui_indexer.rust_pui_auxiliary_documents_enabled?).to be_truthy

      # When set to a truthy symbol, both indexers enable acceleration
      AppConfig[:rust_pui_auxiliary_documents_enabled] = :yes
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be_truthy
      expect(pui_indexer.rust_pui_auxiliary_documents_enabled?).to be_truthy
    ensure
      AppConfig[:rust_pui_auxiliary_documents_enabled] = false
    end
  end
end
