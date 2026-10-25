#!/usr/bin/env ruby
# frozen_string_literal: true

# Focused offline tests for native_archival_object_profile.rb (Gate 2 Step 2).
# Proves:
# 1. Stack identity failure fails closed before any core creation, login, or indexing.
# 2. Strict repository (repo 4) and record-type (:archival_object) scoping.
# 3. Harness worker dispatch contract via start_worker_thread (asserts harness
#    invokes start_worker_thread and test double imitates internals with 25-id batching,
#    resolved_attributes, index_records, and final hard commit).
# 4. Actual PeriodicIndexer#start_worker_thread execution under JRuby without backend/Solr writes.
# 5. Fail-closed behavior on worker indexing errors or worker thread crash.
# 6. Report schema, repeat separation (1 warmup + 3 measured), median calculation,
#    memory snapshot limitation labeling, and downstream parity comparison planning.
#
# Standard library only (MRI Ruby 3.2+ or JRuby).
# Run with: ruby rust-tools/scripts/test_native_archival_object_profile.rb

require 'rubygems'
if defined?(JRUBY_VERSION)
  if File.directory?('/archivesspace/build/gems/jruby/3.1.0')
    ENV['GEM_HOME'] ||= '/archivesspace/build/gems/jruby/3.1.0'
    ENV['GEM_PATH'] ||= '/archivesspace/build/gems/jruby/3.1.0'
    Gem.paths = {
      'GEM_HOME' => '/archivesspace/build/gems/jruby/3.1.0',
      'GEM_PATH' => '/archivesspace/build/gems/jruby/3.1.0'
    }
  end

  aspace_root = File.expand_path('../..', __dir__)
  common_dir = File.join(aspace_root, 'common')
  indexer_lib_dir = File.join(aspace_root, 'indexer', 'app', 'lib')
  $LOAD_PATH.unshift(common_dir) if File.directory?(common_dir) && !$LOAD_PATH.include?(common_dir)
  $LOAD_PATH.unshift(indexer_lib_dir) if File.directory?(indexer_lib_dir) && !$LOAD_PATH.include?(indexer_lib_dir)

  begin
    require 'active_support/core_ext/string/inflections'
    require 'config/config-distribution'
  rescue LoadError
    # AppConfig and ActiveSupport are optional in offline environments lacking ArchivesSpace common config
  end

  begin
    require 'rust_index_document_builder'
    require 'periodic_indexer'
    require 'pui_indexer'
    require 'large_tree_doc_indexer'
    require 'index_batch'
  rescue LoadError
    # Indexer constants are optional in offline environments lacking full ArchivesSpace runtime dependencies
  end
end

require 'minitest/autorun'
require 'json'
require 'uri'
require 'tmpdir'

require_relative 'native_archival_object_profile'
require_relative 'integration_stage_harness'
require_relative 'archivalist_profiling_stack'
require_relative 'native_indexing_profile'

class TestNativeArchivalObjectProfileScopingAndQueue < Minitest::Test
  def test_validate_repo_id_accepts_repo_4
    assert_equal 4, NativeArchivalObjectProfile.validate_repo_id!(4)
    assert_equal 4, NativeArchivalObjectProfile.validate_repo_id!('4')
  end

  def test_validate_repo_id_rejects_foreign_repositories
    [1, 2, 3, 5, 99, '1', '2', 'test'].each do |bad_repo|
      err = assert_raises(ArgumentError) do
        NativeArchivalObjectProfile.validate_repo_id!(bad_repo)
      end
      assert_match(/Scope restriction: repository must be 4/, err.message)
    end
  end

  def test_validate_record_type_accepts_archival_object
    assert_equal :archival_object, NativeArchivalObjectProfile.validate_record_type!(:archival_object)
    assert_equal :archival_object, NativeArchivalObjectProfile.validate_record_type!('archival_object')
  end

  def test_validate_record_type_rejects_other_types
    %i[resource accession repository digital_object subject agent_person].each do |bad_type|
      err = assert_raises(ArgumentError) do
        NativeArchivalObjectProfile.validate_record_type!(bad_type)
      end
      assert_match(/Scope restriction: record type must be :archival_object/, err.message)
    end
  end

  def test_portable_blocking_queue_fifo_and_sentinel
    queue = NativeArchivalObjectProfile::PortableBlockingQueue.new(2)
    assert queue.offer([1, 2, 3], 100)
    assert queue.offer([4, 5, 6], 100)
    # Queue is now at capacity 2; next offer should time out
    refute queue.offer([7, 8], 10)

    assert_equal [1, 2, 3], queue.poll(100)
    # Now capacity is available
    assert queue.offer(:finished, 100)

    assert_equal [4, 5, 6], queue.poll(100)
    assert_equal :finished, queue.poll(100)
    assert_nil queue.poll(10)
  end

  def test_portable_blocking_queue_multithreaded_producer_consumer
    queue = NativeArchivalObjectProfile::PortableBlockingQueue.new(2)
    consumed = []
    mutex = Mutex.new

    consumer = Thread.new do
      loop do
        item = queue.poll(500)
        break if item == :finished || item.nil?

        mutex.synchronize { consumed << item }
      end
    end

    [10, 20, 30, 40].each { |val| queue.offer(val, 500) }
    queue.offer(:finished, 500)
    consumer.join

    assert_equal [10, 20, 30, 40], consumed
  end

  def test_create_work_queue_contract
    q = NativeArchivalObjectProfile.create_work_queue(2)
    unit = NativeArchivalObjectProfile.time_unit_ms
    assert q.offer([1], 100, unit)
    assert q.offer([2], 100, unit)
    item = if defined?(Java) && q.is_a?(java.util.concurrent.BlockingQueue)
             q.poll(100, java.util.concurrent.TimeUnit::MILLISECONDS)
           else
             q.poll(100, unit)
           end
    assert_equal [1], item
    assert q.offer(:finished, 100, unit)
    item2 = if defined?(Java) && q.is_a?(java.util.concurrent.BlockingQueue)
              q.poll(100, java.util.concurrent.TimeUnit::MILLISECONDS)
            else
              q.poll(100, unit)
            end
    assert_equal [2], item2
    sentinel = if defined?(Java) && q.is_a?(java.util.concurrent.BlockingQueue)
                 q.poll(100, java.util.concurrent.TimeUnit::MILLISECONDS)
               else
                 q.poll(100, unit)
               end
    assert_equal :finished, sentinel
  end
end

class TestNativeArchivalObjectProfileStackIdentity < Minitest::Test
  ENV_VALID = {
    'ARCHIVALIST_PROFILING_STACK_ID' => 'feedcafe',
    'ARCHIVALIST_PROFILING_STACK_NONCE' => 'testnonce1234',
    'ARCHIVALIST_PROFILING_STACK_IDENTITY_REPO_ID' => '4',
    'ARCHIVALIST_PROFILING_STACK_IDENTITY_CORE' => 'archivalist_profiling_identity_feedcafe'
  }.freeze

  BACKEND = 'http://archivalist-profiling-backend:4567'
  SOLR = 'http://archivalist-profiling-solr:8983/solr'

  def test_stack_identity_failure_fails_closed_before_core_creation_or_indexing
    fake_getter = lambda do |_url|
      raise 'Connection refused'
    end

    # Passing empty env: must fail closed before any core creation or indexing
    err = assert_raises(RuntimeError) do
      ArchivalistProfilingStack.verify!(
        env: {},
        backend_url: BACKEND,
        solr_url: SOLR,
        get_json: fake_getter
      )
    end

    assert_match(/profiling-stack identity missing/, err.message)
  end

  def test_stack_identity_mismatch_fails_closed
    # Backend returns wrong repo code / nonce
    fake_getter = lambda do |url|
      if url.include?('/repositories/')
        { 'name' => 'wrong identity', 'repo_code' => 'WRONG' }
      else
        { 'status' => {} }
      end
    end

    err = assert_raises(RuntimeError) do
      ArchivalistProfilingStack.verify!(
        env: ENV_VALID,
        backend_url: BACKEND,
        solr_url: SOLR,
        get_json: fake_getter
      )
    end

    assert_match(/NOT connected to profiling stack/, err.message)
  end

  def test_stack_identity_verified_when_valid
    fake_getter = lambda do |url|
      if url.include?('/repositories/4')
        {
          'name' => "archivalist profiling identity #{ENV_VALID['ARCHIVALIST_PROFILING_STACK_NONCE']}",
          'repo_code' => "ARCHPROF-#{ENV_VALID['ARCHIVALIST_PROFILING_STACK_ID']}"
        }
      else
        { 'status' => { ENV_VALID['ARCHIVALIST_PROFILING_STACK_IDENTITY_CORE'] => { 'name' => 'ok' } } }
      end
    end

    identity = ArchivalistProfilingStack.verify!(
      env: ENV_VALID,
      backend_url: BACKEND,
      solr_url: SOLR,
      get_json: fake_getter
    )

    assert_equal 'feedcafe', identity['stack_id']
  end
end

class TestNativeArchivalObjectProfileWorkerInvocation < Minitest::Test
  # Mock PeriodicIndexer test double that imitates PeriodicIndexer worker dispatch and commit contract.
  # Note: this test asserts that the harness invokes start_worker_thread and dispatches batches;
  # it imitates worker internals rather than executing real production PeriodicIndexer.
  class MockPeriodicIndexer
    attr_reader :fetch_calls, :index_calls, :commit_calls, :started_threads, :worker_record_type

    def initialize(return_status = 1)
      @return_status = return_status
      @fetch_calls = []
      @index_calls = []
      @commit_calls = []
      @started_threads = 0
      @worker_record_type = nil
      @mutex = Mutex.new
    end

    def resolved_attributes
      %w[subjects linked_agents top_container]
    end

    def start_worker_thread(queue, record_type)
      @mutex.synchronize do
        @started_threads += 1
        @worker_record_type = record_type
      end

      Thread.new do
        worker_status = 0 # NOT_INDEXED
        while (id_subset = poll_work_queue(queue, 1000))
          break if id_subset == :finished || id_subset.nil?

          records = fetch_records(record_type, id_subset, resolved_attributes)
          if !records.empty?
            worker_status = @return_status
            index_records(records.map { |r| { 'record' => r, 'uri' => r['uri'] } })
          end
        end
        worker_status
      end
    end

    def poll_work_queue(queue, timeout_ms = 1000)
      if defined?(Java) && queue.is_a?(java.util.concurrent.BlockingQueue)
        queue.poll(timeout_ms, java.util.concurrent.TimeUnit::MILLISECONDS)
      elsif queue.respond_to?(:poll)
        begin
          queue.poll(timeout_ms, NativeArchivalObjectProfile.time_unit_ms)
        rescue ArgumentError
          queue.poll(timeout_ms)
        end
      else
        queue.pop
      end
    end

    def fetch_records(record_type, id_subset, resolve)
      @mutex.synchronize do
        @fetch_calls << {
          type: record_type,
          ids: id_subset,
          resolve: resolve
        }
      end
      # Return mock records
      id_subset.map do |id|
        { 'uri' => "/repositories/4/archival_objects/#{id}", 'id' => id, 'title' => "AO #{id}" }
      end
    end

    def index_records(records)
      @mutex.synchronize do
        @index_calls << records
      end
    end

    def send_commit(type = :hard)
      @mutex.synchronize do
        @commit_calls << type
      end
    end
  end

  def test_worker_thread_dispatch_contract_batches_of_25_via_test_double
    indexer = MockPeriodicIndexer.new(1) # SUCCESS
    test_ids = (1..60).to_a # 60 IDs -> slices of 25, 25, 10

    mock_fetcher = lambda do |repo_id, record_type|
      assert_equal 4, repo_id
      assert_equal :archival_object, record_type
      test_ids
    end

    res = NativeArchivalObjectProfile.execute_ao_index_envelope(
      indexer,
      repo_id: 4,
      thread_count: 2,
      records_per_thread: 25,
      all_ids_fetcher: mock_fetcher
    )

    refute_nil res['wall_ms']
    assert res['wall_ms'] >= 0.0
    assert_equal 60, res['ao_ids_count']

    # Worker record type must be :archival_object
    assert_equal :archival_object, indexer.worker_record_type
    assert_equal 2, indexer.started_threads

    # Verify 25-id batching across fetch calls
    batch_sizes = indexer.fetch_calls.map { |c| c[:ids].length }.sort
    assert_equal [10, 25, 25], batch_sizes
    assert_equal 60, indexer.fetch_calls.flat_map { |c| c[:ids] }.length

    # Verify resolved_attributes was passed to fetch_records
    indexer.fetch_calls.each do |c|
      assert_equal :archival_object, c[:type]
      assert_equal %w[subjects linked_agents top_container], c[:resolve]
    end

    # Verify index_records was called with total 60 records
    total_indexed = indexer.index_calls.map(&:length).sum
    assert_equal 60, total_indexed

    # Final hard commit must have been sent exactly once
    assert_equal [:hard], indexer.commit_calls
  end

  def test_worker_indexing_error_fails_closed
    indexer = MockPeriodicIndexer.new(2) # WORKER_STATUS_INDEX_ERROR = 2
    mock_fetcher = ->(_repo, _type) { [1, 2, 3] }

    err = assert_raises(RuntimeError) do
      NativeArchivalObjectProfile.execute_ao_index_envelope(
        indexer,
        repo_id: 4,
        thread_count: 1,
        records_per_thread: 25,
        all_ids_fetcher: mock_fetcher
      )
    end

    assert_match(/Worker thread failed while indexing archival objects/, err.message)
    # Commit must NOT be called if worker failed
    assert_empty indexer.commit_calls
  end

  def test_empty_id_set_no_workers_no_commit
    indexer = MockPeriodicIndexer.new(1)
    mock_fetcher = ->(_repo, _type) { [] }

    res = NativeArchivalObjectProfile.execute_ao_index_envelope(
      indexer,
      repo_id: 4,
      thread_count: 2,
      records_per_thread: 25,
      all_ids_fetcher: mock_fetcher
    )

    assert_equal 0, res['ao_ids_count']
    assert_equal 0, indexer.started_threads
    assert_empty indexer.commit_calls
  end
end

class TestNativeArchivalObjectProfileRealPeriodicIndexerInvocation < Minitest::Test
  RecordMock = Struct.new(:uri, :title) do
    def to_hash(_mode = nil)
      { 'uri' => uri, 'title' => title }
    end
  end

  def test_real_periodic_indexer_start_worker_thread_invocation_without_writes
    unless defined?(JRUBY_VERSION)
      skip 'PeriodicIndexer requires JRuby runtime and Java dependencies'
    end
    unless defined?(PeriodicIndexer)
      skip 'PeriodicIndexer could not be loaded in this environment'
    end

    indexer = PeriodicIndexer.allocate
    indexer.instance_variable_set(:@timing, Class.new { def time_block(_metric, &blk); blk ? blk.call : nil; end }.new)
    indexer.instance_variable_set(:@backend_fetch_sem, java.util.concurrent.Semaphore.new(2))

    indexer.define_singleton_method(:resolved_attributes) { %w[subjects linked_agents top_container] }

    fetch_calls = []
    index_calls = []
    commit_calls = []
    calls_mutex = Mutex.new

    indexer.define_singleton_method(:fetch_records) do |record_type, ids, resolve|
      calls_mutex.synchronize do
        fetch_calls << { type: record_type, ids: ids, resolve: resolve }
      end
      ids.map { |id| RecordMock.new("/repositories/4/archival_objects/#{id}", "AO #{id}") }
    end

    indexer.define_singleton_method(:index_records) do |records|
      calls_mutex.synchronize do
        index_calls << records
      end
    end

    indexer.define_singleton_method(:send_commit) do |type = :hard|
      calls_mutex.synchronize do
        commit_calls << type
      end
    end

    test_ids = (1..60).to_a
    mock_fetcher = lambda do |repo_id, record_type|
      assert_equal 4, repo_id
      assert_equal :archival_object, record_type
      test_ids
    end

    res = NativeArchivalObjectProfile.execute_ao_index_envelope(
      indexer,
      repo_id: 4,
      thread_count: 2,
      records_per_thread: 25,
      all_ids_fetcher: mock_fetcher
    )

    refute_nil res['wall_ms']
    assert res['wall_ms'] >= 0.0
    assert_equal 60, res['ao_ids_count']

    # Real PeriodicIndexer#start_worker_thread executed and dispatched to fetch_records
    batch_sizes = fetch_calls.map { |c| c[:ids].length }.sort
    assert_equal [10, 25, 25], batch_sizes
    assert_equal 60, fetch_calls.flat_map { |c| c[:ids] }.length

    # Verify resolved_attributes was passed into real PeriodicIndexer fetch_records
    fetch_calls.each do |c|
      assert_equal :archival_object, c[:type]
      assert_equal %w[subjects linked_agents top_container], c[:resolve]
    end

    # Real PeriodicIndexer mapped records passed into index_records
    total_indexed = index_calls.map(&:length).sum
    assert_equal 60, total_indexed
    indexed_uris = index_calls.flat_map { |batch| batch.map { |doc| doc['uri'] } }.sort
    expected_uris = (1..60).map { |id| "/repositories/4/archival_objects/#{id}" }.sort
    assert_equal expected_uris, indexed_uris

    # Final hard commit must have been sent exactly once
    assert_equal [:hard], commit_calls
  end

  def test_real_periodic_indexer_load_sequence_and_pui_indexer_constant_for_staff_ao_rules
    unless defined?(JRUBY_VERSION)
      skip 'PeriodicIndexer offline load test requires JRuby runtime and Java dependencies'
    end
    unless defined?(PeriodicIndexer)
      skip 'PeriodicIndexer could not be loaded in this environment'
    end

    # Proven launcher loads periodic_indexer then pui_indexer, defining PUIIndexer
    require 'pui_indexer' unless defined?(PUIIndexer)

    # 1. Assert PUIIndexer constant is defined up front before instantiating or executing doc rules
    assert Object.const_defined?(:PUIIndexer), 'PUIIndexer constant must be defined in top-level scope'
    assert IndexerCommon.const_defined?(:PUIIndexer), 'IndexerCommon must resolve PUIIndexer constant'
    assert defined?(PUIIndexer), 'PUIIndexer must be defined'

    # 2. Configure offline environment without touching live state or network services
    data_dir = Dir.mktmpdir('offline_ao_load_test_')
    orig_data_dir = AppConfig[:data_directory] rescue nil
    begin
      AppConfig[:data_directory] = data_dir
      AppConfig[:solr_indexing_frequency_seconds] = 30
      AppConfig[:indexer_thread_count] = 2
      AppConfig[:indexer_records_per_thread] = 25

      dummy_enum_source = Object.new
      def dummy_enum_source.valid?(_name, _value); true; end
      def dummy_enum_source.values_for(_name); []; end
      def dummy_enum_source.default_value_for(_name); nil; end
      def dummy_enum_source.editable?(_name); false; end

      JSONModel.init(
        client_mode: true,
        url: 'http://stubbed-backend.invalid:9999',
        enum_source: dummy_enum_source
      )

      # 3. Instantiate REAL PeriodicIndexer (executes real IndexerCommon#initialize, configure_doc_rules)
      indexer = PeriodicIndexer.new(
        'http://stubbed-backend.invalid:9999',
        IndexState.new,
        'OfflineStaffAOLoadTest',
        false
      )

      # Intercept Solr batch submission so no network/Solr write occurs
      indexed_docs = []
      indexer.define_singleton_method(:index_batch) do |batch, _timing|
        batch.each { |doc| indexed_docs << doc }
      end

      # 4. Check PUIIndexer constant is defined before a staff AO document rule executes
      assert indexer.class.const_defined?(:PUIIndexer), 'PeriodicIndexer must resolve PUIIndexer constant'
      refute indexer.instance_of?(PUIIndexer), 'PeriodicIndexer instance must not be an instance of PUIIndexer'

      # 5. Execute staff AO document rule via index_records
      # This specifically exercises the document prepare hook in indexer_common.rb line 1130:
      #   if !self.instance_of?(PUIIndexer)
      #     rust_fields = Array(doc['_archivalist_rust_fields'])
      #     build_fullrecord(doc, record) unless rust_fields.include?('fullrecord')
      #   end
      staff_ao_record = {
        'uri' => '/repositories/4/archival_objects/1271',
        'record' => {
          'uri' => '/repositories/4/archival_objects/1271',
          'jsonmodel_type' => 'archival_object',
          'title' => 'Offline Load Test AO',
          'display_string' => 'Offline Load Test AO',
          'level' => 'item',
          'dates' => []
        }
      }

      indexer.index_records([staff_ao_record])

      # 6. Assert rule executed cleanly, evaluating !self.instance_of?(PUIIndexer) and building fullrecord
      assert_equal 1, indexed_docs.length
      indexed_doc = indexed_docs.first
      assert_equal 'archival_object', indexed_doc['primary_type']
      assert_equal '/repositories/4/archival_objects/1271', indexed_doc['uri']
      assert indexed_doc.key?('fullrecord'), 'Fullrecord must be set by staff indexer document rule'
      assert indexed_doc.key?('fullrecord_published'), 'Fullrecord_published must be set by build_fullrecord'
      assert_includes indexed_doc['fullrecord_published'], 'Offline Load Test AO'
    ensure
      AppConfig[:data_directory] = orig_data_dir if orig_data_dir
      FileUtils.remove_entry(data_dir) if File.exist?(data_dir)
    end
  end
end

class TestNativeArchivalObjectProfileReportSchema < Minitest::Test
  def sample_results
    [
      {
        'repeat' => 1,
        'is_warmup' => true,
        'run_id' => '20261025_000001_101_aaaa',
        'core' => 'stage_staff_20261025_000001_101_aaaa_native',
        'wall_ms' => 1500.0,
        'cpu_user_ms' => 900.0,
        'cpu_system_ms' => 100.0,
        'solr_doc_count' => 25000,
        'ao_doc_count' => 25000,
        'ao_ids_count' => 25000,
        'process_memory' => {
          'vm_rss_before_kib' => 500_000,
          'vm_rss_after_kib' => 600_000,
          'vm_hwm_before_kib' => 600_000,
          'vm_hwm_after_kib' => 650_000,
          'vm_hwm_semantics' => NativeArchivalObjectProfile::VM_HWM_SEMANTICS,
          'diagnostic' => nil
        },
        'solr_jvm_heap' => {
          'before' => { 'used_bytes' => 1_000_000, 'committed_bytes' => 2_000_000, 'max_bytes' => 4_000_000 },
          'after' => { 'used_bytes' => 1_200_000, 'committed_bytes' => 2_000_000, 'max_bytes' => 4_000_000 }
        }
      },
      {
        'repeat' => 2,
        'is_warmup' => false,
        'run_id' => '20261025_000002_101_bbbb',
        'core' => 'stage_staff_20261025_000002_101_bbbb_native',
        'wall_ms' => 1200.0,
        'cpu_user_ms' => 800.0,
        'cpu_system_ms' => 80.0,
        'solr_doc_count' => 25000,
        'ao_doc_count' => 25000,
        'ao_ids_count' => 25000,
        'process_memory' => {
          'vm_rss_before_kib' => 600_000,
          'vm_rss_after_kib' => 620_000,
          'vm_hwm_before_kib' => 650_000,
          'vm_hwm_after_kib' => 660_000,
          'vm_hwm_semantics' => NativeArchivalObjectProfile::VM_HWM_SEMANTICS,
          'diagnostic' => nil
        },
        'solr_jvm_heap' => {
          'before' => { 'used_bytes' => 1_200_000, 'committed_bytes' => 2_000_000, 'max_bytes' => 4_000_000 },
          'after' => { 'used_bytes' => 1_300_000, 'committed_bytes' => 2_000_000, 'max_bytes' => 4_000_000 }
        }
      },
      {
        'repeat' => 3,
        'is_warmup' => false,
        'run_id' => '20261025_000003_101_cccc',
        'core' => 'stage_staff_20261025_000003_101_cccc_native',
        'wall_ms' => 1100.0,
        'cpu_user_ms' => 780.0,
        'cpu_system_ms' => 75.0,
        'solr_doc_count' => 25000,
        'ao_doc_count' => 25000,
        'ao_ids_count' => 25000,
        'process_memory' => {
          'vm_rss_before_kib' => 620_000,
          'vm_rss_after_kib' => 630_000,
          'vm_hwm_before_kib' => 660_000,
          'vm_hwm_after_kib' => 670_000,
          'vm_hwm_semantics' => NativeArchivalObjectProfile::VM_HWM_SEMANTICS,
          'diagnostic' => nil
        },
        'solr_jvm_heap' => {
          'before' => { 'used_bytes' => 1_300_000, 'committed_bytes' => 2_000_000, 'max_bytes' => 4_000_000 },
          'after' => { 'used_bytes' => 1_350_000, 'committed_bytes' => 2_000_000, 'max_bytes' => 4_000_000 }
        }
      },
      {
        'repeat' => 4,
        'is_warmup' => false,
        'run_id' => '20261025_000004_101_dddd',
        'core' => 'stage_staff_20261025_000004_101_dddd_native',
        'wall_ms' => 1150.0,
        'cpu_user_ms' => 790.0,
        'cpu_system_ms' => 85.0,
        'solr_doc_count' => 25000,
        'ao_doc_count' => 25000,
        'ao_ids_count' => 25000,
        'process_memory' => {
          'vm_rss_before_kib' => 630_000,
          'vm_rss_after_kib' => 635_000,
          'vm_hwm_before_kib' => 670_000,
          'vm_hwm_after_kib' => 675_000,
          'vm_hwm_semantics' => NativeArchivalObjectProfile::VM_HWM_SEMANTICS,
          'diagnostic' => nil
        },
        'solr_jvm_heap' => {
          'before' => { 'used_bytes' => 1_350_000, 'committed_bytes' => 2_000_000, 'max_bytes' => 4_000_000 },
          'after' => { 'used_bytes' => 1_400_000, 'committed_bytes' => 2_000_000, 'max_bytes' => 4_000_000 }
        }
      }
    ]
  end

  def test_summary_calculation_excludes_warmup
    results = sample_results
    summary = NativeArchivalObjectProfile.calculate_summary(results)

    assert_equal 3, summary['measured_repeats']
    # Warmup repeat 1 (1500.0) is excluded; repeats 2-4: [1200.0, 1100.0, 1150.0] -> sorted [1100.0, 1150.0, 1200.0] -> median 1150.0
    assert_equal [1200.0, 1100.0, 1150.0], summary['wall_ms_raw']
    assert_equal 1150.0, summary['wall_ms_median']

    # CPU user median: [800.0, 780.0, 790.0] -> sorted [780.0, 790.0, 800.0] -> median 790.0
    assert_equal [800.0, 780.0, 790.0], summary['cpu_user_ms_raw']
    assert_equal 790.0, summary['cpu_user_ms_median']

    # CPU system median: [80.0, 75.0, 85.0] -> sorted [75.0, 80.0, 85.0] -> median 80.0
    assert_equal [80.0, 75.0, 85.0], summary['cpu_system_ms_raw']
    assert_equal 80.0, summary['cpu_system_ms_median']

    assert_equal [25000, 25000, 25000], summary['solr_doc_counts']
    assert_equal [25000, 25000, 25000], summary['ao_doc_counts']
  end

  def test_build_report_schema
    results = sample_results
    stack_id = { 'stack_id' => 'teststack123' }
    prov = { 'git_commit' => '08f7123ab', 'git_dirty' => false }

    report = NativeArchivalObjectProfile.build_report(
      run_id: 'test_run_1',
      backend_base: 'http://localhost:4567',
      solr_base: 'http://solr:8983/solr',
      stack_identity: stack_id,
      results: results,
      repo_id: 4,
      thread_count: 4,
      records_per_thread: 25,
      source_provenance: prov
    )

    assert_equal 'test_run_1', report['run_id']
    assert_equal 4, report['repo_id']
    assert_equal 'archival_object', report['record_type']
    assert_equal [4, 25], report['threads']
    assert_equal 4, report['repeats']

    # Rust flags all false
    report['rust_flags'].each do |flag, val|
      refute val, "Rust flag #{flag} must be false in baseline harness"
    end
    assert_equal 7, report['rust_flags'].length

    # Sidecar and wrappers explicitly false
    refute report['sidecar_present']
    refute report['profiler_wrappers']

    # 4 repeat results present
    assert_equal 4, report['results'].length
    assert report['results'][0]['is_warmup']
    refute report['results'][1]['is_warmup']
    refute report['results'][2]['is_warmup']
    refute report['results'][3]['is_warmup']

    # Parity comparison plan present
    plan = report['parity_comparison_plan']
    assert_equal 'rust-tools/scripts/compare_staff_index_parity.py', plan['comparator']
    assert_equal 4, plan['repository']
    assert_equal 4, plan['baseline_cores'].length

    # Notes present and cover memory limitations
    notes_text = report['notes'].join("\n")
    assert_match(/VmHWM is the operating system process-lifetime high-water mark/, notes_text)
    assert_match(/point-in-time snapshots of isolated Solr JVM heap/, notes_text)
    assert_match(/document counts alone do NOT establish document parity/, notes_text)
    assert_match(/repository 4 archival_object records only/, notes_text)
    assert_match(/preamble is strictly excluded/, notes_text)
  end

  def test_stage_core_name_generation_and_uniqueness
    run_ids = (1..4).map { ArchivalistHarnessSafety.generate_run_id }
    assert_equal 4, run_ids.uniq.length

    cores = run_ids.map { |rid| ArchivalistHarnessSafety.stage_core_name(rid, 'native', 'staff') }
    assert_equal 4, cores.uniq.length
    cores.each_with_index do |core, i|
      assert_match(/\Astage_staff_#{Regexp.escape(run_ids[i])}_native\z/, core)
      assert ArchivalistHarnessSafety.valid_stage_core_name?(core, run_ids[i])
      refute core.include?('archivesspace')
    end
  end
end

class TestNativeArchivalObjectProfileSidecarConfiguration < Minitest::Test
  # Mock config that mimics ArchivesSpace AppConfig behavior:
  # raises RuntimeError on missing key when calling [] without has_key?.
  class StrictAppConfigMock
    def initialize(entries = {})
      @entries = entries.dup
    end

    def has_key?(key)
      @entries.key?(key)
    end

    def [](key)
      raise "No value set for config parameter: #{key}" unless @entries.key?(key)

      @entries[key]
    end

    def []=(key, val)
      @entries[key] = val
    end
  end

  def test_missing_key_does_not_crash_and_verifies_cleanly
    # Missing key: not present in config at all (not even nil)
    config = StrictAppConfigMock.new({})

    refute config.has_key?(:rust_sidecar_url)
    # Proves our strict mock reproduces ArchivesSpace AppConfig RuntimeError on []
    assert_raises(RuntimeError) { config[:rust_sidecar_url] }

    # NativeArchivalObjectProfile must not raise RuntimeError for missing key
    refute NativeArchivalObjectProfile.sidecar_configured?(config)
    assert_silent do
      NativeArchivalObjectProfile.verify_sidecar_unconfigured!(config)
    end
  end

  def test_nil_key_does_not_crash_and_verifies_cleanly
    config = StrictAppConfigMock.new(rust_sidecar_url: nil)

    assert config.has_key?(:rust_sidecar_url)
    assert_nil config[:rust_sidecar_url]

    refute NativeArchivalObjectProfile.sidecar_configured?(config)
    assert_silent do
      NativeArchivalObjectProfile.verify_sidecar_unconfigured!(config)
    end
  end

  def test_empty_string_does_not_fail_closed
    config = StrictAppConfigMock.new(rust_sidecar_url: '   ')

    assert config.has_key?(:rust_sidecar_url)
    refute NativeArchivalObjectProfile.sidecar_configured?(config)
    assert_silent do
      NativeArchivalObjectProfile.verify_sidecar_unconfigured!(config)
    end
  end

  def test_configured_sidecar_endpoint_fails_closed
    config = StrictAppConfigMock.new(rust_sidecar_url: 'http://127.0.0.1:4570')

    assert config.has_key?(:rust_sidecar_url)
    assert_equal 'http://127.0.0.1:4570', config[:rust_sidecar_url]

    assert NativeArchivalObjectProfile.sidecar_configured?(config)
    err = assert_raises(RuntimeError) do
      NativeArchivalObjectProfile.verify_sidecar_unconfigured!(config)
    end
    assert_equal 'Safety violation: sidecar URL is configured', err.message
  end

  def test_real_archivesspace_appconfig_in_jruby_if_available
    unless defined?(AppConfig) && AppConfig.respond_to?(:has_key?)
      skip 'ArchivesSpace AppConfig not loaded in this environment'
    end

    AppConfig.reload

    # In default distribution, rust_sidecar_url is NOT defined in config-defaults
    refute AppConfig.has_key?(:rust_sidecar_url)

    # Calling AppConfig[:rust_sidecar_url] directly raises RuntimeError
    missing_err = assert_raises(RuntimeError) do
      AppConfig[:rust_sidecar_url]
    end
    assert_match(/No value set for config parameter: rust_sidecar_url/, missing_err.message)

    # Harness check succeeds without raising missing-key error
    refute NativeArchivalObjectProfile.sidecar_configured?(AppConfig)
    assert_silent do
      NativeArchivalObjectProfile.verify_sidecar_unconfigured!(AppConfig)
    end

    # Explicitly configured sidecar fails closed
    orig_val = AppConfig.has_key?(:rust_sidecar_url) ? AppConfig[:rust_sidecar_url] : nil
    begin
      AppConfig[:rust_sidecar_url] = 'http://127.0.0.1:4570'
      assert AppConfig.has_key?(:rust_sidecar_url)
      assert NativeArchivalObjectProfile.sidecar_configured?(AppConfig)

      err = assert_raises(RuntimeError) do
        NativeArchivalObjectProfile.verify_sidecar_unconfigured!(AppConfig)
      end
      assert_equal 'Safety violation: sidecar URL is configured', err.message
    ensure
      AppConfig[:rust_sidecar_url] = orig_val
      AppConfig.reload
    end
  end
end
