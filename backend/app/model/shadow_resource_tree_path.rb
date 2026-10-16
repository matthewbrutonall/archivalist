require 'net/http'
require 'uri'

class ShadowResourceTreePath

  REQUIRED_ENTRY_KEYS = %w[
    node
    root_record_uri
    offset
    jsonmodel_type
    title
    parsed_title
  ].freeze

  DEFAULT_MIN_NODES = 64
  MAX_NODES = 128

  def self.fetch(repo_id:, resource_id:, root_title:, node_ids:, published_only:)
    return nil unless eligible?(repo_id, resource_id, root_title, node_ids)

    resource_uri = "/repositories/#{repo_id}/resources/#{resource_id}"
    base_url = (AppConfig[:shadow_resource_tree_sidecar_url] || 'http://127.0.0.1:4570').to_s.chomp('/')
    sidecar_uri = URI.parse("#{base_url}/shadow/resource-tree/node-from-root")
    sidecar_uri.query = URI.encode_www_form('resource_uri' => resource_uri,
                                             'root_title' => root_title,
                                             'node_ids' => node_ids.join(','),
                                             'published_only' => published_only ? 'true' : 'false')

    timeout = (AppConfig[:shadow_resource_tree_sidecar_timeout_seconds] || 1).to_f
    timeout = 1.0 if timeout <= 0
    response = ASHTTP.start_uri(sidecar_uri, :open_timeout => timeout, :read_timeout => timeout) do |http|
      http.request(Net::HTTP::Get.new(sidecar_uri.request_uri))
    end

    unless response && response.code.to_s.match?(/\A2\d\d\z/)
      status = response ? "HTTP #{response.code}" : 'no response'
      Log.warn("Shadow resource tree path sidecar returned #{status}, falling back to LargeTree")
      return nil
    end

    paths = ASUtils.json_parse(response.body)
    unless valid_response?(paths, repo_id, resource_uri, node_ids)
      Log.warn('Shadow resource tree path sidecar returned an invalid response, falling back to LargeTree')
      return nil
    end

    Log.debug("Shadow resource tree path fast path served #{resource_uri} for #{node_ids.length} nodes")
    paths
  rescue StandardError => e
    Log.warn("Shadow resource tree path request failed, falling back to LargeTree: #{e.class} - #{e.message}")
    nil
  end

  def self.eligible?(repo_id, resource_id, root_title, node_ids)
    return false unless sidecar_enabled?
    return false unless positive_integer?(repo_id) && positive_integer?(resource_id)
    return false unless root_title.is_a?(String) && !root_title.empty?
    return false unless node_ids.is_a?(Array) && node_ids.length.between?(minimum_nodes, MAX_NODES)

    node_ids.all? {|id| positive_integer?(id)} && node_ids.uniq.length == node_ids.length
  end

  def self.valid_response?(paths, repo_id, resource_uri, node_ids)
    return false unless paths.is_a?(Hash) && paths.keys.sort == node_ids.map(&:to_s).sort

    paths.all? do |_node_id, path|
      next false unless path.is_a?(Array) && !path.empty?

      path.each_with_index.all? do |entry, index|
        next false unless entry.is_a?(Hash) && (REQUIRED_ENTRY_KEYS - entry.keys).empty?
        next false unless entry['root_record_uri'] == resource_uri
        next false unless entry['offset'].is_a?(Integer) && entry['offset'] >= 0
        next false unless entry['title'].is_a?(String) && entry['parsed_title'].is_a?(String)

        if index == 0
          entry['node'].nil? && entry['jsonmodel_type'] == 'resource'
        else
          entry['jsonmodel_type'] == 'archival_object' &&
            entry['node'].is_a?(String) &&
            entry['node'].match?(%r{\A/repositories/#{Regexp.escape(repo_id.to_s)}/archival_objects/\d+\z})
        end
      end
    end
  end

  def self.minimum_nodes
    minimum = if AppConfig.has_key?(:shadow_resource_tree_path_min_nodes)
                AppConfig[:shadow_resource_tree_path_min_nodes].to_i
              else
                DEFAULT_MIN_NODES
              end
    minimum.between?(1, MAX_NODES) ? minimum : DEFAULT_MIN_NODES
  end

  def self.positive_integer?(value)
    value.is_a?(Integer) && value > 0
  end

  def self.sidecar_enabled?
    AppConfig.has_key?(:shadow_resource_tree_sidecar_enabled) && AppConfig[:shadow_resource_tree_sidecar_enabled]
  end

  private_class_method :minimum_nodes, :positive_integer?, :sidecar_enabled?

end
