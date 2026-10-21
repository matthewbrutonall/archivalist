require 'spec_helper'
require_relative '../app/lib/indexer_common'

describe "indexer common" do
  let (:indexer) { IndexerCommon.new(AppConfig[:backend_url]) }

  let (:resource) {
    resource = create(:json_resource,
                      publish: true,
                      instances: [],
                      notes: [
                        build(:json_note_multipart,
                              subnotes: [
                                build(:json_note_text, publish: true, content: "had 5 wives"),
                                build(:json_note_text, publish: false, content: "..and 12 mistresses")
                              ])
                      ])
    resolved = IndexerCommonConfig.resolved_attributes
    resource = JSONModel(:resource).find(resource.id, 'resolve[]' => resolved)
    resource
  }

  let(:container_profile) {
    create(:json_container_profile, notes: "Unlike other record types, my notes property is a String")
  }

  describe "Mapping JSONModel(:type) records to documents for Solr" do

    it "partitions published and unpublished note strings in native hook order" do
      notes = [
        {'type' => 'abstract', 'publish' => true, 'content' => ['Public abstract']},
        {'type' => 'scopecontent', 'publish' => false, 'content' => ['Private note']},
      ]
      published, unpublished = IndexerCommon.extract_string_values(notes)
      expect(published).to eq(['abstract', 'Public abstract'])
      expect(unpublished).to eq(['scopecontent', 'Private note'])
    end

    it "falls back to the native document builder when the Rust builder is unavailable" do
      values = {
        'title' => 'Fallback record',
        'publish' => true,
        'system_generated' => false,
      }
      prepared = [{
        :uri => '/repositories/2/resources/7',
        :record_type => 'resource',
        :values => values,
      }]
      allow(indexer).to receive(:rust_index_document_builder_enabled?).and_return(true)
      allow_any_instance_of(RustIndexDocumentBuilder).to receive(:build).and_raise(Errno::ECONNREFUSED)

      expect(indexer.rust_index_documents(prepared)).to be_nil
      fallback = indexer.native_index_document(prepared[0][:uri], prepared[0][:record_type], values)
      expect(fallback['publish']).to be(true)
      expect(fallback['primary_type']).to eq('resource')
    end

    it "creates published-only versions of 'fullrecord' and 'notes' index fields" do
      allow(IndexBatch).to receive(:new) { [] }

      allow(indexer).to receive(:index_batch) do |batch, timing = IndexerTiming.new, opts = {}|
        fullrecord = batch.first['fullrecord'].join(' ')
        expect(fullrecord).not_to include("5 wives")
        expect(fullrecord).to include("12 mistresses")

        fullrecord_published = batch.first['fullrecord_published'].join(' ')
        expect(fullrecord_published).to include("5 wives")
        expect(fullrecord_published).not_to include("12 mistresses")

        notes = batch.first['notes'].join(' ')
        expect(notes).not_to include("5 wives")
        expect(notes).to include("12 mistresses")

        notes_published = batch.first['notes_published'].join(' ')
        expect(notes_published).to include("5 wives")
        expect(notes_published).not_to include("12 mistresses")
      end

      records = [
        {
          'uri' => resource.uri,
          'record' => resource.to_hash(:trusted)
        }
      ]

      indexer.index_records(records)
    end

  end

  it "can deal with schema that have a 'notes' property that takes strings" do
    allow(IndexBatch).to receive(:new) { [] }

    allow(indexer).to receive(:index_batch) do |batch, timing = IndexerTiming.new, opts = {}|
      fullrecord = (batch.first['fullrecord_published'] + batch.first['fullrecord']).join(' ')
      expect(fullrecord).to include(container_profile.notes)
    end

    records = [
      {
        'uri' => container_profile.uri,
        'record' => container_profile.to_hash(:trusted)
      }
    ]

    indexer.index_records(records)
  end

  describe "Dynamic enum whitespace preservation and frozen-string pipeline" do
    # WHY FREEZING IS NECESSARY:
    # In native ArchivesSpace, enum strings (such as those populated through JSONModel
    # schema lookups or converted from Set instances) behave as frozen strings in the
    # document hash. The final indexing pipeline invokes clean_whitespace(doc), which
    # checks:
    #   if doc.is_a?(String) && !doc.frozen?
    #     doc.strip!
    #   end
    # Because native enum strings were frozen, clean_whitespace skipped strip! and preserved
    # any leading/trailing whitespace (e.g. '/repositories/4/resources/8.label_enum_s').
    # In contrast, when Rust accelerates dynamic enum projection, JSON deserialization
    # creates fresh unfrozen Ruby strings. Without freezing, clean_whitespace mutated
    # them in-place with strip!, causing a candidate vs native Solr parity mismatch.
    # Calling freeze_dynamic_enum_strings!(doc) freezes only strings within *_enum_s
    # arrays, ensuring exact stored enum parity while allowing ordinary fields to be cleaned.
    it "preserves leading and trailing whitespace on dynamic enum fields while cleaning ordinary fields" do
      doc = {
        'title' => '  Uncleaned Title String  ',
        'description' => "\tOrdinary description\n",
        'label_enum_s' => ['  leading_space_enum', 'trailing_space_enum  ', 'clean_enum'],
        'type_enum_s' => ['  both_spaces_enum  '],
        'unrelated_array' => ['  should_be_stripped  ']
      }

      # Prior to freezing, strings in arrays are mutable and unfrozen
      expect(doc['label_enum_s'][0].frozen?).to be(false)

      # freeze_dynamic_enum_strings! freezes only *_enum_s string items
      indexer.freeze_dynamic_enum_strings!(doc)

      expect(doc['label_enum_s'][0].frozen?).to be(true)
      expect(doc['label_enum_s'][1].frozen?).to be(true)
      expect(doc['label_enum_s'][2].frozen?).to be(true)
      expect(doc['type_enum_s'][0].frozen?).to be(true)

      # Unrelated fields are NOT frozen
      expect(doc['title'].frozen?).to be(false)
      expect(doc['description'].frozen?).to be(false)
      expect(doc['unrelated_array'][0].frozen?).to be(false)

      # Run final document whitespace cleaning
      cleaned = indexer.clean_whitespace(doc)

      # Preserved exact enum values (including leading/trailing whitespace)
      expect(cleaned['label_enum_s']).to eq(['  leading_space_enum', 'trailing_space_enum  ', 'clean_enum'])
      expect(cleaned['type_enum_s']).to eq(['  both_spaces_enum  '])

      # Ordinary fields stripped as expected
      expect(cleaned['title']).to eq('Uncleaned Title String')
      expect(cleaned['description']).to eq('Ordinary description')
      expect(cleaned['unrelated_array']).to eq(['should_be_stripped'])
    end
  end

end
