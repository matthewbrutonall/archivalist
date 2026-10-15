require 'spec_helper'

describe ShadowResourceTreeWaypoint do

  let(:repo_id) { 2 }
  let(:resource_id) { 18 }
  let(:resource_uri) { "/repositories/#{repo_id}/resources/#{resource_id}" }
  let(:parent_node) { "/repositories/#{repo_id}/archival_objects/1712" }
  let(:waypoint_node) do
    {
      'child_count' => 0,
      'waypoints' => 0,
      'waypoint_size' => LargeTree::WAYPOINT_SIZE,
      'title' => 'A node',
      'slugged_url' => '',
      'parsed_title' => 'A node',
      'uri' => "/repositories/#{repo_id}/archival_objects/1800",
      'position' => 0,
      'parent_id' => nil,
      'suppressed' => 0,
      'jsonmodel_type' => 'archival_object',
      'has_digital_instance' => false,
      'identifier' => nil,
      'level' => 'file',
    }
  end

  before do
    allow(AppConfig).to receive(:[]).and_call_original
    allow(AppConfig).to receive(:has_key?).and_call_original
    allow(AppConfig).to receive(:has_key?).with(:shadow_resource_tree_sidecar_enabled).and_return(true)
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_sidecar_enabled).and_return(true)
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_sidecar_url).and_return('http://127.0.0.1:4570')
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_sidecar_timeout_seconds).and_return(1)
    allow(AppConfig).to receive(:has_key?).with(:shadow_resource_tree_root_min_children).and_return(true)
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_root_min_children).and_return(200)
    allow(AppConfig).to receive(:has_key?).with(:shadow_resource_tree_node_min_children).and_return(true)
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_node_min_children).and_return(100)
  end

  it 'does not request a root waypoint below the configured child threshold' do
    expect(described_class).not_to receive(:fetch)

    expect(described_class.fetch_root(repo_id: repo_id,
                                      resource_id: resource_id,
                                      child_count: 199,
                                      published_only: false)).to be_nil
  end

  it 'does not enable root acceleration for published-only requests' do
    expect(described_class.root_enabled?(true)).to be(false)
    expect(described_class.root_enabled?(false)).to be(true)
    expect(described_class.root_enabled?(nil)).to be(true)
  end

  it 'requests a root waypoint at the configured child threshold' do
    expected = [waypoint_node]
    expect(described_class).to receive(:fetch).with(repo_id: repo_id,
                                                    resource_id: resource_id,
                                                    offset: 0,
                                                    parent_node: nil,
                                                    published_only: false)
                                             .and_return(expected)

    expect(described_class.fetch_root(repo_id: repo_id,
                                      resource_id: resource_id,
                                      child_count: 200,
                                      published_only: false)).to eq(expected)
  end

  it 'does not request a node waypoint below the configured child threshold' do
    expect(described_class).not_to receive(:fetch)

    expect(described_class.fetch_node(repo_id: repo_id,
                                      resource_id: resource_id,
                                      node_uri: parent_node,
                                      child_count: 99,
                                      published_only: false)).to be_nil
  end

  it 'requests a node waypoint at the configured child threshold' do
    expected = [waypoint_node]
    expect(described_class).to receive(:fetch).with(repo_id: repo_id,
                                                    resource_id: resource_id,
                                                    offset: 0,
                                                    parent_node: parent_node,
                                                    published_only: false)
                                             .and_return(expected)

    expect(described_class.fetch_node(repo_id: repo_id,
                                      resource_id: resource_id,
                                      node_uri: parent_node,
                                      child_count: 100,
                                      published_only: false)).to eq(expected)
  end

  it 'is default-off and does not read an absent enable setting' do
    allow(AppConfig).to receive(:has_key?).with(:shadow_resource_tree_sidecar_enabled).and_return(false)
    allow(AppConfig).to receive(:[]).with(:shadow_resource_tree_sidecar_enabled).and_raise('missing setting')

    expect(ASHTTP).not_to receive(:start_uri)
    expect(described_class.fetch(repo_id: repo_id,
                                 resource_id: resource_id,
                                 offset: 0,
                                 parent_node: nil,
                                 published_only: false)).to be_nil
  end

  it 'rejects published-only, invalid offsets, and mismatched parent repositories' do
    expect(ASHTTP).not_to receive(:start_uri)

    expect(described_class.fetch(repo_id: repo_id,
                                 resource_id: resource_id,
                                 offset: 0,
                                 parent_node: nil,
                                 published_only: true)).to be_nil
    expect(described_class.fetch(repo_id: repo_id,
                                 resource_id: resource_id,
                                 offset: -1,
                                 parent_node: nil,
                                 published_only: false)).to be_nil
    expect(described_class.fetch(repo_id: repo_id,
                                 resource_id: resource_id,
                                 offset: 0,
                                 parent_node: '/repositories/3/archival_objects/1712',
                                 published_only: false)).to be_nil
  end

  it 'returns a validated waypoint and sends the exact sidecar query' do
    response = double('Net::HTTPResponse', :code => '200', :body => ASUtils.to_json([waypoint_node]))
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
                                   offset: 0,
                                   parent_node: nil,
                                   published_only: false)

    expect(result).to eq([waypoint_node])
    expect(captured_uri.path).to eq('/shadow/resource-tree/waypoint')
    expect(URI.decode_www_form(captured_uri.query).to_h).to eq({
                                                                 'resource_uri' => resource_uri,
                                                                 'offset' => '0',
                                                               })
    expect(captured_options).to eq(:open_timeout => 1.0, :read_timeout => 1.0)
  end

  it 'accepts a valid nested waypoint' do
    nested_node = waypoint_node.merge('position' => 200, 'parent_id' => 1712)
    response = double('Net::HTTPResponse', :code => '200', :body => ASUtils.to_json([nested_node]))
    http = double('Net::HTTP', :request => response)
    captured_uri = nil
    allow(ASHTTP).to receive(:start_uri) do |uri, _options, &block|
      captured_uri = uri
      block.call(http)
    end

    result = described_class.fetch(repo_id: repo_id,
                                   resource_id: resource_id,
                                   offset: 1,
                                   parent_node: parent_node,
                                   published_only: false)

    expect(result).to eq([nested_node])
    expect(URI.decode_www_form(captured_uri.query).to_h['parent_node']).to eq(parent_node)
  end

  it 'rejects malformed successful responses' do
    malformed = waypoint_node.merge('waypoint_size' => 100)
    response = double('Net::HTTPResponse', :code => '200', :body => ASUtils.to_json([malformed]))
    http = double('Net::HTTP', :request => response)
    allow(ASHTTP).to receive(:start_uri) { |_uri, _options, &block| block.call(http) }

    expect(described_class.fetch(repo_id: repo_id,
                                 resource_id: resource_id,
                                 offset: 0,
                                 parent_node: nil,
                                 published_only: false)).to be_nil
  end

  it 'returns nil on non-success responses and transport errors' do
    response = double('Net::HTTPResponse', :code => '503', :body => '{}')
    http = double('Net::HTTP', :request => response)
    allow(ASHTTP).to receive(:start_uri) { |_uri, _options, &block| block.call(http) }

    expect(described_class.fetch(repo_id: repo_id,
                                 resource_id: resource_id,
                                 offset: 0,
                                 parent_node: nil,
                                 published_only: false)).to be_nil

    allow(ASHTTP).to receive(:start_uri).and_raise(Timeout::Error.new('timed out'))
    expect(described_class.fetch(repo_id: repo_id,
                                 resource_id: resource_id,
                                 offset: 0,
                                 parent_node: nil,
                                 published_only: false)).to be_nil
  end

end
