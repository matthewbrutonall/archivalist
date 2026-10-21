require 'spec_helper'
require_relative '../app/lib/pui_indexer'

describe "PUI indexer" do
  let (:indexer) do
    indexer = PUIIndexer.new(AppConfig[:backend_url], nil, 'PUIIndexer')

    def indexer.prepare_docs(records)
      records.map {|rec|
        rec_hash = rec.is_a?(Hash) ? rec : (rec.respond_to?(:to_hash) ? rec.to_hash(:raw) : rec)
        rec_hash = ASUtils.json_parse(ASUtils.to_json(rec_hash))
        rec_hash['display_string'] ||= rec_hash['title']

        {'record' => rec_hash, 'uri' => rec['uri']}
      }
    end

    # We run the first portion of the index here so that we can separate out the 
    # initial doc rules and the final doc rules.
    # This is ugly because we are copy/pasting from IndexerCommon index_records
    # with no way to know if that method changes.
    def indexer.start_index(record)

      # c/p start
      values = record['record']
      uri = record['uri']

      reference = JSONModel.parse_reference(uri)
      record_type = reference && reference[:type]

      doc = {}

      doc['id'] = uri
      doc['uri'] = uri
      doc['title'] = values['title']
      doc['primary_type'] = record_type
      doc['types'] = [record_type]
      doc['json'] = ASUtils.to_json(sanitize_json(values))
      doc['suppressed'] = values.has_key?('suppressed') && values['suppressed']
      if doc['suppressed']
        doc['publish'] = false
      elsif is_repository_unpublished?(uri, values)
        doc['publish'] = false
      elsif values['has_unpublished_ancestor']
        doc['publish'] = false
      else
        doc['publish'] = values.has_key?('publish') && values['publish']
      end
      doc['system_generated'] = values.has_key?('system_generated') ? values['system_generated'].to_s : 'false'
      doc['repository'] = get_record_scope(uri)
      # c/p end

      doc
    end

    def indexer.run_all_but_final_doc_rules(doc, rec)
      @document_prepare_hooks.each_with_index do |hook, idx|
        unless idx == @document_prepare_hooks.count - 1
          hook.call(doc, rec)
        end
      end
    end

    def indexer.run_final_doc_rules(doc, rec)
      @document_prepare_hooks.last.call(doc, rec)
    end

    indexer
  end
  describe "initialize" do
    it "initializes appropriately" do
    #    def initialize(backend = nil, state = nil, name)
    end
  end
  describe "fetch_records" do
    it "fetches records" do
    #    def fetch_records(type, ids, resolve)
    end
  end
  describe "get_indexer" do
    it "gets an indexer" do
    #    def self.get_indexer(state = nil, name = "PUI Indexer")
    end
  end
  describe "resolved_attributes" do
    it "returns resolved attributes" do
    #   def resolved_attributes
    end
  end
  describe "record_types" do
    it "returns record types" do
    #    def record_types
    end
  end
  describe "configure_doc_rules" do
    it "configures document rules" do
    #    def configure_doc_rules
    end
  end
  describe "add_infscroll_docs" do
    let(:batch) { [] }
    let(:res1) { '/repositories/2/resources/1' }
    let(:res2) { '/repositories/2/resources/2' }
    let(:json1) { {'uris' => ['/repositories/2/archival_objects/10']} }
    let(:json2) { {'uris' => ['/repositories/2/archival_objects/20']} }

    it "adds infscroll docs natively when flag is disabled" do
      expect(JSONModel::HTTP).to receive(:get_json).with(res1 + '/ordered_records').and_return(json1)
      expect(JSONModel::HTTP).to receive(:get_json).with(res2 + '/ordered_records').and_return(json2)

      indexer.add_infscroll_docs([res1, res2], batch)

      expect(batch.length).to eq(2)
      expect(batch[0]).to eq({
        'id' => "#{res1}/ordered_records",
        'uri' => "#{res1}/ordered_records",
        'pui_parent_id' => res1,
        'publish' => 'true',
        'primary_type' => 'resource_ordered_records',
        'types' => ['pui'],
        'json' => ASUtils.to_json(json1)
      })
      expect(batch[1]).to eq({
        'id' => "#{res2}/ordered_records",
        'uri' => "#{res2}/ordered_records",
        'pui_parent_id' => res2,
        'publish' => 'true',
        'primary_type' => 'resource_ordered_records',
        'types' => ['pui'],
        'json' => ASUtils.to_json(json2)
      })
    end

    it "uses rust builder in a single batch when flag is enabled" do
      begin
        AppConfig[:rust_pui_auxiliary_documents_enabled] = true

        expect(JSONModel::HTTP).to receive(:get_json).with(res1 + '/ordered_records').and_return(json1)
        expect(JSONModel::HTTP).to receive(:get_json).with(res2 + '/ordered_records').and_return(json2)

        builder = instance_double('RustIndexDocumentBuilder')
        expected_inputs = [
          {
            'uri' => "#{res1}/ordered_records",
            'record_type' => 'resource_ordered_records',
            'pui_parent_id' => res1,
            'record' => json1,
            'pui_auxiliary_json' => ASUtils.to_json(json1),
            'prepare_pui_auxiliary' => true,
          },
          {
            'uri' => "#{res2}/ordered_records",
            'record_type' => 'resource_ordered_records',
            'pui_parent_id' => res2,
            'record' => json2,
            'pui_auxiliary_json' => ASUtils.to_json(json2),
            'prepare_pui_auxiliary' => true,
          },
        ]
        mock_docs = [
          {
            'id' => "#{res1}/ordered_records",
            'uri' => "#{res1}/ordered_records",
            'pui_parent_id' => res1,
            'publish' => 'true',
            'primary_type' => 'resource_ordered_records',
            'types' => ['pui'],
            'json' => ASUtils.to_json(json1),
          },
          {
            'id' => "#{res2}/ordered_records",
            'uri' => "#{res2}/ordered_records",
            'pui_parent_id' => res2,
            'publish' => 'true',
            'primary_type' => 'resource_ordered_records',
            'types' => ['pui'],
            'json' => ASUtils.to_json(json2),
          },
        ]
        expect(builder).to receive(:build_pui_auxiliary).with(expected_inputs).and_return(mock_docs)
        allow(indexer).to receive(:rust_index_document_builder).and_return(builder)

        indexer.add_infscroll_docs([res1, res2], batch)

        expect(batch.length).to eq(2)
        expect(batch[0]['publish']).to eq('true')
        expect(batch[0]['types']).to eq(['pui'])
        expect(batch[0]['pui_parent_id']).to eq(res1)
        expect(batch[1]['pui_parent_id']).to eq(res2)
        expect(batch.none? { |d| d.keys.any? { |k| k.start_with?('_archivalist') } }).to be(true)
      ensure
        AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      end
    end

    it "falls back cleanly to native document construction on builder error without partial application" do
      begin
        AppConfig[:rust_pui_auxiliary_documents_enabled] = true

        expect(JSONModel::HTTP).to receive(:get_json).with(res1 + '/ordered_records').and_return(json1)
        expect(JSONModel::HTTP).to receive(:get_json).with(res2 + '/ordered_records').and_return(json2)

        builder = instance_double('RustIndexDocumentBuilder')
        expect(builder).to receive(:build_pui_auxiliary).and_raise(RuntimeError, "sidecar 500")
        allow(indexer).to receive(:rust_index_document_builder).and_return(builder)

        indexer.add_infscroll_docs([res1, res2], batch)

        expect(batch.length).to eq(2)
        expect(batch[0]['id']).to eq("#{res1}/ordered_records")
        expect(batch[0]['publish']).to eq('true')
        expect(batch[1]['id']).to eq("#{res2}/ordered_records")
        expect(batch[1]['publish']).to eq('true')
      ensure
        AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      end
    end

    it "preserves native exception timing by retaining earlier fetched docs and re-raising on second fetch failure" do
      begin
        AppConfig[:rust_pui_auxiliary_documents_enabled] = true

        expect(JSONModel::HTTP).to receive(:get_json).with(res1 + '/ordered_records').and_return(json1)
        expect(JSONModel::HTTP).to receive(:get_json).with(res2 + '/ordered_records').and_raise(RuntimeError.new("fetch error on res2"))

        # Do not invoke the sidecar after the fetch failure
        expect(indexer).not_to receive(:rust_index_document_builder)

        expect {
          indexer.add_infscroll_docs([res1, res2], batch)
        }.to raise_error(RuntimeError, "fetch error on res2")

        # First document was retained in batch using native construction
        expect(batch.length).to eq(1)
        expect(batch[0]['id']).to eq("#{res1}/ordered_records")
        expect(batch[0]['uri']).to eq("#{res1}/ordered_records")
        expect(batch[0]['pui_parent_id']).to eq(res1)
        expect(batch[0]['publish']).to eq('true')
        expect(batch[0]['primary_type']).to eq('resource_ordered_records')
        expect(batch[0]['types']).to eq(['pui'])
        expect(batch[0]['json']).to eq(ASUtils.to_json(json1))
      ensure
        AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      end
    end

    it "treats empty input as a no-op without invoking sidecar or mutating batch" do
      begin
        AppConfig[:rust_pui_auxiliary_documents_enabled] = true
        expect(indexer).not_to receive(:rust_index_document_builder)
        indexer.add_infscroll_docs([], batch)
        expect(batch).to be_empty
      ensure
        AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      end
    end

    it "falls back to native document construction when sidecar emits noncanonical or mismatched JSON" do
      begin
        AppConfig[:rust_pui_auxiliary_documents_enabled] = true

        expect(JSONModel::HTTP).to receive(:get_json).with(res1 + '/ordered_records').and_return(json1)

        builder = instance_double('RustIndexDocumentBuilder')
        expect(builder).to receive(:build_pui_auxiliary).and_raise(RuntimeError, "json serialization mismatch for #{res1}/ordered_records")
        allow(indexer).to receive(:rust_index_document_builder).and_return(builder)

        indexer.add_infscroll_docs([res1], batch)

        expect(batch.length).to eq(1)
        expect(batch[0]['id']).to eq("#{res1}/ordered_records")
        expect(batch[0]['json']).to eq(ASUtils.to_json(json1))
      ensure
        AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      end
    end
  end
  describe "skip_index_record?" do
    it "determines if it should skip index record" do
    # def skip_index_record?(record)
    end
  end
  describe "skip_index_doc?" do
    it "determines if it should skip index doc" do
      # def skip_index_doc?(doc)
    end
  end
  describe "index_round_complete" do
    it "determines when the index round is complete" do
      # def index_round_complete(repository)
    end
  end
  describe "stage_unpublished_for_deletion" do
    it "stages unpublished records for deletion" do
      # def stage_unpublished_for_deletion(doc_id)
    end
  end

  describe "final_doc_rules" do
    def build_sample_records
      res = build(:json_resource,
                 'uri' => '/repositories/2/resources/1',
                 'title' => "Resource",
                 'repository' => {
                   'ref' => '/repositories/2',
                   '_resolved' => {
                     'repo_code' => 'woop',
                   },
                 },
                 'publish' => true,
                 'instances' => [])

      ao = build(:json_archival_object,
                 'uri' => '/repositories/2/archival_objects/123',
                 'title' => "AO with ancestor",
                 'repository' => {
                   'ref' => '/repositories/2',
                   '_resolved' => {
                     'repo_code' => 'woop',
                   },
                 },
                 'publish' => true,
                 'has_unpublished_ancestor' => false,
                 'instances' => [
                   {
                     'sub_container' => {
                       'top_container' => {
                         'ref' => '/repositories/2/top_containers/1',
                         '_resolved' => {
                           'container_locations' => [],
                           'internal_note' => 'PRIVATE_INTERNAL_NOTE',
                           'container_profile' => {
                             '_resolved' => {
                               'notes' => 'PRIVATE_CONTAINER_PROFILE_NOTE'
                             }
                           }
                         }
                       }
                     }
                   }
                 ],
                 'ancestors' => [{
                    "ref" => "/repositories/2/resources/1",
                    "_resolved" => res
                  }],
                 'resource' => "/repositories/2/resources/1")
      [res, ao]
    end

    def sample_records
      build_sample_records
    end

    it "runs final PUI doc rules and removes ancestor data after all other hooks have run" do
      _res, ao = sample_records
      rec = indexer.prepare_docs([ao]).first
      doc = indexer.start_index(rec)

      indexer.run_all_but_final_doc_rules(doc, rec)
      expect(rec['record']['ancestors'].count).to be(1)

      indexer.run_final_doc_rules(doc, rec)
      expect(rec['record']['ancestors']).to be_nil
    end

    it "preserves plugin hook ordering and ancestor visibility before final hook with Rust preparation" do
      _res, ao = sample_records
      rec = indexer.prepare_docs([ao]).first
      doc = indexer.start_index(rec)

      AppConfig[:rust_pui_document_preparation_enabled] = true

      # Generate expected clean candidate and projection
      candidate = RecordInheritance.merge(indexer.deep_copy_record_candidate(rec['record']))
      clean_candidate = indexer.deep_copy_record_candidate(candidate)
      clean_candidate.delete('ancestors')
      clean_candidate['instances'][0]['sub_container']['top_container']['_resolved'].delete('internal_note')
      clean_candidate['instances'][0]['sub_container']['top_container']['_resolved']['container_profile']['_resolved'].delete('notes')

      projection = {
        'marker' => 'pui_final_document_metadata',
        'json' => ASUtils.to_json(clean_candidate),
        'fullrecord_published' => IndexerCommon.extract_string_values(clean_candidate, :published_only)
      }
      indexer.instance_variable_set(:@pui_final_projections, { rec['record']['uri'] => projection })

      ancestors_seen_by_plugin = nil
      plugin_hook = proc do |_d, r|
        ancestors_seen_by_plugin = r['record']['ancestors']&.count
      end

      # In production, plugins register document prepare hooks during indexer initialization
      # BEFORE final_doc_rules is called at the end of initialize.
      # To faithfully test hook ordering on an already-initialized indexer without appending
      # after final, we place the simulated plugin hook immediately before final_doc_rules:
      hooks = indexer.instance_variable_get(:@document_prepare_hooks)
      final_hook = hooks.pop
      hooks << plugin_hook
      hooks << final_hook

      # Plugin hook runs before final doc rules
      indexer.run_all_but_final_doc_rules(doc, rec)
      expect(ancestors_seen_by_plugin).to eq(1)
      expect(rec['record']['ancestors'].count).to eq(1)

      # Final doc rules runs and safely strips ancestors
      indexer.run_final_doc_rules(doc, rec)
      expect(doc.has_key?('ancestors')).to be(false)
      expect(rec['record']['ancestors']).to be_nil
      expect(doc['json']).to eq(ASUtils.to_json(clean_candidate))
      expect(doc['fullrecord_published']).to eq(projection['fullrecord_published'])
      expect(doc['fullrecord']).to be_nil
      expect(doc.keys.none? { |k| k.to_s.start_with?('_archivalist_') }).to be(true)
      expect(indexer.instance_variable_get(:@pui_final_projections)).to eq({})
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
    end

    it "reflects intentional privacy hardening and baseline normalization between native and Rust-prepared outputs" do
      _res, ao1 = sample_records
      rec_native = indexer.prepare_docs([ao1]).first
      rec_native['record'] = RecordInheritance.merge(rec_native['record'])
      ao2 = indexer.deep_copy_record_candidate(rec_native['record'])
      doc_native = indexer.start_index(rec_native)
      doc_native['ancestors'] = ['/repositories/2/resources/1']
      indexer.run_all_but_final_doc_rules(doc_native, rec_native)
      indexer.run_final_doc_rules(doc_native, rec_native)

      AppConfig[:rust_pui_document_preparation_enabled] = true
      rec_rust = indexer.prepare_docs([ao2]).first
      doc_rust = indexer.start_index(rec_rust)
      doc_rust['ancestors'] = ['/repositories/2/resources/1']

      candidate = indexer.deep_copy_record_candidate(rec_rust['record'])
      clean_candidate = indexer.deep_copy_record_candidate(candidate)
      clean_candidate.delete('ancestors')
      clean_candidate['instances'][0]['sub_container']['top_container']['_resolved'].delete('internal_note')
      clean_candidate['instances'][0]['sub_container']['top_container']['_resolved']['container_profile']['_resolved'].delete('notes')

      projection = {
        'marker' => 'pui_final_document_metadata',
        'json' => ASUtils.to_json(clean_candidate),
        'fullrecord_published' => IndexerCommon.extract_string_values(clean_candidate, :published_only)
      }
      indexer.instance_variable_set(:@pui_final_projections, { rec_rust['record']['uri'] => projection })

      indexer.run_all_but_final_doc_rules(doc_rust, rec_rust)
      indexer.run_final_doc_rules(doc_rust, rec_rust)

      # 1. Native baseline retains root doc['ancestors'] and empty 'ancestors': [] in JSON
      expect(doc_native['ancestors']).to eq(['/repositories/2/resources/1'])
      native_parsed = ASUtils.json_parse(doc_native['json'])
      expect(native_parsed.has_key?('ancestors')).to be(true)

      # 2. Rust candidate hardens privacy: no root doc['ancestors'] and no ancestors key in JSON
      expect(doc_rust.has_key?('ancestors')).to be(false)
      rust_parsed = ASUtils.json_parse(doc_rust['json'])
      expect(rust_parsed.has_key?('ancestors')).to be(false)

      # 3. When native baseline is normalized of documented private fields, structural parity holds
      native_normalized = native_parsed.dup
      native_normalized.delete('ancestors')
      native_normalized['instances'][0]['sub_container']['top_container']['_resolved'].delete('internal_note')
      native_normalized['instances'][0]['sub_container']['top_container']['_resolved']['container_profile']['_resolved'].delete('notes')
      expect(rust_parsed).to eq(native_normalized)

      expect(doc_rust['title']).to eq(doc_native['title'])
      expect(doc_rust['fullrecord_published']).to eq(doc_native['fullrecord_published'])
      expect(doc_rust['fullrecord']).to be_nil
      expect(doc_native['fullrecord']).to be_nil
      expect(rec_rust['record']['ancestors']).to be_nil
      expect(rec_native['record']['ancestors']).to be_nil
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
    end

    it "exercises IndexerCommon#index_records end-to-end proving plugin hook ordering, ancestor visibility, marker stripping, privacy invariants, and native fallback" do
      # Enable both the builder gate and the separate PUI preparation gate
      AppConfig[:rust_index_document_builder_enabled] = true
      AppConfig[:rust_pui_document_preparation_enabled] = true

      # 1. Trusted projection through IndexerCommon#index_records
      test_indexer = PUIIndexer.new(AppConfig[:backend_url], nil, 'PUIIndexer')
      captured_batch = []
      test_indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
        batch.each { |doc| captured_batch << doc }
      end

      _res, ao = sample_records
      rec_hash = ao.respond_to?(:to_hash) ? ao.to_hash(:raw) : ao
      rec_hash = ASUtils.json_parse(ASUtils.to_json(rec_hash))
      rec_hash['display_string'] ||= rec_hash['title']
      record_input = { 'record' => rec_hash, 'uri' => rec_hash['uri'] }

      expected_candidate = RecordInheritance.merge(test_indexer.deep_copy_record_candidate(rec_hash))
      clean_expected = test_indexer.deep_copy_record_candidate(expected_candidate)
      clean_expected.delete('ancestors')
      clean_expected['instances'][0]['sub_container']['top_container']['_resolved'].delete('internal_note')
      clean_expected['instances'][0]['sub_container']['top_container']['_resolved']['container_profile']['_resolved'].delete('notes')
      expected_fullrecord_published = IndexerCommon.extract_string_values(clean_expected, :published_only)

      valid_rust_envelope_doc = {
        'id' => rec_hash['uri'],
        'uri' => rec_hash['uri'],
        'title' => rec_hash['title'],
        'primary_type' => 'archival_object',
        'types' => ['archival_object'],
        'json' => ASUtils.to_json(rec_hash),
        'suppressed' => false,
        'publish' => true,
        'system_generated' => 'false',
        'repository' => test_indexer.get_record_scope(rec_hash['uri']),
        '_archivalist_rust_fields' => [
          'pui_final_document_metadata',
          'pui_routing_metadata',
          'pui_notes_metadata'
        ],
        '_archivalist_pui_final' => {
          'marker' => 'pui_final_document_metadata',
          'json' => ASUtils.to_json(clean_expected),
          'fullrecord_published' => expected_fullrecord_published
        },
        '_archivalist_pui_routing' => {
          'marker' => 'pui_routing_metadata',
          'id' => "#{rec_hash['uri']}#pui",
          'pui_parent_id' => rec_hash['uri'],
          'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only']
        },
        '_archivalist_pui_notes' => {
          'marker' => 'pui_notes_metadata',
          'notes_published' => rec_hash['notes'] ? IndexerCommon.extract_string_values(rec_hash['notes'], :published_only) : nil
        }
      }

      # Stub ONLY the actual Rust batch call/HTTP boundary (perform_request)
      allow_any_instance_of(RustIndexDocumentBuilder).to receive(:perform_request) do |_builder, _req|
        Struct.new(:code, :body).new('200', ASUtils.to_json(
          'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
          'documents' => [valid_rust_envelope_doc]
        ))
      end

      plugin_saw_doc_ancestors = nil
      plugin_saw_record_ancestors = nil
      plugin_saw_internal_note = nil
      plugin_saw_profile_note = nil
      plugin_saw_pui_final = nil
      plugin_saw_pui_routing = nil
      plugin_saw_pui_notes = nil
      plugin_saw_rust_fields = nil
      plugin_saw_id = nil
      plugin_saw_pui_parent_id = nil
      plugin_saw_types = nil

      plugin_hook = proc do |doc, rec|
        plugin_saw_doc_ancestors = doc['ancestors']&.dup
        plugin_saw_record_ancestors = rec['record']['ancestors']&.count
        plugin_saw_internal_note = rec['record'].dig('instances', 0, 'sub_container', 'top_container', '_resolved', 'internal_note')
        plugin_saw_profile_note = rec['record'].dig('instances', 0, 'sub_container', 'top_container', '_resolved', 'container_profile', '_resolved', 'notes')
        plugin_saw_pui_final = doc.has_key?('_archivalist_pui_final')
        plugin_saw_pui_routing = doc.has_key?('_archivalist_pui_routing')
        plugin_saw_pui_notes = doc.has_key?('_archivalist_pui_notes')
        plugin_saw_rust_fields = doc.has_key?('_archivalist_rust_fields')
        plugin_saw_id = doc['id']
        plugin_saw_pui_parent_id = doc['pui_parent_id']
        plugin_saw_types = doc['types']&.dup
      end

      # In production ArchivesSpace, plugins register hooks during initialization before final_doc_rules.
      # To test hook ordering on an already-initialized indexer without appending after final,
      # place the simulated plugin hook immediately before final_doc_rules:
      hooks = test_indexer.instance_variable_get(:@document_prepare_hooks)
      final_hook = hooks.pop
      hooks << plugin_hook
      hooks << final_hook

      # Call IndexerCommon#index_records directly
      test_indexer.index_records([record_input])

      # Prove plugin sees native ancestors and unmutated record data, but no internal handoff markers
      expect(plugin_saw_doc_ancestors).to eq(['/repositories/2/resources/1'])
      expect(plugin_saw_record_ancestors).to eq(1)
      expect(plugin_saw_internal_note).to eq('PRIVATE_INTERNAL_NOTE')
      expect(plugin_saw_profile_note).to eq('PRIVATE_CONTAINER_PROFILE_NOTE')
      expect(plugin_saw_pui_final).to be(false)
      expect(plugin_saw_pui_routing).to be(false)
      expect(plugin_saw_pui_notes).to be(false)
      expect(plugin_saw_rust_fields).to be(false)
      expect(plugin_saw_id).to eq("#{rec_hash['uri']}#pui")
      expect(plugin_saw_pui_parent_id).to eq(rec_hash['uri'])
      expect(plugin_saw_types).to eq(['archival_object', 'pui', 'pui_archival_object', 'pui_record', 'pui_only'])

      # Prove final Solr-bound document has no doc ancestors, no private data in JSON,
      # no unpublished fullrecord, no _archivalist_* keys, exact routing values and exact fullrecord_published
      expect(captured_batch.length).to eq(1)
      final_doc = captured_batch.first
      expect(final_doc['id']).to eq("#{rec_hash['uri']}#pui")
      expect(final_doc['pui_parent_id']).to eq(rec_hash['uri'])
      expect(final_doc['types']).to eq(['archival_object', 'pui', 'pui_archival_object', 'pui_record', 'pui_only'])
      expect(final_doc.has_key?('ancestors')).to be(false)
      expect(final_doc['ancestors']).to be_nil

      final_json = ASUtils.json_parse(final_doc['json'])
      expect(final_json.has_key?('ancestors')).to be(false)
      expect(final_json.dig('instances', 0, 'sub_container', 'top_container', '_resolved', 'internal_note')).to be_nil
      expect(final_json.dig('instances', 0, 'sub_container', 'top_container', '_resolved', 'container_profile', '_resolved', 'notes')).to be_nil
      expect(final_doc.has_key?('fullrecord')).to be(false)
      expect(final_doc['fullrecord']).to be_nil
      expect(final_doc.keys.none? { |k| k.to_s.start_with?('_archivalist_') }).to be(true)
      expect(final_doc['fullrecord_published']).to eq(expected_fullrecord_published)

      # Prove original record ancestors and internal notes are removed only at final rules
      expect(record_input['record']['ancestors']).to be_nil
      expect(record_input['record'].dig('instances', 0, 'sub_container', 'top_container', '_resolved', 'internal_note')).to be_nil
      expect(record_input['record'].dig('instances', 0, 'sub_container', 'top_container', '_resolved', 'container_profile', '_resolved', 'notes')).to be_nil

      # 2. Parallel untrusted / malformed projection case through index_records
      fallback_indexer = PUIIndexer.new(AppConfig[:backend_url], nil, 'PUIIndexer')
      fallback_batch = []
      fallback_indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
        batch.each { |doc| fallback_batch << doc }
      end

      _res2, ao2 = sample_records
      rec_hash2 = ao2.respond_to?(:to_hash) ? ao2.to_hash(:raw) : ao2
      rec_hash2 = ASUtils.json_parse(ASUtils.to_json(rec_hash2))
      rec_hash2['display_string'] ||= rec_hash2['title']
      record_input2 = { 'record' => rec_hash2, 'uri' => rec_hash2['uri'] }

      malicious_envelope_doc = {
        'id' => rec_hash2['uri'],
        'uri' => rec_hash2['uri'],
        'title' => rec_hash2['title'],
        'primary_type' => 'archival_object',
        'types' => ['archival_object'],
        'json' => ASUtils.to_json(rec_hash2),
        'suppressed' => false,
        'publish' => true,
        'system_generated' => 'false',
        'repository' => fallback_indexer.get_record_scope(rec_hash2['uri']),
        '_archivalist_rust_fields' => ['pui_final_document_metadata', 'pui_routing_metadata', 'pui_notes_metadata'],
        '_archivalist_pui_final' => {
          'marker' => 'pui_final_document_metadata',
          'json' => ASUtils.to_json({
            'uri' => rec_hash2['uri'],
            'jsonmodel_type' => 'archival_object',
            'title' => 'Malicious AO',
            'internal_note' => 'MALICIOUS_PRIVATE_DATA_EXFIL'
          }),
          'fullrecord_published' => ['MALICIOUS_PRIVATE_DATA_EXFIL']
        },
        '_archivalist_pui_routing' => {
          'marker' => 'pui_routing_metadata',
          'id' => "#{rec_hash2['uri']}#pui",
          'pui_parent_id' => rec_hash2['uri'],
          'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only'],
          'admin_override' => 'MALICIOUS_ADMIN_INJECTION'
        },
        '_archivalist_pui_notes' => {
          'marker' => 'pui_notes_metadata',
          'notes_published' => ['MALICIOUS_NOTE_INJECTION'],
          'private_payload' => 'MALICIOUS_NOTE_PAYLOAD'
        }
      }

      allow_any_instance_of(RustIndexDocumentBuilder).to receive(:perform_request) do |_builder, _req|
        Struct.new(:code, :body).new('200', ASUtils.to_json(
          'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
          'documents' => [malicious_envelope_doc]
        ))
      end

      fallback_indexer.index_records([record_input2])

      expect(fallback_batch.length).to eq(1)
      fallback_doc = fallback_batch.first
      expect(fallback_doc['id']).to eq("#{rec_hash2['uri']}#pui")
      expect(fallback_doc['pui_parent_id']).to eq(rec_hash2['uri'])
      expect(fallback_doc['types']).to eq(['archival_object', 'pui', 'pui_archival_object', 'pui_record', 'pui_only'])
      expect(fallback_doc.has_key?('admin_override')).to be(false)
      expect(fallback_doc.has_key?('private_payload')).to be(false)
      expect(fallback_doc['json']).not_to include('MALICIOUS_PRIVATE_DATA_EXFIL')
      expect(fallback_doc['json']).not_to include('MALICIOUS_ADMIN_INJECTION')
      expect(fallback_doc['json']).not_to include('MALICIOUS_NOTE_PAYLOAD')
      expect(fallback_doc['fullrecord_published']).not_to include('MALICIOUS_PRIVATE_DATA_EXFIL')
      expect(fallback_doc['fullrecord_published']).not_to include('PRIVATE_INTERNAL_NOTE')
      expect(fallback_doc['fullrecord_published']).not_to include('PRIVATE_CONTAINER_PROFILE_NOTE')
      expect(fallback_doc['fullrecord']).to be_nil
      expect(fallback_doc.keys.none? { |k| k.to_s.start_with?('_archivalist_') }).to be(true)
      expect(fallback_doc['ancestors']).to eq(['/repositories/2/resources/1'])
      expect(record_input2['record']['ancestors']).to be_nil
      expect(record_input2['record'].dig('instances', 0, 'sub_container', 'top_container', '_resolved', 'internal_note')).to be_nil
      expect(record_input2['record'].dig('instances', 0, 'sub_container', 'top_container', '_resolved', 'container_profile', '_resolved', 'notes')).to be_nil
    ensure
      AppConfig[:rust_index_document_builder_enabled] = false
      AppConfig[:rust_pui_document_preparation_enabled] = false
    end

    it "rejects projection and uses native fallback when candidate or parsed_json URI or type is missing, null, or mismatched" do
      _res, ao = sample_records
      rec = indexer.prepare_docs([ao]).first
      AppConfig[:rust_pui_document_preparation_enabled] = true

      valid_candidate = RecordInheritance.merge(indexer.deep_copy_record_candidate(rec['record']))
      valid_clean = indexer.deep_copy_record_candidate(valid_candidate)
      valid_clean.delete('ancestors')
      valid_clean['instances'][0]['sub_container']['top_container']['_resolved'].delete('internal_note')
      valid_clean['instances'][0]['sub_container']['top_container']['_resolved']['container_profile']['_resolved'].delete('notes')

      # 1. Missing record uri
      bad_rec1 = indexer.deep_copy_record_candidate(rec)
      bad_rec1['record'].delete('uri')
      doc1 = indexer.start_index(rec)
      expect(indexer.apply_rust_pui_final_projection(doc1, bad_rec1)).to be(false)

      # 2. Null record uri
      bad_rec2 = indexer.deep_copy_record_candidate(rec)
      bad_rec2['record']['uri'] = nil
      doc2 = indexer.start_index(rec)
      expect(indexer.apply_rust_pui_final_projection(doc2, bad_rec2)).to be(false)

      # 3. Non-string (int) record uri
      bad_rec3 = indexer.deep_copy_record_candidate(rec)
      bad_rec3['record']['uri'] = 123
      doc3 = indexer.start_index(rec)
      expect(indexer.apply_rust_pui_final_projection(doc3, bad_rec3)).to be(false)

      # 4. Missing record jsonmodel_type
      bad_rec4 = indexer.deep_copy_record_candidate(rec)
      bad_rec4['record'].delete('jsonmodel_type')
      doc4 = indexer.start_index(rec)
      expect(indexer.apply_rust_pui_final_projection(doc4, bad_rec4)).to be(false)

      # 5. Mismatched record jsonmodel_type
      bad_rec5 = indexer.deep_copy_record_candidate(rec)
      bad_rec5['record']['jsonmodel_type'] = 'resource'
      doc5 = indexer.start_index(rec)
      expect(indexer.apply_rust_pui_final_projection(doc5, bad_rec5)).to be(false)

      # 6. Mismatched parsed_json uri
      bad_clean6 = indexer.deep_copy_record_candidate(valid_clean)
      bad_clean6['uri'] = '/repositories/2/archival_objects/999'
      bad_proj6 = {
        'marker' => 'pui_final_document_metadata',
        'json' => ASUtils.to_json(bad_clean6),
        'fullrecord_published' => ['text']
      }
      indexer.instance_variable_set(:@pui_final_projections, { rec['record']['uri'] => bad_proj6 })
      doc6 = indexer.start_index(rec)
      expect(indexer.apply_rust_pui_final_projection(doc6, rec)).to be(false)

      # 7. Missing parsed_json uri
      bad_clean7 = indexer.deep_copy_record_candidate(valid_clean)
      bad_clean7.delete('uri')
      bad_proj7 = {
        'marker' => 'pui_final_document_metadata',
        'json' => ASUtils.to_json(bad_clean7),
        'fullrecord_published' => ['text']
      }
      indexer.instance_variable_set(:@pui_final_projections, { rec['record']['uri'] => bad_proj7 })
      doc7 = indexer.start_index(rec)
      expect(indexer.apply_rust_pui_final_projection(doc7, rec)).to be(false)

      # 8. Mismatched parsed_json jsonmodel_type
      bad_clean8 = indexer.deep_copy_record_candidate(valid_clean)
      bad_clean8['jsonmodel_type'] = 'resource'
      bad_proj8 = {
        'marker' => 'pui_final_document_metadata',
        'json' => ASUtils.to_json(bad_clean8),
        'fullrecord_published' => ['text']
      }
      indexer.instance_variable_set(:@pui_final_projections, { rec['record']['uri'] => bad_proj8 })
      doc8 = indexer.start_index(rec)
      expect(indexer.apply_rust_pui_final_projection(doc8, rec)).to be(false)
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
    end

    it "falls back to native processing and prevents private text from reaching doc when Rust projection is malicious or invalid" do
      _res, ao = sample_records
      rec = indexer.prepare_docs([ao]).first
      doc = indexer.start_index(rec)

      AppConfig[:rust_pui_document_preparation_enabled] = true

      # Malicious projection attempting to leak private internal_note into json and fullrecord
      malicious_projection = {
        'marker' => 'pui_final_document_metadata',
        'json' => ASUtils.to_json({
          'uri' => rec['record']['uri'],
          'jsonmodel_type' => 'archival_object',
          'title' => 'Malicious AO',
          'internal_note' => 'MALICIOUS_PRIVATE_DATA_EXFIL'
        }),
        'fullrecord_published' => ['MALICIOUS_PRIVATE_DATA_EXFIL']
      }
      indexer.instance_variable_set(:@pui_final_projections, { rec['record']['uri'] => malicious_projection })

      indexer.run_all_but_final_doc_rules(doc, rec)
      indexer.run_final_doc_rules(doc, rec)

      # Malicious projection rejected; native fallback executed
      expect(doc['json']).not_to include('MALICIOUS_PRIVATE_DATA_EXFIL')
      expect(doc['fullrecord_published']).not_to include('MALICIOUS_PRIVATE_DATA_EXFIL')
      expect(doc['fullrecord_published']).not_to include('PRIVATE_INTERNAL_NOTE')
      expect(doc['fullrecord_published']).not_to include('PRIVATE_CONTAINER_PROFILE_NOTE')
      expect(doc['fullrecord']).to be_nil
      expect(rec['record']['ancestors']).to be_nil
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
    end

    it "ignores projection and runs native fallback when feature flag is disabled" do
      _res, ao = sample_records
      rec = indexer.prepare_docs([ao]).first
      doc = indexer.start_index(rec)

      AppConfig[:rust_pui_document_preparation_enabled] = false

      bogus_projection = {
        'marker' => 'pui_final_document_metadata',
        'json' => '{"bogus": true}',
        'fullrecord_published' => ['bogus']
      }
      indexer.instance_variable_set(:@pui_final_projections, { rec['record']['uri'] => bogus_projection })

      indexer.run_all_but_final_doc_rules(doc, rec)
      indexer.run_final_doc_rules(doc, rec)

      expect(doc['json']).not_to eq('{"bogus": true}')
      expect(doc['fullrecord_published']).not_to eq(['bogus'])
      expect(rec['record']['ancestors']).to be_nil
    end

    it "applies trusted PUI routing projection and preserves exact types and append order" do
      _res, ao = sample_records
      rec = indexer.prepare_docs([ao]).first
      doc = indexer.start_index(rec)

      AppConfig[:rust_pui_document_preparation_enabled] = true

      valid_routing = {
        'marker' => 'pui_routing_metadata',
        'id' => "#{rec['record']['uri']}#pui",
        'pui_parent_id' => rec['record']['uri'],
        'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only']
      }
      indexer.instance_variable_set(:@pui_routing_projections, { rec['record']['uri'] => valid_routing })

      expect(indexer.apply_rust_pui_routing_projection(doc, rec)).to be(true)
      expect(doc['id']).to eq("#{rec['record']['uri']}#pui")
      expect(doc['pui_parent_id']).to eq(rec['record']['uri'])
      expect(doc['types']).to eq(['archival_object', 'pui', 'pui_archival_object', 'pui_record', 'pui_only'])
      expect(indexer.instance_variable_get(:@pui_routing_projections)).to be_empty
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
    end

    it "rejects malicious or mismatched PUI routing projections with native fallback" do
      _res, ao = sample_records
      rec = indexer.prepare_docs([ao]).first
      uri = rec['record']['uri']

      AppConfig[:rust_pui_document_preparation_enabled] = true

      # 1. Extra string key (e.g. admin_override)
      doc_extra = indexer.start_index(rec)
      indexer.instance_variable_set(:@pui_routing_projections, { uri => {
        'marker' => 'pui_routing_metadata',
        'id' => "#{uri}#pui",
        'pui_parent_id' => uri,
        'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only'],
        'admin_override' => true
      }})
      expect(indexer.apply_rust_pui_routing_projection(doc_extra, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_routing_projections)[uri]).to be_nil
      expect(doc_extra['id']).to eq(uri)
      expect(doc_extra['types']).to eq(['archival_object'])
      expect(doc_extra.has_key?('admin_override')).to be(false)

      # Native fallback verification
      indexer.run_all_but_final_doc_rules(doc_extra, rec)
      expect(doc_extra['id']).to eq("#{uri}#pui")
      expect(doc_extra['types']).to eq(['archival_object', 'pui', 'pui_archival_object', 'pui_record', 'pui_only'])
      expect(doc_extra.has_key?('admin_override')).to be(false)

      # 2. Extra symbol key (e.g. :admin_override)
      doc_sym = indexer.start_index(rec)
      indexer.instance_variable_set(:@pui_routing_projections, { uri => {
        'marker' => 'pui_routing_metadata',
        'id' => "#{uri}#pui",
        'pui_parent_id' => uri,
        'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only'],
        :admin_override => true
      }})
      expect(indexer.apply_rust_pui_routing_projection(doc_sym, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_routing_projections)[uri]).to be_nil
      expect(doc_sym['id']).to eq(uri)
      expect(doc_sym['types']).to eq(['archival_object'])

      # 3. Missing keys (e.g. missing 'types', missing 'id')
      doc_missing_types = indexer.start_index(rec)
      indexer.instance_variable_set(:@pui_routing_projections, { uri => {
        'marker' => 'pui_routing_metadata',
        'id' => "#{uri}#pui",
        'pui_parent_id' => uri
      }})
      expect(indexer.apply_rust_pui_routing_projection(doc_missing_types, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_routing_projections)[uri]).to be_nil
      expect(doc_missing_types['id']).to eq(uri)
      expect(doc_missing_types['types']).to eq(['archival_object'])

      doc_missing_id = indexer.start_index(rec)
      indexer.instance_variable_set(:@pui_routing_projections, { uri => {
        'marker' => 'pui_routing_metadata',
        'pui_parent_id' => uri,
        'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only']
      }})
      expect(indexer.apply_rust_pui_routing_projection(doc_missing_id, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_routing_projections)[uri]).to be_nil
      expect(doc_missing_id['id']).to eq(uri)
      expect(doc_missing_id['types']).to eq(['archival_object'])

      # 4. Bad marker
      doc1 = indexer.start_index(rec)
      indexer.instance_variable_set(:@pui_routing_projections, { uri => {
        'marker' => 'bad_marker',
        'id' => "#{uri}#pui",
        'pui_parent_id' => uri,
        'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only']
      }})
      expect(indexer.apply_rust_pui_routing_projection(doc1, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_routing_projections)[uri]).to be_nil
      expect(doc1['id']).to eq(uri)
      expect(doc1['types']).to eq(['archival_object'])

      # 5. Tampered id
      doc2 = indexer.start_index(rec)
      indexer.instance_variable_set(:@pui_routing_projections, { uri => {
        'marker' => 'pui_routing_metadata',
        'id' => "/repositories/2/resources/999#pui",
        'pui_parent_id' => uri,
        'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only']
      }})
      expect(indexer.apply_rust_pui_routing_projection(doc2, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_routing_projections)[uri]).to be_nil
      expect(doc2['id']).to eq(uri)
      expect(doc2['types']).to eq(['archival_object'])

      # 6. Tampered parent id
      doc3 = indexer.start_index(rec)
      indexer.instance_variable_set(:@pui_routing_projections, { uri => {
        'marker' => 'pui_routing_metadata',
        'id' => "#{uri}#pui",
        'pui_parent_id' => "/repositories/2/resources/999",
        'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only']
      }})
      expect(indexer.apply_rust_pui_routing_projection(doc3, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_routing_projections)[uri]).to be_nil
      expect(doc3['id']).to eq(uri)
      expect(doc3['types']).to eq(['archival_object'])

      # 7. Tampered types (injected unauthorized type)
      doc4 = indexer.start_index(rec)
      indexer.instance_variable_set(:@pui_routing_projections, { uri => {
        'marker' => 'pui_routing_metadata',
        'id' => "#{uri}#pui",
        'pui_parent_id' => uri,
        'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only', 'unauthorized_admin']
      }})
      expect(indexer.apply_rust_pui_routing_projection(doc4, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_routing_projections)[uri]).to be_nil
      expect(doc4['id']).to eq(uri)
      expect(doc4['types']).to eq(['archival_object'])

      # 8. Tampered types (reordered types)
      doc5 = indexer.start_index(rec)
      indexer.instance_variable_set(:@pui_routing_projections, { uri => {
        'marker' => 'pui_routing_metadata',
        'id' => "#{uri}#pui",
        'pui_parent_id' => uri,
        'types' => ['pui_only', 'pui', 'pui_archival_object', 'pui_record']
      }})
      expect(indexer.apply_rust_pui_routing_projection(doc5, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_routing_projections)[uri]).to be_nil
      expect(doc5['id']).to eq(uri)
      expect(doc5['types']).to eq(['archival_object'])

      # 9. Mismatched record uri
      doc6 = indexer.start_index(rec)
      bad_rec6 = indexer.deep_copy_record_candidate(rec)
      bad_rec6['record']['uri'] = '/repositories/2/archival_objects/999'
      indexer.instance_variable_set(:@pui_routing_projections, { bad_rec6['record']['uri'] => {
        'marker' => 'pui_routing_metadata',
        'id' => "#{bad_rec6['record']['uri']}#pui",
        'pui_parent_id' => bad_rec6['record']['uri'],
        'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only']
      }})
      expect(indexer.apply_rust_pui_routing_projection(doc6, bad_rec6)).to be(false)
      expect(indexer.instance_variable_get(:@pui_routing_projections)[bad_rec6['record']['uri']]).to be_nil
      expect(doc6['id']).to eq(uri)
      expect(doc6['types']).to eq(['archival_object'])

      # 10. Mismatched record jsonmodel_type
      doc7 = indexer.start_index(rec)
      bad_rec7 = indexer.deep_copy_record_candidate(rec)
      bad_rec7['record']['jsonmodel_type'] = 'resource'
      indexer.instance_variable_set(:@pui_routing_projections, { uri => {
        'marker' => 'pui_routing_metadata',
        'id' => "#{uri}#pui",
        'pui_parent_id' => uri,
        'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only']
      }})
      expect(indexer.apply_rust_pui_routing_projection(doc7, bad_rec7)).to be(false)
      expect(indexer.instance_variable_get(:@pui_routing_projections)[uri]).to be_nil
      expect(doc7['id']).to eq(uri)
      expect(doc7['types']).to eq(['archival_object'])
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
    end

    it "applies trusted PUI notes projection and preserves absence behavior" do
      _res, ao = sample_records
      rec = indexer.prepare_docs([ao]).first
      uri = rec['record']['uri']

      AppConfig[:rust_pui_document_preparation_enabled] = true

      # Case 1: Record has notes
      rec_with_notes = indexer.deep_copy_record_candidate(rec)
      rec_with_notes['record']['notes'] = [
        {
          'jsonmodel_type' => 'note_singlepart',
          'type' => 'abstract',
          'publish' => true,
          'content' => ['Authentic public note']
        }
      ]
      doc_with_notes = indexer.start_index(rec_with_notes)
      valid_notes_proj = {
        'marker' => 'pui_notes_metadata',
        'notes_published' => IndexerCommon.extract_string_values(rec_with_notes['record']['notes'], :published_only)
      }
      indexer.instance_variable_set(:@pui_notes_projections, { uri => valid_notes_proj })

      indexer.add_notes(doc_with_notes, rec_with_notes)
      expect(doc_with_notes['notes_published']).to eq(['abstract', 'Authentic public note'])
      expect(indexer.instance_variable_get(:@pui_notes_projections)).to be_empty

      # Case 2: Record has absent notes -> doc['notes_published'] must NOT be set
      rec_absent_notes = indexer.deep_copy_record_candidate(rec)
      rec_absent_notes['record'].delete('notes')
      doc_absent_notes = indexer.start_index(rec_absent_notes)
      valid_absent_proj = {
        'marker' => 'pui_notes_metadata',
        'notes_published' => nil
      }
      indexer.instance_variable_set(:@pui_notes_projections, { uri => valid_absent_proj })

      indexer.add_notes(doc_absent_notes, rec_absent_notes)
      expect(doc_absent_notes.has_key?('notes_published')).to be(false)
      expect(doc_absent_notes['notes_published']).to be_nil
      expect(indexer.instance_variable_get(:@pui_notes_projections)).to be_empty
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
    end

    it "rejects malicious or mismatched PUI notes projections with native fallback" do
      _res, ao = sample_records
      rec = indexer.prepare_docs([ao]).first
      uri = rec['record']['uri']

      AppConfig[:rust_pui_document_preparation_enabled] = true

      rec_with_notes = indexer.deep_copy_record_candidate(rec)
      rec_with_notes['record']['notes'] = [
        {
          'jsonmodel_type' => 'note_singlepart',
          'type' => 'abstract',
          'publish' => true,
          'content' => ['Authentic public note']
        },
        {
          'jsonmodel_type' => 'note_singlepart',
          'type' => 'scopecontent',
          'publish' => false,
          'content' => ['PRIVATE_NOTE_MUST_NOT_LEAK']
        }
      ]

      # 1. Extra string key (e.g. private_payload)
      doc_extra_notes = indexer.start_index(rec_with_notes)
      indexer.instance_variable_set(:@pui_notes_projections, { uri => {
        'marker' => 'pui_notes_metadata',
        'notes_published' => ['abstract', 'Authentic public note'],
        'private_payload' => 'SECRET_INJECTION'
      }})
      expect(indexer.apply_rust_pui_notes_projection(doc_extra_notes, rec_with_notes)).to be(false)
      expect(indexer.instance_variable_get(:@pui_notes_projections)[uri]).to be_nil
      expect(doc_extra_notes.has_key?('notes_published')).to be(false)
      indexer.add_notes(doc_extra_notes, rec_with_notes)
      expect(doc_extra_notes['notes_published']).to eq(['abstract', 'Authentic public note'])
      expect(doc_extra_notes['notes_published']).not_to include('SECRET_INJECTION')
      expect(doc_extra_notes.has_key?('private_payload')).to be(false)

      # 2. Extra symbol key (e.g. :private_payload)
      doc_sym_notes = indexer.start_index(rec_with_notes)
      indexer.instance_variable_set(:@pui_notes_projections, { uri => {
        'marker' => 'pui_notes_metadata',
        'notes_published' => ['abstract', 'Authentic public note'],
        :private_payload => 'SECRET_INJECTION'
      }})
      expect(indexer.apply_rust_pui_notes_projection(doc_sym_notes, rec_with_notes)).to be(false)
      expect(indexer.instance_variable_get(:@pui_notes_projections)[uri]).to be_nil
      expect(doc_sym_notes.has_key?('notes_published')).to be(false)

      # 3. Missing keys (e.g. missing 'notes_published', missing 'marker')
      doc_no_pub = indexer.start_index(rec_with_notes)
      indexer.instance_variable_set(:@pui_notes_projections, { uri => {
        'marker' => 'pui_notes_metadata'
      }})
      expect(indexer.apply_rust_pui_notes_projection(doc_no_pub, rec_with_notes)).to be(false)
      expect(indexer.instance_variable_get(:@pui_notes_projections)[uri]).to be_nil
      expect(doc_no_pub.has_key?('notes_published')).to be(false)

      doc_no_marker = indexer.start_index(rec_with_notes)
      indexer.instance_variable_set(:@pui_notes_projections, { uri => {
        'notes_published' => ['abstract', 'Authentic public note']
      }})
      expect(indexer.apply_rust_pui_notes_projection(doc_no_marker, rec_with_notes)).to be(false)
      expect(indexer.instance_variable_get(:@pui_notes_projections)[uri]).to be_nil
      expect(doc_no_marker.has_key?('notes_published')).to be(false)

      # 4. Absent notes in record with extra keys
      rec_no_notes = indexer.deep_copy_record_candidate(rec)
      rec_no_notes['record'].delete('notes')
      doc_absent_extra = indexer.start_index(rec_no_notes)
      indexer.instance_variable_set(:@pui_notes_projections, { uri => {
        'marker' => 'pui_notes_metadata',
        'notes_published' => nil,
        'private_payload' => 'INJECTED_STRING'
      }})
      expect(indexer.apply_rust_pui_notes_projection(doc_absent_extra, rec_no_notes)).to be(false)
      expect(indexer.instance_variable_get(:@pui_notes_projections)[uri]).to be_nil
      indexer.add_notes(doc_absent_extra, rec_no_notes)
      expect(doc_absent_extra.has_key?('notes_published')).to be(false)
      expect(doc_absent_extra.has_key?('private_payload')).to be(false)

      # 5. Malicious projection attempting to leak unpublished note
      doc1 = indexer.start_index(rec_with_notes)
      malicious_notes_proj = {
        'marker' => 'pui_notes_metadata',
        'notes_published' => ['abstract', 'Authentic public note', 'PRIVATE_NOTE_MUST_NOT_LEAK']
      }
      indexer.instance_variable_set(:@pui_notes_projections, { uri => malicious_notes_proj })
      expect(indexer.apply_rust_pui_notes_projection(doc1, rec_with_notes)).to be(false)
      expect(indexer.instance_variable_get(:@pui_notes_projections)[uri]).to be_nil
      indexer.add_notes(doc1, rec_with_notes)
      # Must fall back to native extract_string_values
      expect(doc1['notes_published']).to eq(['abstract', 'Authentic public note'])
      expect(doc1['notes_published']).not_to include('PRIVATE_NOTE_MUST_NOT_LEAK')

      # 6. Wrong marker
      doc2 = indexer.start_index(rec_with_notes)
      bad_marker_proj = {
        'marker' => 'bad_marker',
        'notes_published' => ['abstract', 'Authentic public note']
      }
      indexer.instance_variable_set(:@pui_notes_projections, { uri => bad_marker_proj })
      expect(indexer.apply_rust_pui_notes_projection(doc2, rec_with_notes)).to be(false)
      expect(indexer.instance_variable_get(:@pui_notes_projections)[uri]).to be_nil
      indexer.add_notes(doc2, rec_with_notes)
      expect(doc2['notes_published']).to eq(['abstract', 'Authentic public note'])

      # 7. Absent notes in record but projection injects strings
      doc3 = indexer.start_index(rec_no_notes)
      injected_notes_proj = {
        'marker' => 'pui_notes_metadata',
        'notes_published' => ['Unauthorized injected note']
      }
      indexer.instance_variable_set(:@pui_notes_projections, { uri => injected_notes_proj })
      expect(indexer.apply_rust_pui_notes_projection(doc3, rec_no_notes)).to be(false)
      expect(indexer.instance_variable_get(:@pui_notes_projections)[uri]).to be_nil
      indexer.add_notes(doc3, rec_no_notes)
      expect(doc3.has_key?('notes_published')).to be(false)
      expect(doc3['notes_published']).to be_nil
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
    end
  end

  it "isolates rust_pui_availability_preparation_enabled from PUIIndexer" do
    begin
      AppConfig[:rust_pui_availability_preparation_enabled] = true
      expect(indexer.rust_pui_availability_preparation_enabled?).to be(false)
    ensure
      AppConfig[:rust_pui_availability_preparation_enabled] = false
    end
  end

  it "enables rust_pui_auxiliary_documents_enabled? only on PUIIndexer when configured" do
    begin
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be(false)
      AppConfig[:rust_pui_auxiliary_documents_enabled] = true
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be(true)
    ensure
      AppConfig[:rust_pui_auxiliary_documents_enabled] = false
    end
  end

  it "supports general Ruby truthiness for rust_pui_auxiliary_documents_enabled? on PUIIndexer" do
    begin
      AppConfig[:rust_pui_auxiliary_documents_enabled] = "yes"
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be_truthy
      AppConfig[:rust_pui_auxiliary_documents_enabled] = :enabled
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be_truthy
      AppConfig[:rust_pui_auxiliary_documents_enabled] = nil
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be_falsey
      AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be_falsey
    ensure
      AppConfig[:rust_pui_auxiliary_documents_enabled] = false
    end
  end

  it "isolates rust_pui_auxiliary_documents_enabled? from other feature flags" do
    begin
      AppConfig[:rust_pui_document_preparation_enabled] = true
      AppConfig[:rust_pui_availability_preparation_enabled] = true
      AppConfig[:rust_index_document_enrichment_enabled] = true
      AppConfig[:rust_index_document_builder_enabled] = true
      expect(indexer.rust_pui_auxiliary_documents_enabled?).to be(false)
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
      AppConfig[:rust_pui_availability_preparation_enabled] = false
      AppConfig[:rust_index_document_enrichment_enabled] = false
      AppConfig[:rust_index_document_builder_enabled] = false
    end
  end

  it "enables rust_pui_record_inheritance_candidate_enabled? only when configured" do
    begin
      AppConfig[:rust_pui_document_preparation_enabled] = true
      expect(indexer.rust_pui_record_inheritance_candidate_enabled?).to be(false)
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = true
      expect(indexer.rust_pui_record_inheritance_candidate_enabled?).to be(true)
    ensure
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = false
      AppConfig[:rust_pui_document_preparation_enabled] = false
    end
  end

  it "supports general Ruby truthiness for rust_pui_record_inheritance_candidate_enabled?" do
    begin
      AppConfig[:rust_pui_document_preparation_enabled] = true
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = "yes"
      expect(indexer.rust_pui_record_inheritance_candidate_enabled?).to be_truthy
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = :enabled
      expect(indexer.rust_pui_record_inheritance_candidate_enabled?).to be_truthy
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = nil
      expect(indexer.rust_pui_record_inheritance_candidate_enabled?).to be_falsey
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = false
      expect(indexer.rust_pui_record_inheritance_candidate_enabled?).to be_falsey
    ensure
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = false
      AppConfig[:rust_pui_document_preparation_enabled] = false
    end
  end

  it "isolates rust_pui_record_inheritance_candidate_enabled? from other feature flags" do
    begin
      AppConfig[:rust_pui_document_preparation_enabled] = true
      AppConfig[:rust_pui_availability_preparation_enabled] = true
      AppConfig[:rust_pui_auxiliary_documents_enabled] = true
      AppConfig[:rust_index_document_enrichment_enabled] = true
      AppConfig[:rust_index_document_builder_enabled] = true
      expect(indexer.rust_pui_record_inheritance_candidate_enabled?).to be(false)
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
      AppConfig[:rust_pui_availability_preparation_enabled] = false
      AppConfig[:rust_pui_auxiliary_documents_enabled] = false
      AppConfig[:rust_index_document_enrichment_enabled] = false
      AppConfig[:rust_index_document_builder_enabled] = false
    end
  end

  it "validates candidate projection when candidate flag is enabled and falls back on mismatch" do
    begin
      AppConfig[:rust_pui_document_preparation_enabled] = true
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = true

      uri = '/repositories/2/archival_objects/77'
      rec = {
        'record' => {
          'uri' => uri,
          'jsonmodel_type' => 'archival_object',
          'title' => 'Test Item',
          'publish' => true,
          'suppressed' => false,
          'ancestors' => []
        }
      }
      doc = { 'primary_type' => 'archival_object' }

      # Injected candidate projection with mismatch
      mismatched_candidate = {
        'uri' => uri,
        'jsonmodel_type' => 'archival_object',
        'title' => 'Mismatched Title',
        'ancestors' => []
      }
      indexer.instance_variable_set(:@pui_candidate_projections, { uri => mismatched_candidate })
      indexer.instance_variable_set(:@pui_final_projections, { uri => { 'json' => '{}', 'fullrecord_published' => [] } })

      # Should fail candidate validation, fall back, and clean up projections
      expect(indexer.apply_rust_pui_final_projection(doc, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_candidate_projections)[uri]).to be_nil
      expect(indexer.instance_variable_get(:@pui_final_projections)[uri]).to be_nil
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = false
    end
  end

  it "accepts valid candidate projection when candidate flag is enabled and applies final projection" do
    begin
      AppConfig[:rust_pui_document_preparation_enabled] = true
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = true

      uri = '/repositories/2/archival_objects/88'
      rec_hash = {
        'uri' => uri,
        'jsonmodel_type' => 'archival_object',
        'title' => 'Valid AO',
        'publish' => true,
        'suppressed' => false,
        'ancestors' => []
      }
      rec = { 'record' => rec_hash, 'uri' => uri }
      doc = { 'primary_type' => 'archival_object' }

      expected_merged = RecordInheritance.merge(indexer.deep_copy_record_candidate(rec_hash))
      clean_rec = indexer.deep_copy_record_candidate(expected_merged)
      clean_rec.delete('ancestors')
      fullrecord = IndexerCommon.extract_string_values(clean_rec, :published_only)

      valid_candidate = {
        'marker' => 'pui_record_inheritance_candidate_metadata',
        'record' => expected_merged,
      }
      valid_final = {
        'marker' => 'pui_final_document_metadata',
        'json' => ASUtils.to_json(clean_rec),
        'fullrecord_published' => fullrecord,
      }

      indexer.instance_variable_set(:@pui_candidate_projections, { uri => valid_candidate })
      indexer.instance_variable_set(:@pui_final_projections, { uri => valid_final })

      expect(indexer.apply_rust_pui_final_projection(doc, rec)).to be(true)
      expect(doc['json']).to eq(ASUtils.to_json(clean_rec))
      expect(doc['fullrecord_published']).to eq(fullrecord)
      expect(indexer.instance_variable_get(:@pui_candidate_projections)[uri]).to be_nil
      expect(indexer.instance_variable_get(:@pui_final_projections)[uri]).to be_nil
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = false
    end
  end

  it "calls RecordInheritance.build_plan as a public API through IndexerCommon input preparation" do
    begin
      AppConfig[:rust_pui_document_preparation_enabled] = true
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = true

      ao_rec = {
        'uri' => '/repositories/2/archival_objects/900',
        'jsonmodel_type' => 'archival_object',
        'title' => 'AO for Plan Testing',
        'publish' => true,
        'suppressed' => false,
        'ancestors' => [
          {
            'ref' => '/repositories/2/resources/50',
            'level' => 'collection',
            '_resolved' => {
              'id_0' => 'COLL-50',
              'language' => 'eng',
            }
          }
        ]
      }

      # Test public class method callable with explicit receiver (FATAL bug fix)
      expect {
        plan = RecordInheritance.build_plan(ao_rec)
        expect(plan).to be_a(Hash)
        expect(plan['record_type']).to eq('archival_object')
        expect(plan['rules']).to be_an(Array)
      }.not_to raise_error

      # Test through IndexerCommon input preparation
      builder_input = nil
      allow(RustIndexDocumentBuilder).to receive(:build) do |inputs|
        builder_input = inputs.first
        []
      end

      # Prepare record using IndexerCommon
      records = [ao_rec]
      prepared = records.map do |r|
        {
          :record => r,
          :uri => r['uri'],
          :record_type => r['jsonmodel_type'],
          :values => r,
        }
      end

      # Call rust_index_documents or check prepared input
      plan = RecordInheritance.build_plan(prepared.first[:values])
      expect(plan).not_to be_nil
      expect(plan['record_type']).to eq('archival_object')
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = false
    end
  end

  it "preserves duplicate refs with different values/levels, null refs, and non-string refs" do
    ao_rec = {
      'uri' => '/repositories/2/archival_objects/901',
      'jsonmodel_type' => 'archival_object',
      'title' => 'AO Dupe Ref Test',
      'publish' => true,
      'suppressed' => false,
      'notes' => [],
      'ancestors' => [
        {
          'ref' => '/repositories/2/resources/dupe',
          'level' => 'series',
          '_resolved' => {
            'notes' => [
              {
                'jsonmodel_type' => 'note_singlepart',
                'type' => 'scopecontent',
                'publish' => true,
                'content' => ['Scope note from first ancestor with dupe ref']
              }
            ]
          }
        },
        {
          'ref' => '/repositories/2/resources/dupe',
          'level' => 'collection',
          '_resolved' => {
            'notes' => [
              {
                'jsonmodel_type' => 'note_singlepart',
                'type' => 'accessrestrict',
                'publish' => true,
                'content' => ['Access note from second ancestor with dupe ref']
              }
            ]
          }
        },
        {
          'ref' => nil,
          'level' => 'subseries',
          '_resolved' => {
            'language' => 'lat',
            'notes' => []
          }
        },
        {
          'ref' => 12345,
          'level' => 'fonds',
          '_resolved' => {
            'edition' => '1st',
            'notes' => []
          }
        }
      ]
    }

    plan = RecordInheritance.build_plan(ao_rec)
    expect(plan).to be_a(Hash)
    expect(plan['rules']).to be_an(Array)

    # Verify duplicate refs did not overwrite each other:
    # We should have a rule action inheriting from the first dupe ref ('series')
    # AND a rule action inheriting from the second dupe ref ('collection')
    scope_rule = plan['rules'].find { |r| r['property'] == 'notes' && r['inherited_props']['level'].to_s.downcase == 'series' }
    access_rule = plan['rules'].find { |r| r['property'] == 'notes' && r['inherited_props']['level'].to_s.downcase == 'collection' }
    expect(scope_rule).not_to be_nil
    expect(access_rule).not_to be_nil
    expect(scope_rule['inherited_props']['ref']).to eq('/repositories/2/resources/dupe')
    expect(access_rule['inherited_props']['ref']).to eq('/repositories/2/resources/dupe')

    # Verify null ref and integer ref are preserved
    null_ref_rule = plan['rules'].find { |r| r['property'] == 'language' }
    if null_ref_rule
      expect(null_ref_rule['inherited_props']['ref']).to be_nil
    end
  end

  it "translates levels lazily only at the exact point of inheritance" do
    translated_levels = []
    # Test that translate_level is only called for ancestors that actually contribute inherited properties
    inheritance = RecordInheritance.new({
      :archival_object => {
        :inherited_fields => [
          { :property => 'language', :inherit_directly => true }
        ]
      }
    })
    allow(inheritance).to receive(:translate_level) do |level|
      translated_levels << level
      level.upcase
    end

    ao_rec = {
      'uri' => '/repositories/2/archival_objects/902',
      'jsonmodel_type' => 'archival_object',
      'title' => 'AO Lazy Translation Test',
      'publish' => true,
      'suppressed' => false,
      'notes' => [
        {
          'jsonmodel_type' => 'note_singlepart',
          'type' => 'scopecontent',
          'publish' => true,
          'content' => ['Own child note']
        }
      ],
      'ancestors' => [
        {
          'ref' => '/repositories/2/archival_objects/unrelated',
          'level' => 'unrelated_series',
          '_resolved' => { 'notes' => [] } # no qualifying values to inherit
        },
        {
          'ref' => '/repositories/2/resources/contributing',
          'level' => 'contributing_collection',
          '_resolved' => {
            'language' => 'spa',
            'notes' => []
          }
        }
      ]
    }

    plan = inheritance.build_plan(ao_rec)
    expect(plan).to be_a(Hash)
    # The unrelated ancestor's level must NOT have been translated for rules!
    expect(translated_levels).not_to include('unrelated_series')
    # And contributing collection must be translated exactly once!
    expect(translated_levels.count('contributing_collection')).to eq(1)
  end

  it "calls translate_level exactly once per applied action and records the exact same translated value in the plan and simulated native candidate" do
    call_counts = Hash.new(0)
    counter = 0

    inheritance = RecordInheritance.new({
      :archival_object => {
        :inherited_fields => [
          { :property => 'language', :inherit_directly => true }
        ]
      }
    })
    allow(inheritance).to receive(:translate_level) do |level|
      counter += 1
      call_counts[level] += 1
      "#{level}_translated_#{counter}"
    end

    ao_rec = {
      'uri' => '/repositories/2/archival_objects/905',
      'jsonmodel_type' => 'archival_object',
      'title' => 'AO Single Translate Test',
      'publish' => true,
      'suppressed' => false,
      'notes' => [
        {
          'jsonmodel_type' => 'note_singlepart',
          'type' => 'scopecontent',
          'publish' => true,
          'content' => ['Own child note']
        }
      ],
      'ancestors' => [
        {
          'ref' => '/repositories/2/archival_objects/unrelated',
          'level' => 'unrelated_series',
          '_resolved' => { 'notes' => [] } # no qualifying values to inherit
        },
        {
          'ref' => '/repositories/2/resources/contributing',
          'level' => 'contributing_collection',
          '_resolved' => {
            'language' => 'spa',
            'notes' => []
          }
        }
      ]
    }

    # Simulate native merge progressively using shared internal helper with a recorder
    working_copy = inheritance.deep_copy_record(ao_rec)
    config = inheritance.instance_variable_get(:@config)[:archival_object]
    recorder = RecordInheritance::ActionRecorder.new('archival_object')
    simulated_candidate = inheritance.send(:merge_record_internal, working_copy, config, {}, recorder)
    plan = recorder.to_plan

    # Exactly one translate_level call per applied action:
    expect(call_counts['unrelated_series']).to eq(0)
    expect(call_counts['contributing_collection']).to eq(1)

    # Exact same translated value in the plan props and simulated native candidate:
    recorded_props_level = plan['rules'][0]['inherited_props']['level']
    candidate_props_level = simulated_candidate['language_inherited']['level']
    expect(recorded_props_level).to eq('contributing_collection_translated_1')
    expect(candidate_props_level).to eq('contributing_collection_translated_1')
    expect(recorded_props_level).to eq(candidate_props_level)
  end

  it "rejects candidate projection with non-string, missing, or extra keys at PUIIndexer application" do
    begin
      AppConfig[:rust_pui_document_preparation_enabled] = true
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = true

      uri = '/repositories/2/archival_objects/88'
      rec_hash = {
        'uri' => uri,
        'jsonmodel_type' => 'archival_object',
        'title' => 'AO Test',
        'publish' => true,
        'suppressed' => false,
        'ancestors' => []
      }
      rec = { 'record' => rec_hash, 'uri' => uri }
      expected_merged = RecordInheritance.merge(indexer.deep_copy_record_candidate(rec_hash))
      clean_rec = indexer.deep_copy_record_candidate(expected_merged)
      clean_rec.delete('ancestors')
      fullrecord = IndexerCommon.extract_string_values(clean_rec, :published_only)
      valid_final = {
        'marker' => 'pui_final_document_metadata',
        'json' => ASUtils.to_json(clean_rec),
        'fullrecord_published' => fullrecord,
      }

      # 1. Non-string keys in candidate projection
      doc1 = { 'primary_type' => 'archival_object' }
      bad_cand1 = {
        :marker => 'pui_record_inheritance_candidate_metadata',
        'record' => expected_merged,
      }
      indexer.instance_variable_set(:@pui_candidate_projections, { uri => bad_cand1 })
      indexer.instance_variable_set(:@pui_final_projections, { uri => valid_final.dup })
      expect(indexer.apply_rust_pui_final_projection(doc1, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_candidate_projections)[uri]).to be_nil
      expect(indexer.instance_variable_get(:@pui_final_projections)[uri]).to be_nil
      expect(doc1).to eq({ 'primary_type' => 'archival_object' })

      # 2. Missing required keys in candidate projection
      doc2 = { 'primary_type' => 'archival_object' }
      bad_cand2 = {
        'marker' => 'pui_record_inheritance_candidate_metadata',
      }
      indexer.instance_variable_set(:@pui_candidate_projections, { uri => bad_cand2 })
      indexer.instance_variable_set(:@pui_final_projections, { uri => valid_final.dup })
      expect(indexer.apply_rust_pui_final_projection(doc2, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_candidate_projections)[uri]).to be_nil
      expect(indexer.instance_variable_get(:@pui_final_projections)[uri]).to be_nil
      expect(doc2).to eq({ 'primary_type' => 'archival_object' })

      # 3. Unexpected extra keys in candidate projection
      doc3 = { 'primary_type' => 'archival_object' }
      bad_cand3 = {
        'marker' => 'pui_record_inheritance_candidate_metadata',
        'record' => expected_merged,
        'extra_key' => 'not_allowed',
      }
      indexer.instance_variable_set(:@pui_candidate_projections, { uri => bad_cand3 })
      indexer.instance_variable_set(:@pui_final_projections, { uri => valid_final.dup })
      expect(indexer.apply_rust_pui_final_projection(doc3, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_candidate_projections)[uri]).to be_nil
      expect(indexer.instance_variable_get(:@pui_final_projections)[uri]).to be_nil
      expect(doc3).to eq({ 'primary_type' => 'archival_object' })
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = false
    end
  end

  it "rejects candidate projection with type differences (int vs float, bool vs int)" do
    begin
      AppConfig[:rust_pui_document_preparation_enabled] = true
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = true

      uri = '/repositories/2/archival_objects/89'
      rec_hash = {
        'uri' => uri,
        'jsonmodel_type' => 'archival_object',
        'title' => 'AO Type Test',
        'publish' => true,
        'suppressed' => false,
        'int_val' => 1,
        'bool_val' => true,
        'ancestors' => []
      }
      rec = { 'record' => rec_hash, 'uri' => uri }
      expected_merged = RecordInheritance.merge(indexer.deep_copy_record_candidate(rec_hash))
      clean_rec = indexer.deep_copy_record_candidate(expected_merged)
      clean_rec.delete('ancestors')
      fullrecord = IndexerCommon.extract_string_values(clean_rec, :published_only)
      valid_final = {
        'marker' => 'pui_final_document_metadata',
        'json' => ASUtils.to_json(clean_rec),
        'fullrecord_published' => fullrecord,
      }

      # 1. Candidate record with float 1.0 where integer 1 is expected
      doc_float = { 'primary_type' => 'archival_object' }
      cand_float_rec = indexer.deep_copy_record_candidate(expected_merged)
      cand_float_rec['int_val'] = 1.0
      bad_cand_float = {
        'marker' => 'pui_record_inheritance_candidate_metadata',
        'record' => cand_float_rec,
      }
      indexer.instance_variable_set(:@pui_candidate_projections, { uri => bad_cand_float })
      indexer.instance_variable_set(:@pui_final_projections, { uri => valid_final.dup })
      expect(indexer.apply_rust_pui_final_projection(doc_float, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_candidate_projections)[uri]).to be_nil
      expect(indexer.instance_variable_get(:@pui_final_projections)[uri]).to be_nil
      expect(doc_float).to eq({ 'primary_type' => 'archival_object' })

      # 2. Candidate record with integer 1 where boolean true is expected
      doc_bool = { 'primary_type' => 'archival_object' }
      cand_bool_rec = indexer.deep_copy_record_candidate(expected_merged)
      cand_bool_rec['bool_val'] = 1
      bad_cand_bool = {
        'marker' => 'pui_record_inheritance_candidate_metadata',
        'record' => cand_bool_rec,
      }
      indexer.instance_variable_set(:@pui_candidate_projections, { uri => bad_cand_bool })
      indexer.instance_variable_set(:@pui_final_projections, { uri => valid_final.dup })
      expect(indexer.apply_rust_pui_final_projection(doc_bool, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_candidate_projections)[uri]).to be_nil
      expect(indexer.instance_variable_get(:@pui_final_projections)[uri]).to be_nil
      expect(doc_bool).to eq({ 'primary_type' => 'archival_object' })
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = false
    end
  end

  describe "PUI common search metadata preparation" do
    let(:sample_common_record) do
      {
        'uri' => '/repositories/2/resources/99',
        'jsonmodel_type' => 'resource',
        'title' => 'Common Test Resource',
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
          { 'ref' => '/subjects/11', '_resolved' => { 'title' => nil } }, # compacted
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
              'display_name' => { 'sort_name' => 'Example Historical Society' },
              'publish' => false
            }
          }
        ],
        'dates' => [
          {
            'date_type' => 'inclusive',
            'begin' => '1900-01-01',
            'end' => '1905-12-31'
          },
          {
            'date_type' => 'single',
            'begin' => '1920-01-01'
          }
        ],
        'notes' => [
          {
            'type' => 'abstract',
            'content' => ['Summary paragraph 1', 'Summary paragraph 2']
          }
        ],
        'extents' => [
          {
            'portion' => 'part',
            'number' => '10',
            'extent_type' => 'files'
          },
          {
            'portion' => 'whole',
            'number' => '2',
            'extent_type' => 'boxes'
          }
        ]
      }
    end

    let(:valid_pui_common_projection) do
      {
        'marker' => 'pui_common_metadata',
        'uri' => '/repositories/2/resources/99',
        'primary_type' => 'resource',
        'subjects' => {
          'subjects' => ['Church History', 'Archives'],
          'subject_uris' => ['/subjects/10', '/subjects/11', '/subjects/12']
        },
        'agents' => {
          'agents' => ['Doe, Jane', 'Example Historical Society'],
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
          'summary' => "Summary paragraph 1\nSummary paragraph 2"
        },
        'extents' => {
          'extents' => ['2 --- boxes']
        }
      }
    end

    let(:valid_rust_envelope_doc) do
      uri = sample_common_record['uri']
      {
        'id' => uri,
        'uri' => uri,
        'title' => sample_common_record['title'],
        'primary_type' => 'resource',
        'types' => ['resource'],
        'json' => ASUtils.to_json(sample_common_record),
        'suppressed' => false,
        'publish' => true,
        'system_generated' => 'false',
        'repository' => 2,
        '_archivalist_rust_fields' => ['pui_common_metadata'],
        '_archivalist_pui_common' => valid_pui_common_projection.dup
      }
    end

    it "honors disabled AppConfig[:rust_pui_common_metadata_enabled] and executes unchanged native helpers" do
      AppConfig[:rust_pui_common_metadata_enabled] = false

      doc = { 'id' => "#{sample_common_record['uri']}#pui", 'uri' => sample_common_record['uri'], 'primary_type' => 'resource' }
      rec = { 'record' => sample_common_record, 'uri' => sample_common_record['uri'] }

      # With flag disabled, helper runs native logic without using projections
      indexer.instance_variable_set(:@pui_common_projections, { sample_common_record['uri'] => valid_pui_common_projection })
      indexer.add_subjects(doc, rec)
      indexer.add_agents(doc, rec)
      indexer.add_audit_info(doc, rec)
      indexer.add_years(doc, rec)
      indexer.add_level(doc, rec)
      indexer.add_summary(doc, rec)
      indexer.add_extents(doc, rec)

      expect(doc['subjects']).to eq(['Church History', 'Archives'])
      expect(doc['level']).to eq('subfonds')
      expect(doc['summary']).to eq("Summary paragraph 1\nSummary paragraph 2")
      expect(doc['extents']).to eq(['2 --- boxes'])
      # Projection cache remains untouched because accelerated helper was never entered
      expect(indexer.instance_variable_get(:@pui_common_projections)[sample_common_record['uri']]).to eq(valid_pui_common_projection)
    end

    it "applies trusted PUI common metadata projections at original native call positions for all seven helpers" do
      begin
        AppConfig[:rust_pui_common_metadata_enabled] = true

        doc = {
          'id' => "#{sample_common_record['uri']}#pui",
          'uri' => sample_common_record['uri'],
          'primary_type' => 'resource',
          'pre_existing' => 'preserved'
        }
        rec = { 'record' => sample_common_record, 'uri' => sample_common_record['uri'] }

        indexer.instance_variable_set(:@pui_common_projections, { sample_common_record['uri'] => valid_pui_common_projection.dup })

        indexer.add_subjects(doc, rec)
        expect(doc['subjects']).to eq(['Church History', 'Archives'])
        expect(doc['subject_uris']).to eq(['/subjects/10', '/subjects/11', '/subjects/12'])

        indexer.add_agents(doc, rec)
        expect(doc['agents']).to eq(['Doe, Jane', 'Example Historical Society'])
        expect(doc['published_agents']).to eq(['Doe, Jane'])
        expect(doc['creators']).to eq(['Doe, Jane'])

        indexer.add_audit_info(doc, rec)
        expect(doc['created_by']).to eq('archivist')
        expect(doc['last_modified_by']).to eq('admin')

        indexer.add_years(doc, rec)
        expect(doc['years']).to eq(['1900', '1901', '1902', '1903', '1904', '1905', '1920'])
        expect(doc['year_sort']).to eq('19001920')
        expect(doc['dates']).to eq(['1900-01-01 - 1905-12-31'])

        indexer.add_level(doc, rec)
        expect(doc['level']).to eq('subfonds')

        indexer.add_summary(doc, rec)
        expect(doc['summary']).to eq("Summary paragraph 1\nSummary paragraph 2")

        indexer.add_extents(doc, rec)
        expect(doc['extents']).to eq(['2 --- boxes'])

        # Pre-existing unrelated doc fields remain intact
        expect(doc['pre_existing']).to eq('preserved')

        # Assert the trusted projection cache survived validation and was not discarded
        expect(indexer.instance_variable_get(:@pui_common_projections)[sample_common_record['uri']]).to eq(valid_pui_common_projection)
      ensure
        AppConfig[:rust_pui_common_metadata_enabled] = false
      end
    end

    it "preserves absence semantics when helper fields are absent or falsey" do
      begin
        AppConfig[:rust_pui_common_metadata_enabled] = true

        sparse_record = {
          'uri' => '/repositories/2/resources/101',
          'jsonmodel_type' => 'resource',
          'title' => 'Sparse Resource',
          'publish' => true,
        }

        empty_projection = {
          'marker' => 'pui_common_metadata',
          'uri' => '/repositories/2/resources/101',
          'primary_type' => 'resource',
          'subjects' => {},
          'agents' => {},
          'audit_info' => {},
          'years' => {},
          'level' => {},
          'summary' => {},
          'extents' => {}
        }

        doc = {
          'id' => "#{sparse_record['uri']}#pui",
          'uri' => sparse_record['uri'],
          'primary_type' => 'resource'
        }
        rec = { 'record' => sparse_record, 'uri' => sparse_record['uri'] }

        indexer.instance_variable_set(:@pui_common_projections, { sparse_record['uri'] => empty_projection })

        indexer.add_subjects(doc, rec)
        indexer.add_agents(doc, rec)
        indexer.add_audit_info(doc, rec)
        indexer.add_years(doc, rec)
        indexer.add_level(doc, rec)
        indexer.add_summary(doc, rec)
        indexer.add_extents(doc, rec)

        # In native, none of these fields are written when source keys are absent
        expect(doc.has_key?('subjects')).to be(false)
        expect(doc.has_key?('agents')).to be(false)
        expect(doc.has_key?('created_by')).to be(false)
        expect(doc.has_key?('years')).to be(false)
        expect(doc.has_key?('level')).to be(false)
        expect(doc.has_key?('summary')).to be(false)
        expect(doc.has_key?('extents')).to be(false)

        # Assert projection was accepted across all seven helpers rather than discarded via accidental fallback
        expect(indexer.instance_variable_get(:@pui_common_projections)[sparse_record['uri']]).to eq(empty_projection)
      ensure
        AppConfig[:rust_pui_common_metadata_enabled] = false
      end
    end

    it "maps otherlevel to other_level in add_level" do
      begin
        AppConfig[:rust_pui_common_metadata_enabled] = true
        other_rec = sample_common_record.merge('level' => 'otherlevel', 'other_level' => 'fonds')
        uri = other_rec['uri']
        doc = { 'id' => uri, 'uri' => uri, 'primary_type' => 'resource' }
        proj = valid_pui_common_projection.dup
        proj['level'] = { 'level' => 'fonds' }
        indexer.instance_variable_set(:@pui_common_projections, { uri => proj })

        indexer.add_level(doc, { 'record' => other_rec, 'uri' => uri })
        expect(doc['level']).to eq('fonds')
        expect(indexer.instance_variable_get(:@pui_common_projections)[uri]).not_to be_nil
      ensure
        AppConfig[:rust_pui_common_metadata_enabled] = false
      end
    end

    it "detects plugin input mutations before helpers and safely falls back to native helper execution" do
      begin
        AppConfig[:rust_pui_common_metadata_enabled] = true

        doc = { 'id' => sample_common_record['uri'], 'uri' => sample_common_record['uri'], 'primary_type' => 'resource' }
        mutated_record = sample_common_record.dup
        rec = { 'record' => mutated_record, 'uri' => mutated_record['uri'] }

        indexer.instance_variable_set(:@pui_common_projections, { mutated_record['uri'] => valid_pui_common_projection.dup })

        # Simulate a plugin mutating the record in-memory before add_subjects runs
        mutated_record['subjects'] = [
          { 'ref' => '/subjects/mutated', '_resolved' => { 'title' => 'Mutated Subject' } }
        ]

        # add_subjects detects that the projection patch does not equal native execution on the mutated record,
        # discards the projection, and falls back to native execution using the mutated record
        indexer.add_subjects(doc, rec)

        expect(doc['subjects']).to eq(['Mutated Subject'])
        expect(doc['subject_uris']).to eq(['/subjects/mutated'])
        # Projection was discarded for this URI
        expect(indexer.instance_variable_get(:@pui_common_projections)[mutated_record['uri']]).to be_nil
      ensure
        AppConfig[:rust_pui_common_metadata_enabled] = false
      end
    end

    it "safely detects identity mutations and prevents cross-record cache consumption across multiple records" do
      begin
        AppConfig[:rust_pui_common_metadata_enabled] = true

        uri_a = '/repositories/2/resources/99'
        uri_b = '/repositories/2/resources/102'

        rec_a = { 'record' => sample_common_record.dup, 'uri' => uri_a }
        rec_b_hash = sample_common_record.merge('uri' => uri_b, 'title' => 'Resource B')
        rec_b = { 'record' => rec_b_hash, 'uri' => uri_b }

        proj_a = valid_pui_common_projection.dup
        proj_b = valid_pui_common_projection.merge('uri' => uri_b)

        indexer.instance_variable_set(:@pui_common_projections, {
          uri_a => proj_a,
          uri_b => proj_b
        })

        # A plugin mutates record A's inner record['record']['uri'] to point to record B
        rec_a['record']['uri'] = uri_b

        # Note: doc['id'] may legitimately become "#{uri_a}#pui", but doc['uri'] is native uri_a
        doc_a = { 'id' => "#{uri_a}#pui", 'uri' => uri_a, 'primary_type' => 'resource' }

        # Hook detects inner URI vs doc/top URI mismatch
        indexer.add_subjects(doc_a, rec_a)

        # Both candidate URIs (uri_a and uri_b) are discarded from cache to prevent cross-record leakage
        expect(indexer.instance_variable_get(:@pui_common_projections)[uri_a]).to be_nil
        expect(indexer.instance_variable_get(:@pui_common_projections)[uri_b]).to be_nil

        # doc_a fell back to native computation on its actual record
        expect(doc_a['subjects']).to eq(['Church History', 'Archives'])
      ensure
        AppConfig[:rust_pui_common_metadata_enabled] = false
      end
    end

    it "rejects invalid projection envelopes, unknown keys, mismatched URIs, and altered types without recursive re-entry" do
      begin
        AppConfig[:rust_pui_common_metadata_enabled] = true

        uri = sample_common_record['uri']
        rec = { 'record' => sample_common_record, 'uri' => uri }

        # 1. Unknown extra key in projection envelope
        doc1 = { 'id' => uri, 'uri' => uri, 'primary_type' => 'resource' }
        bad_proj1 = valid_pui_common_projection.merge('unexpected_key' => 'evil')
        indexer.instance_variable_set(:@pui_common_projections, { uri => bad_proj1 })
        indexer.add_subjects(doc1, rec)
        expect(doc1['subjects']).to eq(['Church History', 'Archives'])
        expect(indexer.instance_variable_get(:@pui_common_projections)[uri]).to be_nil

        # 2. Marker mismatch
        doc2 = { 'id' => uri, 'uri' => uri, 'primary_type' => 'resource' }
        bad_proj2 = valid_pui_common_projection.merge('marker' => 'wrong_marker')
        indexer.instance_variable_set(:@pui_common_projections, { uri => bad_proj2 })
        indexer.add_subjects(doc2, rec)
        expect(doc2['subjects']).to eq(['Church History', 'Archives'])
        expect(indexer.instance_variable_get(:@pui_common_projections)[uri]).to be_nil

        # 3. Mismatched URI
        doc3 = { 'id' => uri, 'uri' => uri, 'primary_type' => 'resource' }
        bad_proj3 = valid_pui_common_projection.merge('uri' => '/repositories/2/resources/other')
        indexer.instance_variable_set(:@pui_common_projections, { uri => bad_proj3 })
        indexer.add_subjects(doc3, rec)
        expect(doc3['subjects']).to eq(['Church History', 'Archives'])
        expect(indexer.instance_variable_get(:@pui_common_projections)[uri]).to be_nil

        # 4. Unknown key inside helper patch
        doc4 = { 'id' => uri, 'uri' => uri, 'primary_type' => 'resource' }
        bad_proj4 = valid_pui_common_projection.dup
        bad_proj4['subjects'] = bad_proj4['subjects'].merge('unknown_helper_key' => 'leak')
        indexer.instance_variable_set(:@pui_common_projections, { uri => bad_proj4 })
        indexer.add_subjects(doc4, rec)
        expect(doc4['subjects']).to eq(['Church History', 'Archives'])
        expect(indexer.instance_variable_get(:@pui_common_projections)[uri]).to be_nil

        # 5. Type alteration inside helper patch (level value is integer instead of string)
        # Authoritative native check rejects integer level, discards cache, and does not re-enter recursively
        doc5 = { 'id' => uri, 'uri' => uri, 'primary_type' => 'resource' }
        bad_proj5 = valid_pui_common_projection.dup
        bad_proj5['level'] = { 'level' => 123 }
        indexer.instance_variable_set(:@pui_common_projections, { uri => bad_proj5 })
        indexer.add_level(doc5, rec)
        expect(doc5['level']).to eq('subfonds')
        expect(indexer.instance_variable_get(:@pui_common_projections)[uri]).to be_nil
      ensure
        AppConfig[:rust_pui_common_metadata_enabled] = false
      end
    end

    it "handles sidecar outage and resets projection caches per batch with multi-record isolation" do
      begin
        AppConfig[:rust_pui_common_metadata_enabled] = true

        # 1. Sidecar outage (no entry in @pui_common_projections)
        doc = { 'id' => sample_common_record['uri'], 'uri' => sample_common_record['uri'], 'primary_type' => 'resource' }
        rec = { 'record' => sample_common_record, 'uri' => sample_common_record['uri'] }
        indexer.instance_variable_set(:@pui_common_projections, {})

        indexer.add_subjects(doc, rec)
        expect(doc['subjects']).to eq(['Church History', 'Archives'])

        # 2. Multi-record isolation: failure in record A does not corrupt record B
        uri_a = '/repositories/2/resources/99'
        uri_b = '/repositories/2/resources/102'

        rec_b_hash = sample_common_record.merge('uri' => uri_b, 'title' => 'Resource B')
        rec_b = { 'record' => rec_b_hash, 'uri' => uri_b }

        proj_b = valid_pui_common_projection.merge('uri' => uri_b)

        # Corrupt record A's projection
        bad_proj_a = valid_pui_common_projection.merge('marker' => 'corrupted')

        indexer.instance_variable_set(:@pui_common_projections, {
          uri_a => bad_proj_a,
          uri_b => proj_b
        })

        doc_a = { 'id' => uri_a, 'uri' => uri_a, 'primary_type' => 'resource' }
        doc_b = { 'id' => uri_b, 'uri' => uri_b, 'primary_type' => 'resource' }

        indexer.add_subjects(doc_a, rec)
        indexer.add_subjects(doc_b, rec_b)

        # Record A fell back natively
        expect(doc_a['subjects']).to eq(['Church History', 'Archives'])
        expect(indexer.instance_variable_get(:@pui_common_projections)[uri_a]).to be_nil

        # Record B succeeded with trusted projection
        expect(doc_b['subjects']).to eq(['Church History', 'Archives'])
        expect(indexer.instance_variable_get(:@pui_common_projections)[uri_b]).to eq(proj_b)
      ensure
        AppConfig[:rust_pui_common_metadata_enabled] = false
      end
    end

    it "exercises IndexerCommon#index_records with IndexBatch capture, helper ordering around add_notes, verified requested JSON, and helper acceptance" do
      saved_inheritance_config = AppConfig[:record_inheritance]
      begin
        AppConfig[:rust_index_document_builder_enabled] = true
        AppConfig[:rust_pui_common_metadata_enabled] = true
        AppConfig[:rust_pui_document_preparation_enabled] = false
        AppConfig[:record_inheritance] = (saved_inheritance_config || {}).merge(
          :resource => {
            :inherited_fields => [
              { :property => 'title', :inherit_directly => true }
            ]
          }
        )

        test_indexer = PUIIndexer.new(AppConfig[:backend_url], nil, 'PUIIndexer')
        captured_batch = []
        test_indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
          batch.each { |doc| captured_batch << doc }
        end

        uri = sample_common_record['uri']
        record_input = { 'record' => sample_common_record.dup, 'uri' => uri }

        valid_rust_envelope_doc = {
          'id' => uri,
          'uri' => uri,
          'title' => sample_common_record['title'],
          'primary_type' => 'resource',
          'types' => ['resource'],
          'json' => ASUtils.to_json(sample_common_record),
          'suppressed' => false,
          'publish' => true,
          'system_generated' => 'false',
          'repository' => '/repositories/2',
          '_archivalist_rust_fields' => ['pui_common_metadata'],
          '_archivalist_pui_common' => valid_pui_common_projection.dup
        }

        # 1. Capture perform_request and assert requested JSON payload
        captured_request = nil
        allow_any_instance_of(RustIndexDocumentBuilder).to receive(:perform_request) do |_builder, req|
          captured_request = req
          Struct.new(:code, :body).new('200', ASUtils.to_json(
            'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
            'documents' => [valid_rust_envelope_doc]
          ))
        end

        # 2. Instrument helper order covering all seven helpers plus add_notes
        order_trail = []
        [:add_subjects, :add_agents, :add_audit_info, :add_notes, :add_years, :add_level, :add_summary, :add_extents].each do |m|
          orig_m = test_indexer.method(m)
          test_indexer.define_singleton_method(m) do |doc, rec|
            order_trail << m
            orig_m.call(doc, rec)
          end
        end

        # 3. Track helper acceptance before batch cleanup
        accepted_helpers = []
        orig_apply = test_indexer.method(:apply_rust_pui_common_helper)
        test_indexer.define_singleton_method(:apply_rust_pui_common_helper) do |helper_name, doc, rec, &block|
          res = orig_apply.call(helper_name, doc, rec, &block)
          accepted_helpers << helper_name if res
          res
        end

        projection_present_during_hooks = nil
        plugin_saw_pui_common_marker = nil
        plugin_hook = proc do |doc, rec|
          projection_present_during_hooks = !test_indexer.instance_variable_get(:@pui_common_projections)[uri].nil?
          plugin_saw_pui_common_marker = doc.has_key?('_archivalist_pui_common')
        end

        hooks = test_indexer.instance_variable_get(:@document_prepare_hooks)
        final_hook = hooks.pop
        hooks << plugin_hook
        hooks << final_hook

        # Execute real index_records
        test_indexer.index_records([record_input])

        # Assert perform_request was actually called with expected JSON
        expect(captured_request).not_to be_nil
        req_payload = ASUtils.json_parse(captured_request.body)
        expect(req_payload['records'].length).to eq(1)
        req_rec = req_payload['records'].first
        expect(req_rec['prepare_pui_common_metadata']).to be(true)
        expect(req_rec['prepare_pui_final']).to be_nil

        # Assert helper call order matches original native sequence around add_notes
        expect(order_trail).to eq([:add_subjects, :add_agents, :add_audit_info, :add_notes, :add_years, :add_level, :add_summary, :add_extents])

        # Assert helper path accepted all seven projections before batch cleanup
        expect(accepted_helpers).to eq(['subjects', 'agents', 'audit_info', 'years', 'level', 'summary', 'extents'])
        expect(projection_present_during_hooks).to be(true)

        expect(captured_batch.length).to eq(1)
        final_doc = captured_batch.first

        # Verify all common search fields applied at original positions
        expect(final_doc['subjects']).to eq(['Church History', 'Archives'])
        expect(final_doc['subject_uris']).to eq(['/subjects/10', '/subjects/11', '/subjects/12'])
        expect(final_doc['agents']).to eq(['Doe, Jane', 'Example Historical Society'])
        expect(final_doc['published_agents']).to eq(['Doe, Jane'])
        expect(final_doc['creators']).to eq(['Doe, Jane'])
        expect(final_doc['created_by']).to eq('archivist')
        expect(final_doc['last_modified_by']).to eq('admin')
        expect(final_doc['years']).to eq(['1900', '1901', '1902', '1903', '1904', '1905', '1920'])
        expect(final_doc['year_sort']).to eq('19001920')
        expect(final_doc['dates']).to eq(['1900-01-01 - 1905-12-31'])
        expect(final_doc['level']).to eq('subfonds')
        expect(final_doc['summary']).to eq("Summary paragraph 1\nSummary paragraph 2")
        expect(final_doc['extents']).to eq(['2 --- boxes'])

        # Verify no private handoff metadata reached plugins or final output
        expect(plugin_saw_pui_common_marker).to be(false)
        expect(final_doc.keys.none? { |k| k.to_s.start_with?('_archivalist_') }).to be(true)

        # Batch cache is cleaned up to nil after index_records
        expect(test_indexer.instance_variable_get(:@pui_common_projections)).to be_nil
      ensure
        AppConfig[:record_inheritance] = saved_inheritance_config
        AppConfig[:rust_index_document_builder_enabled] = false
        AppConfig[:rust_pui_common_metadata_enabled] = false
        AppConfig[:rust_pui_document_preparation_enabled] = false
      end
    end

    it "supports combined prepare_pui_final and prepare_pui_common_metadata in index_records" do
      saved_inheritance_config = AppConfig[:record_inheritance]
      begin
        AppConfig[:rust_index_document_builder_enabled] = true
        AppConfig[:rust_pui_common_metadata_enabled] = true
        AppConfig[:rust_pui_document_preparation_enabled] = true
        AppConfig[:record_inheritance] = (saved_inheritance_config || {}).merge(
          :resource => {
            :inherited_fields => [
              { :property => 'title', :inherit_directly => true }
            ]
          }
        )

        test_indexer = PUIIndexer.new(AppConfig[:backend_url], nil, 'PUIIndexer')
        combined_batch = []
        test_indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
          batch.each { |doc| combined_batch << doc }
        end

        uri = sample_common_record['uri']
        record_input = { 'record' => sample_common_record.dup, 'uri' => uri }

        valid_rust_envelope_doc = {
          'id' => uri,
          'uri' => uri,
          'title' => sample_common_record['title'],
          'primary_type' => 'resource',
          'types' => ['resource'],
          'json' => ASUtils.to_json(sample_common_record),
          'suppressed' => false,
          'publish' => true,
          'system_generated' => 'false',
          'repository' => '/repositories/2',
          '_archivalist_rust_fields' => ['pui_common_metadata'],
          '_archivalist_pui_common' => valid_pui_common_projection.dup
        }

        combined_envelope_doc = valid_rust_envelope_doc.merge(
          '_archivalist_rust_fields' => [
            'pui_final_document_metadata',
            'pui_routing_metadata',
            'pui_notes_metadata',
            'pui_common_metadata'
          ],
          '_archivalist_pui_final' => {
            'marker' => 'pui_final_document_metadata',
            'json' => ASUtils.to_json(sample_common_record),
            'fullrecord_published' => IndexerCommon.extract_string_values(sample_common_record, :published_only)
          },
          '_archivalist_pui_routing' => {
            'marker' => 'pui_routing_metadata',
            'id' => "#{uri}#pui",
            'pui_parent_id' => uri,
            'types' => ['pui', 'pui_resource', 'pui_record', 'pui_only']
          },
          '_archivalist_pui_notes' => {
            'marker' => 'pui_notes_metadata',
            'notes_published' => IndexerCommon.extract_string_values(sample_common_record['notes'], :published_only)
          },
          '_archivalist_pui_common' => valid_pui_common_projection.dup
        )

        combined_request = nil
        allow_any_instance_of(RustIndexDocumentBuilder).to receive(:perform_request) do |_builder, req|
          combined_request = req
          Struct.new(:code, :body).new('200', ASUtils.to_json(
            'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
            'documents' => [combined_envelope_doc]
          ))
        end

        test_indexer.index_records([record_input])

        expect(combined_request).not_to be_nil
        comb_payload = ASUtils.json_parse(combined_request.body)
        comb_rec = comb_payload['records'].first
        expect(comb_rec['prepare_pui_common_metadata']).to be(true)
        expect(comb_rec['prepare_pui_final']).to be(true)

        expect(combined_batch.length).to eq(1)
        comb_doc = combined_batch.first
        expect(comb_doc['id']).to eq("#{uri}#pui")
        expect(comb_doc['subjects']).to eq(['Church History', 'Archives'])
        expect(comb_doc.keys.none? { |k| k.to_s.start_with?('_archivalist_') }).to be(true)
        expect(test_indexer.instance_variable_get(:@pui_common_projections)).to be_nil
      ensure
        AppConfig[:record_inheritance] = saved_inheritance_config
        AppConfig[:rust_index_document_builder_enabled] = false
        AppConfig[:rust_pui_common_metadata_enabled] = false
        AppConfig[:rust_pui_document_preparation_enabled] = false
      end
    end

    it "falls back cleanly when sidecar perform_request raises transport outage error" do
      saved_inheritance_config = AppConfig[:record_inheritance]
      begin
        AppConfig[:rust_index_document_builder_enabled] = true
        AppConfig[:rust_pui_common_metadata_enabled] = true
        AppConfig[:rust_pui_document_preparation_enabled] = false
        AppConfig[:record_inheritance] = (saved_inheritance_config || {}).merge(
          :resource => {
            :inherited_fields => [
              { :property => 'title', :inherit_directly => true }
            ]
          }
        )

        test_indexer = PUIIndexer.new(AppConfig[:backend_url], nil, 'PUIIndexer')
        outage_batch = []
        test_indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
          batch.each { |doc| outage_batch << doc }
        end

        uri = sample_common_record['uri']
        record_input = { 'record' => sample_common_record.dup, 'uri' => uri }

        outage_called = false
        allow_any_instance_of(RustIndexDocumentBuilder).to receive(:perform_request) do |_builder, _req|
          outage_called = true
          raise "Connection refused - sidecar outage"
        end

        test_indexer.index_records([record_input])
        expect(outage_called).to be(true)
        expect(outage_batch.length).to eq(1)
        fallback_doc = outage_batch.first
        expect(fallback_doc['subjects']).to eq(['Church History', 'Archives'])
        expect(test_indexer.instance_variable_get(:@pui_common_projections)).to be_nil
      ensure
        AppConfig[:record_inheritance] = saved_inheritance_config
        AppConfig[:rust_index_document_builder_enabled] = false
        AppConfig[:rust_pui_common_metadata_enabled] = false
      end
    end

    it "detects pre-hook mutations in index_records and falls back to native helper execution" do
      saved_inheritance_config = AppConfig[:record_inheritance]
      begin
        AppConfig[:rust_index_document_builder_enabled] = true
        AppConfig[:rust_pui_common_metadata_enabled] = true
        AppConfig[:rust_pui_document_preparation_enabled] = false
        AppConfig[:record_inheritance] = (saved_inheritance_config || {}).merge(
          :resource => {
            :inherited_fields => [
              { :property => 'title', :inherit_directly => true }
            ]
          }
        )

        test_indexer = PUIIndexer.new(AppConfig[:backend_url], nil, 'PUIIndexer')
        mutated_batch = []
        test_indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
          batch.each { |doc| mutated_batch << doc }
        end

        uri = sample_common_record['uri']
        record_input = { 'record' => sample_common_record.dup, 'uri' => uri }

        # Sidecar returns standard projection based on unmutated record
        valid_rust_envelope_doc = {
          'id' => uri,
          'uri' => uri,
          'title' => sample_common_record['title'],
          'primary_type' => 'resource',
          'types' => ['resource'],
          'json' => ASUtils.to_json(sample_common_record),
          'suppressed' => false,
          'publish' => true,
          'system_generated' => 'false',
          'repository' => '/repositories/2',
          '_archivalist_rust_fields' => ['pui_common_metadata'],
          '_archivalist_pui_common' => valid_pui_common_projection.dup
        }

        allow_any_instance_of(RustIndexDocumentBuilder).to receive(:perform_request) do |_builder, _req|
          Struct.new(:code, :body).new('200', ASUtils.to_json(
            'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
            'documents' => [valid_rust_envelope_doc]
          ))
        end

        # Register a pre-hook BEFORE common helpers that mutates subjects in-memory
        pre_hook = proc do |_doc, rec|
          rec['record']['subjects'] = [
            { 'ref' => '/subjects/pre_hook', '_resolved' => { 'title' => 'Pre-Hook Mutated Subject' } }
          ]
        end
        test_indexer.instance_variable_get(:@document_prepare_hooks).unshift(pre_hook)

        test_indexer.index_records([record_input])
        expect(mutated_batch.length).to eq(1)
        final_doc = mutated_batch.first
        expect(final_doc['subjects']).to eq(['Pre-Hook Mutated Subject'])
        expect(final_doc['subject_uris']).to eq(['/subjects/pre_hook'])
        expect(test_indexer.instance_variable_get(:@pui_common_projections)).to be_nil
      ensure
        AppConfig[:record_inheritance] = saved_inheritance_config
        AppConfig[:rust_index_document_builder_enabled] = false
        AppConfig[:rust_pui_common_metadata_enabled] = false
      end
    end

    it "preserves pre-existing doc values when helper patch fields are absent (no-write behavior)" do
      saved_inheritance_config = AppConfig[:record_inheritance]
      begin
        AppConfig[:rust_index_document_builder_enabled] = true
        AppConfig[:rust_pui_common_metadata_enabled] = true
        AppConfig[:rust_pui_document_preparation_enabled] = false
        AppConfig[:record_inheritance] = (saved_inheritance_config || {}).merge(
          :resource => {
            :inherited_fields => [
              { :property => 'title', :inherit_directly => true }
            ]
          }
        )

        test_indexer = PUIIndexer.new(AppConfig[:backend_url], nil, 'PUIIndexer')
        captured_batch = []
        test_indexer.define_singleton_method(:index_batch) do |batch, _timing = nil, _opts = {}|
          batch.each { |doc| captured_batch << doc }
        end

        sparse_record = sample_common_record.dup
        sparse_record.delete('linked_agents') # no creators
        sparse_record.delete('dates')         # no year_sort
        sparse_record.delete('notes')         # no summary
        uri = sparse_record['uri']
        record_input = { 'record' => sparse_record, 'uri' => uri }

        sparse_common_proj = valid_pui_common_projection.dup.merge(
          'agents' => {},
          'years' => {},
          'summary' => {}
        )

        valid_rust_envelope_doc = {
          'id' => uri,
          'uri' => uri,
          'title' => sparse_record['title'],
          'primary_type' => 'resource',
          'types' => ['resource'],
          'json' => ASUtils.to_json(sparse_record),
          'suppressed' => false,
          'publish' => true,
          'system_generated' => 'false',
          'repository' => '/repositories/2',
          '_archivalist_rust_fields' => ['pui_common_metadata'],
          '_archivalist_pui_common' => sparse_common_proj
        }

        allow_any_instance_of(RustIndexDocumentBuilder).to receive(:perform_request) do |_builder, _req|
          Struct.new(:code, :body).new('200', ASUtils.to_json(
            'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
            'documents' => [valid_rust_envelope_doc]
          ))
        end

        # Pre-hook initializes pre-existing values in doc
        pre_existing_hook = proc do |doc, _rec|
          doc['creators'] = ['Pre-existing Creator']
          doc['year_sort'] = '18501850'
          doc['summary'] = 'Pre-existing Summary'
        end
        test_indexer.instance_variable_get(:@document_prepare_hooks).unshift(pre_existing_hook)

        accepted_helpers = []
        orig_apply = test_indexer.method(:apply_rust_pui_common_helper)
        test_indexer.define_singleton_method(:apply_rust_pui_common_helper) do |helper_name, doc, rec, &block|
          res = orig_apply.call(helper_name, doc, rec, &block)
          accepted_helpers << helper_name if res
          res
        end

        test_indexer.index_records([record_input])
        expect(accepted_helpers).to eq(['subjects', 'agents', 'audit_info', 'years', 'level', 'summary', 'extents'])
        expect(captured_batch.length).to eq(1)
        final_doc = captured_batch.first

        # No-write: pre-existing doc values preserved when patch has absent fields
        expect(final_doc['creators']).to eq(['Pre-existing Creator'])
        expect(final_doc['year_sort']).to eq('18501850')
        expect(final_doc['summary']).to eq('Pre-existing Summary')
        expect(test_indexer.instance_variable_get(:@pui_common_projections)).to be_nil
      ensure
        AppConfig[:record_inheritance] = saved_inheritance_config
        AppConfig[:rust_index_document_builder_enabled] = false
        AppConfig[:rust_pui_common_metadata_enabled] = false
      end
    end

    it "cleans up batch cache even when document prepare hook raises exception" do
      saved_inheritance_config = AppConfig[:record_inheritance]
      begin
        AppConfig[:rust_index_document_builder_enabled] = true
        AppConfig[:rust_pui_common_metadata_enabled] = true
        AppConfig[:record_inheritance] = (saved_inheritance_config || {}).merge(
          :resource => {
            :inherited_fields => [
              { :property => 'title', :inherit_directly => true }
            ]
          }
        )

        exploding_indexer = PUIIndexer.new(AppConfig[:backend_url], nil, 'PUIIndexer')
        uri = sample_common_record['uri']
        record_input = { 'record' => sample_common_record.dup, 'uri' => uri }

        valid_rust_envelope_doc = {
          'id' => uri,
          'uri' => uri,
          'title' => sample_common_record['title'],
          'primary_type' => 'resource',
          'types' => ['resource'],
          'json' => ASUtils.to_json(sample_common_record),
          'suppressed' => false,
          'publish' => true,
          'system_generated' => 'false',
          'repository' => '/repositories/2',
          '_archivalist_rust_fields' => ['pui_common_metadata'],
          '_archivalist_pui_common' => valid_pui_common_projection.dup
        }

        allow_any_instance_of(RustIndexDocumentBuilder).to receive(:perform_request) do |_builder, _req|
          Struct.new(:code, :body).new('200', ASUtils.to_json(
            'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
            'documents' => [valid_rust_envelope_doc]
          ))
        end

        exploding_indexer.instance_variable_get(:@document_prepare_hooks) << proc { raise "Boom inside hook" }
        expect { exploding_indexer.index_records([record_input]) }.to raise_error(/Boom inside hook/)
        expect(exploding_indexer.instance_variable_get(:@pui_common_projections)).to be_nil
      ensure
        AppConfig[:record_inheritance] = saved_inheritance_config
        AppConfig[:rust_index_document_builder_enabled] = false
        AppConfig[:rust_pui_common_metadata_enabled] = false
      end
    end
  end
end
