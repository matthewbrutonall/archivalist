require 'spec_helper'

describe ShadowResourceTreePath do

  let(:repo_id) { 2 }
  let(:resource_id) { 18 }
  let(:resource_uri) { "/repositories/#{repo_id}/resources/#{resource_id}" }
  let(:node_ids) { (1001..1064).to_a }
  let(:path) do
    [
      {
        'node' => nil,
        'root_record_uri' => resource_uri,
        'offset' => 0,
        'jsonmodel_type' => 'resource',
        'title' => 'Resource title',
        'parsed_title' => 'Resource title',
      },
      {
        'node' => "/repositories/#{repo_id}/archival_objects/900",
        'root_record_uri' => resource_uri,
        'offset' => 1,
        'jsonmodel_type' => 'archival_object',
        'title' => 'Series title',
        'parsed_title' => 'Series title',
      },
    ]
  end
  let(:paths) { node_ids.to_h {|id| [id.to_s, path]} }

  before do
    allow(AppConfig).to receive(:[]).and_call_original
    allow(AppConfig).to receive(:has_key?).and_call_original
    allow(AppConfig).to receive(:has_key?).with(:shadow_resource_tree_sidecar_enabled).and_return(true)
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_sidecar_enabled).and_return(true)
    allow(AppConfig).to receive(:has_key?).with(:shadow_resource_tree_path_sidecar_enabled).and_return(true)
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_path_sidecar_enabled).and_return(true)
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_sidecar_url).and_return('http://127.0.0.1:4570')
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_sidecar_timeout_seconds).and_return(1)
    allow(AppConfig).to receive(:has_key?).with(:shadow_resource_tree_path_min_nodes).and_return(true)
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_path_min_nodes).and_return(64)
  end

  it 'is default-off and does not read an absent enable setting' do
    allow(AppConfig).to receive(:has_key?).with(:shadow_resource_tree_sidecar_enabled).and_return(false)
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_sidecar_enabled).and_raise('missing setting')
    expect(ASHTTP).not_to receive(:start_uri)

    expect(described_class.fetch(repo_id: repo_id,
                                 resource_id: resource_id,
                                 root_title: 'Resource title',
                                 node_ids: node_ids,
                                 published_only: true)).to be_nil
  end

  it 'remains off when the path-specific flag is disabled' do
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_path_sidecar_enabled).and_return(false)
    expect(ASHTTP).not_to receive(:start_uri)

    expect(described_class.fetch(repo_id: repo_id,
                                 resource_id: resource_id,
                                 root_title: 'Resource title',
                                 node_ids: node_ids,
                                 published_only: false)).to be_nil
  end

  it 'rejects undersized, oversized, duplicate, and invalid node batches' do
    expect(ASHTTP).not_to receive(:start_uri)

    [node_ids.first(63), (1..129).to_a, node_ids.first(63) + [node_ids.first], [0] * 64].each do |ids|
      expect(described_class.fetch(repo_id: repo_id,
                                   resource_id: resource_id,
                                   root_title: 'Resource title',
                                   node_ids: ids,
                                   published_only: false)).to be_nil
    end
  end

  it 'returns validated paths and sends the exact sidecar query' do
    response = double('Net::HTTPResponse', :code => '200', :body => ASUtils.to_json(paths))
    http = double('Net::HTTP', :request => response)
    captured_uri = nil
    captured_options = nil
    allow(ASHTTP).to receive(:start_uri) do |uri, options, &block|
      captured_uri = uri
      captured_options = options
      block.call(http)
    end

    result = described_class.fetch(repo_id: repo_id,
                                   resource_id: resource_id,
                                   root_title: 'Resource title',
                                   node_ids: node_ids,
                                   published_only: true)

    expect(result).to eq(paths)
    expect(captured_uri.path).to eq('/shadow/resource-tree/node-from-root')
    expect(URI.decode_www_form(captured_uri.query).to_h).to eq({
                                                                 'resource_uri' => resource_uri,
                                                                 'root_title' => 'Resource title',
                                                                 'node_ids' => node_ids.join(','),
                                                                 'published_only' => 'true',
                                                               })
    expect(captured_options).to eq(:open_timeout => 1.0, :read_timeout => 1.0)
  end

  it 'rejects missing paths and malformed path entries' do
    [paths.reject {|key, _| key == node_ids.first.to_s},
     paths.merge(node_ids.first.to_s => [path.first.merge('root_record_uri' => '/repositories/3/resources/18')]),
     paths.merge(node_ids.first.to_s => [path.first.merge('node' => '/repositories/2/archival_objects/1')])].each do |body|
      response = double('Net::HTTPResponse', :code => '200', :body => ASUtils.to_json(body))
      http = double('Net::HTTP', :request => response)
      allow(ASHTTP).to receive(:start_uri) { |_uri, _options, &block| block.call(http) }

      expect(described_class.fetch(repo_id: repo_id,
                                   resource_id: resource_id,
                                   root_title: 'Resource title',
                                   node_ids: node_ids,
                                   published_only: false)).to be_nil
    end
  end

  it 'returns nil on non-success responses and transport errors' do
    response = double('Net::HTTPResponse', :code => '503', :body => '{}')
    http = double('Net::HTTP', :request => response)
    allow(ASHTTP).to receive(:start_uri) { |_uri, _options, &block| block.call(http) }

    expect(described_class.fetch(repo_id: repo_id,
                                 resource_id: resource_id,
                                 root_title: 'Resource title',
                                 node_ids: node_ids,
                                 published_only: false)).to be_nil

    allow(ASHTTP).to receive(:start_uri).and_raise(Timeout::Error.new('timed out'))
    expect(described_class.fetch(repo_id: repo_id,
                                 resource_id: resource_id,
                                 root_title: 'Resource title',
                                 node_ids: node_ids,
                                 published_only: false)).to be_nil
  end

end
