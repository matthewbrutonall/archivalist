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
require 'fileutils'
require_relative 'integration_stage_harness'

backend = ENV.fetch('BACKEND_URL')
resource_uri = ENV.fetch('RESOURCE_URI')
output = File.expand_path(ENV.fetch('OUTPUT_FILE'))
raise 'unapproved backend URL' unless ArchivalistHarnessSafety.valid_backend_url?(backend)
raise 'invalid resource URI' unless resource_uri.match?(%r{\A/repositories/([1-9]\d*)/resources/([1-9]\d*)\z})
benchmarks = File.expand_path('../benchmarks', __dir__)
raise 'output must be under benchmarks' unless File.dirname(output) == benchmarks
raise 'invalid output file name' unless File.basename(output).match?(/\Apui_tree_reference_[A-Za-z0-9_]+\.json\z/)
raise 'output exists' if File.exist?(output)

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

indexer = PUIIndexer.get_indexer(nil, 'PUI resource tree spike reference')
indexer.login
JSONModel.set_repository(resource_uri.split('/')[2].to_i)

started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
documents = []
indexer.send(:add_infscroll_docs, [resource_uri], documents)
tree = LargeTreeDocIndexer.new(documents)
tree.add_largetree_docs([resource_uri])
elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - started

raise 'duplicate auxiliary document IDs' unless documents.map { |doc| doc.fetch('id') }.uniq.length == documents.length
raise 'unexpected private marker' if documents.any? { |doc| doc.keys.any? { |key| key.start_with?('_archivalist_') } }

report = {
  resource_uri: resource_uri,
  elapsed_seconds: elapsed,
  document_count: documents.length,
  delete_candidates: tree.deletes,
  documents: documents
}
File.open(output, File::WRONLY | File::CREAT | File::EXCL, 0600) do |file|
  file.write(JSON.generate(report))
  file.flush
  file.fsync
end
puts JSON.generate(report.reject { |key, _| [:documents, :delete_candidates].include?(key) }.merge(delete_candidate_count: tree.deletes.length))
