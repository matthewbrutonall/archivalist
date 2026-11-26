# frozen_string_literal: true

require 'fileutils'
require_relative '../../indexer/app/lib/index_state'
require_relative '../../indexer/app/lib/indexer_common_config'

module StaffFullHandoffSpike
  WINDOW_SECONDS = 30

  def self.apply(summary, state:, promoted:)
    raise ArgumentError, 'candidate was not committed' unless summary.fetch('committed') == true
    raise ArgumentError, 'candidate core has not been promoted' unless promoted == true

    epoch = summary.fetch('full_reindex_started_at_epoch_sec')
    unless epoch.is_a?(Integer) && epoch.positive? && epoch <= Time.now.to_i
      raise ArgumentError, 'invalid full reindex start'
    end

    repo_ids = summary.fetch('repository_ids')
    unless repo_ids.is_a?(Array) && !repo_ids.empty? && repo_ids.all? { |id| id.is_a?(Integer) && id.positive? } && repo_ids.uniq == repo_ids
      raise ArgumentError, 'invalid repository IDs'
    end

    keys = [['repositories', 'repositories'], ['_deletes', 'deletes']]
    repo_ids.each do |repo_id|
      IndexerCommonConfig.record_types.each { |type| keys << [repo_id, type] }
    end

    raise ArgumentError, 'incremental state advanced during full reindex' if keys.any? { |key| state.get_last_mtime(*key) > epoch }

    keys.each { |key| state.set_last_mtime(*key, epoch) }
    { 'checkpoint_keys' => keys.length, 'epoch' => epoch, 'ruby_modified_since' => [epoch - WINDOW_SECONDS, 0].max }
  end
end
