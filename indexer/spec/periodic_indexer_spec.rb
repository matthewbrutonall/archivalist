require 'spec_helper'
require_relative '../app/lib/periodic_indexer'

describe "periodic indexer" do

  let (:indexer) do
    indexer = PeriodicIndexer.new

    def indexer.index_batch(batch, timing)
      @index_batch = batch
    end

    def indexer.prepare_docs(records)
      prepared_records = records.map {|rec|
        rec_hash = rec.to_hash(:raw)
        rec_hash['display_string'] = rec_hash['title']

        {'record' => rec_hash, 'uri' => rec['uri']}}

      index_records(prepared_records)
    end

    def indexer.records
      result = []

      @index_batch.each do |rec|
        result << rec
      end

      result
    end

    indexer
  end

  describe "initialize" do
    it "initializes the indexer" do
      # def initialize(backend_url = nil, state = nil, indexer_name = nil)
    end
  end
  describe "start_worker_thread" do
    it "starts a worker thread" do
      # def start_worker_thread(queue, record_type)
    end
  end
  describe "run_index_round" do
    it "runs and index round" do
      # def run_index_round
    end
  end
  describe "index_round_complete" do
    it "determines if the index round is complete" do
    # def index_round_complete(repository)
    end
  end
  describe "handle_deletes" do
    it "handles deletes" do
      # def handle_deletes(opts = {})
    end
  end
  describe "run" do
    it "runs the indexer" do
      # def run
    end
  end
  describe "log" do
    it "logs stuff" do
    # def log(line)
    end
  end
  describe "get_indexer" do
    it "gets the indexer" do
      # def self.get_indexer(state = nil, name = "Staff Indexer")
    end
  end
  describe "fetch_records" do
    it "fetches records" do
      # def fetch_records(type, ids, resolve)
    end
  end
  describe "has_unpublished_ancestor" do
    it "indexes the 'has_unpublished_ancestor' property" do
      ao = build(:json_archival_object,
                 'uri' => '/repositories/2/archival_objects/123',
                 'title' => "AO with unpublished ancestor",
                 'repository' => {
                   'ref' => '/repositories/5',
                   '_resolved' => {
                     'repo_code' => 'woop',
                   },
                 },
                 'publish' => true,
                 'has_unpublished_ancestor' => true,
                 'instances' => [],
                 'resource' => "/repositories/5/resources/1")

      indexer.prepare_docs([ao])

      doc = indexer.records[0]
      expect(doc.fetch('publish')).to be_falsey
    end
  end

  describe "identifiers" do
    it "indexes concatenated four part ids as identifiers" do
      records = []

      ['accession', 'resource'].each do |t|
        records << build(:"json_#{t}",
                       'uri' => "/repositories/2/#{t}s/1",
                       'title' => "Test #{t}",
                       'repository' => {
                         'ref' => '/repositories/2',
                         '_resolved' => {
                           'repo_code' => 'test',
                         },
                       },
                       'id_0' => '0',
                       'id_1' => '1',
                       'id_2' => '2',
                       'id_3' => '3',
                       'instances' => [])
      end

      indexer.prepare_docs(records)

      doc = indexer.records
      doc.each do |d|
        expect(d.fetch('identifier')).to eq('0-1-2-3')
      end
    end

    it "indexes component unique ids as identifiers" do
      records = []

      ['archival_object', 'digital_object_component'].each do |t|
        parent_type = (t == 'archival_object') ? 'resource' : 'digital_object'
        records << build(:"json_#{t}",
                       'uri' => "/repositories/2/#{t}s/1",
                       'title' => "Test #{t}",
                       'repository' => {
                         'ref' => '/repositories/2',
                         '_resolved' => {
                           'repo_code' => 'test',
                         },
                       },
                       'component_id' => '0123',
                       "#{parent_type}" => "/repositories/2/#{parent_type}s/1")
      end

      indexer.prepare_docs(records)

      doc = indexer.records
      doc.each do |d|
        expect(d.fetch('identifier')).to eq('0123')
      end
    end

    it "indexes digital object id as identifier" do
      digital_object = build(:json_digital_object,
                     'uri' => "/repositories/2/digital_objects/1",
                     'title' => "Test Digital Object",
                     'repository' => {
                       'ref' => '/repositories/2',
                       '_resolved' => {
                         'repo_code' => 'test',
                       },
                     },
                     'digital_object_id' => '0123')

      indexer.prepare_docs([digital_object])

      doc = indexer.records[0]
      expect(doc.fetch('identifier')).to eq('0123')
    end
  end

  describe "Rust document enrichment" do
    it "retains Rust fields, skips their native hooks, and removes handoff metadata" do
      resource = build(:json_resource,
                       'uri' => '/repositories/2/resources/77',
                       'title' => 'Rust-enriched resource',
                       'publish' => true,
                       'instances' => [],
                       'notes' => [])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['notes_published'] = ['prepared by Rust']
          doc['notes'] = ['private Rust text']
          doc['fullrecord_published'] = ['prepared by Rust']
          doc['fullrecord'] = ['private Rust text']
          doc['finding_aid_title'] = 'Rust finding aid title'
          doc['identifier'] = 'RUST-77'
          doc['external_id'] = ['rust-external-id']
          doc['ancestors'] = ['/repositories/2/resources/1']
          doc['langcode'] = ['gle']
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'resource_archival_object_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([resource])
      doc = indexer.records.first
      expect(doc['notes_published']).to eq(['prepared by Rust'])
      expect(doc['notes']).to eq(['private Rust text'])
      expect(doc['fullrecord_published']).to eq(['prepared by Rust'])
      expect(doc['fullrecord']).to eq(['private Rust text'])
      expect(doc['finding_aid_title']).to eq('Rust finding aid title')
      expect(doc['identifier']).to eq('RUST-77')
      expect(doc['external_id']).to eq(['rust-external-id'])
      expect(doc['ancestors']).to eq(['/repositories/2/resources/1'])
      expect(doc['langcode']).to eq(['gle'])
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "retains Rust accession fields while keeping localized date handling native" do
      accession = build(:json_accession,
                        'uri' => '/repositories/2/accessions/78',
                        'title' => 'Native accession title',
                        'display_string' => 'Native accession display',
                        'accession_date' => '2026-10-17',
                        'id_0' => 'NATIVE',
                        'instances' => [])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['notes_published'] = []
          doc['notes'] = []
          doc['fullrecord_published'] = []
          doc['fullrecord'] = []
          doc['title'] = 'Rust accession display'
          doc['identifier'] = 'RUST-78'
          doc['acquisition_type'] = 'gift'
          doc['external_id'] = ['rust-accession-id']
          doc['ancestors'] = []
          doc['langcode'] = ['eng']
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'accession_digital_object_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([accession])
      doc = indexer.records.first
      expect(doc['title']).to eq('Rust accession display')
      expect(doc['identifier']).to eq('RUST-78')
      expect(doc['acquisition_type']).to eq('gift')
      expect(doc['external_id']).to eq(['rust-accession-id'])
      expect(doc['accession_date']).to eq('2026-10-17')
      expect(doc['accession_date_year']).to eq(2026)
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "retains Rust subject authority fields and removes handoff metadata" do
      subject = build(:json_subject,
                      'uri' => '/subjects/88',
                      'title' => 'Monastic archives',
                      'publish' => true,
                      'is_linked_to_published_record' => true,
                      'source' => 'local',
                      'terms' => [{'term' => 'Monastic archives', 'term_type' => 'topical'}],
                      'vocabulary' => '/vocabularies/1',
                      'used_within_repositories' => ['/repositories/2'],
                      'used_within_published_repositories' => ['/repositories/2'])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['fullrecord_published'] = []
          doc['fullrecord'] = []
          doc['source'] = 'rust-authority-source'
          doc['first_term_type'] = 'rust-topical'
          doc['publish'] = false
          doc['used_within_repository'] = ['/repositories/9']
          doc['external_id'] = []
          doc['ancestors'] = []
          doc['langcode'] = []
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'agent_subject_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([subject])
      doc = indexer.records.first
      expect(doc['source']).to eq('rust-authority-source')
      expect(doc['first_term_type']).to eq('rust-topical')
      expect(doc['publish']).to be_falsey
      expect(doc['used_within_repository']).to eq(['/repositories/9'])
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end
  end
end
