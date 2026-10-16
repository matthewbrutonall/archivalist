require 'net/http'
require 'uri'

class ShadowResourceTreeWaypoint

  REQUIRED_NODE_KEYS = %w[
    child_count
    waypoints
    waypoint_size
    title
    slugged_url
    parsed_title
    uri
    position
    parent_id
    suppressed
    jsonmodel_type
    has_digital_instance
    identifier
    level
  ].freeze

  DEFAULT_ROOT_MIN_CHILDREN = 200
  DEFAULT_NODE_MIN_CHILDREN = 100

  def self.root_enabled?(published_only)
    sidecar_enabled? && (published_only.nil? || published_only == false)
  end

  def self.fetch_root(repo_id:, resource_id:, child_count:, published_only:)
    return nil unless root_candidate?(child_count)

    fetch(repo_id: repo_id,
          resource_id: resource_id,
          offset: 0,
          parent_node: nil,
          published_only: published_only)
  end

  def self.node_enabled?(published_only)
    sidecar_enabled? && (published_only.nil? || published_only == false)
  end

  def self.fetch_node(repo_id:, resource_id:, node_uri:, child_count:, published_only:)
    return nil unless node_candidate?(child_count)

    fetch(repo_id: repo_id,
          resource_id: resource_id,
          offset: 0,
          parent_node: node_uri,
          published_only: published_only)
  end

  def self.fetch(repo_id:, resource_id:, offset:, parent_node:, published_only:)
    return nil unless eligible?(repo_id, resource_id, offset, parent_node, published_only)

    resource_uri = "/repositories/#{repo_id}/resources/#{resource_id}"
    base_url = (AppConfig[:shadow_resource_tree_sidecar_url] || 'http://127.0.0.1:4570').to_s.chomp('/')
    sidecar_uri = URI.parse("#{base_url}/shadow/resource-tree/waypoint")
    query_params = {
      'resource_uri' => resource_uri,
      'offset' => offset.to_s,
    }
    query_params['parent_node'] = parent_node if parent_node
    sidecar_uri.query = URI.encode_www_form(query_params)

    timeout = (AppConfig[:shadow_resource_tree_sidecar_timeout_seconds] || 1).to_f
    timeout = 1.0 if timeout <= 0

    request = Net::HTTP::Get.new(sidecar_uri.request_uri)
    response = ASHTTP.start_uri(sidecar_uri, :open_timeout => timeout, :read_timeout => timeout) do |http|
      http.request(request)
    end

    unless response && response.code.to_s.match?(/\A2\d\d\z/)
      status = response ? "HTTP #{response.code}" : 'no response'
      Log.warn("Shadow resource tree waypoint sidecar returned #{status}, falling back to LargeTree")
      return nil
    end

    nodes = ASUtils.json_parse(response.body)
    unless valid_response?(nodes, repo_id, offset, parent_node)
      Log.warn('Shadow resource tree waypoint sidecar returned an invalid response, falling back to LargeTree')
      return nil
    end

    Log.debug("Shadow resource tree waypoint fast path served #{resource_uri} offset #{offset}")
    nodes
  rescue StandardError => e
    Log.warn("Shadow resource tree waypoint request failed, falling back to LargeTree: #{e.class} - #{e.message}")
    nil
  end

  def self.eligible?(repo_id, resource_id, offset, parent_node, published_only)
    return false unless sidecar_enabled?
    return false unless published_only.nil? || published_only == false
    return false unless positive_integer?(repo_id) && positive_integer?(resource_id)
    return false unless offset.is_a?(Integer) && offset >= 0
    return true if parent_node.nil?

    parent_node.match?(%r{\A/repositories/#{Regexp.escape(repo_id.to_s)}/archival_objects/\d+\z})
  end

  def self.root_candidate?(child_count)
    return false unless child_count.is_a?(Integer) && child_count >= 0

    minimum = if AppConfig.has_key?(:shadow_resource_tree_root_min_children)
                AppConfig[:shadow_resource_tree_root_min_children].to_i
              else
                DEFAULT_ROOT_MIN_CHILDREN
              end
    minimum = DEFAULT_ROOT_MIN_CHILDREN if minimum <= 0

    child_count >= minimum
  end

  def self.node_candidate?(child_count)
    return false unless child_count.is_a?(Integer) && child_count >= 0

    minimum = if AppConfig.has_key?(:shadow_resource_tree_node_min_children)
                AppConfig[:shadow_resource_tree_node_min_children].to_i
              else
                DEFAULT_NODE_MIN_CHILDREN
              end
    minimum = DEFAULT_NODE_MIN_CHILDREN if minimum <= 0

    child_count >= minimum
  end

  def self.valid_response?(nodes, repo_id, offset, parent_node)
    return false unless nodes.is_a?(Array)

    expected_parent_id = parent_node && parent_node.split('/').last.to_i
    expected_position = offset * LargeTree::WAYPOINT_SIZE
    seen_uris = {}

    nodes.each_with_index.all? do |node, index|
      next false unless node.is_a?(Hash) && (REQUIRED_NODE_KEYS - node.keys).empty?
      next false unless node['uri'].is_a?(String) && node['uri'].match?(%r{\A/repositories/#{Regexp.escape(repo_id.to_s)}/archival_objects/\d+\z})
      next false if seen_uris[node['uri']]

      seen_uris[node['uri']] = true
      node['jsonmodel_type'] == 'archival_object' &&
        node['child_count'].is_a?(Integer) && node['child_count'] >= 0 &&
        node['waypoints'].is_a?(Integer) && node['waypoints'] >= 0 &&
        node['waypoint_size'] == LargeTree::WAYPOINT_SIZE &&
        node['title'].is_a?(String) &&
        node['slugged_url'].is_a?(String) &&
        node['parsed_title'].is_a?(String) &&
        node['position'] == expected_position + index &&
        node['parent_id'] == expected_parent_id &&
        [0, 1].include?(node['suppressed']) &&
        [true, false].include?(node['has_digital_instance']) &&
        (node['identifier'].nil? || node['identifier'].is_a?(String)) &&
        (node['level'].nil? || node['level'].is_a?(String)) &&
        (!node.key?('dates') || node['dates'].is_a?(Array)) &&
        (!node.key?('containers') || node['containers'].is_a?(Array))
    end
  end

  def self.positive_integer?(value)
    value.is_a?(Integer) && value > 0
  end

  def self.sidecar_enabled?
    AppConfig.has_key?(:shadow_resource_tree_sidecar_enabled) && AppConfig[:shadow_resource_tree_sidecar_enabled]
  end

  private_class_method :positive_integer?, :sidecar_enabled?

end
