# frozen_string_literal: true

# Bounded Native Pre-Solr Document Capture for STAFF Archival Objects
#
# Uses actual PeriodicIndexer, index_records, IndexBatch with all ordinary hooks,
# sanitation, and clean_whitespace, but intercepts immediately before Solr update
# so exact outgoing document JSON is captured without Solr writes.
#
# Does NOT alter any production code.

require 'rubygems'
if defined?(JRUBY_VERSION) && File.directory?('/archivesspace/build/gems/jruby/3.1.0')
  ENV['GEM_HOME'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  ENV['GEM_PATH'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  Gem.paths = {
    'GEM_HOME' => '/archivesspace/build/gems/jruby/3.1.0',
    'GEM_PATH' => '/archivesspace/build/gems/jruby/3.1.0'
  }
end

$LOAD_PATH.unshift('/archivesspace/common')
$LOAD_PATH.unshift('/archivesspace/indexer')
$LOAD_PATH.unshift('/archivesspace/indexer/app/lib')

require 'active_support/core_ext/string/inflections'
require 'config/config-distribution'
require 'asutils'
require 'ashttp'
require 'jsonmodel'
require 'jsonmodel_type'
require 'indexer_common_config'
require 'indexer_common'
require 'pui_indexer'
require 'periodic_indexer'
require 'index_batch'
require 'json'
require 'net/http'
require 'optparse'

options = {
  :repo_id => 4,
  :batch_size => 25,
  :backend_url => nil,
  :session => nil,
  :ids => nil,
  :ids_file => nil,
  :all => false,
  :output => nil
}

OptionParser.new do |opts|
  opts.banner = 'Usage: jruby capture_native_pre_solr_ao.rb [options]'
  opts.on('--repo-id ID', Integer, 'Repository ID (default: 4)') { |v| options[:repo_id] = v }
  opts.on('--batch-size SIZE', Integer, 'Batch size (default: 25)') { |v| options[:batch_size] = v }
  opts.on('--backend-url URL', String, 'Backend URL') { |v| options[:backend_url] = v }
  opts.on('--session TOKEN', String, 'Session token') { |v| options[:session] = v }
  opts.on('--ids IDS', String, 'Comma-separated AO IDs') { |v| options[:ids] = v.split(',').map(&:strip).map(&:to_i) }
  opts.on('--ids-file PATH', String, 'Path to file containing IDs') { |v| options[:ids_file] = v }
  opts.on('--all', 'Process all repository AO IDs') { options[:all] = true }
  opts.on('--output PATH', String, 'Path to output JSON file (mode 0600)') { |v| options[:output] = v }
end.parse!

raise ArgumentError, 'Missing --output path' unless options[:output]
raise ArgumentError, 'Missing --backend-url' unless options[:backend_url]

backend_url = options[:backend_url]
repo_id = options[:repo_id]

# Force all Rust acceleration flags off so native logic is exercised cleanly
AppConfig.reload
AppConfig[:backend_url] = backend_url
AppConfig[:rust_index_document_builder_enabled] = false
AppConfig[:rust_index_document_enrichment_enabled] = false
AppConfig[:rust_pui_availability_preparation_enabled] = false
AppConfig[:rust_pui_document_preparation_enabled] = false
AppConfig[:rust_pui_common_metadata_enabled] = false
AppConfig[:rust_pui_record_inheritance_candidate_enabled] = false
AppConfig[:rust_pui_auxiliary_documents_enabled] = false

JSONModel::init(:client_mode => true, :url => backend_url)
if options[:session]
  JSONModel::HTTP.current_backend_session = options[:session]
end
JSONModel.set_repository(repo_id)

class PreSolrInterceptingIndexer < PeriodicIndexer
  attr_reader :captured_docs, :solr_requests_intercepted

  def initialize(backend_url, state, name, verbose)
    super(backend_url, state, name, verbose)
    @captured_docs = []
    @solr_requests_intercepted = 0
  end

  def do_http_request(url, req)
    @solr_requests_intercepted += 1
    if req.is_a?(Net::HTTP::Post) && req.body_stream
      # Exact outgoing JSON stream for documents from batch.to_json_stream
      content = req.body_stream.read
      parsed = ASUtils.json_parse(content)
      if parsed.is_a?(Array)
        @captured_docs.concat(parsed)
      elsif parsed.is_a?(Hash)
        @captured_docs << parsed
      end
      res = Net::HTTPOK.new('1.1', '200', 'OK')
      def res.body; '{"responseHeader":{"status":0}}'; end
      return res
    elsif req.is_a?(Net::HTTP::Post)
      # e.g. child delete query or commit
      res = Net::HTTPOK.new('1.1', '200', 'OK')
      def res.body; '{"responseHeader":{"status":0}}'; end
      return res
    end
    super
  end
end

indexer = PreSolrInterceptingIndexer.new(backend_url, nil, 'PreSolrInterceptingIndexer', false)
unless options[:session]
  indexer.login
end

ids = if options[:all]
  JSONModel(:archival_object).all(:all_ids => true, :modified_since => 0) || []
elsif options[:ids_file]
  content = File.read(options[:ids_file]).strip
  if content.start_with?('[')
    JSON.parse(content).map(&:to_i)
  else
    content.split(/[\s,]+/).reject(&:empty?).map(&:to_i)
  end
elsif options[:ids]
  options[:ids]
else
  raise ArgumentError, 'Must specify --ids, --ids-file, or --all'
end

ids = Array(ids).map(&:to_i).uniq

total_ids = ids.length
batch_idx = 0
ids.each_slice(options[:batch_size]) do |id_subset|
  records = indexer.fetch_records(:archival_object, id_subset, indexer.resolved_attributes)
  next if records.empty?

  prepared = records.map do |r|
    {
      'record' => r.to_hash(:trusted),
      'uri' => r.uri
    }
  end
  indexer.index_records(prepared)
  batch_idx += 1
  if batch_idx % 100 == 0 || indexer.captured_docs.length == total_ids
    $stderr.puts "Captured #{indexer.captured_docs.length}/#{total_ids} native pre-Solr docs..."
  end
end

# Write captured docs atomically mode 0600
tmp_out = "#{options[:output]}.tmp_#{Process.pid}_#{Time.now.to_i}"
File.open(tmp_out, 'w', 0600) do |f|
  f.chmod(0600)
  f.write(JSON.generate({
    'count' => indexer.captured_docs.length,
    'repo_id' => repo_id,
    'solr_requests_intercepted' => indexer.solr_requests_intercepted,
    'documents' => indexer.captured_docs
  }))
end
File.rename(tmp_out, options[:output])
File.chmod(0600, options[:output])
puts "Successfully captured #{indexer.captured_docs.length} native pre-Solr documents to #{options[:output]}"
