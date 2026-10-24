#!/usr/bin/env ruby
# frozen_string_literal: true

# Offline unit tests for pui_concurrency_safety_gate.rb (no live backend, Solr, or sidecar).
# Run with MRI or JRuby: ruby rust-tools/scripts/test_pui_concurrency_safety_gate.rb

require 'minitest/autorun'
require 'open3'
require_relative 'pui_concurrency_safety_gate'

class TestPuiConcurrencySafetyGate < Minitest::Test
  def test_validate_repo_id_accepts_only_repo_3
    assert_equal 3, ArchivalistPuiConcurrencyGate.validate_repo_id!('3', identity_repo_id: 2)
    assert_equal 3, ArchivalistPuiConcurrencyGate.validate_repo_id!(3, identity_repo_id: 2)

    # Reject identity repo 2 and medium repo 4
    err2 = assert_raises(RuntimeError) { ArchivalistPuiConcurrencyGate.validate_repo_id!('2', identity_repo_id: 2) }
    assert_match(/forbidden/, err2.message)
    err4 = assert_raises(RuntimeError) { ArchivalistPuiConcurrencyGate.validate_repo_id!('4', identity_repo_id: 2) }
    assert_match(/medium repo/, err4.message)

    # Reject other repo IDs and malformed inputs
    assert_raises(RuntimeError) { ArchivalistPuiConcurrencyGate.validate_repo_id!('5', identity_repo_id: 2) }
    assert_raises(RuntimeError) { ArchivalistPuiConcurrencyGate.validate_repo_id!('0', identity_repo_id: 2) }
    assert_raises(RuntimeError) { ArchivalistPuiConcurrencyGate.validate_repo_id!('3; DROP', identity_repo_id: 2) }
    assert_raises(RuntimeError) { ArchivalistPuiConcurrencyGate.validate_repo_id!(nil, identity_repo_id: 2) }
  end

  def test_configure_and_reset_gate_flags_strict_scope
    cfg = {
      pui_indexer_thread_count: 1,
      pui_indexer_records_per_thread: 25
    }
    ArchivalistPuiConcurrencyGate::ALL_RUST_FLAGS.each { |f| cfg[f] = false }

    ArchivalistPuiConcurrencyGate.configure_gate_flags!(cfg, sidecar_url: 'http://127.0.0.1:4599', threads: 4)
    assert_equal 4, cfg[:pui_indexer_thread_count]
    assert_equal 25, cfg[:pui_indexer_records_per_thread]
    assert_equal 'http://127.0.0.1:4599', cfg[:rust_index_document_builder_url]

    ArchivalistPuiConcurrencyGate::ENABLED_RUST_FLAGS.each do |flag|
      assert_equal true, cfg[flag], "Expected #{flag} to be enabled"
    end
    ArchivalistPuiConcurrencyGate::DISABLED_RUST_FLAGS.each do |flag|
      assert_equal false, cfg[flag], "Expected #{flag} to remain false"
    end

    # Perturbing a disabled flag fails verification
    cfg[:rust_pui_auxiliary_documents_enabled] = true
    assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.verify_gate_flags!(cfg, sidecar_url: 'http://127.0.0.1:4599', threads: 4)
    end

    ArchivalistPuiConcurrencyGate.reset_gate_flags!(cfg, default_pui_threads: 1)
    assert_equal 1, cfg[:pui_indexer_thread_count]
    ArchivalistPuiConcurrencyGate::ALL_RUST_FLAGS.each do |flag|
      assert_equal false, cfg[flag], "Expected #{flag} to be reset to false"
    end
  end

  def valid_instrumentation(records: 100, batches: 4, threads: 4)
    inst = ArchivalistPuiConcurrencyGate.new_instrumentation(threads: threads, records_per_thread: 25)
    inst['worker_batches_dispatched'] = batches
    inst['fetch_batches'] = batches
    inst['records_fetched'] = records
    inst['index_batches'] = batches
    inst['builder_calls'] = batches
    inst['builder_inputs_sent'] = records
    inst['builder_docs_received'] = records
    inst['native_fallbacks'] = 0
    inst['accepted_base_docs'] = records
    inst['accepted_routing'] = records
    inst['accepted_notes'] = records
    inst['accepted_final'] = records
    inst['accepted_candidate'] = records
    ArchivalistPuiConcurrencyGate::COMMON_HELPERS.each do |h|
      inst['accepted_common_helpers'][h] = records
    end
    inst['rejections'] = 0
    inst['thread_leak_violations'] = 0
    inst['worker_statuses'] = { 'archival_object' => Array.new(threads, 1) }
    inst['fetch_thread_ids'] = (1..threads).to_a
    inst['index_thread_ids'] = (1..threads).to_a
    inst['builder_thread_ids'] = (1..threads).to_a
    inst
  end

  def test_validate_acceptance_passes_on_complete_acceptance_and_fails_on_any_fallback_or_rejection
    inst = valid_instrumentation(records: 100, batches: 4, threads: 4)
    assert ArchivalistPuiConcurrencyGate.validate_acceptance!(
      inst,
      expected_records: 100,
      expected_batches: 4,
      expected_threads: 4,
      sidecar_requests_delta: 4
    )

    # Silent native fallback must fail
    bad_fb = valid_instrumentation
    bad_fb['native_fallbacks'] = 1
    bad_fb['accepted_base_docs'] = 99
    err = assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.validate_acceptance!(
        bad_fb, expected_records: 100, expected_batches: 4, expected_threads: 4, sidecar_requests_delta: 4
      )
    end
    assert_match(/native_fallbacks/, err.message)

    # Single rejection must fail
    bad_rej = valid_instrumentation
    bad_rej['rejections'] = 1
    assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.validate_acceptance!(
        bad_rej, expected_records: 100, expected_batches: 4, expected_threads: 4, sidecar_requests_delta: 4
      )
    end

    # Missing candidate acceptance must fail
    bad_cand = valid_instrumentation
    bad_cand['accepted_candidate'] = 99
    assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.validate_acceptance!(
        bad_cand, expected_records: 100, expected_batches: 4, expected_threads: 4, sidecar_requests_delta: 4
      )
    end

    # Missing common helper acceptance must fail
    bad_helper = valid_instrumentation
    bad_helper['accepted_common_helpers']['subjects'] = 99
    assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.validate_acceptance!(
        bad_helper, expected_records: 100, expected_batches: 4, expected_threads: 4, sidecar_requests_delta: 4
      )
    end

    # Worker status error (2) must fail
    bad_status = valid_instrumentation
    bad_status['worker_statuses'] = { 'archival_object' => [1, 2, 1, 1] }
    assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.validate_acceptance!(
        bad_status, expected_records: 100, expected_batches: 4, expected_threads: 4, sidecar_requests_delta: 4
      )
    end

    # Fewer observed threads than configured must fail
    bad_threads = valid_instrumentation
    bad_threads['index_thread_ids'] = [1, 2, 3]
    assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.validate_acceptance!(
        bad_threads, expected_records: 100, expected_batches: 4, expected_threads: 4, sidecar_requests_delta: 4
      )
    end

    # Thread leak violation must fail
    bad_leak = valid_instrumentation
    bad_leak['thread_leak_violations'] = 1
    assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.validate_acceptance!(
        bad_leak, expected_records: 100, expected_batches: 4, expected_threads: 4, sidecar_requests_delta: 4
      )
    end
  end

  def test_verify_snapshot_equivalence
    d1 = { '/repositories/3/archival_objects/1' => 'aaa', '/repositories/3/archival_objects/2' => 'bbb' }
    d2 = { '/repositories/3/archival_objects/2' => 'bbb', '/repositories/3/archival_objects/1' => 'aaa' }
    assert ArchivalistPuiConcurrencyGate.verify_snapshot_equivalence!(d1, d2, expected_records: 2)

    # Altered digest fails
    d_bad = { '/repositories/3/archival_objects/1' => 'aaa', '/repositories/3/archival_objects/2' => 'ccc' }
    assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.verify_snapshot_equivalence!(d1, d_bad, expected_records: 2)
    end

    # Count mismatch fails
    assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.verify_snapshot_equivalence!(d1, d2, expected_records: 3)
    end
  end

  def test_cursor_paginated_comparison_strict_types_and_privacy
    core_a = 'stage_pui_20261024_000001_100_aaaaaaaa_candidate'
    core_b = 'stage_pui_20261024_000001_100_bbbbbbbb_candidate'

    pages_by_core = {
      [core_a, '*'] => {
        'response' => {
          'numFound' => 2,
          'docs' => [{ 'id' => '/repositories/3/archival_objects/1#pui', 'publish' => true, 'json' => '{"uri":"/repositories/3/archival_objects/1"}', '_version_' => 10 }]
        },
        'nextCursorMark' => 'c1'
      },
      [core_a, 'c1'] => {
        'response' => {
          'numFound' => 2,
          'docs' => [{ 'id' => '/repositories/3/archival_objects/2#pui', 'publish' => true, 'types' => %w[pui pui_record], 'score' => 1.0 }]
        },
        'nextCursorMark' => 'c1'
      },
      [core_b, '*'] => {
        'response' => {
          'numFound' => 2,
          'docs' => [{ 'id' => '/repositories/3/archival_objects/1#pui', 'publish' => true, 'json' => '{"uri":"/repositories/3/archival_objects/1"}', '_version_' => 99 }]
        },
        'nextCursorMark' => 'c1'
      },
      [core_b, 'c1'] => {
        'response' => {
          'numFound' => 2,
          'docs' => [{ 'id' => '/repositories/3/archival_objects/2#pui', 'publish' => true, 'types' => %w[pui pui_record], '_version_' => 100 }]
        },
        'nextCursorMark' => 'c1'
      }
    }

    getter = lambda do |url|
      uri = URI(url)
      core = uri.path.split('/')[2]
      params = URI.decode_www_form(uri.query).to_h
      pages_by_core.fetch([core, params.fetch('cursorMark')])
    end

    res = ArchivalistPuiConcurrencyGate.compare_cores_cursor_paginated!(
      'http://archivalist-profiling-solr:8983/solr',
      core_a,
      core_b,
      expected_count: 2,
      page_size: 1,
      http_get_json: getter
    )
    assert_equal 2, res['num_found']
    assert_equal 0, res['mismatches']

    # Array order difference must fail
    pages_by_core[[core_b, 'c1']] = {
      'response' => {
        'numFound' => 2,
        'docs' => [{ 'id' => '/repositories/3/archival_objects/2#pui', 'publish' => true, 'types' => %w[pui_record pui] }]
      },
      'nextCursorMark' => 'c1'
    }
    err = assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.compare_cores_cursor_paginated!(
        'http://archivalist-profiling-solr:8983/solr', core_a, core_b, expected_count: 2, page_size: 1, http_get_json: getter
      )
    end
    assert_match(/types\[0\]/, err.message)

    # Leaked _archivalist_ marker must fail even if both cores have it
    pages_by_core[[core_a, 'c1']]['response']['docs'][0]['_archivalist_pui_final'] = 'leak'
    pages_by_core[[core_b, 'c1']] = Marshal.load(Marshal.dump(pages_by_core[[core_a, 'c1']]))
    err_leak = assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.compare_cores_cursor_paginated!(
        'http://archivalist-profiling-solr:8983/solr', core_a, core_b, expected_count: 2, page_size: 1, http_get_json: getter
      )
    end
    assert_match(/Internal marker leak/, err_leak.message)
  end

  def test_run_worker_batches_for_repo_spawns_threads_and_commits
    fake_queue_class = Class.new do
      def initialize
        @q = Queue.new
      end

      def offer(val, _timeout = nil, _unit = nil)
        @q << val
        true
      end

      def poll
        @q.pop
      end
    end

    processed_slices = []
    commits = 0
    slice_mutex = Mutex.new

    fake_indexer = Object.new
    fake_indexer.define_singleton_method(:start_worker_thread) do |queue, type|
      Thread.new do
        status = 0
        loop do
          item = queue.poll
          break if item == :finished

          slice_mutex.synchronize { processed_slices << [type, item] }
          status = 1
        end
        status
      end
    end
    fake_indexer.define_singleton_method(:send_commit) { commits += 1 }

    mutex = Mutex.new
    inst = ArchivalistPuiConcurrencyGate.new_instrumentation(threads: 4, records_per_thread: 2)
    ArchivalistPuiConcurrencyGate.run_worker_batches_for_repo(
      fake_indexer,
      { archival_object: [1, 2, 3, 4, 5, 6, 7, 8] },
      threads: 4,
      records_per_thread: 2,
      mutex: mutex,
      instrumentation: inst,
      queue_factory: ->(_t) { fake_queue_class.new }
    )

    assert_equal 4, inst['worker_batches_dispatched']
    assert_equal 1, commits
    assert_equal [[1, 2], [3, 4], [5, 6], [7, 8]], processed_slices.map(&:last).sort
  end

  def test_prepare_isolated_data_dir_and_with_pass_harness_state_preserves_directory
    bench_base = File.expand_path('../benchmarks/pui_concurrency_gate_index_state/unit_test_runs', __dir__)
    FileUtils.mkdir_p(bench_base)
    primary_dir = '/archivesspace/data/primary'

    # Rejects overlapping primary data_directory
    assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.prepare_isolated_data_dir!(
        base_dir: primary_dir,
        run_id: '20261024_000001_1_aaaaaaaa',
        threads: 1,
        primary_data_dir: primary_dir
      )
    end
    assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.prepare_isolated_data_dir!(
        base_dir: "#{primary_dir}/sub",
        run_id: '20261024_000001_1_aaaaaaaa',
        threads: 1,
        primary_data_dir: primary_dir
      )
    end

    fake_builder_class = Class.new do
      def build(inputs)
        inputs.to_h { |i| [i, { 'ok' => true }] }
      end
    end
    orig_build_method = fake_builder_class.instance_method(:build)

    cfg = {
      data_directory: primary_dir,
      solr_url: 'http://archivalist-profiling-solr:8983/solr',
      pui_indexer_thread_count: 1,
      pui_indexer_records_per_thread: 25
    }
    ArchivalistPuiConcurrencyGate::ALL_RUST_FLAGS.each { |f| cfg[f] = false }

    mutex = Mutex.new
    inst = ArchivalistPuiConcurrencyGate.new_instrumentation(threads: 4, records_per_thread: 25)
    yielded_dir = nil

    ArchivalistPuiConcurrencyGate.with_pass_harness_state(
      app_config: cfg,
      builder_class: fake_builder_class,
      sidecar_url: 'http://127.0.0.1:4599',
      threads: 4,
      default_pui_threads: 1,
      target_solr_url: 'http://archivalist-profiling-solr:8983/solr/stage_pui_20261024_000001_1_aaaaaaaa_candidate',
      index_state_base_dir: bench_base,
      run_id: '20261024_000001_1_aaaaaaaa',
      mutex: mutex,
      instrumentation: inst
    ) do |isolated_dir|
      yielded_dir = isolated_dir
      assert_equal isolated_dir, cfg[:data_directory]
      refute_equal primary_dir, cfg[:data_directory]
      assert File.directory?(File.join(isolated_dir, 'indexer_pui_state'))
      res = fake_builder_class.new.build(%w[u1 u2])
      assert_equal 2, res.length
    end

    # Directory is preserved on disk (never deleted) and recorded in instrumentation
    refute_nil yielded_dir
    assert File.directory?(yielded_dir), "Expected isolated index-state dir #{yielded_dir} to be preserved"
    assert File.directory?(File.join(yielded_dir, 'indexer_pui_state'))
    assert_equal yielded_dir, inst['index_state_data_dir']
    assert_equal 1, inst['builder_calls']
    assert_equal 2, inst['builder_inputs_sent']
    assert_equal 2, inst['builder_docs_received']

    # AppConfig and builder method are restored after pass completes
    assert_equal primary_dir, cfg[:data_directory]
    assert_equal 'http://archivalist-profiling-solr:8983/solr', cfg[:solr_url]
    assert_equal 1, cfg[:pui_indexer_thread_count]
    ArchivalistPuiConcurrencyGate::ALL_RUST_FLAGS.each do |flag|
      assert_equal false, cfg[flag]
    end
    assert_equal orig_build_method, fake_builder_class.instance_method(:build)
  end

  def test_early_queue_error_joins_workers_before_outer_ensure_restores_state
    bench_base = File.expand_path('../benchmarks/pui_concurrency_gate_index_state/unit_test_runs', __dir__)
    FileUtils.mkdir_p(bench_base)
    primary_dir = '/archivesspace/data/primary'

    worker_entered_batch = Queue.new
    release_worker_batch = Queue.new
    worker_observed_during_error = Queue.new
    spawned_workers = []
    spawned_mutex = Mutex.new

    fake_builder_class = Class.new do
      def build(inputs)
        inputs.to_h { |i| [i, { 'ok' => true }] }
      end
    end

    cfg = {
      data_directory: primary_dir,
      solr_url: 'http://archivalist-profiling-solr:8983/solr',
      pui_indexer_thread_count: 1,
      pui_indexer_records_per_thread: 2
    }
    ArchivalistPuiConcurrencyGate::ALL_RUST_FLAGS.each { |f| cfg[f] = false }

    # Queue allows first batch slice, then waits until a worker is actively running batch 1
    # before raising a queue failure on batch 2.
    failing_queue_class = Class.new do
      def initialize(worker_entered_q, release_worker_q)
        @q = Queue.new
        @batch_offers = 0
        @worker_entered_q = worker_entered_q
        @release_worker_q = release_worker_q
      end

      def offer(val, _timeout = nil, _unit = nil)
        if val == :finished
          @q << :finished
          return true
        end

        @batch_offers += 1
        if @batch_offers == 1
          @q << val
          true
        else
          # Wait until worker 1 has started batch 1, then release it right as the queue raises
          @worker_entered_q.pop
          @release_worker_q << true
          raise 'simulated early queue offer failure'
        end
      end

      def poll
        @q.pop
      end

      def clear
        @q.clear
      end
    end

    mutex = Mutex.new
    inst = ArchivalistPuiConcurrencyGate.new_instrumentation(threads: 4, records_per_thread: 2)

    fake_indexer = Object.new
    fake_indexer.define_singleton_method(:start_worker_thread) do |queue, _type|
      t = Thread.new do
        status = 0
        loop do
          item = queue.poll
          break if item == :finished

          worker_entered_batch << true
          release_worker_batch.pop
          # Sleep briefly so if run_worker_batches_for_repo failed to join in ensure,
          # outer with_pass_harness_state ensure would already have reset cfg and builder!
          sleep 0.03
          builder_res = fake_builder_class.new.build(item)
          worker_observed_during_error << {
            data_dir: cfg[:data_directory],
            pui_prep_flag: cfg[:rust_pui_document_preparation_enabled],
            threads_flag: cfg[:pui_indexer_thread_count],
            builder_count: builder_res.length
          }
          status = 1
        end
        status
      end
      spawned_mutex.synchronize { spawned_workers << t }
      t
    end
    fake_indexer.define_singleton_method(:send_commit) { raise 'send_commit must not be called on failure' }

    err = assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.with_pass_harness_state(
        app_config: cfg,
        builder_class: fake_builder_class,
        sidecar_url: 'http://127.0.0.1:4599',
        threads: 4,
        default_pui_threads: 1,
        target_solr_url: 'http://archivalist-profiling-solr:8983/solr/stage_pui_20261024_000001_1_bbbbbbbb_candidate',
        index_state_base_dir: bench_base,
        run_id: '20261024_000001_1_bbbbbbbb',
        mutex: mutex,
        instrumentation: inst
      ) do |_isolated_dir|
        ArchivalistPuiConcurrencyGate.run_worker_batches_for_repo(
          fake_indexer,
          { archival_object: [1, 2, 3, 4, 5, 6] },
          threads: 4,
          records_per_thread: 2,
          mutex: mutex,
          instrumentation: inst,
          queue_factory: ->(_t) { failing_queue_class.new(worker_entered_batch, release_worker_batch) }
        )
      end
    end

    assert_match(/simulated early queue offer failure/, err.message)

    # All 4 gate-owned worker threads were joined before unwinding completed
    assert_equal 4, spawned_workers.length
    assert spawned_workers.none?(&:alive?), 'Expected all gate-owned worker threads to be stopped before outer ensure finished'

    # While worker was active during error unwinding, pass state had NOT been prematurely restored
    observed = worker_observed_during_error.pop
    assert_equal inst['index_state_data_dir'], observed[:data_dir]
    assert_equal true, observed[:pui_prep_flag]
    assert_equal 4, observed[:threads_flag]
    assert_equal 2, observed[:builder_count]
    assert_equal 1, inst['builder_calls']

    # After outer ensure finished, AppConfig was restored and isolated data_dir was preserved
    assert_equal primary_dir, cfg[:data_directory]
    assert_equal 1, cfg[:pui_indexer_thread_count]
    ArchivalistPuiConcurrencyGate::ALL_RUST_FLAGS.each { |flag| assert_equal false, cfg[flag] }
    assert File.directory?(inst['index_state_data_dir'])
  end

  def test_early_worker_exception_with_full_queue_joins_remaining_workers_and_wedged_worker_is_terminated
    # Part 1: Worker 1 raises an unhandled exception while queue is full; remaining workers are joined cleanly
    full_queue_class = Class.new do
      def initialize
        @q = Queue.new
        @cleared = false
      end

      def offer(val, _timeout = nil, _unit = nil)
        if val == :finished || @cleared
          @q << val
          true
        else
          false
        end
      end

      def poll
        @q.pop
      end

      def clear
        @cleared = true
        @q.clear
      end
    end

    workers_part1 = []
    w_mutex = Mutex.new
    indexer_part1 = Object.new
    indexer_part1.define_singleton_method(:start_worker_thread) do |queue, _type|
      idx = w_mutex.synchronize { workers_part1.length }
      t = Thread.new do
        t_cur = Thread.current
        t_cur.report_on_exception = false if t_cur.respond_to?(:report_on_exception=)
        raise 'unhandled worker thread crash' if idx == 0

        loop do
          break if queue.poll == :finished
        end
        1
      end
      w_mutex.synchronize { workers_part1 << t }
      t
    end
    indexer_part1.define_singleton_method(:send_commit) {}

    mutex1 = Mutex.new
    inst1 = ArchivalistPuiConcurrencyGate.new_instrumentation(threads: 3, records_per_thread: 2)
    err1 = assert_raises(RuntimeError) do
      ArchivalistPuiConcurrencyGate.run_worker_batches_for_repo(
        indexer_part1,
        { archival_object: [1, 2, 3, 4] },
        threads: 3,
        records_per_thread: 2,
        mutex: mutex1,
        instrumentation: inst1,
        queue_factory: ->(_t) { full_queue_class.new }
      )
    end
    assert_match(/unhandled worker thread crash/, err1.message)
    assert_equal 3, workers_part1.length
    assert workers_part1.none?(&:alive?), 'Expected all remaining worker threads to be joined on early worker crash'

    # Part 2: Wedged gate-owned worker ignores :finished -> terminated after bounded join_timeout_s,
    # unrelated thread remains untouched, and gate fails closed.
    unrelated_stop = Queue.new
    unrelated_thread = Thread.new { unrelated_stop.pop }
    wedged_workers = []

    simple_queue_class = Class.new do
      def initialize
        @q = Queue.new
      end

      def offer(val, _timeout = nil, _unit = nil)
        @q << val
        true
      end

      def poll
        @q.pop
      end

      def clear
        @q.clear
      end
    end

    indexer_part2 = Object.new
    indexer_part2.define_singleton_method(:start_worker_thread) do |queue, _type|
      idx = w_mutex.synchronize { wedged_workers.length }
      t = Thread.new do
        if idx == 0
          # Wedged worker ignores queue :finished tokens until killed
          sleep 60
        else
          loop do
            break if queue.poll == :finished
          end
        end
        1
      end
      w_mutex.synchronize { wedged_workers << t }
      t
    end
    indexer_part2.define_singleton_method(:send_commit) { raise 'must not commit when worker is wedged' }

    mutex2 = Mutex.new
    inst2 = ArchivalistPuiConcurrencyGate.new_instrumentation(threads: 2, records_per_thread: 2)
    begin
      err2 = assert_raises(RuntimeError) do
        ArchivalistPuiConcurrencyGate.run_worker_batches_for_repo(
          indexer_part2,
          { archival_object: [1, 2] },
          threads: 2,
          records_per_thread: 2,
          mutex: mutex2,
          instrumentation: inst2,
          queue_factory: ->(_t) { simple_queue_class.new },
          join_timeout_s: 0.05
        )
      end
      assert_match(/Terminated 1 gate-owned worker thread/, err2.message)
      assert_equal 1, inst2['terminated_worker_threads']
      assert wedged_workers.none?(&:alive?), 'Expected wedged gate-owned worker thread to be terminated'
      assert unrelated_thread.alive?, 'Unrelated thread must never be terminated by gate cleanup'
    ensure
      unrelated_stop << true
      unrelated_thread.join(2)
    end
  end

  def test_shell_preflight_refuses_preexisting_running_isolated_services_and_preserves_ownership
    test_root = File.expand_path(
      "../benchmarks/pui_concurrency_gate_index_state/unit_test_runs/shell_preflight_#{Process.pid}_#{SecureRandom.hex(4)}",
      __dir__
    )
    bin_dir = File.join(test_root, 'bin')
    FileUtils.mkdir_p(bin_dir)

    stack_state_path = File.join(test_root, 'stack_state.json')
    docker_log_path = File.join(test_root, 'docker_calls.log')
    inspect_map_path = File.join(test_root, 'inspect_map.json')

    stack_id = 'teststack01'
    containers = {
      'db' => "archivalist-profiling-db-#{stack_id}",
      'solr' => "archivalist-profiling-solr-#{stack_id}",
      'backend' => "archivalist-profiling-backend-#{stack_id}"
    }
    File.write(
      stack_state_path,
      JSON.pretty_generate(
        'stack_id' => stack_id,
        'containers' => containers,
        'solr_host_url' => 'http://127.0.0.1:18983/solr',
        'backend_host_url' => 'http://127.0.0.1:14567'
      )
    )

    mock_docker_path = File.join(bin_dir, 'docker')
    File.write(mock_docker_path, <<~'PY')
      #!/usr/bin/env python3
      import json, os, sys

      log_path = os.environ["MOCK_DOCKER_LOG"]
      inspect_map_path = os.environ["MOCK_DOCKER_INSPECT_MAP"]
      args = sys.argv[1:]
      with open(log_path, "a") as fh:
          fh.write(" ".join(args) + "\n")

      with open(inspect_map_path) as fh:
          inspect_map = json.load(fh)

      if args[:2] == ["container", "inspect"] and len(args) == 3:
          name = args[2]
          if name not in inspect_map:
              sys.exit(1)
          print(json.dumps([inspect_map[name]]))
          sys.exit(0)

      if args[:1] == ["start"] and len(args) == 2:
          fail_on = os.environ.get("MOCK_DOCKER_FAIL_START", "")
          if args[1] == fail_on:
              sys.stderr.write(f"mock start failure for {args[1]}\n")
              sys.exit(1)
          sys.exit(0)

      if args[:1] == ["stop"] and len(args) == 2:
          sys.exit(0)

      if args[:1] == ["ps"]:
          sys.exit(0)

      sys.exit(1)
    PY
    File.chmod(0o755, mock_docker_path)

    build_inspect_map = lambda do |solr_running:, backend_running:, main_solr_running: false, bench_solr_running: false|
      {
        containers['db'] => {
          'Config' => { 'Labels' => { 'archivalist.profiling-stack' => stack_id, 'archivalist.profiling-role' => 'db' } },
          'State' => { 'Running' => true, 'Status' => 'running' }
        },
        containers['solr'] => {
          'Config' => { 'Labels' => { 'archivalist.profiling-stack' => stack_id, 'archivalist.profiling-role' => 'solr' } },
          'State' => { 'Running' => solr_running, 'Status' => solr_running ? 'running' : 'exited' }
        },
        containers['backend'] => {
          'Config' => { 'Labels' => { 'archivalist.profiling-stack' => stack_id, 'archivalist.profiling-role' => 'backend' } },
          'State' => { 'Running' => backend_running, 'Status' => backend_running ? 'running' : 'exited' }
        },
        'archivesspaceai-solr-1' => {
          'Config' => { 'Labels' => {} },
          'State' => { 'Running' => main_solr_running, 'Status' => main_solr_running ? 'running' : 'exited' }
        },
        'archivesspaceai-solr-bench' => {
          'Config' => { 'Labels' => {} },
          'State' => { 'Running' => bench_solr_running, 'Status' => bench_solr_running ? 'running' : 'exited' }
        }
      }
    end

    script_path = File.expand_path('run_pui_concurrency_safety_gate.sh', __dir__)
    base_env = {
      'PATH' => "#{bin_dir}:#{ENV.fetch('PATH', '')}",
      'STACK_STATE' => stack_state_path,
      'MOCK_DOCKER_LOG' => docker_log_path,
      'MOCK_DOCKER_INSPECT_MAP' => inspect_map_path,
      'BACKEND_URL' => nil,
      'SOLR_BASE_URL' => nil
    }

    # Case 1: Isolated Solr already running -> refuses in preflight, zero start/stop calls
    File.write(docker_log_path, '')
    File.write(inspect_map_path, JSON.pretty_generate(build_inspect_map.call(solr_running: true, backend_running: false)))
    _out1, err1, status1 = Open3.capture3(base_env, 'bash', script_path)
    refute status1.success?, 'Expected preflight to fail when isolated Solr is already running'
    assert_match(/Refusing to run while isolated solr container #{Regexp.escape(containers['solr'])} is already running/, err1)
    calls1 = File.readlines(docker_log_path, chomp: true)
    assert calls1.none? { |c| c.start_with?('start ') || c.start_with?('stop ') },
           "Expected zero docker start/stop calls when isolated Solr was preexisting, got: #{calls1.inspect}"

    # Case 2: Isolated backend already running -> refuses in preflight, zero start/stop calls
    File.write(docker_log_path, '')
    File.write(inspect_map_path, JSON.pretty_generate(build_inspect_map.call(solr_running: false, backend_running: true)))
    _out2, err2, status2 = Open3.capture3(base_env, 'bash', script_path)
    refute status2.success?, 'Expected preflight to fail when isolated backend is already running'
    assert_match(/Refusing to run while isolated backend container #{Regexp.escape(containers['backend'])} is already running/, err2)
    calls2 = File.readlines(docker_log_path, chomp: true)
    assert calls2.none? { |c| c.start_with?('start ') || c.start_with?('stop ') },
           "Expected zero docker start/stop calls when isolated backend was preexisting, got: #{calls2.inspect}"

    # Case 3: Bench Solr running -> refuses in preflight, zero start/stop calls
    File.write(docker_log_path, '')
    File.write(
      inspect_map_path,
      JSON.pretty_generate(build_inspect_map.call(solr_running: false, backend_running: false, bench_solr_running: true))
    )
    _out3, err3, status3 = Open3.capture3(base_env, 'bash', script_path)
    refute status3.success?, 'Expected preflight to fail when bench Solr is running'
    assert_match(/Refusing to run while archivesspaceai-solr-bench is running/, err3)
    calls3 = File.readlines(docker_log_path, chomp: true)
    assert calls3.none? { |c| c.start_with?('start ') || c.start_with?('stop ') }

    # Case 4: Both isolated Solr and backend stopped initially; Solr starts, backend start fails ->
    # cleanup trap stops ONLY Solr (started by this invocation) and never stops backend, DB, or main/bench.
    File.write(docker_log_path, '')
    File.write(inspect_map_path, JSON.pretty_generate(build_inspect_map.call(solr_running: false, backend_running: false)))
    env4 = base_env.merge('MOCK_DOCKER_FAIL_START' => containers['backend'])
    _out4, _err4, status4 = Open3.capture3(env4, 'bash', script_path)
    refute status4.success?, 'Expected failure when backend docker start fails'
    calls4 = File.readlines(docker_log_path, chomp: true)
    start_calls = calls4.select { |c| c.start_with?('start ') }
    stop_calls = calls4.select { |c| c.start_with?('stop ') }
    assert_equal ["start #{containers['solr']}", "start #{containers['backend']}"], start_calls
    assert_equal ["stop #{containers['solr']}"], stop_calls
  end
end
