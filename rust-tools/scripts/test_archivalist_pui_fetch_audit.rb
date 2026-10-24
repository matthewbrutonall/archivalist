#!/usr/bin/env ruby
# frozen_string_literal: true

# Deterministic offline tests for archivalist_pui_fetch_audit.rb (synthetic classes and responses)
# and run_native_indexing_profile.sh environment forwarding (stubbed external commands).
# Run with MRI or JRuby: ruby rust-tools/scripts/test_archivalist_pui_fetch_audit.rb

require 'rubygems'
if defined?(JRUBY_VERSION) && File.directory?('/archivesspace/build/gems/jruby/3.1.0')
  ENV['GEM_HOME'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  ENV['GEM_PATH'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  Gem.paths = {
    'GEM_HOME' => '/archivesspace/build/gems/jruby/3.1.0',
    'GEM_PATH' => '/archivesspace/build/gems/jruby/3.1.0'
  }
end

require 'minitest/autorun'
require 'json'
require 'uri'
require 'tmpdir'
require 'open3'
require_relative 'archivalist_pui_fetch_audit'
require_relative 'archivalist_ruby_side_timing'

AuditFakeResponse = Struct.new(:code, :body)

class UnwrappedFakePuiIndexer
  def fetch_records(type, ids, _resolve)
    [type, ids]
  end

  def index_round_complete(repo)
    repo
  end
end

module UnwrappedFakeJsonModelHttp
  def self.get_response(url)
    AuditFakeResponse.new('200', "body:#{url}")
  end
end

module AuditFakeJsonModelHttp
  @responder = nil
  @lock = Mutex.new

  class << self
    def with_responder(callable)
      @lock.synchronize { @responder = callable }
      yield
    ensure
      @lock.synchronize { @responder = nil }
    end

    def get_response(url)
      do_http_request(url, nil)
    end

    def do_http_request(url, _req)
      callable = @lock.synchronize { @responder }
      raise "No responder configured for #{url}" unless callable

      callable.call(url)
    end
  end
end

class AuditFakePeriodicIndexer
  def fetch_records(repo_id, type, ids, resolve = ['subjects'])
    path = "/repositories/#{repo_id}/#{type}s"
    params = [['id_set', ids.join(',')]] + Array(resolve).map { |r| ['resolve[]', r] }
    uri = URI("http://localhost:4567#{path}?#{URI.encode_www_form(params)}")
    res = AuditFakeJsonModelHttp.get_response(uri)
    raise res.body.to_s unless res.code == '200'

    res
  end
end

class AuditFakePuiIndexer < AuditFakePeriodicIndexer
  def fetch_records(repo_id, type, ids, resolve = ['subjects', 'ancestors'])
    super(repo_id, type, ids, resolve)
  end

  def fetch_all_ids(repo_id, type)
    uri = URI("http://localhost:4567/repositories/#{repo_id}/#{type}s?all_ids=true&modified_since=0")
    AuditFakeJsonModelHttp.get_response(uri)
  end

  def index_round_complete(repo_id, batches, resolve: [])
    fetch_all_ids(repo_id, :archival_object)
    batches.each do |ids|
      params = [['id_set', ids.join(',')]] + Array(resolve).map { |r| ['resolve[]', r] }
      uri = URI("http://localhost:4567/repositories/#{repo_id}/archival_objects?#{URI.encode_www_form(params)}")
      res = AuditFakeJsonModelHttp.get_response(uri)
      raise res.body.to_s unless res.code == '200'
    end
  end
end

AUDIT_HOOKED = ArchivalistPuiFetchAudit.install!(
  pui_indexer: AuditFakePuiIndexer,
  jsonmodel_http: AuditFakeJsonModelHttp
)

class TestArchivalistPuiFetchAudit < Minitest::Test
  def test_default_off_and_explicit_opt_in_flag
    refute ArchivalistPuiFetchAudit.enabled?({})
    refute ArchivalistPuiFetchAudit.enabled?('PUI_FETCH_AUDIT' => '')
    refute ArchivalistPuiFetchAudit.enabled?('PUI_FETCH_AUDIT' => '0')
    refute ArchivalistPuiFetchAudit.enabled?('PUI_FETCH_AUDIT' => 'false')
    refute ArchivalistPuiFetchAudit.enabled?('PUI_FETCH_AUDIT' => 'true')
    assert ArchivalistPuiFetchAudit.enabled?('PUI_FETCH_AUDIT' => '1')
    assert ArchivalistPuiFetchAudit.enabled?('PUI_FETCH_AUDIT' => ' 1 ')

    # Uninstalled classes have no audit hook in ancestors.
    refute UnwrappedFakePuiIndexer.ancestors.include?(ArchivalistPuiFetchAudit::PuiIndexerHook)
    refute UnwrappedFakeJsonModelHttp.singleton_class.ancestors.include?(ArchivalistPuiFetchAudit::JsonModelHttpHook)
    assert_equal({ 'PUIIndexer' => %w[fetch_records index_round_complete], 'JSONModel::HTTP' => %w[get_response] }, AUDIT_HOOKED)
  end

  def test_inactive_by_default_records_nothing_and_ignores_staff_or_inventory
    pui = AuditFakePuiIndexer.new
    staff = AuditFakePeriodicIndexer.new

    AuditFakeJsonModelHttp.with_responder(->(_u) { AuditFakeResponse.new('200', '[{"id":1}]') }) do
      # Inactive (before start!): PUI pass does not record
      pui.fetch_records(2, :archival_object, [1, 2])
      assert_empty ArchivalistPuiFetchAudit::STATE[:repositories]

      # Active during a round, but STAFF indexer (PeriodicIndexer#fetch_records, not PUIIndexer)
      # and direct HTTP inventory calls outside PUIIndexer must not be recorded.
      _, summary = ArchivalistPuiFetchAudit.capture do
        staff.fetch_records(2, :archival_object, [10, 11])
        AuditFakeJsonModelHttp.get_response(URI('http://localhost:4567/repositories/2/archival_objects?id_set=99'))
      end
      assert_equal 0, summary.dig('phases', 'ordinary_indexing', 'calls')
      assert_equal 0, summary.dig('phases', 'tree_discovery', 'calls')
      assert_empty summary['by_repository']
    end
  end

  def test_phase_attribution_by_execution_context_and_overlap_accounting
    pui = AuditFakePuiIndexer.new
    # Even if ordinary indexing passes empty resolve[] and tree discovery passes non-empty resolve[],
    # attribution follows the execution context (fetch_records vs index_round_complete).
    AuditFakeJsonModelHttp.with_responder(->(u) { AuditFakeResponse.new('200', "payload:#{u.query}") }) do
      _, summary = ArchivalistPuiFetchAudit.capture do
        # Ordinary indexing fetches [1, 2, 3] and [4, 5], plus a resource batch (ignored) and all_ids (ignored)
        pui.fetch_all_ids(2, :archival_object)
        pui.fetch_records(2, :resource, [100, 101], [])
        pui.fetch_records(2, :archival_object, [1, 2, 3], [])
        pui.fetch_records(2, :archival_object, [4, 5], [])

        # Tree discovery fetches [2, 3, 4] and [6] with non-empty resolve[]
        pui.index_round_complete(2, [[2, 3, 4], [6]], resolve: ['subjects'])
      end

      ord = summary.dig('phases', 'ordinary_indexing')
      tree = summary.dig('phases', 'tree_discovery')
      ov = summary['overlap']

      assert_equal 2, ord['calls']
      assert_equal 2, ord['successful_calls']
      assert_equal 0, ord['failed_calls']
      assert_equal 5, ord['unique_successful_requested_ids']
      assert_equal 5, ord['requested_ids_total']
      assert_equal 5, ord['successful_requested_ids_total']
      assert_operator ord['response_bytes'], :>, 0
      assert_equal 'summed call time across threads (not wall time)', ord['duration_kind']

      assert_equal 2, tree['calls']
      assert_equal 2, tree['successful_calls']
      assert_equal 0, tree['failed_calls']
      assert_equal 4, tree['unique_successful_requested_ids']
      assert_equal 4, tree['requested_ids_total']
      assert_equal 4, tree['successful_requested_ids_total']

      # Ordinary={1,2,3,4,5}, Tree={2,3,4,6} -> intersection={2,3,4} (3), ord_only={1,5} (2), tree_only={6} (1), union=6
      assert_equal 5, ov['ordinary_unique_successful_requested_ids']
      assert_equal 4, ov['tree_discovery_unique_successful_requested_ids']
      assert_equal 3, ov['intersection_successful_requested_ids']
      assert_equal 2, ov['ordinary_only_successful_requested_ids']
      assert_equal 1, ov['tree_discovery_only_successful_requested_ids']
      assert_equal 6, ov['union_unique_successful_requested_ids']
      assert_in_delta 0.75, ov['tree_discovery_overlap_ratio'], 0.0001
      assert_in_delta 0.6, ov['ordinary_overlap_ratio'], 0.0001
    end
  end

  def test_http_200_empty_array_response_counts_successful_requested_ids_without_claiming_returned_records
    pui = AuditFakePuiIndexer.new
    # Backend returns HTTP 200 with empty JSON array `[]` (e.g., all requested IDs deleted/unpublished).
    AuditFakeJsonModelHttp.with_responder(->(_u) { AuditFakeResponse.new('200', '[]') }) do
      _, summary = ArchivalistPuiFetchAudit.capture do
        pui.fetch_records(2, :archival_object, [101, 102, 103])
        pui.index_round_complete(2, [[102, 104]])
      end

      assert_includes summary['id_accounting_basis'], 'requested_by_http_200_calls'
      assert_includes summary['id_accounting_basis'], 'response bodies are not parsed'
      assert_includes summary['note'], 'not parsed response-body records'

      ord = summary.dig('phases', 'ordinary_indexing')
      tree = summary.dig('phases', 'tree_discovery')
      ov = summary['overlap']

      # Explicitly labeled as successful requested IDs, never `unique_fetched_ids`, `unique_ids`, or returned records
      refute ord.key?('unique_fetched_ids')
      refute ord.key?('unique_ids')
      refute ord.key?('repeated_fetched_ids')
      refute ov.key?('ordinary_unique_ids')
      refute ov.key?('intersection_ids')

      assert_equal 1, ord['successful_calls']
      assert_equal 3, ord['successful_requested_ids_total']
      assert_equal 3, ord['unique_successful_requested_ids']
      assert_equal 0, ord['repeated_successful_requested_ids']
      assert_equal 2, ord['successful_response_bytes'] # '[]'.bytesize == 2

      assert_equal 1, tree['successful_calls']
      assert_equal 2, tree['successful_requested_ids_total']
      assert_equal 2, tree['unique_successful_requested_ids']
      assert_equal 2, tree['successful_response_bytes']

      assert_equal 1, ov['intersection_successful_requested_ids'] # 102
      assert_equal 2, ov['ordinary_only_successful_requested_ids'] # 101, 103
      assert_equal 1, ov['tree_discovery_only_successful_requested_ids'] # 104
      assert_equal 4, ov['union_unique_successful_requested_ids']
    end
  end

  def test_retries_errors_and_bare_raise_exception_identity_and_backtrace_preservation
    pui = AuditFakePuiIndexer.new
    origin_exception = nil

    responder = lambda do |uri|
      query = uri.query.to_s
      if query.include?('id_set=10%2C11') || query.include?('id_set=10,11')
        # Batch call fails with HTTP 500
        AuditFakeResponse.new('500', 'Internal Server Error: secret_body_must_not_leak')
      elsif query.include?('id_set=10')
        # Individual fallback for 10 succeeds
        AuditFakeResponse.new('200', '[{"id":10}]')
      elsif query.include?('id_set=11')
        # Individual fallback for 11 raises a specific exception instance; bare `raise` must preserve
        # the exact object identity and original top-of-backtrace line rather than rewriting it.
        begin
          raise Errno::ECONNRESET, 'Connection reset by peer'
        rescue Errno::ECONNRESET => e
          origin_exception = e
          origin_top_frame = e.backtrace.first
          e.instance_variable_set(:@origin_top_frame, origin_top_frame)
          raise
        end
      else
        AuditFakeResponse.new('200', '[]')
      end
    end

    AuditFakeJsonModelHttp.with_responder(responder) do
      _, summary = ArchivalistPuiFetchAudit.capture do
        # Simulate PeriodicIndexer#start_worker_thread happy-path failure + per-ID salvage retry:
        err500 = assert_raises(RuntimeError) { pui.fetch_records(3, :archival_object, [10, 11]) }
        assert_includes err500.message, 'Internal Server Error'

        pui.fetch_records(3, :archival_object, [10])
        caught = assert_raises(Errno::ECONNRESET) { pui.fetch_records(3, :archival_object, [11]) }
        assert_same origin_exception, caught, 'bare raise must preserve exact exception instance'
        assert_equal origin_exception.instance_variable_get(:@origin_top_frame), caught.backtrace.first,
                     'bare raise must preserve original top backtrace frame'
      end

      ord = summary.dig('phases', 'ordinary_indexing')
      assert_equal 3, ord['calls']
      assert_equal 1, ord['successful_calls']
      assert_equal 2, ord['failed_calls']
      # Total requested across all 3 attempts: [10, 11] (2) + [10] (1) + [11] (1) = 4
      assert_equal 4, ord['requested_ids_total']
      assert_equal 1, ord['successful_requested_ids_total']
      assert_equal 3, ord['failed_requested_ids_total']
      # Failed calls MUST NOT include ID 11 in successful requested IDs; only ID 10 succeeded.
      assert_equal 1, ord['unique_successful_requested_ids']
      assert_equal 0, ord['repeated_successful_requested_ids']
      assert_equal 2, ord['unique_failed_requested_ids'] # 10 and 11 were both in failed calls
      assert_equal '[{"id":10}]'.bytesize, ord['successful_response_bytes']
      assert_equal 'Internal Server Error: secret_body_must_not_leak'.bytesize, ord['failed_response_bytes']

      # Privacy check: no response body or secret string is retained in the summary
      serialized = JSON.generate(summary)
      refute_includes serialized, 'secret_body_must_not_leak'
    end
  end

  def test_repeated_ids_and_per_repository_scoping
    pui = AuditFakePuiIndexer.new
    AuditFakeJsonModelHttp.with_responder(->(_u) { AuditFakeResponse.new('200', '[{"ok":true}]') }) do
      _, summary = ArchivalistPuiFetchAudit.capture do
        # Repo 2: repeated ID 1 within same call and across calls
        pui.fetch_records(2, :archival_object, [1, 1, 2])
        pui.fetch_records(2, :archival_object, [2, 3])
        pui.index_round_complete(2, [[2, 3, 3]])

        # Repo 5: uses same numeric ID 1, scoped separately to repo 5
        pui.fetch_records(5, :archival_object, [1, 4])
        pui.index_round_complete(5, [[1, 5]])
      end

      repo2 = summary.dig('by_repository', '2')
      repo5 = summary.dig('by_repository', '5')
      assert_equal 3, repo2.dig('phases', 'ordinary_indexing', 'unique_successful_requested_ids')
      assert_equal 5, repo2.dig('phases', 'ordinary_indexing', 'requested_ids_total')
      assert_equal 2, repo2.dig('phases', 'ordinary_indexing', 'repeated_successful_requested_ids')
      assert_equal 2, repo2.dig('phases', 'tree_discovery', 'unique_successful_requested_ids')
      assert_equal 3, repo2.dig('phases', 'tree_discovery', 'requested_ids_total')
      assert_equal 1, repo2.dig('phases', 'tree_discovery', 'repeated_successful_requested_ids')
      assert_equal 2, repo2.dig('overlap', 'intersection_successful_requested_ids')

      assert_equal 2, repo5.dig('phases', 'ordinary_indexing', 'unique_successful_requested_ids')
      assert_equal 2, repo5.dig('phases', 'tree_discovery', 'unique_successful_requested_ids')
      assert_equal 1, repo5.dig('overlap', 'intersection_successful_requested_ids')

      # Round aggregate sums per-repository scoped unique counts (3 + 2 = 5 ordinary, 2 + 2 = 4 tree, 2 + 1 = 3 intersection)
      assert_equal 5, summary.dig('phases', 'ordinary_indexing', 'unique_successful_requested_ids')
      assert_equal 4, summary.dig('phases', 'tree_discovery', 'unique_successful_requested_ids')
      assert_equal 3, summary.dig('overlap', 'intersection_successful_requested_ids')
    end
  end

  def test_concurrent_worker_threads_and_no_double_counting_with_timing_hooks
    # Also install ArchivalistRubySideTiming on AuditFakeJsonModelHttp and AuditFakePuiIndexer
    # to verify there is zero double counting or hook interference.
    ArchivalistRubySideTiming.install_native_pipeline!(
      indexer_common: AuditFakePeriodicIndexer,
      periodic_indexer: AuditFakePeriodicIndexer,
      pui_indexer: AuditFakePuiIndexer,
      large_tree: Class.new { def add_largetree_docs(*); end },
      index_batch: Class.new { def <<(*); end },
      jsonmodel_http: AuditFakeJsonModelHttp
    )

    pui = AuditFakePuiIndexer.new
    AuditFakeJsonModelHttp.with_responder(lambda { |_u|
      sleep 0.002
      AuditFakeResponse.new('200', '[{"id":1}]')
    }) do
      ArchivalistRubySideTiming.start!
      audit_summary = nil
      begin
        _, audit_summary = ArchivalistPuiFetchAudit.capture do
          threads = 4.times.map do |t|
            Thread.new do
              base = t * 10
              pui.fetch_records(2, :archival_object, [base + 1, base + 2, base + 3])
            end
          end
          threads.each(&:join)
          pui.index_round_complete(2, [[1, 11, 21, 31, 99]])
        end
      ensure
        timing_raw = ArchivalistRubySideTiming.stop!
      end

      ord = audit_summary.dig('phases', 'ordinary_indexing')
      tree = audit_summary.dig('phases', 'tree_discovery')
      ov = audit_summary['overlap']

      assert_equal 4, ord['calls']
      assert_equal 12, ord['unique_successful_requested_ids']
      assert_equal 12, ord['requested_ids_total']
      assert_operator ord['call_duration_sum_ms'], :>=, 6.0

      assert_equal 1, tree['calls']
      assert_equal 5, tree['unique_successful_requested_ids']
      assert_equal 4, ov['intersection_successful_requested_ids'] # 1, 11, 21, 31
      assert_equal 8, ov['ordinary_only_successful_requested_ids']
      assert_equal 1, ov['tree_discovery_only_successful_requested_ids'] # 99

      # Timing hook counted 4 ordinary + 1 all_ids + 1 tree batch = 6 backend_http requests
      assert_equal 6, timing_raw.dig('counts', 'backend_http')
    end
  end

  def test_empty_and_malformed_queries_are_ignored
    assert_nil ArchivalistPuiFetchAudit.extract_archival_object_batch_target(nil)
    assert_nil ArchivalistPuiFetchAudit.extract_archival_object_batch_target(':::not_a_uri:::')
    assert_nil ArchivalistPuiFetchAudit.extract_archival_object_batch_target(URI('http://localhost:4567/repositories/2/archival_objects'))
    assert_nil ArchivalistPuiFetchAudit.extract_archival_object_batch_target(URI('http://localhost:4567/repositories/2/archival_objects?id_set='))
    assert_nil ArchivalistPuiFetchAudit.extract_archival_object_batch_target(URI('http://localhost:4567/repositories/2/archival_objects?id_set=1,2&all_ids=true'))
    assert_nil ArchivalistPuiFetchAudit.extract_archival_object_batch_target(URI('http://localhost:4567/repositories/2/archival_objects?id_set=1,abc'))
    assert_nil ArchivalistPuiFetchAudit.extract_archival_object_batch_target(URI('http://localhost:4567/repositories/2/resources?id_set=1,2'))
    assert_equal({ repo_id: 2, ids: [1, 2, 3] },
                 ArchivalistPuiFetchAudit.extract_archival_object_batch_target(URI('http://localhost:4567/repositories/2/archival_objects?id_set=1,2,3&resolve%5B%5D=subjects')))
  end

  def test_median_summary_and_observational_note
    pui = AuditFakePuiIndexer.new
    summaries = []
    AuditFakeJsonModelHttp.with_responder(->(_u) { AuditFakeResponse.new('200', '[{"id":1}]') }) do
      2.times do |i|
        _, s = ArchivalistPuiFetchAudit.capture do
          pui.fetch_records(2, :archival_object, [1, 2, i + 3])
          pui.index_round_complete(2, [[1, 2]])
        end
        summaries << s
      end
    end

    med = ArchivalistPuiFetchAudit.median_summary(summaries)
    assert_includes med['note'], 'Observational'
    assert_includes med['note'], 'Cannot imply safe cache reuse'
    assert_includes med['id_accounting_basis'], 'requested_by_http_200_calls'
    assert_equal 1, med.dig('phases', 'ordinary_indexing', 'calls')
    assert_equal 3, med.dig('phases', 'ordinary_indexing', 'unique_successful_requested_ids')
    assert_equal 2, med.dig('overlap', 'intersection_successful_requested_ids')
    assert_nil ArchivalistPuiFetchAudit.median_summary([])
  end

  def test_wrapper_script_forwards_pui_fetch_audit_only_when_set_with_stubbed_commands
    wrapper_src = File.expand_path('run_native_indexing_profile.sh', __dir__)
    assert File.exist?(wrapper_src)

    Dir.mktmpdir('archivalist_wrapper_test_') do |tmp|
      scripts_dir = File.join(tmp, 'rust-tools', 'scripts')
      bin_dir = File.join(tmp, 'bin')
      FileUtils.mkdir_p(scripts_dir)
      FileUtils.mkdir_p(bin_dir)
      FileUtils.cp(wrapper_src, File.join(scripts_dir, 'run_native_indexing_profile.sh'))
      FileUtils.chmod(0o755, File.join(scripts_dir, 'run_native_indexing_profile.sh'))

      # Stub python3 so `profiling_stack.py verify` never touches a real stack or network
      python_stub = File.join(bin_dir, 'python3')
      File.write(python_stub, <<~SH)
        #!/usr/bin/env bash
        echo "ARCHIVALIST_PROFILING_STACK_BACKEND_URL=http://archivalist-profiling-backend:4567"
        echo "ARCHIVALIST_PROFILING_STACK_SOLR_URL=http://archivalist-profiling-solr:8983/solr"
        echo "ARCHIVALIST_PROFILING_STACK_ID=stub_stack"
      SH
      FileUtils.chmod(0o755, python_stub)

      # Stub git so build.json creation does not touch real repository state
      git_stub = File.join(bin_dir, 'git')
      File.write(git_stub, <<~SH)
        #!/usr/bin/env bash
        if [[ "$*" == *"rev-parse"* ]]; then
          echo "deadbeef"
        fi
        exit 0
      SH
      FileUtils.chmod(0o755, git_stub)

      # Stub docker to record exact argv of the profiler `docker exec` call without running Docker
      docker_log = File.join(tmp, 'docker_argv.json')
      docker_stub = File.join(bin_dir, 'docker')
      File.write(docker_stub, <<~SH)
        #!/usr/bin/env bash
        if [[ "$*" == *"printenv JAVA_OPTS"* ]]; then
          echo "-Xmx512m"
          exit 0
        fi
        ruby -rjson -e 'File.write(ENV.fetch("DOCKER_ARGV_LOG"), JSON.generate(ARGV))' -- "$@"
        exit 0
      SH
      FileUtils.chmod(0o755, docker_stub)

      base_env = {
        'PATH' => "#{bin_dir}:#{ENV.fetch('PATH', '')}",
        'STACK_STATE' => File.join(tmp, 'fake_stack.json'),
        'DOCKER_ARGV_LOG' => docker_log,
        'BACKEND_URL' => nil,
        'SOLR_BASE_URL' => nil,
        'REPEATS' => nil,
        'MODES' => nil,
        'TIMERS' => nil,
        'STAFF_THREADS' => nil,
        'PUI_THREADS' => nil,
        'PUI_FETCH_AUDIT' => nil
      }

      # 1. When PUI_FETCH_AUDIT is unset, docker exec argv must omit PUI_FETCH_AUDIT
      out1, err1, st1 = Open3.capture3(base_env, File.join(scripts_dir, 'run_native_indexing_profile.sh'))
      assert st1.success?, "wrapper failed when unset: stdout=#{out1} stderr=#{err1}"
      argv_unset = JSON.parse(File.read(docker_log))
      assert_equal 'exec', argv_unset.first
      refute argv_unset.any? { |arg| arg.include?('PUI_FETCH_AUDIT') },
             "expected PUI_FETCH_AUDIT to be omitted when unset, got: #{argv_unset.inspect}"

      # 2. When PUI_FETCH_AUDIT=1 is set, docker exec argv must contain `-e PUI_FETCH_AUDIT=1`
      FileUtils.rm_f(docker_log)
      env_set = base_env.merge('PUI_FETCH_AUDIT' => '1', 'MODES' => 'pui')
      out2, err2, st2 = Open3.capture3(env_set, File.join(scripts_dir, 'run_native_indexing_profile.sh'))
      assert st2.success?, "wrapper failed when set: stdout=#{out2} stderr=#{err2}"
      argv_set = JSON.parse(File.read(docker_log))
      idx = argv_set.each_index.find { |i| argv_set[i] == '-e' && argv_set[i + 1] == 'PUI_FETCH_AUDIT=1' }
      refute_nil idx, "expected `-e PUI_FETCH_AUDIT=1` in docker exec argv, got: #{argv_set.inspect}"
    end
  end
end
