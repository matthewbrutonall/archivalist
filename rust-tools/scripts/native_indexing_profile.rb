#!/usr/bin/env ruby
# frozen_string_literal: true

# Archivalist native indexing profile
#
# Runs complete, real index rounds (PeriodicIndexer#run_index_round for STAFF and PUIIndexer's for
# PUI) from an empty index state into fresh, run-scoped stage cores, with all Rust flags forced
# off, and attributes the wall time to pipeline categories with archivalist_ruby_side_timing.rb:
# backend HTTP, record fetching, document preparation, JSON, Solr update/commit/delete HTTP,
# PUI tree work, and the coordinating thread's waiting. Nothing is read from a pre-fetched
# snapshot, so backend fetching and Solr writes are inside the measured region.
#
# Guardrails: it only runs against a disposable profiling stack (profiling_stack.py). Before
# anything else it re-checks the stack's database-identity nonce through BACKEND_URL (anonymous
# GET) and the stack's identity core through SOLR_BASE_URL, and refuses otherwise, so a main or
# shared test backend/Solr is rejected before any login, core creation or indexing. URLs must
# also pass the shared allowlist. Cores are created per run as stage_{staff,pui}_<run_id>_native
# and retained; index state lives in a fresh temporary directory per round. A full round indexes
# EVERY repository the (verified) backend can see; the report's inventory lists them.
#
# Environment:
#   SOLR_BASE_URL   allowlisted Solr base (default http://solr:8983/solr)
#   BACKEND_URL     backend (default http://localhost:4567)
#   REPEATS         full rounds per mode (default 1; each uses a new run id and new cores)
#   MODES           staff,pui (default both)
#   TIMERS          1 (default) attributes categories; 0 measures wall time only (overhead check)
#   PUI_FETCH_AUDIT 1 (opt-in; default off) accounts for PUI archival_objects batch GETs & overlap
#   PUI_TREE_BATCH_SIZE override PUI tree-discovery batch size (clamped 1..200; default unchanged)
#   PUI_RECORDS_PER_THREAD override PUI records per thread (range 1..200; default unchanged)
#   STAFF_THREADS / PUI_THREADS  override indexer_thread_count / pui_indexer_thread_count
#
# With more than one indexer thread the category times are summed across threads and can
# exceed the wall time; 'threads' in each result says how many threads were seen.

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
require_relative 'archivalist_ruby_side_timing'
require_relative 'archivalist_profiling_stack'
require_relative 'archivalist_pui_fetch_audit'

RUST_FLAGS = %i[
  rust_index_document_builder_enabled rust_index_document_enrichment_enabled
  rust_pui_availability_preparation_enabled rust_pui_document_preparation_enabled
  rust_pui_common_metadata_enabled rust_pui_record_inheritance_candidate_enabled
  rust_pui_auxiliary_documents_enabled
].freeze

def native_profile_median(values)
  sorted = values.sort
  mid = sorted.length / 2
  sorted.length.odd? ? sorted[mid] : ((sorted[mid - 1] + sorted[mid]) / 2.0).round(3)
end

def validate_pui_records_per_thread(raw)
  return nil if raw.nil? || (raw.respond_to?(:to_str) && raw.to_str.strip.empty?)

  val = if raw.is_a?(Integer)
          raw
        elsif raw.is_a?(String) && raw.strip =~ /\A[+-]?\d+\z/
          Integer(raw.strip, 10)
        else
          raise ArgumentError, "PUI_RECORDS_PER_THREAD must be an integer in 1..200 (got #{raw.inspect})"
        end

  unless val.between?(1, 200)
    raise ArgumentError, "PUI_RECORDS_PER_THREAD must be between 1 and 200 (got #{val})"
  end

  val
end

if __FILE__ == $PROGRAM_NAME
  SOLR_BASE = ENV['SOLR_BASE_URL'] || 'http://solr:8983/solr'
  BACKEND_BASE = ENV['BACKEND_URL'] || 'http://localhost:4567'
  raise "Safety violation: Invalid or unapproved backend URL '#{BACKEND_BASE}'" unless ArchivalistHarnessSafety.valid_backend_url?(BACKEND_BASE)
  raise "Safety violation: Invalid or unapproved Solr URL '#{SOLR_BASE}'" unless ArchivalistHarnessSafety.valid_solr_url?(SOLR_BASE)

  PUI_RECORDS_PER_THREAD_OVERRIDE = validate_pui_records_per_thread(ENV['PUI_RECORDS_PER_THREAD'])

  # Database/Solr identity of the disposable profiling stack, before any write or login.
  STACK_IDENTITY = ArchivalistProfilingStack.verify!(env: ENV, backend_url: BACKEND_BASE, solr_url: SOLR_BASE)

  REPEATS = Integer(ENV['REPEATS'] || '1')
  raise 'REPEATS must be >= 1' unless REPEATS >= 1
  MODES = (ENV['MODES'] || 'staff,pui').split(',').map(&:strip)
  raise "MODES must be a subset of staff,pui (got #{MODES.inspect})" if MODES.empty? || !(MODES - %w[staff pui]).empty?
  TIMERS = ENV.fetch('TIMERS', '1') != '0'
  PUI_FETCH_AUDIT = ArchivalistPuiFetchAudit.enabled?(ENV)

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
  AppConfig[:indexer_thread_count] = Integer(ENV['STAFF_THREADS']) if ENV['STAFF_THREADS']
  AppConfig[:pui_indexer_thread_count] = Integer(ENV['PUI_THREADS']) if ENV['PUI_THREADS']
  if PUI_RECORDS_PER_THREAD_OVERRIDE
    AppConfig[:pui_indexer_records_per_thread] = PUI_RECORDS_PER_THREAD_OVERRIDE
  end
  if ENV['PUI_TREE_BATCH_SIZE'] && !ENV['PUI_TREE_BATCH_SIZE'].empty?
    AppConfig[:pui_tree_discovery_batch_size] = ENV['PUI_TREE_BATCH_SIZE']
  end
  RUST_FLAGS.each { |flag| AppConfig[flag] = false }

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

  effective_pui_tree_batch_size = PUIIndexer.resolve_tree_discovery_batch_size(
    AppConfig.has_key?(:pui_tree_discovery_batch_size) ? AppConfig[:pui_tree_discovery_batch_size] : nil,
    AppConfig[:pui_indexer_records_per_thread]
  )

  hooks = nil
  if TIMERS
    hooks = ArchivalistRubySideTiming.install!(
      builder_class: RustIndexDocumentBuilder, indexer_common: IndexerCommon, pui_indexer: PUIIndexer,
      record_inheritance: RecordInheritance, asutils: ASUtils
    ).merge(ArchivalistRubySideTiming.install_native_pipeline!(
      indexer_common: IndexerCommon, periodic_indexer: PeriodicIndexer, pui_indexer: PUIIndexer,
      large_tree: LargeTreeDocIndexer, index_batch: IndexBatch, jsonmodel_http: JSONModel::HTTP
    ))
  end

  pui_fetch_audit_hooks = nil
  if PUI_FETCH_AUDIT && MODES.include?('pui')
    pui_fetch_audit_hooks = ArchivalistPuiFetchAudit.install!(
      pui_indexer: PUIIndexer, jsonmodel_http: JSONModel::HTTP
    )
  end

  puts '=' * 80
  puts 'Archivalist Native Indexing Profile'
  puts "Runtime: #{RUBY_DESCRIPTION}"
  puts "Backend: #{BACKEND_BASE}  Solr: #{SOLR_BASE}"
  puts "Profiling stack: #{STACK_IDENTITY['stack_id']} (database identity and Solr identity core verified)"
  puts "Modes: #{MODES.join(', ')}  Repeats: #{REPEATS}  Timers: #{TIMERS ? 'on' : 'off (wall time only)'}  " \
       "PUI fetch audit: #{PUI_FETCH_AUDIT ? 'on (PUI pass only; observational)' : 'off'}"
  puts "Threads: staff #{AppConfig[:indexer_thread_count]} x #{AppConfig[:indexer_records_per_thread]} records, " \
       "pui #{AppConfig[:pui_indexer_thread_count]} x #{AppConfig[:pui_indexer_records_per_thread]} records " \
       "(tree discovery batch: #{effective_pui_tree_batch_size})"
  puts "Rust flags: all false (#{RUST_FLAGS.map { |f| AppConfig[f] }.uniq.inspect})"
  if TIMERS
    puts "Timer hooks: #{hooks.transform_values(&:length).inspect}; " \
         "calibrated #{(ArchivalistRubySideTiming::CALIBRATION[:per_call_ms] * 1000.0).round(2)} us per timer call"
  end
  puts "PUI fetch audit hooks: #{pui_fetch_audit_hooks.transform_values(&:length).inspect}" if pui_fetch_audit_hooks
  puts '=' * 80

  # Inventory: what a full round will index (counts only).
  inventory_indexer = PeriodicIndexer.new(BACKEND_BASE, nil, 'Native Profile Inventory', false)
  inventory_indexer.login
  inventory = JSONModel(:repository).all.map do |repo|
    JSONModel.set_repository(repo.id)
    counts = inventory_indexer.record_types.each_with_object({}) do |type, out|
      ids = JSONModel::HTTP.get_json(JSONModel(type).uri_for, :all_ids => true) rescue nil
      out[type.to_s] = ids.length if ids.is_a?(Array) && !ids.empty?
    end
    { 'uri' => repo.uri, 'repo_code' => repo.repo_code, 'publish' => repo.publish, 'record_counts' => counts }
  end
  inventory.each { |r| puts "Repository #{r['uri']} (#{r['repo_code']}, publish=#{r['publish']}): #{r['record_counts'].values.sum} records #{r['record_counts'].inspect}" }

  def stage_doc_count(core)
    uri = URI("#{SOLR_BASE}/#{core}/select?q=*:*&rows=0&wt=json")
    res = Net::HTTP.get_response(uri)
    raise "Solr count failed for #{core}: HTTP #{res.code}" unless res.is_a?(Net::HTTPSuccess)
    JSON.parse(res.body).dig('response', 'numFound')
  end

  results = []
  first_run_id = nil
  REPEATS.times do |repeat|
    run_id = ArchivalistHarnessSafety.generate_run_id
    first_run_id ||= run_id
    MODES.each do |mode|
      core = ArchivalistHarnessSafety.stage_core_name(run_id, 'native', mode)
      ArchivalistHarnessSafety.ensure_stage_core(core, run_id, solr_base: SOLR_BASE)
      data_dir = Dir.mktmpdir("archivalist_native_profile_#{mode}_")
      AppConfig[:data_directory] = data_dir
      AppConfig[:solr_url] = "#{SOLR_BASE}/#{core}"
      RUST_FLAGS.each { |flag| raise "Rust flag #{flag} unexpectedly enabled" if AppConfig[flag] }

      indexer = if mode == 'staff'
                  PeriodicIndexer.new(BACKEND_BASE, IndexState.new, 'Native Profile Staff', false)
                else
                  PUIIndexer.new(BACKEND_BASE, IndexState.new('indexer_pui_state'), 'Native Profile PUI')
                end

      puts "\n[repeat #{repeat + 1}/#{REPEATS}] #{mode.upcase} full index round into #{core} ..."
      audit_this_pass = PUI_FETCH_AUDIT && mode == 'pui'
      ArchivalistRubySideTiming.start! if TIMERS
      ArchivalistPuiFetchAudit.start! if audit_this_pass
      started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      pui_fetch_audit_summary = nil
      begin
        indexer.run_index_round
      ensure
        pui_fetch_audit_summary = ArchivalistPuiFetchAudit.stop! if audit_this_pass
        raw = ArchivalistRubySideTiming.stop! if TIMERS
      end
      wall_ms = ((Process.clock_gettime(Process::CLOCK_MONOTONIC) - started) * 1000.0).round(2)
      doc_count = stage_doc_count(core)
      summary = TIMERS ? ArchivalistRubySideTiming.summarize(raw, wall_ms) : nil

      puts "  wall #{wall_ms} ms, #{doc_count} Solr documents"
      if summary && summary['worker_threads'].to_i.zero?
        puts '    categories (wall time, single thread):'
        summary['category_ms'].first(12).each { |cat, ms| puts format('      %-30s %10.1f ms', cat, ms) }
        puts format('      %-30s %10.1f ms', 'unattributed', summary['unattributed_ms'])
      elsif summary
        puts '    coordinating thread (wall time; index_round_main_thread includes waiting for workers):'
        summary['main_thread_category_ms'].first(8).each { |cat, ms| puts format('      %-30s %10.1f ms', cat, ms) }
        puts format('      %-30s %10.1f ms', 'unattributed (main thread)', summary['main_thread_unattributed_ms'])
        puts "    #{summary['worker_threads']} worker threads (SUMMED thread time, overlaps the waiting above; " \
             "total #{summary['worker_summed_thread_ms'].round(1)} ms):"
        summary['worker_category_ms'].first(12).each { |cat, ms| puts format('      %-30s %10.1f ms', cat, ms) }
      end
      if summary
        puts "    timer calls #{summary['timer_calls']}, est. timer overhead #{summary['estimated_timer_overhead_ms']} ms"
      end
      if pui_fetch_audit_summary
        ord_p = pui_fetch_audit_summary.dig('phases', ArchivalistPuiFetchAudit::PHASE_ORDINARY)
        tree_p = pui_fetch_audit_summary.dig('phases', ArchivalistPuiFetchAudit::PHASE_TREE_DISCOVERY)
        ov = pui_fetch_audit_summary['overlap']
        puts '    pui_fetch_audit (archival_objects batch GETs; IDs requested by HTTP 200 calls; observational only):'
        puts format('      ordinary_indexing: calls=%d (ok=%d, fail=%d), unique_ok_req_ids=%d, ok_req_ids=%d, req_ids=%d, bytes=%d, summed_call_ms=%.1f',
                    ord_p['calls'], ord_p['successful_calls'], ord_p['failed_calls'],
                    ord_p['unique_successful_requested_ids'], ord_p['successful_requested_ids_total'],
                    ord_p['requested_ids_total'], ord_p['response_bytes'], ord_p['call_duration_sum_ms'])
        puts format('      tree_discovery:    calls=%d (ok=%d, fail=%d), unique_ok_req_ids=%d, ok_req_ids=%d, req_ids=%d, bytes=%d, summed_call_ms=%.1f',
                    tree_p['calls'], tree_p['successful_calls'], tree_p['failed_calls'],
                    tree_p['unique_successful_requested_ids'], tree_p['successful_requested_ids_total'],
                    tree_p['requested_ids_total'], tree_p['response_bytes'], tree_p['call_duration_sum_ms'])
        puts format('      requested_id_overlap (HTTP 200): intersection=%d, ordinary_only=%d, tree_discovery_only=%d, union=%d (tree_overlap_ratio=%.4f)',
                    ov['intersection_successful_requested_ids'], ov['ordinary_only_successful_requested_ids'],
                    ov['tree_discovery_only_successful_requested_ids'], ov['union_unique_successful_requested_ids'],
                    ov['tree_discovery_overlap_ratio'])
      end
      result_row = { 'repeat' => repeat + 1, 'mode' => mode, 'run_id' => run_id, 'core' => core, 'wall_ms' => wall_ms,
                     'solr_doc_count' => doc_count, 'timing' => summary }
      if mode == 'pui'
        result_row['pui_records_per_thread'] = indexer.instance_variable_get(:@records_per_thread)
        result_row['pui_tree_batch_size'] = indexer.tree_discovery_batch_size
        result_row['pui_tree_discovery_batch_size'] = indexer.tree_discovery_batch_size
      end
      result_row['pui_fetch_audit'] = pui_fetch_audit_summary if audit_this_pass
      results << result_row
    end
  end

  per_mode = MODES.to_h do |mode|
    rows = results.select { |r| r['mode'] == mode }
    timed = rows.map { |r| r['timing'] }.compact
    audited = rows.map { |r| r['pui_fetch_audit'] }.compact
    mode_summary = {
      'repeats' => rows.length,
      'wall_ms_raw' => rows.map { |r| r['wall_ms'] },
      'wall_ms_median' => native_profile_median(rows.map { |r| r['wall_ms'] }),
      'solr_doc_counts' => rows.map { |r| r['solr_doc_count'] },
      'timing_median' => timed.empty? ? nil : ArchivalistRubySideTiming.median_summary(timed)
    }
    mode_summary['pui_fetch_audit_median'] = ArchivalistPuiFetchAudit.median_summary(audited) unless audited.empty?
    [mode, mode_summary]
  end

  notes = [
    'With worker threads, category_ms is summed thread time and unattributed_ms is null; use main_thread_category_ms and main_thread_unattributed_ms for wall-time attribution and worker_category_ms for the work itself.',
    'index_round_main_thread includes the coordinating thread waiting for its workers.',
    'A full round indexes every repository visible to the backend.'
  ]
  if PUI_FETCH_AUDIT
    notes << 'pui_fetch_audit is observational accounting of backend archival_objects batch GETs (id_set) only; ID sets and overlap measure IDs requested by successful HTTP 200 calls (without parsing JSON response bodies to verify returned records), and cannot imply safe cache reuse or deduplication across phases because records and ancestor trees may change between ordinary indexing and tree discovery or across checkpoints.'
  end

  report = {
    'run_id' => first_run_id,
    'timestamp' => Time.now.utc.iso8601,
    'runtime' => RUBY_DESCRIPTION,
    'backend_base' => BACKEND_BASE,
    'solr_base' => SOLR_BASE,
    'profiling_stack' => STACK_IDENTITY,
    'timers' => TIMERS,
    'pui_fetch_audit' => PUI_FETCH_AUDIT,
    'pui_records_per_thread' => AppConfig[:pui_indexer_records_per_thread],
    'pui_tree_batch_size' => effective_pui_tree_batch_size,
    'pui_tree_discovery_batch_size' => effective_pui_tree_batch_size,
    'threads' => { 'staff' => [AppConfig[:indexer_thread_count], AppConfig[:indexer_records_per_thread]],
                   'pui' => [AppConfig[:pui_indexer_thread_count], AppConfig[:pui_indexer_records_per_thread]] },
    'rust_flags' => RUST_FLAGS.to_h { |f| [f.to_s, AppConfig[f]] },
    'inventory' => inventory,
    'results' => results,
    'per_mode' => per_mode,
    'notes' => notes
  }
  path = File.join(BENCHMARK_DIR, "native_profile_#{first_run_id}.json")
  File.write(path, JSON.pretty_generate(report))
  puts "\nReport: #{path}"
  per_mode.each { |mode, m| puts "#{mode.upcase}: wall median #{m['wall_ms_median']} ms over #{m['repeats']} round(s); docs #{m['solr_doc_counts'].inspect}" }
end
