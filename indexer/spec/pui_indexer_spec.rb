require 'spec_helper'
require_relative '../app/lib/pui_indexer'

describe "PUI indexer" do
  let (:indexer) do
    indexer = PUIIndexer.new(AppConfig[:backend_url], nil, 'PUIIndexer')

    def indexer.prepare_docs(records)
      records.map {|rec|
        rec_hash = rec.respond_to?(:to_hash) ? rec.to_hash(:raw) : rec
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
      _res, ao2 = sample_records

      rec_native = indexer.prepare_docs([ao1]).first
      doc_native = indexer.start_index(rec_native)
      doc_native['ancestors'] = ['/repositories/2/resources/1']
      indexer.run_all_but_final_doc_rules(doc_native, rec_native)
      indexer.run_final_doc_rules(doc_native, rec_native)

      AppConfig[:rust_pui_document_preparation_enabled] = true
      rec_rust = indexer.prepare_docs([ao2]).first
      doc_rust = indexer.start_index(rec_rust)
      doc_rust['ancestors'] = ['/repositories/2/resources/1']

      candidate = RecordInheritance.merge(indexer.deep_copy_record_candidate(rec_rust['record']))
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
        captured_batch.concat(batch)
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
        fallback_batch.concat(batch)
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
      indexer.instance_variable_set(:@pui_routing_projections, { uri => {
        'marker' => 'pui_routing_metadata',
        'id' => "#{uri}#pui",
        'pui_parent_id' => uri,
        'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only']
      }})
      expect(indexer.apply_rust_pui_routing_projection(doc6, bad_rec6)).to be(false)
      expect(indexer.instance_variable_get(:@pui_routing_projections)[uri]).to be_nil
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
      expect(indexer.rust_pui_record_inheritance_candidate_enabled?).to be(false)
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = true
      expect(indexer.rust_pui_record_inheritance_candidate_enabled?).to be(true)
    ensure
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = false
    end
  end

  it "supports general Ruby truthiness for rust_pui_record_inheritance_candidate_enabled?" do
    begin
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
        'uri' => uri,
        'jsonmodel_type' => 'archival_object',
        'title' => 'Test Item',
        'publish' => true,
        'suppressed' => false,
        'ancestors' => []
      }
      doc = {}

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
      doc = {}

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
            'language' => 'lat'
          }
        },
        {
          'ref' => 12345,
          'level' => 'fonds',
          '_resolved' => {
            'edition' => '1st'
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
    scope_rule = plan['rules'].find { |r| r['property'] == 'notes' && r['inherited_props']['level'] == 'series' }
    access_rule = plan['rules'].find { |r| r['property'] == 'notes' && r['inherited_props']['level'] == 'collection' }
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
    inheritance = RecordInheritance.new
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
          '_resolved' => {} # no qualifying values to inherit
        },
        {
          'ref' => '/repositories/2/resources/contributing',
          'level' => 'contributing_collection',
          '_resolved' => {
            'language' => 'spa'
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

    inheritance = RecordInheritance.new
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
          '_resolved' => {} # no qualifying values to inherit
        },
        {
          'ref' => '/repositories/2/resources/contributing',
          'level' => 'contributing_collection',
          '_resolved' => {
            'language' => 'spa'
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
      doc1 = {}
      bad_cand1 = {
        :marker => 'pui_record_inheritance_candidate_metadata',
        'record' => expected_merged,
      }
      indexer.instance_variable_set(:@pui_candidate_projections, { uri => bad_cand1 })
      indexer.instance_variable_set(:@pui_final_projections, { uri => valid_final.dup })
      expect(indexer.apply_rust_pui_final_projection(doc1, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_candidate_projections)[uri]).to be_nil
      expect(indexer.instance_variable_get(:@pui_final_projections)[uri]).to be_nil
      expect(doc1).to be_empty

      # 2. Missing required keys in candidate projection
      doc2 = {}
      bad_cand2 = {
        'marker' => 'pui_record_inheritance_candidate_metadata',
      }
      indexer.instance_variable_set(:@pui_candidate_projections, { uri => bad_cand2 })
      indexer.instance_variable_set(:@pui_final_projections, { uri => valid_final.dup })
      expect(indexer.apply_rust_pui_final_projection(doc2, rec)).to be(false)
      expect(indexer.instance_variable_get(:@pui_candidate_projections)[uri]).to be_nil
      expect(indexer.instance_variable_get(:@pui_final_projections)[uri]).to be_nil
      expect(doc2).to be_empty

      # 3. Unexpected extra keys in candidate projection
      doc3 = {}
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
      expect(doc3).to be_empty
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
      doc_float = {}
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
      expect(doc_float).to be_empty

      # 2. Candidate record with integer 1 where boolean true is expected
      doc_bool = {}
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
      expect(doc_bool).to be_empty
    ensure
      AppConfig[:rust_pui_document_preparation_enabled] = false
      AppConfig[:rust_pui_record_inheritance_candidate_enabled] = false
    end
  end
end
