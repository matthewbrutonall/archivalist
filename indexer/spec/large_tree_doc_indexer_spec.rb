require 'spec_helper'
require_relative '../app/lib/large_tree_doc_indexer'

describe LargeTreeDocIndexer do
  describe '#index_paths_to_root' do
    it 'requests and indexes paths in batches of 128 nodes' do
      root_uri = '/repositories/2/resources/16'
      node_uris = (1..129).map { |id| "/repositories/2/archival_objects/#{id}" }
      first_paths = (1..128).to_h { |id| [id.to_s, ["path-#{id}"]] }
      final_paths = {'129' => ['path-129']}
      batch = []

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
  end
end
