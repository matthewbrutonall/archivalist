#!/usr/bin/env ruby
# frozen_string_literal: true

# Offline MRI Ruby verification harness for PUI common search metadata preparation.
# Exercises:
#   - Actual saved RustIndexDocumentBuilder validation & validate_json_equal!
#   - Actual saved PUIIndexer helper overrides & apply_rust_pui_common_helper
#   - Actual saved IndexerCommon#index_records & IndexBatch serialization/capture
#
# Standard library only (MRI Ruby 3.2+).
#
# Honest limitations:
#   - Full JRuby JVM, Bundler, ArchivesSpace MySQL database, and live Solr service
#     are not running in this offline environment.
#   - Minimal shims provide Java atomic/list interop, logger, and JSONModel startup.
#   - External HTTP transport (perform_request) is stubbed in-memory to test sidecar
#     JSON payloads and outage injection. The actual validators and helper methods
#     under test are loaded directly from repository sources without duplication.

require 'json'
require 'tempfile'
require 'date'
require 'time'
require 'set'
require 'net/http'

# --- 1. Minimal shims for MRI Ruby without JRuby/ActiveSupport ---

class String
  def constantize
    Object.const_get(self)
  end
end

module Java
  module Util
    class ArrayList < Array
      def add(item)
        self << item
        true
      end
    end

    class Collections
      def self.synchronizedList(list)
        list
      end
    end

    module Concurrent
      class Semaphore
        def initialize(_permits = 1); end
      end

      module TimeUnit
        MILLISECONDS = :milliseconds
      end

      module Atomic
        class AtomicLong
          def initialize(val = 0)
            @val = val
          end

          def get
            @val
          end

          def set(val)
            @val = val
          end

          def addAndGet(delta)
            @val += delta
          end

          def compareAndSet(expected, update)
            if @val == expected
              @val = update
              true
            else
              false
            end
          end
        end

        def self.AtomicLong
          AtomicLong
        end
      end

      def self.atomic; Atomic; end
      def self.TimeUnit; TimeUnit; end
      def self.Semaphore; Semaphore; end
    end

    def self.concurrent; Concurrent; end
    def self.Collections; Collections; end
    def self.ArrayList; ArrayList; end
  end

  module Lang
    class System
      def self.currentTimeMillis
        (Time.now.to_f * 1000).to_i
      end

      def self.get_property(_p)
        nil
      end
    end
  end

  def self.util; Util; end
  def self.lang; Lang; end
end

module Kernel
  def java
    Java
  end
end

class Log
  def self.info(_msg); end
  def self.warn(_msg); end
  def self.error(_msg); end
  def self.debug(_msg); end
end

module ASUtils
  def self.to_json(obj, _opts = {})
    JSON.generate(obj)
  end

  def self.json_parse(s)
    JSON.parse(s)
  end

  def self.as_array(t)
    t.nil? ? [] : (t.is_a?(Array) ? t : [t])
  end

  def self.wrap(t)
    as_array(t)
  end

  def self.tempfile(b)
    Tempfile.new("#{b}_#{Java::Lang::System.currentTimeMillis}")
  end

  def self.find_local_directories(_name)
    []
  end

  def self.search_nested(elt, keys, ignore_keys = [], &block)
    if elt.respond_to?(:key?)
      keys.each do |key|
        if elt.key?(key)
          block.call(key, elt.fetch(key))
        end
      end

      elt.each do |next_key, value|
        unless ignore_keys.include?(next_key)
          search_nested(value, keys, ignore_keys, &block)
        end
      end
    elsif elt.respond_to?(:each)
      elt.each do |value|
        search_nested(value, keys, ignore_keys, &block)
      end
    end
  end
end

AppConfig = {
  :index_state_class => 'DummyIndexState',
  :record_inheritance => {
    :archival_object => { :inherited_fields => [] },
    :resource => {
      :inherited_fields => [
        { :property => 'title', :inherit_directly => true }
      ]
    }
  },
  :record_inheritance_resolves => [],
  :pui_indexing_frequency_seconds => 1,
  :pui_indexer_thread_count => 1,
  :pui_indexer_records_per_thread => 1,
  :backend_url => 'http://localhost:8089',
  :rust_index_document_builder_enabled => true,
  :rust_pui_common_metadata_enabled => true,
  :rust_pui_document_preparation_enabled => false,
  :rust_pui_record_inheritance_candidate_enabled => false,
}

module I18n
  def self.t(k, opts = {})
    opts[:default] || k
  end
end

TYPE_MAP = {
  'resources' => 'resource',
  'archival_objects' => 'archival_object',
  'accessions' => 'accession',
  'digital_objects' => 'digital_object',
  'subjects' => 'subject'
}.freeze

module JSONModel
  def self.parse_reference(uri, _opts = {})
    return nil if uri.nil?
    if uri =~ %r{/repositories/(\d+)/([^/]+)/(\d+)}
      { repository: Regexp.last_match(1).to_i, type: TYPE_MAP.fetch(Regexp.last_match(2), Regexp.last_match(2).sub(/s\z/, '')), id: Regexp.last_match(3).to_i }
    elsif uri =~ %r{/([^/]+)/(\d+)}
      { type: TYPE_MAP.fetch(Regexp.last_match(1), Regexp.last_match(1).sub(/s\z/, '')), id: Regexp.last_match(2).to_i }
    end
  end

  def self.init(_opts = {})
    true
  end

  def self.models
    {}
  end
end

# Intercept requires for heavy backend files that are unnecessary offline
$LOADED_FEATURES << 'java' << 'java.rb' << 'ashttp.rb' << 'aspace_i18n.rb' << 'asutils.rb'
$LOADED_FEATURES << 'jsonmodel.rb' << 'jsonmodel_client.rb' << 'config/config-distribution.rb' << 'log.rb'
$LOADED_FEATURES << File.expand_path('../../indexer/app/lib/index_state.rb', __dir__)
$LOADED_FEATURES << File.expand_path('../../indexer/app/lib/index_state_s3.rb', __dir__)

# --- 2. Load REAL repository files ---
REPO_ROOT = File.expand_path('../..', __dir__)
$LOAD_PATH.unshift(File.join(REPO_ROOT, 'common'))
$LOADED_FEATURES << 'record_inheritance.rb' << 'record_inheritance'

require File.join(REPO_ROOT, 'common/record_inheritance')
require File.join(REPO_ROOT, 'indexer/app/lib/rust_index_document_builder')
require File.join(REPO_ROOT, 'indexer/app/lib/index_batch')
require File.join(REPO_ROOT, 'indexer/app/lib/indexer_common')
require File.join(REPO_ROOT, 'indexer/app/lib/periodic_indexer')
require File.join(REPO_ROOT, 'indexer/app/lib/large_tree_doc_indexer')
require File.join(REPO_ROOT, 'indexer/app/lib/pui_indexer')

class DummyIndexState; end
def RecordInheritance.prepare_schemas; end

# --- 3. Test Harness Framework ---

$total_tests = 0
$passed_tests = 0
$failed_tests = []

def test(name)
  $total_tests += 1
  print "  - #{name} ... "
  yield
  puts "[PASS]"
  $passed_tests += 1
rescue StandardError => e
  puts "[FAIL]"
  puts "    ERROR: #{e.class}: #{e.message}"
  puts "    BACKTRACE:\n      #{e.backtrace.take(6).join("\n      ")}"
  $failed_tests << [name, e]
end

def assert(condition, message = "Assertion failed")
  raise message unless condition
end

def assert_equal(expected, actual, message = nil)
  unless expected == actual
    msg = message || "Expected #{expected.inspect}, got #{actual.inspect}"
    raise msg
  end
end

def assert_nil(actual, message = nil)
  unless actual.nil?
    msg = message || "Expected nil, got #{actual.inspect}"
    raise msg
  end
end

def assert_raises(exception_class, message_pattern = nil)
  begin
    yield
    raised = false
  rescue exception_class => e
    raised = true
    if message_pattern && !(e.message =~ message_pattern)
      raise "Expected exception message matching #{message_pattern.inspect}, got #{e.message.inspect}"
    end
  rescue StandardError => e
    raise "Expected #{exception_class}, but #{e.class} was raised (#{e.message})"
  end
  raise "Expected #{exception_class} to be raised, but nothing was raised" unless raised
end

# --- 4. Fixture helpers ---

def build_sample_resource(uri = '/repositories/2/resources/99')
  {
    'uri' => uri,
    'jsonmodel_type' => 'resource',
    'title' => 'Sample Common Resource',
    'publish' => true,
    'suppressed' => false,
    'created_by' => 'archivist',
    'last_modified_by' => 'admin',
    'user_mtime' => '2026-10-19T10:00:00Z',
    'system_mtime' => '2026-10-19T10:00:01Z',
    'create_time' => '2026-10-01T00:00:00Z',
    'level' => 'subfonds',
    'other_level' => 'subfonds',
    'related_accessions' => [],
    'classifications' => [],
    'instances' => [],
    'external_ids' => [],
    'rights_statements' => [],
    'lang_materials' => [],
    'id_0' => 'RES',
    'id_1' => '99',
    'resource_type' => 'collection',
    'restrictions' => false,
    'ead_id' => nil,
    'finding_aid_title' => nil,
    'finding_aid_filing_title' => nil,
    'finding_aid_status' => nil,
    'slug' => nil,
    'is_slug_auto' => false,
    'subjects' => [
      { 'ref' => '/subjects/10', '_resolved' => { 'title' => 'Church History' } },
      { 'ref' => '/subjects/11', '_resolved' => { 'title' => nil } },
      { 'ref' => '/subjects/12', '_resolved' => { 'title' => 'Archives' } }
    ],
    'linked_agents' => [
      {
        'ref' => '/agents/people/5',
        'role' => 'creator',
        '_resolved' => {
          'display_name' => { 'sort_name' => 'Doe, Jane' },
          'publish' => true
        }
      },
      {
        'ref' => '/agents/corporate_entities/6',
        'role' => 'subject',
        '_resolved' => {
          'display_name' => { 'sort_name' => 'Historical Society' },
          'publish' => false
        }
      }
    ],
    'dates' => [
      { 'date_type' => 'inclusive', 'begin' => '1900-01-01', 'end' => '1905-12-31' },
      { 'date_type' => 'single', 'begin' => '1920-01-01' }
    ],
    'notes' => [
      { 'type' => 'abstract', 'content' => ['Summary line 1', 'Summary line 2'] }
    ],
    'extents' => [
      { 'portion' => 'part', 'number' => '10', 'extent_type' => 'files' },
      { 'portion' => 'whole', 'number' => '2', 'extent_type' => 'boxes' }
    ]
  }
end

def build_valid_projection(uri = '/repositories/2/resources/99')
  {
    'marker' => 'pui_common_metadata',
    'uri' => uri,
    'primary_type' => 'resource',
    'subjects' => {
      'subjects' => ['Church History', 'Archives'],
      'subject_uris' => ['/subjects/10', '/subjects/11', '/subjects/12']
    },
    'agents' => {
      'agents' => ['Doe, Jane', 'Historical Society'],
      'agent_uris' => ['/agents/people/5', '/agents/corporate_entities/6'],
      'published_agents' => ['Doe, Jane'],
      'published_agent_uris' => ['/agents/people/5'],
      'creators' => ['Doe, Jane']
    },
    'audit_info' => {
      'created_by' => 'archivist',
      'last_modified_by' => 'admin',
      'user_mtime' => '2026-10-19T10:00:00Z',
      'system_mtime' => '2026-10-19T10:00:01Z',
      'create_time' => '2026-10-01T00:00:00Z'
    },
    'years' => {
      'years' => ['1900', '1901', '1902', '1903', '1904', '1905', '1920'],
      'year_sort' => '19001920',
      'dates' => ['1900-01-01 - 1905-12-31']
    },
    'level' => {
      'level' => 'subfonds'
    },
    'summary' => {
      'summary' => "Summary line 1\nSummary line 2"
    },
    'extents' => {
      'extents' => ['2 --- boxes']
    }
  }
end

puts "\n=== Running PUI Common Metadata Verification Harness ==="

# --- Group 1: Numeric Type Strictness (validate_json_equal!) ---
puts "\n[Group 1: Numeric Type Strictness in RustIndexDocumentBuilder.validate_json_equal!]"

test "rejects Integer vs Float mismatch (1 vs 1.0)" do
  assert_raises(RuntimeError, /type mismatch.*expected Integer.*got Float/) do
    RustIndexDocumentBuilder.validate_json_equal!({ 'count' => 1 }, { 'count' => 1.0 })
  end
end

test "rejects Boolean vs Integer mismatch (false vs 0)" do
  assert_raises(RuntimeError, /type mismatch.*expected Boolean.*got Integer/) do
    RustIndexDocumentBuilder.validate_json_equal!({ 'flag' => false }, { 'flag' => 0 })
  end
end

test "rejects Array ordering divergence (['a', 'b'] vs ['b', 'a'])" do
  assert_raises(RuntimeError, /value mismatch/) do
    RustIndexDocumentBuilder.validate_json_equal!({ 'arr' => ['a', 'b'] }, { 'arr' => ['b', 'a'] })
  end
end

test "rejects Array length/duplicate divergence (['a', 'a'] vs ['a'])" do
  assert_raises(RuntimeError, /array length mismatch/) do
    RustIndexDocumentBuilder.validate_json_equal!({ 'arr' => ['a', 'a'] }, { 'arr' => ['a'] })
  end
end

test "rejects nested Hash missing keys" do
  assert_raises(RuntimeError, /keys mismatch.*missing: \["k2"\]/) do
    RustIndexDocumentBuilder.validate_json_equal!(
      { 'nested' => { 'k1' => 'v1', 'k2' => 'v2' } },
      { 'nested' => { 'k1' => 'v1' } }
    )
  end
end

test "rejects nested Hash extra keys" do
  assert_raises(RuntimeError, /keys mismatch.*extra: \["extra"\]/) do
    RustIndexDocumentBuilder.validate_json_equal!(
      { 'nested' => { 'k1' => 'v1' } },
      { 'nested' => { 'k1' => 'v1', 'extra' => 'v2' } }
    )
  end
end

test "accepts strictly identical recursive JSON structures" do
  doc = {
    'strings' => ['a', 'b'],
    'nested' => { 'count' => 5, 'active' => true },
    'null_val' => nil
  }
  # Must not raise
  RustIndexDocumentBuilder.validate_json_equal!(doc, doc.dup)
end

# --- Group 2: Builder Request Validation & Exact Field Rejection ---
puts "\n[Group 2: Builder Request Validation & Exact Field Rejection]"

test "builder rejects non-boolean prepare_pui_common_metadata flag" do
  builder = RustIndexDocumentBuilder.new('http://127.0.0.1:4570', 2)
  inp = {
    'uri' => '/repositories/2/resources/99',
    'record_type' => 'resource',
    'repository' => 2,
    'record' => build_sample_resource,
    'prepare_pui_common_metadata' => 'true'
  }
  assert_raises(RuntimeError, /prepare_pui_common_metadata must be explicitly boolean/) do
    builder.build([inp])
  end
end

test "builder rejects invalid flag combinations (enrich + prepare_pui_common_metadata)" do
  builder = RustIndexDocumentBuilder.new('http://127.0.0.1:4570', 2)
  inp = {
    'uri' => '/repositories/2/resources/99',
    'record_type' => 'resource',
    'repository' => 2,
    'record' => build_sample_resource,
    'enrich' => true,
    'prepare_pui_common_metadata' => true
  }
  assert_raises(RuntimeError, /cannot combine enrich and prepare_pui_common_metadata/) do
    builder.build([inp])
  end
end

test "builder rejects invalid flag combinations (prepare_pui_availability + prepare_pui_common_metadata)" do
  builder = RustIndexDocumentBuilder.new('http://127.0.0.1:4570', 2)
  inp = {
    'uri' => '/repositories/2/resources/99',
    'record_type' => 'resource',
    'repository' => 2,
    'record' => build_sample_resource,
    'prepare_pui_availability' => true,
    'prepare_pui_common_metadata' => true
  }
  assert_raises(RuntimeError, /cannot combine prepare_pui_availability and prepare_pui_common_metadata/) do
    builder.build([inp])
  end
end

test "builder rejects input URI mismatch with record URI" do
  builder = RustIndexDocumentBuilder.new('http://127.0.0.1:4570', 2)
  inp = {
    'uri' => '/repositories/2/resources/99',
    'record_type' => 'resource',
    'repository' => 2,
    'record' => build_sample_resource.merge('uri' => '/repositories/2/resources/different'),
    'prepare_pui_common_metadata' => true
  }
  assert_raises(RuntimeError, /record uri mismatch/) do
    builder.build([inp])
  end
end

test "builder rejects input record_type mismatch with record jsonmodel_type" do
  builder = RustIndexDocumentBuilder.new('http://127.0.0.1:4570', 2)
  inp = {
    'uri' => '/repositories/2/resources/99',
    'record_type' => 'resource',
    'repository' => 2,
    'record' => build_sample_resource.merge('jsonmodel_type' => 'archival_object'),
    'prepare_pui_common_metadata' => true
  }
  assert_raises(RuntimeError, /record jsonmodel_type mismatch/) do
    builder.build([inp])
  end
end

test "builder rejects unrequested _archivalist_pui_final in common-only response" do
  builder = RustIndexDocumentBuilder.new('http://127.0.0.1:4570', 2)
  sample_rec = build_sample_resource
  inp = {
    'uri' => sample_rec['uri'],
    'record_type' => 'resource',
    'repository' => 2,
    'record' => sample_rec,
    'prepare_pui_common_metadata' => true
  }
  envelope = {
    'id' => sample_rec['uri'],
    'uri' => sample_rec['uri'],
    'title' => sample_rec['title'],
    'primary_type' => 'resource',
    'types' => ['resource'],
    'json' => ASUtils.to_json(sample_rec),
    'suppressed' => false,
    'publish' => true,
    'system_generated' => 'false',
    'repository' => 2,
    '_archivalist_rust_fields' => ['pui_common_metadata', 'pui_final_document_metadata'],
    '_archivalist_pui_common' => build_valid_projection,
    '_archivalist_pui_final' => { 'marker' => 'pui_final_document_metadata' } # Unrequested!
  }
  builder.define_singleton_method(:perform_request) do |_req|
    Struct.new(:code, :body).new('200', ASUtils.to_json(
      'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
      'documents' => [envelope]
    ))
  end
  assert_raises(RuntimeError, /document contains unexpected fields.*_archivalist_pui_final/) do
    builder.build([inp])
  end
end

test "builder validates combined prepare_pui_final and prepare_pui_common_metadata response" do
  builder = RustIndexDocumentBuilder.new('http://127.0.0.1:4570', 2)
  sample_rec = build_sample_resource
  inp = {
    'uri' => sample_rec['uri'],
    'record_type' => 'resource',
    'repository' => 2,
    'record' => sample_rec,
    'pui_candidate_record' => sample_rec,
    'prepare_pui_final' => true,
    'prepare_pui_common_metadata' => true
  }
  envelope = {
    'id' => sample_rec['uri'],
    'uri' => sample_rec['uri'],
    'title' => sample_rec['title'],
    'primary_type' => 'resource',
    'types' => ['resource'],
    'json' => ASUtils.to_json(sample_rec),
    'suppressed' => false,
    'publish' => true,
    'system_generated' => 'false',
    'repository' => 2,
    '_archivalist_rust_fields' => [
      'pui_final_document_metadata',
      'pui_routing_metadata',
      'pui_notes_metadata',
      'pui_common_metadata'
    ],
    '_archivalist_pui_final' => {
      'marker' => 'pui_final_document_metadata',
      'json' => ASUtils.to_json(sample_rec),
      'fullrecord_published' => IndexerCommon.extract_string_values(sample_rec, :published_only)
    },
    '_archivalist_pui_routing' => {
      'marker' => 'pui_routing_metadata',
      'id' => "#{sample_rec['uri']}#pui",
      'pui_parent_id' => sample_rec['uri'],
      'types' => ['pui', 'pui_resource', 'pui_record', 'pui_only']
    },
    '_archivalist_pui_notes' => {
      'marker' => 'pui_notes_metadata',
      'notes_published' => IndexerCommon.extract_string_values(sample_rec['notes'], :published_only)
    },
    '_archivalist_pui_common' => build_valid_projection(sample_rec['uri'])
  }
  builder.define_singleton_method(:perform_request) do |_req|
    Struct.new(:code, :body).new('200', ASUtils.to_json(
      'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
      'documents' => [envelope]
    ))
  end
  res = builder.build([inp])
  assert_equal(1, res.length)
  doc = res.values.first
  assert_equal('pui_common_metadata', doc['_archivalist_pui_common']['marker'])
  assert_equal('pui_final_document_metadata', doc['_archivalist_pui_final']['marker'])
end

test "builder rejects unrequested pui_candidate in combined request without inheritance plan" do
  builder = RustIndexDocumentBuilder.new('http://127.0.0.1:4570', 2)
  sample_rec = build_sample_resource
  inp = {
    'uri' => sample_rec['uri'],
    'record_type' => 'resource',
    'repository' => 2,
    'record' => sample_rec,
    'pui_candidate_record' => sample_rec,
    'prepare_pui_final' => true,
    'prepare_pui_common_metadata' => true
  }
  envelope = {
    'id' => sample_rec['uri'],
    'uri' => sample_rec['uri'],
    'title' => sample_rec['title'],
    'primary_type' => 'resource',
    'types' => ['resource'],
    'json' => ASUtils.to_json(sample_rec),
    'suppressed' => false,
    'publish' => true,
    'system_generated' => 'false',
    'repository' => 2,
    '_archivalist_rust_fields' => [
      'pui_final_document_metadata',
      'pui_routing_metadata',
      'pui_notes_metadata',
      'pui_common_metadata'
    ],
    '_archivalist_pui_final' => {
      'marker' => 'pui_final_document_metadata',
      'json' => ASUtils.to_json(sample_rec),
      'fullrecord_published' => IndexerCommon.extract_string_values(sample_rec, :published_only)
    },
    '_archivalist_pui_routing' => {
      'marker' => 'pui_routing_metadata',
      'id' => "#{sample_rec['uri']}#pui",
      'pui_parent_id' => sample_rec['uri'],
      'types' => ['pui', 'pui_resource', 'pui_record', 'pui_only']
    },
    '_archivalist_pui_notes' => {
      'marker' => 'pui_notes_metadata',
      'notes_published' => IndexerCommon.extract_string_values(sample_rec['notes'], :published_only)
    },
    '_archivalist_pui_common' => build_valid_projection(sample_rec['uri']),
    '_archivalist_pui_candidate' => { 'marker' => 'pui_record_inheritance_candidate_metadata' } # Unrequested!
  }
  builder.define_singleton_method(:perform_request) do |_req|
    Struct.new(:code, :body).new('200', ASUtils.to_json(
      'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
      'documents' => [envelope]
    ))
  end
  assert_raises(RuntimeError, /unexpected _archivalist_pui_candidate/) do
    builder.build([inp])
  end
end

test "builder validates non-alphabetical key preservation and exact serialization for pui_final" do
  builder = RustIndexDocumentBuilder.new('http://127.0.0.1:4570', 2)
  non_alpha_rec = {
    'z_custom' => 'last_letter',
    'uri' => '/repositories/2/resources/998',
    'title' => 'Émile Cartailhac 📚 — Société Géologique 中文',
    'publish' => true,
    'jsonmodel_type' => 'resource',
    'repository' => { 'ref' => '/repositories/2' },
    'notes' => [
      { 'jsonmodel_type' => 'note_singlepart', 'type' => 'scopecontent', 'publish' => true, 'content' => ['Étude spéléologique.'] }
    ],
    'a_custom' => 'first_letter'
  }
  inp = {
    'uri' => non_alpha_rec['uri'],
    'record_type' => 'resource',
    'repository' => 2,
    'record' => non_alpha_rec,
    'pui_candidate_record' => non_alpha_rec,
    'prepare_pui_final' => true
  }
  envelope = {
    'id' => non_alpha_rec['uri'],
    'uri' => non_alpha_rec['uri'],
    'title' => non_alpha_rec['title'],
    'primary_type' => 'resource',
    'types' => ['resource'],
    'json' => ASUtils.to_json(non_alpha_rec),
    'suppressed' => false,
    'publish' => true,
    'system_generated' => 'false',
    'repository' => 2,
    '_archivalist_rust_fields' => [
      'pui_final_document_metadata',
      'pui_routing_metadata',
      'pui_notes_metadata'
    ],
    '_archivalist_pui_final' => {
      'marker' => 'pui_final_document_metadata',
      'json' => ASUtils.to_json(non_alpha_rec),
      'fullrecord_published' => IndexerCommon.extract_string_values(non_alpha_rec, :published_only)
    },
    '_archivalist_pui_routing' => {
      'marker' => 'pui_routing_metadata',
      'id' => "#{non_alpha_rec['uri']}#pui",
      'pui_parent_id' => non_alpha_rec['uri'],
      'types' => ['pui', 'pui_resource', 'pui_record', 'pui_only']
    },
    '_archivalist_pui_notes' => {
      'marker' => 'pui_notes_metadata',
      'notes_published' => IndexerCommon.extract_string_values(non_alpha_rec['notes'], :published_only)
    }
  }
  builder.define_singleton_method(:perform_request) do |_req|
    Struct.new(:code, :body).new('200', ASUtils.to_json(
      'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
      'documents' => [envelope]
    ))
  end
  res = builder.build([inp])
  assert_equal(1, res.length)
  doc = res.values.first
  assert_equal(ASUtils.to_json(non_alpha_rec), doc['_archivalist_pui_final']['json'])
end

test "builder rejects key-scrambled or alphabetically-sorted pui_final json mismatch" do
  builder = RustIndexDocumentBuilder.new('http://127.0.0.1:4570', 2)
  non_alpha_rec = {
    'z_custom' => 'last_letter',
    'uri' => '/repositories/2/resources/998',
    'title' => 'Émile Cartailhac 📚 — Société Géologique 中文',
    'publish' => true,
    'jsonmodel_type' => 'resource',
    'repository' => { 'ref' => '/repositories/2' },
    'notes' => [
      { 'jsonmodel_type' => 'note_singlepart', 'type' => 'scopecontent', 'publish' => true, 'content' => ['Étude spéléologique.'] }
    ],
    'a_custom' => 'first_letter'
  }
  inp = {
    'uri' => non_alpha_rec['uri'],
    'record_type' => 'resource',
    'repository' => 2,
    'record' => non_alpha_rec,
    'pui_candidate_record' => non_alpha_rec,
    'prepare_pui_final' => true
  }

  sorted_rec = Hash[non_alpha_rec.sort]
  sorted_json = ASUtils.to_json(sorted_rec)

  envelope = {
    'id' => non_alpha_rec['uri'],
    'uri' => non_alpha_rec['uri'],
    'title' => non_alpha_rec['title'],
    'primary_type' => 'resource',
    'types' => ['resource'],
    'json' => ASUtils.to_json(non_alpha_rec),
    'suppressed' => false,
    'publish' => true,
    'system_generated' => 'false',
    'repository' => 2,
    '_archivalist_rust_fields' => [
      'pui_final_document_metadata',
      'pui_routing_metadata',
      'pui_notes_metadata'
    ],
    '_archivalist_pui_final' => {
      'marker' => 'pui_final_document_metadata',
      'json' => sorted_json,
      'fullrecord_published' => IndexerCommon.extract_string_values(non_alpha_rec, :published_only)
    },
    '_archivalist_pui_routing' => {
      'marker' => 'pui_routing_metadata',
      'id' => "#{non_alpha_rec['uri']}#pui",
      'pui_parent_id' => non_alpha_rec['uri'],
      'types' => ['pui', 'pui_resource', 'pui_record', 'pui_only']
    },
    '_archivalist_pui_notes' => {
      'marker' => 'pui_notes_metadata',
      'notes_published' => IndexerCommon.extract_string_values(non_alpha_rec['notes'], :published_only)
    }
  }
  builder.define_singleton_method(:perform_request) do |_req|
    Struct.new(:code, :body).new('200', ASUtils.to_json(
      'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
      'documents' => [envelope]
    ))
  end
  assert_raises(RuntimeError, /pui final json serialization mismatch/) do
    builder.build([inp])
  end
end

test "cross-language live sidecar integration feeds actual response to Ruby builder with non-alphabetical record" do
  port = 4596
  bin = File.expand_path('../target/debug/aspace-shadow-sidecar', __dir__)
  if File.executable?(bin)
    pid = Process.spawn(bin, '--port', port.to_s, '--solr-url', 'http://127.0.0.1:8983/solr', '--collection', 'test_collection', out: '/dev/null', err: '/dev/null')
    begin
      healthy = false
      20.times do
        sleep 0.1
        begin
          res = Net::HTTP.get_response(URI("http://127.0.0.1:#{port}/health"))
          if res.code == '200'
            healthy = true
            break
          end
        rescue Errno::ECONNREFUSED, Errno::EADDRNOTAVAIL
        end
      end
      assert healthy, "Spawned sidecar must report healthy"

      builder = RustIndexDocumentBuilder.new("http://127.0.0.1:#{port}", 5)
      rec = {
        'z_key' => 'omega',
        'uri' => '/repositories/2/archival_objects/999',
        'title' => 'Émile Cartailhac 📚 — Société Géologique 中文',
        'publish' => true,
        'suppressed' => false,
        'jsonmodel_type' => 'archival_object',
        'repository' => { 'ref' => '/repositories/2' },
        'notes' => [
          { 'jsonmodel_type' => 'note_singlepart', 'type' => 'scopecontent', 'publish' => true, 'content' => ['Scope note.'] }
        ],
        'instances' => [],
        'a_key' => 'alpha'
      }
      input = {
        'uri' => rec['uri'],
        'record_type' => 'archival_object',
        'repository' => 2,
        'record' => rec,
        'pui_candidate_record' => rec,
        'prepare_pui_final' => true
      }

      res = builder.build([input])
      assert_equal(1, res.length)
      doc = res[rec['uri']]
      assert doc, "Builder must accept live sidecar projection"
      assert_equal(ASUtils.to_json(rec), doc['_archivalist_pui_final']['json'])
    ensure
      if pid
        Process.kill('TERM', pid) rescue nil
        Process.waitpid(pid) rescue nil
      end
    end
  end
end

# --- Group 3: PUIIndexer Helper Overrides & Cache Survival ---
puts "\n[Group 3: PUIIndexer Helper Overrides & Cache Survival]"

test "all seven helpers consume projection and cache survives across all seven invocations" do
  indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  sample_rec = build_sample_resource
  uri = sample_rec['uri']
  doc = { 'id' => uri, 'uri' => uri, 'primary_type' => 'resource', 'pre_existing' => 'untouched' }
  rec = { 'record' => sample_rec, 'uri' => uri }
  proj = build_valid_projection(uri)

  indexer.instance_variable_set(:@pui_common_projections, { uri => proj.dup })

  indexer.add_subjects(doc, rec)
  assert_equal(['Church History', 'Archives'], doc['subjects'])
  assert_equal(['/subjects/10', '/subjects/11', '/subjects/12'], doc['subject_uris'])
  assert(!indexer.instance_variable_get(:@pui_common_projections)[uri].nil?, "Cache lost after subjects")

  indexer.add_agents(doc, rec)
  assert_equal(['Doe, Jane', 'Historical Society'], doc['agents'])
  assert_equal(['Doe, Jane'], doc['published_agents'])
  assert_equal(['Doe, Jane'], doc['creators'])
  assert(!indexer.instance_variable_get(:@pui_common_projections)[uri].nil?, "Cache lost after agents")

  indexer.add_audit_info(doc, rec)
  assert_equal('archivist', doc['created_by'])
  assert_equal('admin', doc['last_modified_by'])
  assert(!indexer.instance_variable_get(:@pui_common_projections)[uri].nil?, "Cache lost after audit_info")

  indexer.add_years(doc, rec)
  assert_equal(['1900', '1901', '1902', '1903', '1904', '1905', '1920'], doc['years'])
  assert_equal('19001920', doc['year_sort'])
  assert_equal(['1900-01-01 - 1905-12-31'], doc['dates'])
  assert(!indexer.instance_variable_get(:@pui_common_projections)[uri].nil?, "Cache lost after years")

  indexer.add_level(doc, rec)
  assert_equal('subfonds', doc['level'])
  assert(!indexer.instance_variable_get(:@pui_common_projections)[uri].nil?, "Cache lost after level")

  indexer.add_summary(doc, rec)
  assert_equal("Summary line 1\nSummary line 2", doc['summary'])
  assert(!indexer.instance_variable_get(:@pui_common_projections)[uri].nil?, "Cache lost after summary")

  indexer.add_extents(doc, rec)
  assert_equal(['2 --- boxes'], doc['extents'])
  assert(!indexer.instance_variable_get(:@pui_common_projections)[uri].nil?, "Cache lost after extents")

  assert_equal('untouched', doc['pre_existing'])
end

test "rejects unknown keys in helper projection envelope and falls back natively" do
  indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  sample_rec = build_sample_resource
  uri = sample_rec['uri']
  doc = { 'id' => uri, 'uri' => uri, 'primary_type' => 'resource' }
  rec = { 'record' => sample_rec, 'uri' => uri }
  bad_proj = build_valid_projection(uri).merge('extra_envelope_key' => 'corrupt')

  indexer.instance_variable_set(:@pui_common_projections, { uri => bad_proj })
  indexer.add_subjects(doc, rec)

  # Falls back to native execution
  assert_equal(['Church History', 'Archives'], doc['subjects'])
  # Cache evicted for this URI
  assert_nil(indexer.instance_variable_get(:@pui_common_projections)[uri])
end

test "rejects altered types in helper patch (e.g. integer level) and falls back natively" do
  indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  sample_rec = build_sample_resource
  uri = sample_rec['uri']
  doc = { 'id' => uri, 'uri' => uri, 'primary_type' => 'resource' }
  rec = { 'record' => sample_rec, 'uri' => uri }
  bad_proj = build_valid_projection(uri)
  bad_proj['level'] = { 'level' => 999 } # Integer instead of String!

  indexer.instance_variable_set(:@pui_common_projections, { uri => bad_proj })
  indexer.add_level(doc, rec)

  # Falls back to native execution
  assert_equal('subfonds', doc['level'])
  assert_nil(indexer.instance_variable_get(:@pui_common_projections)[uri])
end

test "rejects inner record URI mismatch and clears candidate cache entries" do
  indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  uri_a = '/repositories/2/resources/99'
  uri_b = '/repositories/2/resources/102'

  rec_a = build_sample_resource(uri_a)
  # Malicious plugin alters inner URI
  rec_a['uri'] = uri_b

  doc_a = { 'id' => "#{uri_a}#pui", 'uri' => uri_a, 'primary_type' => 'resource' }
  rec_input = { 'record' => rec_a, 'uri' => uri_a }

  indexer.instance_variable_set(:@pui_common_projections, {
    uri_a => build_valid_projection(uri_a),
    uri_b => build_valid_projection(uri_b)
  })

  indexer.add_subjects(doc_a, rec_input)

  # Both candidate URIs evicted to avoid cross-record cache consumption
  assert_nil(indexer.instance_variable_get(:@pui_common_projections)[uri_a])
  assert_nil(indexer.instance_variable_get(:@pui_common_projections)[uri_b])
  # doc_a computed via native fallback
  assert_equal(['Church History', 'Archives'], doc_a['subjects'])
end

test "stale projection fallback when record is mutated in-memory before helper runs" do
  indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  sample_rec = build_sample_resource
  uri = sample_rec['uri']
  mutated_rec = sample_rec.dup
  # Mutation: subjects changed
  mutated_rec['subjects'] = [
    { 'ref' => '/subjects/mutated', '_resolved' => { 'title' => 'In-Memory Mutated Title' } }
  ]

  doc = { 'id' => uri, 'uri' => uri, 'primary_type' => 'resource' }
  rec = { 'record' => mutated_rec, 'uri' => uri }

  indexer.instance_variable_set(:@pui_common_projections, { uri => build_valid_projection(uri) })
  indexer.add_subjects(doc, rec)

  # Discards stale projection and evaluates native logic on mutated record
  assert_equal(['In-Memory Mutated Title'], doc['subjects'])
  assert_equal(['/subjects/mutated'], doc['subject_uris'])
  assert_nil(indexer.instance_variable_get(:@pui_common_projections)[uri])
end

test "no-write behavior: preserves pre-existing doc values when patch has absent fields" do
  indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  sparse_rec = build_sample_resource
  sparse_rec.delete('linked_agents') # no creators
  sparse_rec.delete('dates')         # no year_sort
  sparse_rec.delete('notes')         # no summary
  uri = sparse_rec['uri']
  doc = {
    'id' => uri,
    'uri' => uri,
    'primary_type' => 'resource',
    'creators' => ['Pre-existing Creator'],
    'year_sort' => '18501850',
    'summary' => 'Pre-existing Summary'
  }
  rec = { 'record' => sparse_rec, 'uri' => uri }

  sparse_proj = build_valid_projection(uri)
  sparse_proj['agents'] = {}
  sparse_proj['years'] = {}
  sparse_proj['summary'] = {}
  indexer.instance_variable_set(:@pui_common_projections, { uri => sparse_proj })

  indexer.add_agents(doc, rec)
  indexer.add_years(doc, rec)
  indexer.add_summary(doc, rec)

  # Absent fields in patch MUST NOT overwrite pre-existing values
  assert_equal(['Pre-existing Creator'], doc['creators'])
  assert_equal('18501850', doc['year_sort'])
  assert_equal('Pre-existing Summary', doc['summary'])
  assert(!indexer.instance_variable_get(:@pui_common_projections)[uri].nil?)
end

test "per-record cache isolation across multi-record batches" do
  indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  uri_a = '/repositories/2/resources/99'
  uri_b = '/repositories/2/resources/102'

  rec_a = build_sample_resource(uri_a)
  rec_b = build_sample_resource(uri_b).merge('title' => 'Resource B')

  doc_a = { 'id' => uri_a, 'uri' => uri_a, 'primary_type' => 'resource' }
  doc_b = { 'id' => uri_b, 'uri' => uri_b, 'primary_type' => 'resource' }

  # Corrupt Record A's projection
  bad_proj_a = build_valid_projection(uri_a).merge('marker' => 'corrupt_marker')
  good_proj_b = build_valid_projection(uri_b)

  indexer.instance_variable_set(:@pui_common_projections, {
    uri_a => bad_proj_a,
    uri_b => good_proj_b
  })

  indexer.add_subjects(doc_a, { 'record' => rec_a, 'uri' => uri_a })
  indexer.add_subjects(doc_b, { 'record' => rec_b, 'uri' => uri_b })

  # Record A fell back natively, evicted from cache
  assert_equal(['Church History', 'Archives'], doc_a['subjects'])
  assert_nil(indexer.instance_variable_get(:@pui_common_projections)[uri_a])

  # Record B succeeded with projection, cache survived
  assert_equal(['Church History', 'Archives'], doc_b['subjects'])
  assert_equal(good_proj_b, indexer.instance_variable_get(:@pui_common_projections)[uri_b])
end

# --- Group 4: End-to-End IndexerCommon#index_records with IndexBatch ---
puts "\n[Group 4: End-to-End IndexerCommon#index_records with IndexBatch Capture]"

test "index_records executes common-only request, checks payload, verifies helper order, and cleans up cache" do
  indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  sample_rec = build_sample_resource
  uri = sample_rec['uri']

  envelope = {
    'id' => uri,
    'uri' => uri,
    'title' => sample_rec['title'],
    'primary_type' => 'resource',
    'types' => ['resource'],
    'json' => ASUtils.to_json(sample_rec),
    'suppressed' => false,
    'publish' => true,
    'system_generated' => 'false',
    'repository' => 2,
    '_archivalist_rust_fields' => ['pui_common_metadata'],
    '_archivalist_pui_common' => build_valid_projection(uri)
  }

  captured_request = nil
  RustIndexDocumentBuilder.class_eval do
    define_method(:perform_request) do |req|
      captured_request = req
      Struct.new(:code, :body).new('200', ASUtils.to_json(
        'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
        'documents' => [envelope]
      ))
    end
  end

  # Instrument helper call order covering all seven helpers + add_notes
  order_trail = []
  [:add_subjects, :add_agents, :add_audit_info, :add_notes, :add_years, :add_level, :add_summary, :add_extents].each do |m|
    orig = indexer.method(m)
    indexer.define_singleton_method(m) do |doc, rec|
      order_trail << m
      orig.call(doc, rec)
    end
  end

  # Instrument helper acceptance before batch cleanup
  accepted_helpers = []
  orig_apply = indexer.method(:apply_rust_pui_common_helper)
  indexer.define_singleton_method(:apply_rust_pui_common_helper) do |helper_name, doc, rec, &block|
    res = orig_apply.call(helper_name, doc, rec, &block)
    accepted_helpers << helper_name if res
    res
  end

  # Capture batch using real IndexBatch #each iteration (no concat)
  captured_docs = []
  indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
    batch.each { |doc| captured_docs << doc }
  end

  indexer.index_records([{ 'record' => sample_rec, 'uri' => uri }])

  # 1. Assert perform_request was actually called and inspect requested JSON
  assert(!captured_request.nil?, "perform_request was not called")
  req_body = ASUtils.json_parse(captured_request.body)
  req_record = req_body['records'].first
  assert_equal(true, req_record['prepare_pui_common_metadata'])
  assert_nil(req_record['prepare_pui_final'])

  # 2. Assert helper ordering matches original native sequence around add_notes
  expected_order = [:add_subjects, :add_agents, :add_audit_info, :add_notes, :add_years, :add_level, :add_summary, :add_extents]
  assert_equal(expected_order, order_trail)

  # 3. Assert all seven helpers accepted the projection before batch cleanup
  expected_helpers = ['subjects', 'agents', 'audit_info', 'years', 'level', 'summary', 'extents']
  assert_equal(expected_helpers, accepted_helpers)

  # 4. Assert batch output doc has common search fields and no private handoff keys
  assert_equal(1, captured_docs.length)
  doc = captured_docs.first
  assert_equal(['Church History', 'Archives'], doc['subjects'])
  assert_equal(['Doe, Jane', 'Historical Society'], doc['agents'])
  assert_equal('subfonds', doc['level'])
  assert_equal("Summary line 1\nSummary line 2", doc['summary'])
  assert_equal(['2 --- boxes'], doc['extents'])
  assert(doc.keys.none? { |k| k.start_with?('_archivalist_') }, "Handoff keys leaked to batch")

  # 5. Assert batch cache cleaned up to nil
  assert_nil(indexer.instance_variable_get(:@pui_common_projections))
end

test "combined flags prepare_pui_final + prepare_pui_common_metadata with #pui routing" do
  AppConfig[:rust_pui_document_preparation_enabled] = true
  begin
    indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
    sample_rec = build_sample_resource
    uri = sample_rec['uri']

    combined_envelope = {
      'id' => uri,
      'uri' => uri,
      'title' => sample_rec['title'],
      'primary_type' => 'resource',
      'types' => ['resource'],
      'json' => ASUtils.to_json(sample_rec),
      'suppressed' => false,
      'publish' => true,
      'system_generated' => 'false',
      'repository' => 2,
      '_archivalist_rust_fields' => [
        'pui_final_document_metadata',
        'pui_routing_metadata',
        'pui_notes_metadata',
        'pui_common_metadata'
      ],
      '_archivalist_pui_final' => {
        'marker' => 'pui_final_document_metadata',
        'json' => ASUtils.to_json(sample_rec),
        'fullrecord_published' => IndexerCommon.extract_string_values(sample_rec, :published_only)
      },
      '_archivalist_pui_routing' => {
        'marker' => 'pui_routing_metadata',
        'id' => "#{uri}#pui",
        'pui_parent_id' => uri,
        'types' => ['pui', 'pui_resource', 'pui_record', 'pui_only']
      },
      '_archivalist_pui_notes' => {
        'marker' => 'pui_notes_metadata',
        'notes_published' => IndexerCommon.extract_string_values(sample_rec['notes'], :published_only)
      },
      '_archivalist_pui_common' => build_valid_projection(uri)
    }

    captured_request = nil
    RustIndexDocumentBuilder.class_eval do
      define_method(:perform_request) do |req|
        captured_request = req
        Struct.new(:code, :body).new('200', ASUtils.to_json(
          'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
          'documents' => [combined_envelope]
        ))
      end
    end

    combined_docs = []
    indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
      batch.each { |doc| combined_docs << doc }
    end

    indexer.index_records([{ 'record' => sample_rec, 'uri' => uri }])

    assert(!captured_request.nil?)
    req_body = ASUtils.json_parse(captured_request.body)
    req_rec = req_body['records'].first
    assert_equal(true, req_rec['prepare_pui_common_metadata'])
    assert_equal(true, req_rec['prepare_pui_final'])

    assert_equal(1, combined_docs.length)
    comb_doc = combined_docs.first
    assert_equal("#{uri}#pui", comb_doc['id'])
    assert_equal(['Church History', 'Archives'], comb_doc['subjects'])
    assert(comb_doc.keys.none? { |k| k.start_with?('_archivalist_') })
    assert_nil(indexer.instance_variable_get(:@pui_common_projections))
  ensure
    AppConfig[:rust_pui_document_preparation_enabled] = false
  end
end

test "transport outage raises in perform_request and falls back cleanly to native execution" do
  indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  sample_rec = build_sample_resource
  uri = sample_rec['uri']

  outage_called = false
  RustIndexDocumentBuilder.class_eval do
    define_method(:perform_request) do |_req|
      outage_called = true
      raise Errno::ECONNREFUSED, 'Sidecar connection refused'
    end
  end

  outage_docs = []
  indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
    batch.each { |doc| outage_docs << doc }
  end

  indexer.index_records([{ 'record' => sample_rec, 'uri' => uri }])

  assert(outage_called, "Outage stub perform_request was not called")
  assert_equal(1, outage_docs.length)
  doc = outage_docs.first
  assert_equal(['Church History', 'Archives'], doc['subjects'])
  assert_nil(indexer.instance_variable_get(:@pui_common_projections))
end

test "batch cache cleanup to nil on hook exception" do
  exploding_indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  sample_rec = build_sample_resource
  uri = sample_rec['uri']

  envelope = {
    'id' => uri,
    'uri' => uri,
    'title' => sample_rec['title'],
    'primary_type' => 'resource',
    'types' => ['resource'],
    'json' => ASUtils.to_json(sample_rec),
    'suppressed' => false,
    'publish' => true,
    'system_generated' => 'false',
    'repository' => 2,
    '_archivalist_rust_fields' => ['pui_common_metadata'],
    '_archivalist_pui_common' => build_valid_projection(uri)
  }

  RustIndexDocumentBuilder.class_eval do
    define_method(:perform_request) do |_req|
      Struct.new(:code, :body).new('200', ASUtils.to_json(
        'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
        'documents' => [envelope]
      ))
    end
  end

  exploding_indexer.instance_variable_get(:@document_prepare_hooks) << proc { raise "Simulated hook crash" }

  assert_raises(RuntimeError, /Simulated hook crash/) do
    exploding_indexer.index_records([{ 'record' => sample_rec, 'uri' => uri }])
  end

  assert_nil(exploding_indexer.instance_variable_get(:@pui_common_projections), "Cache not cleared after hook exception")
end

test "index_records detects pre-hook mutation before common helpers and falls back to native calculation" do
  indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  sample_rec = build_sample_resource
  uri = sample_rec['uri']

  envelope = {
    'id' => uri,
    'uri' => uri,
    'title' => sample_rec['title'],
    'primary_type' => 'resource',
    'types' => ['resource'],
    'json' => ASUtils.to_json(sample_rec),
    'suppressed' => false,
    'publish' => true,
    'system_generated' => 'false',
    'repository' => 2,
    '_archivalist_rust_fields' => ['pui_common_metadata'],
    '_archivalist_pui_common' => build_valid_projection(uri)
  }

  RustIndexDocumentBuilder.class_eval do
    define_method(:perform_request) do |_req|
      Struct.new(:code, :body).new('200', ASUtils.to_json(
        'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
        'documents' => [envelope]
      ))
    end
  end

  # Pre-hook mutates subjects before common helpers run
  pre_hook = proc do |_doc, rec|
    rec['record']['subjects'] = [
      { 'ref' => '/subjects/pre', '_resolved' => { 'title' => 'Pre-Hook Mutated Subject' } }
    ]
  end
  indexer.instance_variable_get(:@document_prepare_hooks).unshift(pre_hook)

  captured_docs = []
  indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
    batch.each { |doc| captured_docs << doc }
  end

  indexer.index_records([{ 'record' => sample_rec, 'uri' => uri }])
  assert_equal(1, captured_docs.length)
  doc = captured_docs.first
  assert_equal(['Pre-Hook Mutated Subject'], doc['subjects'])
  assert_equal(['/subjects/pre'], doc['subject_uris'])
  assert_nil(indexer.instance_variable_get(:@pui_common_projections))
end

test "index_records preserves pre-existing doc values when patch fields are absent (no-write behavior)" do
  indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  sparse_rec = build_sample_resource
  sparse_rec.delete('linked_agents') # no creators
  sparse_rec.delete('dates')         # no year_sort
  sparse_rec.delete('notes')         # no summary
  uri = sparse_rec['uri']

  sparse_proj = build_valid_projection(uri)
  sparse_proj['agents'] = {}
  sparse_proj['years'] = {}
  sparse_proj['summary'] = {}

  envelope = {
    'id' => uri,
    'uri' => uri,
    'title' => sparse_rec['title'],
    'primary_type' => 'resource',
    'types' => ['resource'],
    'json' => ASUtils.to_json(sparse_rec),
    'suppressed' => false,
    'publish' => true,
    'system_generated' => 'false',
    'repository' => 2,
    '_archivalist_rust_fields' => ['pui_common_metadata'],
    '_archivalist_pui_common' => sparse_proj
  }

  RustIndexDocumentBuilder.class_eval do
    define_method(:perform_request) do |_req|
      Struct.new(:code, :body).new('200', ASUtils.to_json(
        'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
        'documents' => [envelope]
      ))
    end
  end

  # Pre-hook initializes pre-existing values on doc
  pre_hook = proc do |doc, _rec|
    doc['creators'] = ['Pre-existing Creator']
    doc['year_sort'] = '18501850'
    doc['summary'] = 'Pre-existing Summary'
  end
  indexer.instance_variable_get(:@document_prepare_hooks).unshift(pre_hook)

  captured_docs = []
  indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
    batch.each { |doc| captured_docs << doc }
  end

  indexer.index_records([{ 'record' => sparse_rec, 'uri' => uri }])
  assert_equal(1, captured_docs.length)
  doc = captured_docs.first
  assert_equal(['Pre-existing Creator'], doc['creators'])
  assert_equal('18501850', doc['year_sort'])
  assert_equal('Pre-existing Summary', doc['summary'])
  assert_nil(indexer.instance_variable_get(:@pui_common_projections))
end

test "add_level maps otherlevel to other_level" do
  indexer = PUIIndexer.new('http://localhost:8089', DummyIndexState.new, 'PUIIndexer')
  other_rec = build_sample_resource.merge('level' => 'otherlevel', 'other_level' => 'fonds')
  uri = other_rec['uri']
  doc = { 'id' => uri, 'uri' => uri, 'primary_type' => 'resource' }
  proj = build_valid_projection(uri)
  proj['level'] = { 'level' => 'fonds' }
  indexer.instance_variable_set(:@pui_common_projections, { uri => proj })

  indexer.add_level(doc, { 'record' => other_rec, 'uri' => uri })
  assert_equal('fonds', doc['level'])
  assert(!indexer.instance_variable_get(:@pui_common_projections)[uri].nil?)
end

# --- 5. Summary & Exit ---

puts "\n=== Test Results Summary ==="
puts "Total: #{$total_tests} | Passed: #{$passed_tests} | Failed: #{$failed_tests.length}"

if $failed_tests.empty?
  puts "All PUI common metadata offline verification tests passed successfully."
  exit 0
else
  puts "\nFAILED TESTS:"
  $failed_tests.each do |(name, err)|
    puts "  - #{name}: #{err.message}"
  end
  exit 1
end
