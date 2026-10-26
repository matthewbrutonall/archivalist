#!/usr/bin/env ruby
# frozen_string_literal: true

# Archivalist Native Archival Object Indexing Profile Harness (Gate 2 Step 2)
#
# Runs isolated native indexing passes for strictly repository 4's :archival_object IDs
# through the REAL PeriodicIndexer native worker/index_records path into run-scoped stage cores
# with all Rust flags forced off.
#
# Guardrails:
# - Strictly repository 4 and :archival_object record type only; any attempt to run across
#   other repositories or record types fails closed before indexing.
# - Re-checks profiling stack identity (database-identity nonce through backend GET and Solr
#   identity core) before any login, core creation, or indexing; fails closed if invalid.
# - Unique run-scoped stage core per repeat (stage_staff_<run_id>_native); cores are retained,
#   never deleted or unloaded. Main or test Solr is strictly forbidden.
# - Measurement envelope:
#     AO ID enumeration -> 25-id batched backend fetch with full resolved_attributes ->
#     mapping/index_records -> Solr update -> final hard commit.
# - Preamble exclusion: backend login, stage core creation, and pre-pass memory snapshots
#   occur outside the timed region; repository and non-AO document preamble is strictly excluded.
# - All seven Rust flags are forced false and verified before each pass; no sidecar is spawned;
#   no profiler wrappers or category timers are installed.
# - 4 repeats: repeat 1 = warmup, repeats 2-4 = measured. Output includes raw wall/CPU,
#   doc counts, stack id, repo id, git commit/dirty, Java/JRuby versions, core names,
#   process RSS/HWM snapshots and Solr heap snapshots. Memory snapshot limitations are labeled.
# - No checkpoint/state mutation outside harness-local state (uses isolated Dir.mktmpdir).
# - Plans for downstream full document parity using compare_staff_index_parity.py, but
#   explicitly discloses that document counts do not establish parity.
#
# Environment:
#   SOLR_BASE_URL    allowlisted Solr base (default http://solr:8983/solr)
#   BACKEND_URL      allowlisted backend (default http://localhost:4567)
#   REPO_ID          repository ID (must be 4; default 4)
#   REPEATS          number of passes (default 4; first is warmup, 3 measured)
#   STAFF_THREADS    worker thread count (default 4)
#   STAFF_RECORDS_PER_THREAD records per batch (default 25)

require 'rubygems'
if defined?(JRUBY_VERSION) && File.directory?('/archivesspace/build/gems/jruby/3.1.0')
  ENV['GEM_HOME'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  ENV['GEM_PATH'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  Gem.paths = {
    'GEM_HOME' => '/archivesspace/build/gems/jruby/3.1.0',
    'GEM_PATH' => '/archivesspace/build/gems/jruby/3.1.0'
  }
end

require 'json'
require 'net/http'
require 'uri'
require 'fileutils'
require 'time'
require 'tmpdir'

require_relative 'integration_stage_harness'
require_relative 'archivalist_profiling_stack'
require_relative 'native_indexing_profile'

module NativeArchivalObjectProfile
  TARGET_REPO_ID = 4
  TARGET_RECORD_TYPE = :archival_object
  DEFAULT_RECORDS_PER_THREAD = 25
  DEFAULT_THREAD_COUNT = 4
  DEFAULT_REPEATS = 4

  RUST_FLAGS = %i[
    rust_index_document_builder_enabled
    rust_index_document_enrichment_enabled
    rust_pui_availability_preparation_enabled
    rust_pui_document_preparation_enabled
    rust_pui_common_metadata_enabled
    rust_pui_record_inheritance_candidate_enabled
    rust_pui_auxiliary_documents_enabled
  ].freeze

  VM_HWM_SEMANTICS = NativeIndexingProfile::VM_HWM_SEMANTICS

  NOTES = [
    'Scope: repository 4 archival_object records only, indexed via PeriodicIndexer#start_worker_thread into run-scoped stage cores.',
    'Measurement envelope: begins with AO ID enumeration and ends with final Solr hard commit; includes 25-id batched backend fetch with full resolved_attributes, native mapping/index_records, and Solr update HTTP POSTs.',
    'Preamble exclusion: backend authentication/login, Solr stage core creation, and pre-pass memory snapshots occur outside the timed region; repository and non-AO document preamble is strictly excluded.',
    'Rust flags and wrappers: all seven Rust integration flags were forced false and verified before each repeat; no sidecar was spawned; no profiler wrappers or category timers were installed.',
    'Repeat structure: repeat 1 is treated as warmup; repeats 2-4 are measured. Median metrics reflect repeats 2-4.',
    'Parity plan: document counts alone do NOT establish document parity; full type-strict parity between native baseline cores and candidate index cores must be verified using compare_staff_index_parity.py.',
    'Process memory: /proc/self/status VmRSS and VmHWM snapshots in KiB before and after each pass are point-in-time boundary measurements. VmHWM is the operating system process-lifetime high-water mark since startup, not a per-pass peak, so later repeats inherit earlier highs.',
    'Solr JVM heap: point-in-time snapshots of isolated Solr JVM heap used, committed, and max bytes via read-only /admin/info/system before and after each pass; snapshots are not continuous samples of peak heap usage during indexing.',
    'Isolation: each repeat indexes into a unique run-scoped stage core (stage_staff_<run_id>_native) and uses a fresh local temporary index state directory; no primary, test, or existing cores are altered or unloaded, and no state is mutated outside harness-local temporary state.'
  ].freeze

  # Portable thread-safe blocking queue matching java.util.concurrent.LinkedBlockingQueue contract
  # for environments without Java / JRuby (e.g. MRI unit tests).
  class PortableBlockingQueue
    def initialize(capacity = 4)
      @capacity = capacity
      @items = []
      @mutex = Mutex.new
      @not_empty = ConditionVariable.new
      @not_full = ConditionVariable.new
    end

    def offer(item, timeout_ms = nil, _unit = nil)
      timeout_sec = timeout_ms ? timeout_ms / 1000.0 : nil
      @mutex.synchronize do
        if timeout_sec
          deadline = Process.clock_gettime(Process::CLOCK_MONOTONIC) + timeout_sec
          while @items.length >= @capacity
            remaining = deadline - Process.clock_gettime(Process::CLOCK_MONOTONIC)
            return false if remaining <= 0

            @not_full.wait(@mutex, remaining)
          end
        else
          while @items.length >= @capacity
            @not_full.wait(@mutex)
          end
        end
        @items.push(item)
        @not_empty.signal
        true
      end
    end

    def poll(timeout_ms = nil, _unit = nil)
      timeout_sec = timeout_ms ? timeout_ms / 1000.0 : nil
      @mutex.synchronize do
        if timeout_sec
          deadline = Process.clock_gettime(Process::CLOCK_MONOTONIC) + timeout_sec
          while @items.empty?
            remaining = deadline - Process.clock_gettime(Process::CLOCK_MONOTONIC)
            return nil if remaining <= 0

            @not_empty.wait(@mutex, remaining)
          end
        else
          while @items.empty?
            @not_empty.wait(@mutex)
          end
        end
        item = @items.shift
        @not_full.signal
        item
      end
    end

    def length
      @mutex.synchronize { @items.length }
    end
  end

  def self.validate_repo_id!(repo_id)
    id = repo_id.to_i
    unless id == TARGET_REPO_ID
      raise ArgumentError, "Scope restriction: repository must be #{TARGET_REPO_ID} (got #{repo_id.inspect})"
    end

    id
  end

  def self.validate_record_type!(type)
    sym = type.to_sym
    unless sym == TARGET_RECORD_TYPE
      raise ArgumentError, "Scope restriction: record type must be :#{TARGET_RECORD_TYPE} (got #{type.inspect})"
    end

    sym
  end

  def self.sidecar_configured?(config = AppConfig)
    return false unless config.respond_to?(:has_key?) && config.has_key?(:rust_sidecar_url)

    val = config[:rust_sidecar_url]
    !val.nil? && val != false && !val.to_s.strip.empty?
  end

  def self.verify_sidecar_unconfigured!(config = AppConfig)
    if sidecar_configured?(config)
      raise 'Safety violation: sidecar URL is configured'
    end
  end

  def self.create_work_queue(capacity)
    if defined?(Java)
      java.util.concurrent.LinkedBlockingQueue.new(capacity)
    else
      PortableBlockingQueue.new(capacity)
    end
  end

  def self.time_unit_ms
    if defined?(Java)
      java.util.concurrent.TimeUnit::MILLISECONDS
    else
      :milliseconds
    end
  end

  def self.resolve_java_version
    if defined?(java.lang.System)
      java.lang.System.getProperty('java.version')
    else
      ENV['JAVA_VERSION']
    end
  end

  def self.resolve_jruby_version
    defined?(JRUBY_VERSION) ? JRUBY_VERSION : nil
  end

  def self.stage_doc_count(solr_base, core, getter: nil)
    url = "#{solr_base.chomp('/')}/#{core}/select?q=*:*&rows=0&wt=json"
    data = getter ? getter.call(url) : ArchivalistProfilingStack.http_get_json(url)
    data.dig('response', 'numFound') || 0
  end

  def self.stage_ao_count(solr_base, core, getter: nil)
    url = "#{solr_base.chomp('/')}/#{core}/select?q=types:archival_object&rows=0&wt=json"
    data = getter ? getter.call(url) : ArchivalistProfilingStack.http_get_json(url)
    data.dig('response', 'numFound') || 0
  end

  # Executes the exact measurement envelope:
  # Starts timer -> enumerates AO IDs -> dispatches 25-id batches to PeriodicIndexer worker threads
  # (which invoke fetch_records with full resolved_attributes and index_records/index_batch) ->
  # joins workers -> sends final hard commit -> stops timer.
  def self.execute_ao_index_envelope(indexer,
                                     repo_id: TARGET_REPO_ID,
                                     thread_count: DEFAULT_THREAD_COUNT,
                                     records_per_thread: DEFAULT_RECORDS_PER_THREAD,
                                     queue_factory: nil,
                                     all_ids_fetcher: nil)
    validate_repo_id!(repo_id)
    validate_record_type!(TARGET_RECORD_TYPE)

    # Measurement envelope begins here: timing starts BEFORE ID enumeration
    cpu_start = Process.times
    started = Process.clock_gettime(Process::CLOCK_MONOTONIC)

    # 1. Enumerate AO IDs for repo 4
    id_set = if all_ids_fetcher
               all_ids_fetcher.call(repo_id, TARGET_RECORD_TYPE)
             else
               JSONModel::HTTP.get_json(
                 JSONModel(TARGET_RECORD_TYPE).uri_for,
                 :all_ids => true,
                 :modified_since => 0
               ) || []
             end
    id_set = Array(id_set)

    worker_statuses = []
    if id_set.empty?
      # No IDs found; nothing to dispatch or index
    else
      work_queue = (queue_factory || method(:create_work_queue)).call(thread_count)
      unit = time_unit_ms

      workers = (0...thread_count).map do
        indexer.start_worker_thread(work_queue, TARGET_RECORD_TYPE)
      end

      begin
        id_set.each_slice(records_per_thread) do |id_subset|
          while !work_queue.offer(id_subset, 5000, unit)
            workers.each do |thread|
              thread.value if thread.status.nil?
            end
          end
        end
      ensure
        thread_count.times do
          work_queue.offer(:finished, 5000, unit)
        end
      end

      worker_statuses = workers.map do |thread|
        thread.join
        thread.value
      end

      error_status = defined?(PeriodicIndexer::WORKER_STATUS_INDEX_ERROR) ? PeriodicIndexer::WORKER_STATUS_INDEX_ERROR : 2
      if worker_statuses.any? { |s| s == error_status }
        raise 'Worker thread failed while indexing archival objects'
      end

      indexer.send_commit(:hard)
    end

    wall_ms = ((Process.clock_gettime(Process::CLOCK_MONOTONIC) - started) * 1000.0).round(2)
    cpu_end = Process.times
    cpu_delta = NativeIndexingProfile.compute_cpu_delta(cpu_start, cpu_end)

    {
      'wall_ms' => wall_ms,
      'cpu_delta' => cpu_delta,
      'ao_ids_count' => id_set.length,
      'worker_statuses' => worker_statuses
    }
  end

  def self.calculate_summary(results)
    measured_rows = results.reject { |r| r['is_warmup'] }
    measured_rows = results if measured_rows.empty?

    wall_raw = measured_rows.map { |r| r['wall_ms'] }
    cpu_user_raw = measured_rows.map { |r| r['cpu_user_ms'] }.compact
    cpu_sys_raw = measured_rows.map { |r| r['cpu_system_ms'] }.compact
    doc_counts = measured_rows.map { |r| r['solr_doc_count'] }
    ao_counts = measured_rows.map { |r| r['ao_doc_count'] }

    {
      'measured_repeats' => measured_rows.length,
      'wall_ms_raw' => wall_raw,
      'wall_ms_median' => NativeIndexingProfile.median(wall_raw),
      'cpu_user_ms_raw' => cpu_user_raw,
      'cpu_user_ms_median' => cpu_user_raw.empty? ? nil : NativeIndexingProfile.median(cpu_user_raw),
      'cpu_system_ms_raw' => cpu_sys_raw,
      'cpu_system_ms_median' => cpu_sys_raw.empty? ? nil : NativeIndexingProfile.median(cpu_sys_raw),
      'solr_doc_counts' => doc_counts,
      'ao_doc_counts' => ao_counts
    }
  end

  def self.build_report(run_id:,
                        backend_base:,
                        solr_base:,
                        stack_identity:,
                        results:,
                        repo_id: TARGET_REPO_ID,
                        thread_count: DEFAULT_THREAD_COUNT,
                        records_per_thread: DEFAULT_RECORDS_PER_THREAD,
                        source_provenance: nil)
    prov = source_provenance || ArchivalistHarnessSafety.source_provenance

    {
      'run_id' => run_id,
      'timestamp' => Time.now.utc.iso8601,
      'runtime' => RUBY_DESCRIPTION,
      'java_version' => resolve_java_version,
      'jruby_version' => resolve_jruby_version,
      'backend_base' => backend_base,
      'solr_base' => solr_base,
      'profiling_stack' => stack_identity,
      'repo_id' => repo_id,
      'record_type' => TARGET_RECORD_TYPE.to_s,
      'source_provenance' => prov,
      'threads' => [thread_count, records_per_thread],
      'rust_flags' => RUST_FLAGS.to_h { |f| [f.to_s, false] },
      'sidecar_present' => false,
      'profiler_wrappers' => false,
      'repeats' => results.length,
      'results' => results,
      'measured_summary' => calculate_summary(results),
      'parity_comparison_plan' => {
        'comparator' => 'rust-tools/scripts/compare_staff_index_parity.py',
        'repository' => repo_id,
        'baseline_cores' => results.map { |r| r['core'] },
        'note' => 'Pass --baseline-url <solr>/<core> --candidate-url <solr>/<candidate_core> --repository 4 for type-strict verification.'
      },
      'notes' => NOTES
    }
  end
end

RUST_FLAGS = NativeArchivalObjectProfile::RUST_FLAGS unless defined?(RUST_FLAGS)

if __FILE__ == $PROGRAM_NAME
  SOLR_BASE = ENV['SOLR_BASE_URL'] || 'http://solr:8983/solr'
  BACKEND_BASE = ENV['BACKEND_URL'] || 'http://localhost:4567'

  raise "Safety violation: Invalid or unapproved backend URL '#{BACKEND_BASE}'" unless ArchivalistHarnessSafety.valid_backend_url?(BACKEND_BASE)
  raise "Safety violation: Invalid or unapproved Solr URL '#{SOLR_BASE}'" unless ArchivalistHarnessSafety.valid_solr_url?(SOLR_BASE)

  # Profiling stack identity check: MUST fail closed before any writes, core creation, or login
  STACK_IDENTITY = ArchivalistProfilingStack.verify!(env: ENV, backend_url: BACKEND_BASE, solr_url: SOLR_BASE)

  REPO_ID = NativeArchivalObjectProfile.validate_repo_id!(ENV['REPO_ID'] || '4')
  REPEATS = Integer(ENV['REPEATS'] || NativeArchivalObjectProfile::DEFAULT_REPEATS.to_s)
  raise 'REPEATS must be >= 1' unless REPEATS >= 1

  STAFF_THREADS = Integer(ENV['STAFF_THREADS'] || NativeArchivalObjectProfile::DEFAULT_THREAD_COUNT.to_s)
  raise 'STAFF_THREADS must be >= 1' unless STAFF_THREADS >= 1

  STAFF_RECORDS_PER_THREAD = Integer(ENV['STAFF_RECORDS_PER_THREAD'] || NativeArchivalObjectProfile::DEFAULT_RECORDS_PER_THREAD.to_s)
  raise 'STAFF_RECORDS_PER_THREAD must be 25 for baseline comparison' unless STAFF_RECORDS_PER_THREAD == 25

  BENCHMARK_DIR = File.expand_path('../benchmarks', __dir__)
  FileUtils.mkdir_p(BENCHMARK_DIR)

  $LOAD_PATH.unshift('/archivesspace/common')
  $LOAD_PATH.unshift('/archivesspace/indexer')
  $LOAD_PATH.unshift('/archivesspace/indexer/app/lib')

  require 'active_support/core_ext/string/inflections'
  require 'config/config-distribution'
  AppConfig.reload
  AppConfig[:backend_url] = BACKEND_BASE
  AppConfig[:search_user_secret] = ENV['SEARCH_USER_SECRET'] || 'devserver'
  AppConfig[:indexer_thread_count] = STAFF_THREADS
  AppConfig[:indexer_records_per_thread] = STAFF_RECORDS_PER_THREAD

  # Strictly disable all Rust flags
  RUST_FLAGS.each { |flag| AppConfig[flag] = false }
  NativeArchivalObjectProfile.verify_sidecar_unconfigured!(AppConfig)

  require 'index_state'
  require 'asutils'
  require 'aspace_i18n'
  require 'ashttp'
  require 'record_inheritance'
  require 'rust_index_document_builder'
  require 'periodic_indexer'
  require 'pui_indexer'
  require 'large_tree_doc_indexer'
  require 'index_batch'

  include JSONModel

  puts '=' * 80
  puts 'Archivalist Native Archival Object Indexing Profile (Gate 2 Step 2)'
  puts "Runtime: #{RUBY_DESCRIPTION}"
  puts "Java: #{NativeArchivalObjectProfile.resolve_java_version || 'unknown'}  JRuby: #{NativeArchivalObjectProfile.resolve_jruby_version || 'unknown'}"
  puts "Backend: #{BACKEND_BASE}  Solr: #{SOLR_BASE}"
  puts "Profiling stack: #{STACK_IDENTITY['stack_id']} (identity verified)"
  puts "Scope: repository #{REPO_ID}, record type :#{NativeArchivalObjectProfile::TARGET_RECORD_TYPE}"
  puts "Repeats: #{REPEATS} (repeat 1 = warmup, repeats 2-#{REPEATS} = measured)"
  puts "Threads: #{STAFF_THREADS} workers x #{STAFF_RECORDS_PER_THREAD} records per thread"
  puts "Rust flags: all false (#{RUST_FLAGS.map { |f| AppConfig[f] }.uniq.inspect})"
  puts 'Profilers/sidecar: none (unwrapped real PeriodicIndexer native worker path)'
  puts '=' * 80

  results = []
  first_run_id = nil

  REPEATS.times do |repeat_idx|
    repeat_num = repeat_idx + 1
    is_warmup = (repeat_num == 1)
    run_id = ArchivalistHarnessSafety.generate_run_id
    first_run_id ||= run_id

    stage_role = ENV['STAGE_ROLE'] || 'native'
    core = ArchivalistHarnessSafety.stage_core_name(run_id, stage_role, 'staff')
    ArchivalistHarnessSafety.ensure_stage_core(core, run_id, solr_base: SOLR_BASE)

    data_dir = Dir.mktmpdir('archivalist_native_ao_profile_')
    AppConfig[:data_directory] = data_dir
    AppConfig[:solr_url] = "#{SOLR_BASE}/#{core}"

    RUST_FLAGS.each { |flag| raise "Rust flag #{flag} unexpectedly enabled" if AppConfig[flag] }

    indexer = PeriodicIndexer.new(BACKEND_BASE, IndexState.new, "Native Staff AO Profile [#{run_id}]", false)
    indexer.login
    JSONModel.set_repository(REPO_ID)

    puts "\n[repeat #{repeat_num}/#{REPEATS}#{is_warmup ? ' (WARMUP)' : ' (MEASURED)'}] Indexing repo #{REPO_ID} :#{NativeArchivalObjectProfile::TARGET_RECORD_TYPE} into #{core} ..."

    proc_mem_before = NativeIndexingProfile.read_proc_status
    solr_heap_before = NativeIndexingProfile.fetch_solr_jvm_heap(SOLR_BASE)

    timing_res = NativeArchivalObjectProfile.execute_ao_index_envelope(
      indexer,
      repo_id: REPO_ID,
      thread_count: STAFF_THREADS,
      records_per_thread: STAFF_RECORDS_PER_THREAD
    )

    proc_mem_after = NativeIndexingProfile.read_proc_status
    solr_heap_after = NativeIndexingProfile.fetch_solr_jvm_heap(SOLR_BASE)

    doc_count = NativeArchivalObjectProfile.stage_doc_count(SOLR_BASE, core)
    ao_count = NativeArchivalObjectProfile.stage_ao_count(SOLR_BASE, core)

    FileUtils.remove_entry(data_dir) rescue nil

    res_fields = NativeIndexingProfile.resource_fields(
      cpu_delta: timing_res['cpu_delta'],
      proc_mem_before: proc_mem_before,
      proc_mem_after: proc_mem_after,
      solr_heap_before: solr_heap_before,
      solr_heap_after: solr_heap_after
    )

    result_row = {
      'repeat' => repeat_num,
      'is_warmup' => is_warmup,
      'run_id' => run_id,
      'core' => core,
      'wall_ms' => timing_res['wall_ms']
    }.merge(res_fields).merge(
      'solr_doc_count' => doc_count,
      'ao_doc_count' => ao_count,
      'ao_ids_count' => timing_res['ao_ids_count']
    )

    results << result_row

    puts "  wall #{timing_res['wall_ms']} ms, CPU user #{timing_res['cpu_delta']['cpu_user_ms'] || 'null'} ms, " \
         "system #{timing_res['cpu_delta']['cpu_system_ms'] || 'null'} ms | " \
         "Solr docs: total=#{doc_count}, archival_objects=#{ao_count} (IDs enumerated: #{timing_res['ao_ids_count']})"
    if proc_mem_after['vm_rss_kib'] || proc_mem_after['vm_hwm_kib']
      puts "    process memory (KiB): VmRSS before=#{proc_mem_before['vm_rss_kib'].inspect}, after=#{proc_mem_after['vm_rss_kib'].inspect} | " \
           "VmHWM after=#{proc_mem_after['vm_hwm_kib'].inspect} (#{NativeArchivalObjectProfile::VM_HWM_SEMANTICS})"
    end
    if solr_heap_after['used_bytes']
      puts "    Solr JVM heap (bytes): used before=#{solr_heap_before['used_bytes']}, after=#{solr_heap_after['used_bytes']} | " \
           "committed=#{solr_heap_after['committed_bytes']} | max=#{solr_heap_after['max_bytes']}"
    end
  end

  report = NativeArchivalObjectProfile.build_report(
    run_id: first_run_id,
    backend_base: BACKEND_BASE,
    solr_base: SOLR_BASE,
    stack_identity: STACK_IDENTITY,
    results: results,
    repo_id: REPO_ID,
    thread_count: STAFF_THREADS,
    records_per_thread: STAFF_RECORDS_PER_THREAD
  )

  path = File.join(BENCHMARK_DIR, "native_ao_profile_#{first_run_id}.json")
  File.write(path, JSON.pretty_generate(report))
  puts "\nReport: #{path}"
  summary = report['measured_summary']
  puts "Measured repeats (2-#{REPEATS}): wall median #{summary['wall_ms_median']} ms over #{summary['measured_repeats']} pass(es); " \
       "Solr doc counts #{summary['solr_doc_counts'].inspect}, AO counts #{summary['ao_doc_counts'].inspect}"
  puts 'Note: Document counts do not establish document parity. Parity must be verified via compare_staff_index_parity.py.'
end
