#!/usr/bin/env ruby
# frozen_string_literal: true

# Offline JRuby runtime integration test for ArchivalistPuiFetchAudit.
# Proves that the opt-in audit hooks work with the REAL JRuby JSONModel client,
# actual JSONModel(:archival_object).all execution, and real PUIIndexer method resolution,
# with strictly in-process stubbed responses and NO network or Solr access.
#
# Run inside app container:
#   docker exec archivesspaceai-app-1 /archivesspace/scripts/jruby /archivesspace/rust-tools/scripts/test_archivalist_pui_fetch_audit_jruby.rb

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

unless defined?(JRUBY_VERSION)
  class TestArchivalistPuiFetchAuditJRuby < Minitest::Test
    def test_jruby_environment_required
      skip 'Offline JRuby integration test requires JRuby runtime inside archivesspaceai-app-1 container'
    end
  end
  # Exit early cleanly when executed under MRI
  return
end

require 'socket'
require 'net/http'
require 'uri'
require 'json'

# 1. Strict safety guard: prevent any physical network socket opening.
class TCPSocket
  def self.open(*args)
    raise "Safety violation: Network call attempted in offline test: TCPSocket.open(#{args.inspect})"
  end

  def initialize(*args)
    raise "Safety violation: Network call attempted in offline test: TCPSocket.new(#{args.inspect})"
  end
end

class Net::HTTP
  def connect
    raise 'Safety violation: Net::HTTP#connect attempted in offline test'
  end
end

# 2. Load actual ArchivesSpace libraries into JRuby environment.
$LOAD_PATH.unshift('/archivesspace/common')
$LOAD_PATH.unshift('/archivesspace/indexer')
$LOAD_PATH.unshift('/archivesspace/indexer/app/lib')

require 'active_support/core_ext/string/inflections'
require 'config/config-distribution'
AppConfig.reload
AppConfig[:backend_url] = 'http://stubbed-backend.invalid:9999'
AppConfig[:solr_url] = 'http://stubbed-solr.invalid:9999'

require 'asutils'
require 'jsonmodel'
require 'jsonmodel_client'
require 'index_state'
require 'aspace_i18n'
require 'ashttp'
require 'record_inheritance'
require 'rust_index_document_builder'
require 'periodic_indexer'
require 'pui_indexer'
require 'large_tree_doc_indexer'
require 'index_batch'

include JSONModel

# 3. Initialize JSONModel offline with dummy enum source.
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
JSONModel.set_repository(2)

# 4. In-process stub router for JSONModel::HTTP.get_response.
module OfflineJRubyHttpStub
  @handler = nil
  @calls = []

  class << self
    attr_accessor :handler
    attr_reader :calls

    def reset!
      @handler = nil
      @calls = []
    end

    def make_response(code_str, body_str)
      cls = (code_str.to_s == '200') ? Net::HTTPOK : Net::HTTPInternalServerError
      res = cls.new('1.1', code_str.to_s, (code_str.to_s == '200') ? 'OK' : 'Error')
      res.instance_variable_set(:@read, true)
      res.define_singleton_method(:body) { body_str }
      res
    end
  end
end

class << JSONModel::HTTP
  def get_response(url, *rest, &blk)
    OfflineJRubyHttpStub.calls << url
    handler = OfflineJRubyHttpStub.handler
    raise "Safety violation: Unhandled offline HTTP request to #{url.inspect}" unless handler

    handler.call(url)
  end
end

# 5. Load and install ArchivalistPuiFetchAudit on real classes.
require_relative 'archivalist_pui_fetch_audit'

REAL_HOOKS = ArchivalistPuiFetchAudit.install!(
  pui_indexer: PUIIndexer,
  jsonmodel_http: JSONModel::HTTP
)

class TestArchivalistPuiFetchAuditJRuby < Minitest::Test
  def setup
    ArchivalistPuiFetchAudit::LOCK.synchronize do
      ArchivalistPuiFetchAudit::STATE[:active] = false
      ArchivalistPuiFetchAudit::STATE[:repositories] = {}
    end
    Thread.current[:archivalist_pui_fetch_audit_phases] = nil
    Thread.current[:archivalist_pui_fetch_audit_in_get_response] = nil
    OfflineJRubyHttpStub.reset!
  end

  def teardown
    ArchivalistPuiFetchAudit::LOCK.synchronize do
      ArchivalistPuiFetchAudit::STATE[:active] = false
      ArchivalistPuiFetchAudit::STATE[:repositories] = {}
    end
    Thread.current[:archivalist_pui_fetch_audit_phases] = nil
    Thread.current[:archivalist_pui_fetch_audit_in_get_response] = nil
    OfflineJRubyHttpStub.reset!
  end

  def build_dummy_indexer
    dummy_state = Object.new
    def dummy_state.get_last_mtime(*_args); 0; end
    def dummy_state.set_last_mtime(*_args); end

    PUIIndexer.new('http://stubbed-backend.invalid:9999', dummy_state, 'Offline Test PUI Indexer')
  end

  def test_real_class_hook_bindings_and_no_instrumentation_when_inactive
    assert PUIIndexer.ancestors.include?(ArchivalistPuiFetchAudit::PuiIndexerHook)
    assert JSONModel::HTTP.singleton_class.ancestors.include?(ArchivalistPuiFetchAudit::JsonModelHttpHook)
    assert_equal %w[fetch_records index_round_complete], REAL_HOOKS['PUIIndexer']
    assert_equal %w[get_response], REAL_HOOKS['JSONModel::HTTP']

    indexer = build_dummy_indexer
    assert_instance_of PUIIndexer, indexer

    OfflineJRubyHttpStub.handler = lambda do |url|
      OfflineJRubyHttpStub.make_response('200', '[]')
    end

    refute ArchivalistPuiFetchAudit.active?
    records = indexer.fetch_records(:archival_object, [10, 11], ['subjects'])
    assert_equal [], records
    assert_empty ArchivalistPuiFetchAudit::STATE[:repositories]
  end

  def test_real_jsonmodel_all_and_pui_fetch_records_attribution_and_byte_count
    indexer = build_dummy_indexer
    captured_urls = []

    body = JSON.generate([
      { 'uri' => '/repositories/2/archival_objects/101', 'title' => 'Record 101' },
      { 'uri' => '/repositories/2/archival_objects/102', 'title' => 'Record 102' }
    ])
    expected_bytes = body.bytesize

    OfflineJRubyHttpStub.handler = lambda do |url|
      captured_urls << url
      OfflineJRubyHttpStub.make_response('200', body)
    end

    records, summary = ArchivalistPuiFetchAudit.capture do
      indexer.fetch_records(:archival_object, [101, 102], ['subjects', 'ancestors'])
    end

    # 1. Real method execution & object instantiation
    assert_equal 2, records.length
    assert records.all? { |r| r.is_a?(JSONModel(:archival_object)) }
    assert_equal '/repositories/2/archival_objects/101', records[0].uri
    assert_equal '/repositories/2/archival_objects/102', records[1].uri

    # 2. Exact URL generated by real JSONModel(:archival_object).all
    assert_equal 1, captured_urls.length
    request_uri = captured_urls.first
    assert_instance_of URI::HTTP, request_uri
    assert_equal '/repositories/2/archival_objects', request_uri.path
    query_params = URI.decode_www_form(request_uri.query)
    assert_equal [['id_set', '101,102'], ['resolve[]', 'subjects'], ['resolve[]', 'ancestors']], query_params

    # 3. Exact phase attribution and response byte count
    ord = summary.dig('phases', 'ordinary_indexing')
    tree = summary.dig('phases', 'tree_discovery')
    ov = summary['overlap']

    assert_equal 1, ord['calls']
    assert_equal 1, ord['successful_calls']
    assert_equal 0, ord['failed_calls']
    assert_equal 2, ord['requested_ids_total']
    assert_equal 2, ord['successful_requested_ids_total']
    assert_equal 2, ord['unique_successful_requested_ids']
    assert_equal 0, ord['repeated_successful_requested_ids']
    assert_equal expected_bytes, ord['response_bytes']
    assert_equal expected_bytes, ord['successful_response_bytes']
    assert_operator ord['call_duration_sum_ms'], :>=, 0.0

    assert_equal 0, tree['calls']
    assert_equal 0, tree['response_bytes']

    assert_equal 2, ov['ordinary_unique_successful_requested_ids']
    assert_equal 0, ov['tree_discovery_unique_successful_requested_ids']
    assert_equal 0, ov['intersection_successful_requested_ids']
    assert_equal 2, ov['union_unique_successful_requested_ids']

    repo_summary = summary.dig('by_repository', '2')
    assert repo_summary
    assert_equal 2, repo_summary['repository_id']
    assert_equal expected_bytes, repo_summary.dig('phases', 'ordinary_indexing', 'response_bytes')
  end

  def test_tree_discovery_context_and_cross_phase_overlap_without_indexing
    indexer = build_dummy_indexer

    # Prevent downstream Solr/tree indexing execution on this test instance
    indexer.define_singleton_method(:add_infscroll_docs) { |*_args| }
    indexer.define_singleton_method(:handle_deletes) { |*_args| }
    indexer.define_singleton_method(:index_batch) { |*_args| }
    indexer.define_singleton_method(:send_commit) { |*_args| }

    # Stub LargeTreeDocIndexer so no tree walk or Solr document creation occurs
    dummy_tree_doc_indexer = Object.new
    def dummy_tree_doc_indexer.add_largetree_docs(*_args); end
    def dummy_tree_doc_indexer.deletes; []; end
    LargeTreeDocIndexer.stub(:new, dummy_tree_doc_indexer) do
      ord_body = JSON.generate([
        { 'uri' => '/repositories/2/archival_objects/101', 'resource' => { 'ref' => '/repositories/2/resources/999' } },
        { 'uri' => '/repositories/2/archival_objects/102', 'resource' => { 'ref' => '/repositories/2/resources/999' } }
      ])
      tree_body = JSON.generate([
        { 'uri' => '/repositories/2/archival_objects/102', 'resource' => { 'ref' => '/repositories/2/resources/999' } },
        { 'uri' => '/repositories/2/archival_objects/201', 'resource' => { 'ref' => '/repositories/2/resources/999' } }
      ])

      OfflineJRubyHttpStub.handler = lambda do |url|
        uri = URI(url.to_s)
        path = uri.path.to_s
        query = uri.query.to_s

        if path == '/repositories/2/archival_objects' && query.include?('id_set=101%2C102')
          OfflineJRubyHttpStub.make_response('200', ord_body)
        elsif path == '/repositories/2/archival_objects' && query.include?('id_set=102%2C201')
          OfflineJRubyHttpStub.make_response('200', tree_body)
        elsif query.include?('all_ids=true') && path == '/repositories/2/archival_objects'
          # Tree discovery queries all_ids for archival_objects: return [102, 201]
          OfflineJRubyHttpStub.make_response('200', '[102, 201]')
        elsif query.include?('all_ids=true')
          OfflineJRubyHttpStub.make_response('200', '[]')
        elsif path == '/delete-feed'
          OfflineJRubyHttpStub.make_response('200', '{"results":[],"last_page":1}')
        else
          raise "Safety violation: Unexpected URL in offline test: #{url}"
        end
      end

      repo = Struct.new(:id, :repo_code).new(2, 'TEST')

      _, summary = ArchivalistPuiFetchAudit.capture do
        # 1. Ordinary indexing pass (fetches 101, 102)
        indexer.fetch_records(:archival_object, [101, 102], ['subjects'])

        # 2. Tree discovery pass via real PUIIndexer#index_round_complete (fetches 102, 201)
        indexer.index_round_complete(repo)
      end

      ord = summary.dig('phases', 'ordinary_indexing')
      tree = summary.dig('phases', 'tree_discovery')
      ov = summary['overlap']

      # Ordinary phase accounting
      assert_equal 1, ord['calls']
      assert_equal 2, ord['unique_successful_requested_ids']
      assert_equal ord_body.bytesize, ord['response_bytes']

      # Tree discovery phase accounting (all_ids was ignored; id_set was counted)
      assert_equal 1, tree['calls']
      assert_equal 2, tree['unique_successful_requested_ids']
      assert_equal tree_body.bytesize, tree['response_bytes']

      # Cross-phase overlap accounting:
      # ordinary = {101, 102}, tree = {102, 201}
      # intersection = {102} (1), union = {101, 102, 201} (3)
      assert_equal 2, ov['ordinary_unique_successful_requested_ids']
      assert_equal 2, ov['tree_discovery_unique_successful_requested_ids']
      assert_equal 1, ov['intersection_successful_requested_ids']
      assert_equal 3, ov['union_unique_successful_requested_ids']
      assert_equal 1, ov['ordinary_only_successful_requested_ids']
      assert_equal 1, ov['tree_discovery_only_successful_requested_ids']
      assert_in_delta 0.5, ov['tree_discovery_overlap_ratio'], 0.0001
      assert_in_delta 0.5, ov['ordinary_overlap_ratio'], 0.0001
    end
  end

  def test_real_jsonmodel_error_response_attribution_and_isolation
    indexer = build_dummy_indexer

    OfflineJRubyHttpStub.handler = lambda do |_url|
      OfflineJRubyHttpStub.make_response('500', 'Internal Server Error')
    end

    ArchivalistPuiFetchAudit.start!
    begin
      assert_raises(RuntimeError) do
        indexer.fetch_records(:archival_object, [901, 902], [])
      end
    ensure
      summary = ArchivalistPuiFetchAudit.stop!
    end

    ord = summary.dig('phases', 'ordinary_indexing')
    assert_equal 1, ord['calls']
    assert_equal 0, ord['successful_calls']
    assert_equal 1, ord['failed_calls']
    assert_equal 2, ord['requested_ids_total']
    assert_equal 0, ord['successful_requested_ids_total']
    assert_equal 2, ord['failed_requested_ids_total']
    assert_equal 0, ord['unique_successful_requested_ids']
    assert_equal 2, ord['unique_failed_requested_ids']
    assert_equal 0, ord['successful_response_bytes']

    ov = summary['overlap']
    assert_equal 0, ov['ordinary_unique_successful_requested_ids']
    assert_equal 0, ov['intersection_successful_requested_ids']

    repo_summary = summary.dig('by_repository', '2')
    assert repo_summary
    repo_ord = repo_summary.dig('phases', 'ordinary_indexing')
    assert_equal 1, repo_ord['calls']
    assert_equal 1, repo_ord['failed_calls']
    assert_equal 2, repo_ord['unique_failed_requested_ids']
  end

  def test_strict_offline_safety_and_fail_closed_on_unhandled_urls
    indexer = build_dummy_indexer

    # No handler registered: must fail closed before any network access
    err = assert_raises(RuntimeError) do
      indexer.fetch_records(:archival_object, [555], [])
    end
    assert_match(/Unhandled offline HTTP request/, err.message)

    # Direct network socket attempt must fail closed
    socket_err = assert_raises(RuntimeError) do
      TCPSocket.open('127.0.0.1', 4567)
    end
    assert_match(/Network call attempted in offline test/, socket_err.message)
  end

  def test_pui_tree_discovery_batch_size_resolution_and_identical_ordering
    # 1. Test batch size resolution in JRuby
    assert_equal 25, PUIIndexer.resolve_tree_discovery_batch_size(nil, 25)
    assert_equal 25, PUIIndexer.resolve_tree_discovery_batch_size(0, 25)
    assert_equal 25, PUIIndexer.resolve_tree_discovery_batch_size(-5, 25)
    assert_equal 25, PUIIndexer.resolve_tree_discovery_batch_size('bad', 25)
    assert_equal 25, PUIIndexer.resolve_tree_discovery_batch_size('0', 25)
    assert_equal 50, PUIIndexer.resolve_tree_discovery_batch_size(50, 25)
    assert_equal 200, PUIIndexer.resolve_tree_discovery_batch_size(200, 25)
    assert_equal 200, PUIIndexer.resolve_tree_discovery_batch_size(250, 25)
    assert_equal 200, PUIIndexer.resolve_tree_discovery_batch_size('500', 25)

    # 2. Test index_round_complete batching and identical ordering across batch sizes
    run_simulation = lambda do |batch_override|
      captured_batches = []
      captured_tree_uris = nil

      indexer = build_dummy_indexer
      indexer.define_singleton_method(:tree_discovery_batch_size) { batch_override }
      indexer.define_singleton_method(:add_infscroll_docs) { |*_args| }
      indexer.define_singleton_method(:index_batch) { |*_args| }
      indexer.define_singleton_method(:send_commit) {}
      indexer.define_singleton_method(:delete_records) { |*_args| }
      indexer.define_singleton_method(:handle_deletes) { |*_args| }
      indexer.define_singleton_method(:log) { |*_args| }

      mock_tree_indexer = Object.new
      mock_tree_indexer.define_singleton_method(:deletes) { [] }
      mock_tree_indexer.define_singleton_method(:add_largetree_docs) { |uris| captured_tree_uris = uris }

      # In-process mock handler for OfflineJRubyHttpStub
      OfflineJRubyHttpStub.handler = lambda do |url|
        u = url.to_s
        if u.include?('/archival_objects?all_ids=true')
          OfflineJRubyHttpStub.make_response('200', JSON.generate((1..100).to_a))
        elsif u.include?('/archival_objects?id_set=')
          id_set_raw = u.split('id_set=')[1].split('&')[0]
          id_set_decoded = URI.decode_www_form_component(id_set_raw)
          ids = id_set_decoded.split(',').map(&:to_i)
          captured_batches << ids
          records = ids.map do |ao_id|
            root_id = if ao_id <= 40
                        10
                      elsif ao_id <= 80
                        20
                      else
                        30
                      end
            {
              'uri' => "/repositories/2/archival_objects/#{ao_id}",
              'resource' => { 'ref' => "/repositories/2/resources/#{root_id}" }
            }
          end
          OfflineJRubyHttpStub.make_response('200', JSON.generate(records))
        else
          OfflineJRubyHttpStub.make_response('200', '[]')
        end
      end

      orig_new = LargeTreeDocIndexer.method(:new)
      LargeTreeDocIndexer.define_singleton_method(:new) { |*_args| mock_tree_indexer }
      repo = Struct.new(:id, :repo_code).new(2, 'test_repo')
      begin
        indexer.index_round_complete(repo)
      ensure
        LargeTreeDocIndexer.define_singleton_method(:new, orig_new)
      end

      [captured_batches, captured_tree_uris]
    end

    b25, uris25 = run_simulation.call(25)
    b50, uris50 = run_simulation.call(50)
    b100, uris100 = run_simulation.call(100)

    assert_equal 4, b25.length
    assert_equal [25, 25, 25, 25], b25.map(&:length)
    assert_equal (1..100).to_a, b25.flatten

    assert_equal 2, b50.length
    assert_equal [50, 50], b50.map(&:length)
    assert_equal (1..100).to_a, b50.flatten

    assert_equal 1, b100.length
    assert_equal [100], b100.map(&:length)
    assert_equal (1..100).to_a, b100.flatten

    expected_uris = [
      '/repositories/2/resources/10',
      '/repositories/2/resources/20',
      '/repositories/2/resources/30'
    ]
    assert_equal expected_uris, uris25
    assert_equal expected_uris, uris50
    assert_equal expected_uris, uris100
  end

  def test_pui_tree_discovery_batch_size_captures_records_per_thread_when_nil_override
    orig_override = AppConfig[:pui_tree_discovery_batch_size] rescue nil
    orig_records = AppConfig[:pui_indexer_records_per_thread] rescue nil
    begin
      AppConfig[:pui_indexer_records_per_thread] = 25
      AppConfig[:pui_tree_discovery_batch_size] = nil
      indexer = build_dummy_indexer
      assert_equal 25, indexer.instance_variable_get(:@records_per_thread)

      # Mutate AppConfig after instance construction
      AppConfig[:pui_indexer_records_per_thread] = 99

      # tree_discovery_batch_size must use the instance's captured @records_per_thread exactly
      assert_equal 25, indexer.tree_discovery_batch_size

      # With explicit valid override, explicit override is used
      AppConfig[:pui_tree_discovery_batch_size] = 150
      assert_equal 150, indexer.tree_discovery_batch_size

      # With explicit clamped override, 200 is used
      AppConfig[:pui_tree_discovery_batch_size] = 300
      assert_equal 200, indexer.tree_discovery_batch_size

      # With invalid override, fails closed to captured @records_per_thread
      AppConfig[:pui_tree_discovery_batch_size] = 'bad'
      assert_equal 25, indexer.tree_discovery_batch_size
    ensure
      AppConfig[:pui_tree_discovery_batch_size] = orig_override
      AppConfig[:pui_indexer_records_per_thread] = orig_records
    end
  end
end
