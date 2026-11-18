#!/usr/bin/env ruby
# frozen_string_literal: true

require 'rubygems'
if defined?(JRUBY_VERSION) && File.directory?('/archivesspace/build/gems/jruby/3.1.0')
  Gem.paths = {
    'GEM_HOME' => '/archivesspace/build/gems/jruby/3.1.0',
    'GEM_PATH' => '/archivesspace/build/gems/jruby/3.1.0'
  }
end

require 'json'
require 'open3'
require_relative 'integration_stage_harness'

backend = ENV.fetch('BACKEND_URL')
resource_uri = ENV.fetch('RESOURCE_URI')
input = File.expand_path(ENV.fetch('AUXILIARY_REFERENCE_FILE'))
output = File.expand_path(ENV.fetch('OUTPUT_FILE'))
raise 'unapproved backend URL' unless ArchivalistHarnessSafety.valid_backend_url?(backend)
raise 'invalid resource URI' unless resource_uri.match?(%r{\A/repositories/([1-9]\d*)/resources/([1-9]\d*)\z})
benchmarks = File.expand_path('../benchmarks', __dir__)
raise 'input must be under benchmarks' unless File.dirname(input) == benchmarks
raise 'output must be under benchmarks' unless File.dirname(output) == benchmarks
raise 'invalid output file name' unless File.basename(output).match?(/\Apui_(?:ordinary|full)_reference_[A-Za-z0-9_]+\.json\z/)
raise 'output exists' if File.exist?(output)
mode = ENV.fetch('AUXILIARY_MODE', 'none')
raise 'invalid auxiliary mode' unless %w[none native candidate candidate_parallel].include?(mode)

$LOAD_PATH.unshift('/archivesspace/common')
$LOAD_PATH.unshift('/archivesspace/indexer')
$LOAD_PATH.unshift('/archivesspace/indexer/app/lib')
require 'active_support/core_ext/string/inflections'
require 'config/config-distribution'
AppConfig.reload
AppConfig[:backend_url] = backend
AppConfig[:search_user_secret] = ENV['SEARCH_USER_SECRET'] || 'devserver'
%i[
  rust_index_document_builder_enabled rust_index_document_enrichment_enabled
  rust_pui_availability_preparation_enabled rust_pui_document_preparation_enabled
  rust_pui_common_metadata_enabled rust_pui_record_inheritance_candidate_enabled
  rust_pui_auxiliary_documents_enabled
].each { |flag| AppConfig[flag] = false }

require 'asutils'
require 'aspace_i18n'
require 'ashttp'
require 'record_inheritance'
require 'pui_indexer'
require 'large_tree_doc_indexer'

indexer = PUIIndexer.get_indexer(nil, 'PUI ordinary spike reference')
indexer.login
JSONModel.set_repository(resource_uri.split('/')[2].to_i)
aux = JSON.parse(File.read(input))
ordered = aux.fetch('documents').find { |doc| doc['primary_type'] == 'resource_ordered_records' }
raise 'missing ordered records' unless ordered
uris = JSON.parse(ordered.fetch('json')).fetch('uris').map { |entry| entry.fetch('ref') }
raise 'resource URI mismatch' unless uris.first == resource_uri
raise 'duplicate ordered URI' unless uris.uniq.length == uris.length
ao_ids = uris.drop(1).map do |uri|
  match = uri.match(%r{\A/repositories/#{resource_uri.split('/')[2]}/archival_objects/([1-9]\d*)\z})
  raise 'unexpected ordered URI' unless match
  match[1].to_i
end

documents = []
sample_input_file = ENV['SAMPLE_INPUT_FILE'] && File.expand_path(ENV['SAMPLE_INPUT_FILE'])
if sample_input_file
  raise 'sample input must be under benchmarks' unless File.dirname(sample_input_file) == benchmarks
  raise 'invalid sample file name' unless File.basename(sample_input_file).match?(/\Apui_ordinary_input_[A-Za-z0-9_]+\.json\z/)
  raise 'sample input exists' if File.exist?(sample_input_file)
end
indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
  instance_variable_get(:@batch_hooks).each { |hook| hook.call(batch) }
  documents.concat(batch.map { |doc| doc })
  batch.destroy
end

def add_native_auxiliary(indexer, resource_uri, documents)
  indexer.send(:add_infscroll_docs, [resource_uri], documents)
  tree = LargeTreeDocIndexer.new(documents)
  tree.add_largetree_docs([resource_uri])
  tree.deletes
end

started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
rust_job = nil
rust_started = nil
rust_process_elapsed = nil
rust_internal_seconds = nil
fallback_used = false
fallback_reason = nil
if %w[candidate candidate_parallel].include?(mode)
  rust_output = File.expand_path(ENV.fetch('RUST_AUXILIARY_OUTPUT_FILE'))
  raise 'Rust output must be under benchmarks' unless File.dirname(rust_output) == benchmarks
  raise 'invalid Rust output name' unless File.basename(rust_output).match?(/\Apui_tree_rust_[A-Za-z0-9_]+\.json\z/)
  raise 'Rust output exists' if File.exist?(rust_output)
  rust_env = {
    'ASPACE_URL' => backend,
    'ASPACE_USER' => ENV.fetch('ASPACE_ADMIN_USER'),
    'ASPACE_PASSWORD' => ENV.fetch('ASPACE_ADMIN_PASSWORD')
  }
  rust_args = ['/archivesspace/rust-tools/target/release/aspace-cli', 'pui-tree-spike',
               '--resource-uri', resource_uri, '--output', rust_output]
  rust_args = ['/bin/false'] if mode == 'candidate_parallel' && ENV['RUST_SPIKE_INJECT_FAILURE'] == '1'
  if mode == 'candidate_parallel'
    rust_started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    rust_job = Open3.popen3(rust_env, *rust_args)
    rust_job[0].close
    at_exit do
      next unless rust_job[3].alive?
      begin
        Process.kill('TERM', rust_job[3].pid)
      rescue Errno::ESRCH
      end
      rust_job[3].join(2)
    end
  end
end
resource_id = resource_uri.split('/').last.to_i
resource_diagnostic = {}
([[:resource, [resource_id]]] + ao_ids.each_slice(100).map { |slice| [:archival_object, slice] }).each do |type, ids|
  records = indexer.fetch_records(type, ids, indexer.resolved_attributes)
  raise 'incomplete fetch' unless records.length == ids.length
  input_records = records.map { |record| { 'record' => record.to_hash(:trusted), 'uri' => record.uri } }
  if type == :archival_object && sample_input_file
    sample = input_records.first
    File.open(sample_input_file, File::WRONLY | File::CREAT | File::EXCL, 0600) { |file| file.write(JSON.generate(sample)) }
    sample_input_file = nil
  end
  if type == :resource
    resource_diagnostic = {
      record_type_allowed: indexer.record_types.include?(:resource),
      record_types: indexer.record_types.map(&:to_s),
      inheritance_resource: RecordInheritance.has_type?('resource'),
      resource_uri: input_records.first['uri'],
      publish: input_records.first['record']['publish'],
      skip: indexer.skip_index_record?(input_records.first)
    }
  end
  indexer.index_records(input_records)
end
ordinary_done = Process.clock_gettime(Process::CLOCK_MONOTONIC)
ordinary_count = documents.length
delete_candidates = []
if mode == 'native'
  delete_candidates = add_native_auxiliary(indexer, resource_uri, documents)
elsif %w[candidate candidate_parallel].include?(mode)
  if mode == 'candidate_parallel'
    _stdout = rust_job[1].read
    stderr = rust_job[2].read
    status = rust_job[3].value
  else
    rust_started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    _stdout, stderr, status = Open3.capture3(rust_env, *rust_args)
  end
  rust_process_elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - rust_started
  begin
    raise "Rust auxiliary candidate failed: #{stderr.lines.last}" unless status.success?
    raise 'Rust auxiliary candidate output missing' unless File.file?(rust_output)
    rust_report = JSON.parse(File.read(rust_output))
    raise 'Rust candidate resource URI mismatch' unless rust_report.fetch('resource_uri') == resource_uri
    aux_documents = rust_report.fetch('documents')
    aux_deletes = rust_report.fetch('delete_candidates')
    raise 'Rust candidate count mismatch' unless aux_documents.is_a?(Array) &&
      rust_report.fetch('document_count') == aux_documents.length
    raise 'malformed Rust auxiliary document' unless aux_documents.all? { |doc| doc.is_a?(Hash) && doc['id'].is_a?(String) &&
      doc.keys.none? { |key| key.start_with?('_archivalist_') } }
    raise 'malformed Rust delete candidates' unless aux_deletes.is_a?(Array) && aux_deletes.all? { |uri| uri.is_a?(String) }
    rust_internal_seconds = rust_report.fetch('elapsed_seconds')
    raise 'malformed Rust elapsed time' unless rust_internal_seconds.is_a?(Numeric) && rust_internal_seconds.finite?
    existing_ids = documents.map { |doc| doc.fetch('id') }
    auxiliary_ids = aux_documents.map { |doc| doc.fetch('id') }
    raise 'duplicate Rust auxiliary document ID' unless (existing_ids + auxiliary_ids).uniq.length == existing_ids.length + auxiliary_ids.length
    documents.concat(aux_documents)
    delete_candidates = aux_deletes
  rescue StandardError => error
    raise unless mode == 'candidate_parallel'
    fallback_used = true
    fallback_reason = error.class.to_s
    delete_candidates = add_native_auxiliary(indexer, resource_uri, documents)
  end
end
elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - started
raise 'duplicate ordinary document IDs' unless documents.map { |doc| doc.fetch('id') }.uniq.length == documents.length
raise 'unexpected private marker' if documents.any? { |doc| doc.keys.any? { |key| key.start_with?('_archivalist_') } }
report = { resource_uri: resource_uri, mode: mode, resource_diagnostic: resource_diagnostic,
           ordered_uri_count: uris.length, elapsed_seconds: elapsed,
           fallback_used: fallback_used, fallback_reason: fallback_reason,
           ordinary_seconds: ordinary_done - started, auxiliary_seconds: elapsed - (ordinary_done - started),
           rust_observed_until_collection_seconds: rust_process_elapsed,
           rust_internal_seconds: rust_internal_seconds,
           ordinary_document_count: ordinary_count, auxiliary_document_count: documents.length - ordinary_count,
           document_count: documents.length, delete_candidates: delete_candidates, documents: documents }
File.open(output, File::WRONLY | File::CREAT | File::EXCL, 0600) do |file|
  file.write(JSON.generate(report))
  file.flush
  file.fsync
end
puts JSON.generate(report.reject { |key, _| [:documents, :delete_candidates].include?(key) }.merge(delete_candidate_count: delete_candidates.length))
