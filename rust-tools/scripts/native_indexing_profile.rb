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

if __FILE__ == $PROGRAM_NAME
  SOLR_BASE = ENV['SOLR_BASE_URL'] || 'http://solr:8983/solr'
  BACKEND_BASE = ENV['BACKEND_URL'] || 'http://localhost:4567'
  raise "Safety violation: Invalid or unapproved backend URL '#{BACKEND_BASE}'" unless ArchivalistHarnessSafety.valid_backend_url?(BACKEND_BASE)
  raise "Safety violation: Invalid or unapproved Solr URL '#{SOLR_BASE}'" unless ArchivalistHarnessSafety.valid_solr_url?(SOLR_BASE)

  # Database/Solr identity of the disposable profiling stack, before any write or login.
  STACK_IDENTITY = ArchivalistProfilingStack.verify!(env: ENV, backend_url: BACKEND_BASE, solr_url: SOLR_BASE)

  REPEATS = Integer(ENV['REPEATS'] || '1')
  raise 'REPEATS must be >= 1' unless REPEATS >= 1
  MODES = (ENV['MODES'] || 'staff,pui').split(',').map(&:strip)
  raise "MODES must be a subset of staff,pui (got #{MODES.inspect})" if MODES.empty? || !(MODES - %w[staff pui]).empty?
  TIMERS = ENV.fetch('TIMERS', '1') != '0'

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

  puts '=' * 80
  puts 'Archivalist Native Indexing Profile'
  puts "Runtime: #{RUBY_DESCRIPTION}"
  puts "Backend: #{BACKEND_BASE}  Solr: #{SOLR_BASE}"
  puts "Profiling stack: #{STACK_IDENTITY['stack_id']} (database identity and Solr identity core verified)"
  puts "Modes: #{MODES.join(', ')}  Repeats: #{REPEATS}  Timers: #{TIMERS ? 'on' : 'off (wall time only)'}"
  puts "Threads: staff #{AppConfig[:indexer_thread_count]} x #{AppConfig[:indexer_records_per_thread]} records, " \
       "pui #{AppConfig[:pui_indexer_thread_count]} x #{AppConfig[:pui_indexer_records_per_thread]} records"
  puts "Rust flags: all false (#{RUST_FLAGS.map { |f| AppConfig[f] }.uniq.inspect})"
  if TIMERS
    puts "Timer hooks: #{hooks.transform_values(&:length).inspect}; " \
         "calibrated #{(ArchivalistRubySideTiming::CALIBRATION[:per_call_ms] * 1000.0).round(2)} us per timer call"
  end
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
      ArchivalistRubySideTiming.start! if TIMERS
      started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      begin
        indexer.run_index_round
      ensure
        raw = ArchivalistRubySideTiming.stop! if TIMERS
      end
      wall_ms = ((Process.clock_gettime(Process::CLOCK_MONOTONIC) - started) * 1000.0).round(2)
      doc_count = stage_doc_count(core)
      summary = TIMERS ? ArchivalistRubySideTiming.summarize(raw, wall_ms) : nil

      puts "  wall #{wall_ms} ms, #{doc_count} Solr documents"
      if summary
        summary['category_ms'].first(12).each { |cat, ms| puts format('    %-30s %10.1f ms', cat, ms) }
        puts "    threads #{summary['threads']}, timer calls #{summary['timer_calls']}, est. timer overhead #{summary['estimated_timer_overhead_ms']} ms"
      end
      results << { 'repeat' => repeat + 1, 'mode' => mode, 'run_id' => run_id, 'core' => core, 'wall_ms' => wall_ms,
                   'solr_doc_count' => doc_count, 'timing' => summary }
    end
  end

  per_mode = MODES.to_h do |mode|
    rows = results.select { |r| r['mode'] == mode }
    timed = rows.map { |r| r['timing'] }.compact
    [mode, {
      'repeats' => rows.length,
      'wall_ms_raw' => rows.map { |r| r['wall_ms'] },
      'wall_ms_median' => native_profile_median(rows.map { |r| r['wall_ms'] }),
      'solr_doc_counts' => rows.map { |r| r['solr_doc_count'] },
      'timing_median' => timed.empty? ? nil : ArchivalistRubySideTiming.median_summary(timed)
    }]
  end

  report = {
    'run_id' => first_run_id,
    'timestamp' => Time.now.utc.iso8601,
    'runtime' => RUBY_DESCRIPTION,
    'backend_base' => BACKEND_BASE,
    'solr_base' => SOLR_BASE,
    'profiling_stack' => STACK_IDENTITY,
    'timers' => TIMERS,
    'threads' => { 'staff' => [AppConfig[:indexer_thread_count], AppConfig[:indexer_records_per_thread]],
                   'pui' => [AppConfig[:pui_indexer_thread_count], AppConfig[:pui_indexer_records_per_thread]] },
    'rust_flags' => RUST_FLAGS.to_h { |f| [f.to_s, AppConfig[f]] },
    'inventory' => inventory,
    'results' => results,
    'per_mode' => per_mode,
    'notes' => [
      'Category times are exclusive and summed across indexer threads; with more than one thread they can exceed wall time.',
      'index_round_main_thread includes the coordinating thread waiting for its workers.',
      'A full round indexes every repository visible to the backend.'
    ]
  }
  path = File.join(BENCHMARK_DIR, "native_profile_#{first_run_id}.json")
  File.write(path, JSON.pretty_generate(report))
  puts "\nReport: #{path}"
  per_mode.each { |mode, m| puts "#{mode.upcase}: wall median #{m['wall_ms_median']} ms over #{m['repeats']} round(s); docs #{m['solr_doc_counts'].inspect}" }
end
