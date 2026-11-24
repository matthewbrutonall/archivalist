# frozen_string_literal: true

require 'minitest/autorun'
require 'tmpdir'
require_relative 'staff_full_handoff_spike'

class StaffFullHandoffSpikeTest < Minitest::Test
  def setup
    @dir = Dir.mktmpdir('staff-handoff-', File.expand_path('../benchmarks', __dir__))
    Object.const_set(:AppConfig, {}) unless defined?(AppConfig)
    AppConfig[:data_directory] = @dir
    @state = IndexState.new
    @epoch = Time.now.to_i - 120
    @summary = {
      'committed' => true,
      'full_reindex_started_at_epoch_sec' => @epoch,
      'repository_ids' => [4, 5]
    }
  end

  def test_ruby_reads_every_handoff_checkpoint_and_overlap_catches_midrun_changes
    @state.set_last_mtime(4, :resource, @epoch - 100)
    result = StaffFullHandoffSpike.apply(@summary, state: @state, promoted: true)
    assert_equal 2 + (2 * 19), result['checkpoint_keys']
    assert_equal @epoch - 30, result['ruby_modified_since']
    assert_equal @epoch, IndexState.new.get_last_mtime(4, :resource)
    assert_equal @epoch, IndexState.new.get_last_mtime(5, :job)
    assert_equal @epoch, IndexState.new.get_last_mtime('repositories', 'repositories')
    assert_equal @epoch, IndexState.new.get_last_mtime('_deletes', 'deletes')
    assert_equal 0, IndexState.new.get_last_mtime(6, :resource)
    assert_equal 0, IndexState.new('indexer_pui_state').get_last_mtime(4, :resource)

    changes = { before_overlap: @epoch - 31, during_full_pass: @epoch + 1, after_full_pass: @epoch + 100 }
    selected = changes.select { |_name, mtime| mtime >= result['ruby_modified_since'] }.keys
    assert_equal %i[during_full_pass after_full_pass], selected
    deletes = { old: @epoch - 31, during_full_pass: @epoch + 1 }
    assert_equal [:during_full_pass], deletes.select { |_name, mtime| mtime >= result['ruby_modified_since'] }.keys
  end

  def test_uncommitted_or_unpromoted_candidate_does_not_change_state
    @state.set_last_mtime(4, :resource, @epoch - 100)
    assert_raises(ArgumentError) { StaffFullHandoffSpike.apply(@summary.merge('committed' => false), state: @state, promoted: true) }
    assert_raises(ArgumentError) { StaffFullHandoffSpike.apply(@summary, state: @state, promoted: false) }
    assert_equal @epoch - 100, IndexState.new.get_last_mtime(4, :resource)
    assert_equal 0, IndexState.new.get_last_mtime('_deletes', 'deletes')
  end

  def test_invalid_future_or_string_epoch_refuses_handoff
    assert_raises(ArgumentError) do
      StaffFullHandoffSpike.apply(@summary.merge('full_reindex_started_at_epoch_sec' => Time.now.to_i + 60), state: @state, promoted: true)
    end
    assert_raises(ArgumentError) do
      StaffFullHandoffSpike.apply(@summary.merge('full_reindex_started_at_epoch_sec' => @epoch.to_s), state: @state, promoted: true)
    end
    assert_equal 0, IndexState.new.get_last_mtime(4, :resource)
  end

  def test_concurrent_incremental_advance_blocks_handoff
    @state.set_last_mtime(4, :resource, @epoch + 10)
    assert_raises(ArgumentError) { StaffFullHandoffSpike.apply(@summary, state: @state, promoted: true) }
    assert_equal @epoch + 10, IndexState.new.get_last_mtime(4, :resource)
    assert_equal 0, IndexState.new.get_last_mtime('_deletes', 'deletes')
  end
end
