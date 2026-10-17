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

    it "accelerates accession date enrichment and removes handoff metadata when accession_date_metadata is trusted" do
      accession = build(:json_accession,
                        'uri' => '/repositories/2/accessions/79',
                        'title' => 'Native accession title',
                        'display_string' => 'ACC-79: Trusted Accession',
                        'accession_date' => '2026-10-17',
                        'id_0' => 'ACC-79',
                        'instances' => [])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['fullrecord_published'], doc['fullrecord'] = IndexerCommon.extract_string_values(prepared[:values])
          doc['accession_date'] = '2026-10-17'
          doc['accession_date_year'] = 2026
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'accession_digital_object_metadata',
            'accession_date_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([accession])
      doc = indexer.records.first
      expect(doc['accession_date']).to eq('2026-10-17')
      expect(doc['accession_date_year']).to eq(2026)
      expected_pub, expected_unpub = IndexerCommon.extract_string_values(accession)
      expect(doc['fullrecord_published']).to eq(expected_pub)
      expect(doc['fullrecord']).to eq(expected_unpub)
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "accelerates sentinel accession date enrichment with localized unknown label and fullrecord append" do
      unknown_label = I18n.t('accession.accession_date_unknown')
      accession = build(:json_accession,
                        'uri' => '/repositories/2/accessions/80',
                        'title' => 'Sentinel accession title',
                        'display_string' => 'ACC-80: Sentinel Accession',
                        'accession_date' => '9999-12-31',
                        'id_0' => 'ACC-80',
                        'instances' => [])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          expected_pub, expected_unpub = IndexerCommon.extract_string_values(prepared[:values])
          doc['fullrecord_published'] = expected_pub
          doc['fullrecord'] = expected_unpub + ["#{unknown_label} "]
          doc['accession_date'] = unknown_label
          doc['accession_date_year'] = 9999
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'accession_digital_object_metadata',
            'accession_date_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([accession])
      doc = indexer.records.first
      expect(doc['accession_date']).to eq(unknown_label)
      expect(doc['accession_date_year']).to eq(9999)
      expect(doc['fullrecord'].last).to eq("#{unknown_label} ")
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "falls back to native accession date handling when accession_date_year is a String (anti-string-coercion)" do
      accession = build(:json_accession,
                        'uri' => '/repositories/2/accessions/81',
                        'title' => 'String year accession',
                        'display_string' => 'ACC-81: String Year Accession',
                        'accession_date' => '2026-10-17',
                        'id_0' => 'ACC-81',
                        'instances' => [])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['fullrecord'] = ['prior content']
          doc['accession_date'] = '2026-10-17'
          doc['accession_date_year'] = '2026' # String instead of Integer
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'accession_digital_object_metadata',
            'accession_date_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([accession])
      doc = indexer.records.first
      expect(doc['accession_date']).to eq('2026-10-17')
      expect(doc['accession_date_year']).to eq(2026)
      expect(doc['accession_date_year'].is_a?(Integer)).to be true
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "falls back to native accession date handling when accession_date value is mismatched" do
      accession = build(:json_accession,
                        'uri' => '/repositories/2/accessions/82',
                        'title' => 'Mismatched date accession',
                        'display_string' => 'ACC-82: Mismatched Date Accession',
                        'accession_date' => '2026-10-17',
                        'id_0' => 'ACC-82',
                        'instances' => [])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['fullrecord'] = ['prior content']
          doc['accession_date'] = '1999-01-01' # Mismatched date
          doc['accession_date_year'] = 1999
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'accession_digital_object_metadata',
            'accession_date_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([accession])
      doc = indexer.records.first
      expect(doc['accession_date']).to eq('2026-10-17')
      expect(doc['accession_date_year']).to eq(2026)
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "falls back to native accession date handling when sentinel fullrecord append is missing" do
      unknown_label = I18n.t('accession.accession_date_unknown')
      accession = build(:json_accession,
                        'uri' => '/repositories/2/accessions/83',
                        'title' => 'Missing sentinel append accession',
                        'display_string' => 'ACC-83: Missing Sentinel Append',
                        'accession_date' => '9999-12-31',
                        'id_0' => 'ACC-83',
                        'instances' => [])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['fullrecord'] = ['prior content without sentinel append']
          doc['accession_date'] = unknown_label
          doc['accession_date_year'] = 9999
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'accession_digital_object_metadata',
            'accession_date_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([accession])
      doc = indexer.records.first
      expect(doc['accession_date']).to eq(unknown_label)
      expect(doc['accession_date_year']).to eq(9999)
      expect(doc['fullrecord'].last).to eq("#{unknown_label} ")
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "falls back to native accession date handling and reconstructs fullrecord without pollution or duplication when sidecar appends wrong localized value" do
      unknown_label = I18n.t('accession.accession_date_unknown')
      accession = build(:json_accession,
                        'uri' => '/repositories/2/accessions/84',
                        'title' => 'Wrong localized label accession',
                        'display_string' => 'ACC-84: Wrong Localized Label',
                        'accession_date' => '9999-12-31',
                        'id_0' => 'ACC-84',
                        'instances' => [])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          # Malformed sidecar projection: wrong localized label and corrupted items
          doc['fullrecord_published'] = ['bogus_published']
          doc['fullrecord'] = ['corrupted_item', 'Wrong Unknown Label ', 'extraneous_suffix']
          doc['accession_date'] = 'Wrong Unknown Label'
          doc['accession_date_year'] = 9999
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'accession_digital_object_metadata',
            'accession_date_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([accession])
      doc = indexer.records.first
      expect(doc['accession_date']).to eq(unknown_label)
      expect(doc['accession_date_year']).to eq(9999)

      # Deterministic reconstruction proof: exactly native fullrecord, no pollution, no duplication
      expected_pub, expected_unpub = IndexerCommon.extract_string_values(accession)
      expect(doc['fullrecord_published']).to eq(expected_pub)
      expect(doc['fullrecord']).to eq(expected_unpub + ["#{unknown_label} "])
      expect(doc['fullrecord']).not_to include('corrupted_item')
      expect(doc['fullrecord']).not_to include('Wrong Unknown Label ')
      expect(doc['fullrecord']).not_to include('extraneous_suffix')
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "accepts ordinary accession whose legitimate record content happens to match unknown label in fullrecord" do
      unknown_label = I18n.t('accession.accession_date_unknown')
      accession = build(:json_accession,
                        'uri' => '/repositories/2/accessions/85',
                        'title' => "Collection containing #{unknown_label} in title",
                        'display_string' => "ACC-85: Legitimate #{unknown_label}",
                        'accession_date' => '2026-10-17',
                        'id_0' => 'ACC-85',
                        'instances' => [])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          expected_pub, expected_unpub = IndexerCommon.extract_string_values(prepared[:values])
          doc['fullrecord_published'] = expected_pub
          doc['fullrecord'] = expected_unpub
          doc['accession_date'] = '2026-10-17'
          doc['accession_date_year'] = 2026
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'accession_digital_object_metadata',
            'accession_date_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([accession])
      doc = indexer.records.first
      expect(doc['accession_date']).to eq('2026-10-17')
      expect(doc['accession_date_year']).to eq(2026)
      expected_pub, expected_unpub = IndexerCommon.extract_string_values(accession)
      expect(doc['fullrecord']).to eq(expected_unpub)
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "accepts non-canonical schema-valid accession dates (year 0, unpadded month, surrounding whitespace)" do
      examples = [
        { 'date' => '0000-01-01', 'expected_year' => 0 },
        { 'date' => '2026-2-03', 'expected_year' => 2026 },
        { 'date' => '  2026-10-17  ', 'expected_year' => 2026 },
      ]

      examples.each_with_index do |ex, i|
        accession = build(:json_accession,
                          'uri' => "/repositories/2/accessions/#{86 + i}",
                          'title' => "Accession #{86 + i}",
                          'display_string' => "ACC-#{86 + i}: Non-canonical",
                          'accession_date' => ex['date'],
                          'id_0' => "ACC-#{86 + i}",
                          'instances' => [])

        allow(indexer).to receive(:rust_index_documents) do |prepared_records|
          prepared_records.each_with_object({}) do |prepared, documents|
            doc = indexer.native_index_document(
              prepared[:uri], prepared[:record_type], prepared[:values]
            )
            expected_pub, expected_unpub = IndexerCommon.extract_string_values(prepared[:values])
            doc['fullrecord_published'] = expected_pub
            doc['fullrecord'] = expected_unpub
            doc['accession_date'] = ex['date']
            doc['accession_date_year'] = ex['expected_year']
            doc['_archivalist_rust_fields'] = [
              'common_metadata',
              'fullrecord',
              'accession_digital_object_metadata',
              'accession_date_metadata',
            ]
            documents[prepared[:uri]] = doc
          end
        end

        indexer.prepare_docs([accession])
        doc = indexer.records.first
        expect(doc['accession_date']).to eq(ex['date'])
        expect(doc['accession_date_year']).to eq(ex['expected_year'])
        expect(doc).not_to have_key('_archivalist_rust_fields')
        indexer.records.clear
      end
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

    it "retains Rust classification fields and removes handoff metadata" do
      classification = build(:json_classification,
                             'uri' => '/repositories/2/classifications/9',
                             'title' => 'Records groups',
                             'identifier' => 'NATIVE-9',
                             'path_from_root' => [],
                             'creator' => nil,
                             'repository' => {
                               'ref' => '/repositories/2',
                               '_resolved' => {'display_string' => 'Example Archives'},
                             },
                             'has_classification_terms' => false,
                             'slug' => 'records-groups',
                             'is_slug_auto' => true)

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['fullrecord_published'] = []
          doc['fullrecord'] = []
          doc['classification_path'] = '[{"identifier":"RUST"}]'
          doc['agent_uris'] = []
          doc['published_agent_uris'] = []
          doc['agents'] = []
          doc['identifier_sort'] = 'rust-sort'
          doc['repo_sort'] = 'Rust repository'
          doc['has_classification_terms'] = false
          doc['slug'] = 'rust-records-groups'
          doc['is_slug_auto'] = true
          doc['identifier'] = 'RUST-9'
          doc['external_id'] = []
          doc['ancestors'] = []
          doc['langcode'] = []
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'component_classification_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([classification])
      doc = indexer.records.first
      expect(doc['classification_path']).to eq('[{"identifier":"RUST"}]')
      expect(doc['identifier_sort']).to eq('rust-sort')
      expect(doc['repo_sort']).to eq('Rust repository')
      expect(doc['identifier']).to eq('RUST-9')
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "retains Rust repository and event fields and removes handoff metadata" do
      repository = JSONModel(:repository).from_hash(
        'uri' => '/repositories/2',
        'repo_code' => 'NATIVE-REPO',
        'name' => 'Native Repository',
        'display_string' => 'Native Repository',
        'slug' => 'native-repo',
        'is_slug_auto' => true,
        'position' => 2
      )
      event = build(:json_event,
                    'uri' => '/repositories/2/events/11',
                    'event_type' => 'ingestion',
                    'outcome' => 'pass',
                    'linked_records' => [
                      {
                        'ref' => '/repositories/2/resources/1',
                        'role' => 'outcome',
                        '_resolved' => {
                          'jsonmodel_type' => 'resource',
                          'title' => 'Native Resource',
                        },
                      },
                    ])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['fullrecord_published'] = []
          doc['fullrecord'] = []
          doc['external_id'] = []
          doc['ancestors'] = []
          doc['langcode'] = []
          if prepared[:record_type] == 'repository'
            doc['repository'] = prepared[:uri]
            doc['title'] = 'RUST-REPO'
            doc['title_sort'] = 'Rust Repository Sort'
            doc['repo_sort'] = 'Rust Repository'
            doc['slug'] = 'rust-repo'
            doc['is_slug_auto'] = false
            doc['position_int_sort'] = 9
          else
            doc['event_type'] = 'custody_transfer'
            doc['title'] = 'custody_transfer'
            doc['outcome'] = 'fail'
            doc['linked_record_uris'] = ['/repositories/2/resources/99']
            doc['linked_record_titles'] = ['Rust Resource']
          end
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'repository_event_location_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([repository, event])
      repo_doc = indexer.records.find {|d| d['primary_type'] == 'repository'}
      event_doc = indexer.records.find {|d| d['primary_type'] == 'event'}

      expect(repo_doc['repository']).to eq('/repositories/2')
      expect(repo_doc['title']).to eq('RUST-REPO')
      expect(repo_doc['title_sort']).to eq('Rust Repository Sort')
      expect(repo_doc['repo_sort']).to eq('Rust Repository')
      expect(repo_doc['slug']).to eq('rust-repo')
      expect(repo_doc['position_int_sort']).to eq(9)
      expect(repo_doc).not_to have_key('_archivalist_rust_fields')

      expect(event_doc['event_type']).to eq('custody_transfer')
      expect(event_doc['title']).to eq('custody_transfer')
      expect(event_doc['outcome']).to eq('fail')
      expect(event_doc['linked_record_uris']).to eq(['/repositories/2/resources/99'])
      expect(event_doc['linked_record_titles']).to eq(['Rust Resource'])
      expect(event_doc).not_to have_key('_archivalist_rust_fields')
    end

    it "retains Rust location fields while keeping unresolved owner_repo lookup native" do
      resolved_location = build(:json_location,
                                'uri' => '/locations/15',
                                'title' => 'Main Vault',
                                'building' => 'Native Building',
                                'floor' => '1',
                                'room' => 'A',
                                'area' => 'Bay 3',
                                'owner_repo' => {'ref' => '/repositories/2'},
                                'location_profile' => {
                                  'ref' => '/location_profiles/4',
                                  '_resolved' => {'display_string' => 'Native Shelf'},
                                })
      unresolved_location = build(:json_location,
                                  'uri' => '/locations/16',
                                  'title' => 'Annex Vault',
                                  'building' => 'Native Annex',
                                  'owner_repo' => {'ref' => '/repositories/3'})

      allow(JSONModel::HTTP).to receive(:get_json).with('/repositories/3').and_return(
        'repo_code' => 'NATIVE-FETCHED-3'
      )

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['fullrecord_published'] = []
          doc['fullrecord'] = []
          doc['building'] = 'Rust Building'
          doc['floor'] = '9'
          doc['room'] = 'R'
          doc['area'] = 'Rust Area'
          doc['external_id'] = []
          doc['ancestors'] = []
          doc['langcode'] = []
          if prepared[:uri] == '/locations/15'
            doc['owner_repo_uri_u_sstr'] = '/repositories/2'
            doc['owner_repo_display_string_u_ssort'] = 'RUST-REPO-2'
            doc['location_profile_uri_u_sstr'] = '/location_profiles/4'
            doc['location_profile_display_string_u_ssort'] = 'Rust Shelf'
          else
            doc['owner_repo_uri_u_sstr'] = '/repositories/3'
          end
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'repository_event_location_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([resolved_location, unresolved_location])
      resolved_doc = indexer.records.find {|d| d['id'] == '/locations/15'}
      unresolved_doc = indexer.records.find {|d| d['id'] == '/locations/16'}

      expect(JSONModel::HTTP).not_to have_received(:get_json).with('/repositories/2')
      expect(JSONModel::HTTP).to have_received(:get_json).with('/repositories/3')

      expect(resolved_doc['building']).to eq('Rust Building')
      expect(resolved_doc['owner_repo_display_string_u_ssort']).to eq('RUST-REPO-2')
      expect(resolved_doc['location_profile_display_string_u_ssort']).to eq('Rust Shelf')
      expect(resolved_doc).not_to have_key('_archivalist_rust_fields')

      expect(unresolved_doc['building']).to eq('Rust Building')
      expect(unresolved_doc['owner_repo_uri_u_sstr']).to eq('/repositories/3')
      expect(unresolved_doc['owner_repo_display_string_u_ssort']).to eq('NATIVE-FETCHED-3')
      expect(unresolved_doc).not_to have_key('_archivalist_rust_fields')
    end

    it "retains Rust container_profile, location_profile, and assessment fields and removes handoff metadata" do
      container_profile = build(:json_container_profile,
                                'uri' => '/container_profiles/3',
                                'name' => 'Native Carton',
                                'display_string' => 'Native Carton [1x2x3]',
                                'width' => '1',
                                'height' => '2',
                                'depth' => '3',
                                'dimension_units' => 'inches')
      location_profile = build(:json_location_profile,
                               'uri' => '/location_profiles/4',
                               'name' => 'Native Shelf',
                               'display_string' => 'Native Shelf [4x5x6]',
                               'width' => '4',
                               'height' => '5',
                               'depth' => '6',
                               'dimension_units' => 'centimeters')
      assessment = JSONModel(:assessment).from_hash(
        'uri' => '/repositories/2/assessments/42',
        'display_string' => 'Native Assessment 42',
        'records' => [
          {
            'ref' => '/repositories/2/resources/7',
            '_resolved' => {'jsonmodel_type' => 'resource', 'title' => 'Native Resource'},
          },
        ],
        'surveyed_by' => [
          {'ref' => '/agents/people/1', '_resolved' => {'title' => 'Native Surveyor'}},
        ],
        'survey_begin' => '2024-01-01',
        'survey_end' => '2025-01-01',
        'review_required' => false,
        'sensitive_material' => false,
        'inactive' => false,
        'reviewer' => [],
        'collections' => [],
        'formats' => [{'label' => 'Native Format', 'value' => 'true'}],
        'ratings' => [{'label' => 'Native Rating', 'note' => 'Native note'}],
        'conservation_issues' => [{'label' => 'Native Issue', 'value' => 'true'}]
      )

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['fullrecord_published'] = []
          doc['fullrecord'] = []
          doc['external_id'] = ['rust-ext']
          doc['ancestors'] = []
          doc['langcode'] = []
          case prepared[:record_type]
          when 'container_profile'
            doc['title'] = 'Rust Carton Display'
            doc['title_sort'] = 'Rust Carton Sort'
            doc['display_string'] = 'Rust Carton Display'
            doc['note'] = 'Rust profile note'
            doc['container_profile_width_u_sstr'] = '10'
            doc['container_profile_height_u_sstr'] = '12'
            doc['container_profile_depth_u_sstr'] = '15'
            doc['container_profile_dimension_units_u_sstr'] = 'inches'
            doc['typeahead_sort_key_u_sort'] = 'Rust Carton Display'
          when 'location_profile'
            doc['title'] = 'Rust Shelf Display'
            doc['title_sort'] = 'Rust Shelf Sort'
            doc['display_string'] = 'Rust Shelf Display'
            doc['location_profile_width_u_sstr'] = '120'
            doc['location_profile_height_u_sstr'] = '40'
            doc['location_profile_depth_u_sstr'] = '60'
            doc['location_profile_dimension_units_u_sstr'] = 'centimeters'
            doc['typeahead_sort_key_u_sort'] = 'Rust Shelf Display'
          when 'assessment'
            doc['assessment_id'] = 42
            doc['title'] = 'Rust Assessment 42'
            doc['display_string'] = 'Rust Assessment 42'
            doc['assessment_record_uris'] = ['/repositories/2/resources/77']
            doc['assessment_records'] = ['Rust Resource']
            doc['assessment_record_types'] = ['resource']
            doc['assessment_surveyor_uris'] = ['/agents/people/9']
            doc['assessment_surveyors'] = ['Rust Surveyor']
            doc['assessment_survey_begin'] = '2026-01-01T00:00:00Z'
            doc['assessment_survey_end'] = '2026-12-31T00:00:00Z'
            doc['assessment_review_required'] = true
            doc['assessment_sensitive_material'] = true
            doc['assessment_inactive'] = false
            doc['assessment_reviewer_uris'] = ['/agents/people/10']
            doc['assessment_reviewers'] = ['Rust Reviewer']
            doc['assessment_survey_year'] = ['2026']
            doc['assessment_collection_uris'] = ['/repositories/2/resources/77']
            doc['assessment_collections'] = ['Rust Collection']
            doc['assessment_completed'] = true
            doc['assessment_formats'] = ['Rust Format']
            doc['assessment_ratings'] = ['Rust Rating']
            doc['assessment_conservation_issues'] = ['Rust Issue']
            doc['title_sort'] = '0000000042'
          end
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'profile_assessment_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.prepare_docs([container_profile, location_profile, assessment])
      cp_doc = indexer.records.find {|d| d['primary_type'] == 'container_profile'}
      lp_doc = indexer.records.find {|d| d['primary_type'] == 'location_profile'}
      assess_doc = indexer.records.find {|d| d['primary_type'] == 'assessment'}

      expect(cp_doc['title']).to eq('Rust Carton Display')
      expect(cp_doc['title_sort']).to eq('Rust Carton Sort')
      expect(cp_doc['note']).to eq('Rust profile note')
      expect(cp_doc['container_profile_width_u_sstr']).to eq('10')
      expect(cp_doc['container_profile_height_u_sstr']).to eq('12')
      expect(cp_doc['container_profile_depth_u_sstr']).to eq('15')
      expect(cp_doc['container_profile_dimension_units_u_sstr']).to eq('inches')
      expect(cp_doc['typeahead_sort_key_u_sort']).to eq('Rust Carton Display')
      expect(cp_doc['external_id']).to eq(['rust-ext'])
      expect(cp_doc).not_to have_key('_archivalist_rust_fields')

      expect(lp_doc['title']).to eq('Rust Shelf Display')
      expect(lp_doc['title_sort']).to eq('Rust Shelf Sort')
      expect(lp_doc['location_profile_width_u_sstr']).to eq('120')
      expect(lp_doc['location_profile_height_u_sstr']).to eq('40')
      expect(lp_doc['location_profile_depth_u_sstr']).to eq('60')
      expect(lp_doc['location_profile_dimension_units_u_sstr']).to eq('centimeters')
      expect(lp_doc['typeahead_sort_key_u_sort']).to eq('Rust Shelf Display')
      expect(lp_doc['external_id']).to eq(['rust-ext'])
      expect(lp_doc).not_to have_key('_archivalist_rust_fields')

      expect(assess_doc['assessment_id']).to eq(42)
      expect(assess_doc['title']).to eq('Rust Assessment 42')
      expect(assess_doc['assessment_records']).to eq(['Rust Resource'])
      expect(assess_doc['assessment_surveyors']).to eq(['Rust Surveyor'])
      expect(assess_doc['assessment_reviewers']).to eq(['Rust Reviewer'])
      expect(assess_doc['assessment_survey_year']).to eq(['2026'])
      expect(assess_doc['assessment_formats']).to eq(['Rust Format'])
      expect(assess_doc['assessment_ratings']).to eq(['Rust Rating'])
      expect(assess_doc['assessment_conservation_issues']).to eq(['Rust Issue'])
      expect(assess_doc['title_sort']).to eq('0000000042')
      expect(assess_doc['external_id']).to eq(['rust-ext'])
      expect(assess_doc).not_to have_key('_archivalist_rust_fields')
    end

    it "retains Rust top_container fields, strips top_container_metadata before downstream hooks, and preserves PUI publication decisions" do
      top_container = {
        'uri' => '/repositories/2/top_containers/44',
        'record' => {
          'jsonmodel_type' => 'top_container',
          'long_display_string' => 'Native Collection: Box 99 [NAT-BC]',
          'display_string' => 'Box 99 [NAT-BC]',
          'type' => 'box',
          'indicator' => '99',
          'barcode' => 'NAT-BC',
          'ils_holding_id' => 'NAT-ILS',
          'internal_note' => 'Native internal note',
          'exported_to_ils' => '2026-01-01T00:00:00Z',
          'subcontainer_barcodes' => ['NAT-SUB'],
          'created_for_collection' => '/repositories/2/resources/1',
          'is_linked_to_published_record' => true,
          'id_0' => 'native-id-0',
          'external_ids' => [{'external_id' => 'native-ext'}],
          'ancestors' => [{'ref' => '/repositories/2/resources/1'}],
          'rights_statements' => [
            {'linked_agents' => [{'ref' => '/agents/people/99'}]},
          ],
          'lang_materials' => [
            {'language_and_script' => {'language' => 'fre'}},
          ],
          'series' => [
            {
              'ref' => '/repositories/2/archival_objects/1',
              'display_string' => 'Native Series',
              'level_display_string' => 'Series',
              'identifier' => 'NAT-1',
              'publish' => true,
            },
          ],
          'collection' => [
            {
              'ref' => '/repositories/2/resources/1',
              'display_string' => 'Native Collection',
              'identifier' => 'NAT-COLL',
            },
          ],
          'container_profile' => {
            'ref' => '/container_profiles/1',
            '_resolved' => {'display_string' => 'Native Carton'},
          },
          'container_locations' => [
            {
              'ref' => '/locations/1',
              'status' => 'current',
              '_resolved' => {
                'title' => 'Native Vault',
                'location_profile' => {
                  '_resolved' => {'display_string' => 'Native Shelf'},
                },
              },
            },
          ],
        },
      }

      marker_seen_in_prepare_hook = :uncalled
      indexer.add_document_prepare_hook do |doc, record|
        if record['record']['jsonmodel_type'] == 'top_container'
          marker_seen_in_prepare_hook = doc['_archivalist_rust_fields']&.dup
        end
      end

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['fullrecord_published'] = []
          doc['fullrecord'] = []
          doc['external_id'] = ['rust-tc-ext']
          doc['four_part_id'] = 'rust-tc-4p'
          doc['ancestors'] = ['/repositories/2/resources/77']
          doc['rights_statement_agent_uris'] = ['/agents/people/5']
          doc['langcode'] = ['eng']
          doc['title'] = 'Rust Long Display'
          doc['display_string'] = 'Rust Box 44'
          doc['type_u_ssort'] = 'rust-box'
          doc['notes'] = 'Rust internal note'
          doc['series_uri_u_sstr'] = ['/repositories/2/archival_objects/900']
          doc['series_title_u_sstr'] = ['Rust Series']
          doc['series_level_u_sstr'] = ['Series']
          doc['series_identifier_stored_u_sstr'] = ['RS-1']
          doc['series_identifier_u_stext'] = ['RS-1', 'RS 1', 'RS1', 'RS- 1']
          doc['published_series_uri_u_sstr'] = ['/repositories/2/archival_objects/900']
          doc['published_series_title_u_sstr'] = ['Rust Series']
          doc['collection_uri_u_sstr'] = ['/repositories/2/resources/77']
          doc['collection_display_string_u_sstr'] = ['Rust Collection']
          doc['collection_identifier_stored_u_sstr'] = ['RC-1']
          doc['collection_identifier_u_stext'] = ['RC-1', 'RC 1', 'RC1', 'RC- 1']
          doc['container_profile_uri_u_sstr'] = '/container_profiles/9'
          doc['container_profile_display_string_u_sstr'] = 'Rust Carton'
          doc['has_location_u_sbool'] = true
          doc['location_uri_u_sstr'] = ['/locations/9']
          doc['location_uris'] = ['/locations/9']
          doc['location_display_string_u_sstr'] = ['Rust Vault']
          doc['location_profile_display_string_u_sstr'] = ['Rust Shelf']
          doc['exported_u_sbool'] = true
          doc['empty_u_sbool'] = false
          doc['indicator_u_icusort'] = '44A'
          doc['top_container_u_typeahead_utext'] = 'rustbox44'
          doc['top_container_u_icusort'] = 'Rust Box 44'
          doc['barcode_u_sstr'] = 'RUST-BC'
          doc['barcode_u_icusort'] = 'RUST-BC'
          doc['ils_holding_id_u_sstr'] = 'RUST-ILS'
          doc['subcontainer_barcodes_u_sstr'] = ['RUST-SUB']
          doc['created_for_collection_u_sstr'] = '/repositories/2/resources/77'
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'top_container_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.index_records([top_container])
      tc_doc = indexer.records.first

      expect(marker_seen_in_prepare_hook).to be_nil
      expect(tc_doc['title']).to eq('Rust Long Display')
      expect(tc_doc['display_string']).to eq('Rust Box 44')
      expect(tc_doc['type_u_ssort']).to eq('rust-box')
      expect(tc_doc['notes']).to eq('Rust internal note')
      expect(tc_doc['series_identifier_u_stext']).to eq(['RS-1', 'RS 1', 'RS1', 'RS- 1'])
      expect(tc_doc['published_series_uri_u_sstr']).to eq(['/repositories/2/archival_objects/900'])
      expect(tc_doc['collection_identifier_u_stext']).to eq(['RC-1', 'RC 1', 'RC1', 'RC- 1'])
      expect(tc_doc['container_profile_display_string_u_sstr']).to eq('Rust Carton')
      expect(tc_doc['location_display_string_u_sstr']).to eq(['Rust Vault'])
      expect(tc_doc['location_profile_display_string_u_sstr']).to eq(['Rust Shelf'])
      expect(tc_doc['indicator_u_icusort']).to eq('44A')
      expect(tc_doc['top_container_u_typeahead_utext']).to eq('rustbox44')
      expect(tc_doc['top_container_u_icusort']).to eq('Rust Box 44')
      expect(tc_doc['barcode_u_sstr']).to eq('RUST-BC')
      expect(tc_doc['external_id']).to eq(['rust-tc-ext'])
      expect(tc_doc['four_part_id']).to eq('rust-tc-4p')
      expect(tc_doc['ancestors']).to eq(['/repositories/2/resources/77'])
      expect(tc_doc['rights_statement_agent_uris']).to eq(['/agents/people/5'])
      expect(tc_doc['langcode']).to eq(['eng'])
      expect(tc_doc['publish']).to eq(true)
      expect(tc_doc['types']).to include('top_container', 'pui_container', 'pui')
      expect(tc_doc).not_to have_key('_archivalist_rust_fields')
    end

    it "falls back to the native top_container hook when the sidecar returns nil and keeps instance container links native" do
      top_container = {
        'uri' => '/repositories/2/top_containers/45',
        'record' => {
          'jsonmodel_type' => 'top_container',
          'long_display_string' => 'Fallback Collection: Box 5 [FB-1]',
          'display_string' => 'Box 5 [FB-1]',
          'type' => 'box',
          'indicator' => '5',
          'barcode' => 'FB-1',
          'ils_holding_id' => 'ILS-FB',
          'internal_note' => 'Fallback note',
          'exported_to_ils' => false,
          'subcontainer_barcodes' => ['FB-SUB'],
          'created_for_collection' => '/repositories/2/resources/9',
          'is_linked_to_published_record' => false,
          'series' => [
            {
              'ref' => '/repositories/2/archival_objects/50',
              'display_string' => 'Fallback Series',
              'level_display_string' => 'Series',
              'identifier' => 'FB-01',
              'publish' => false,
            },
          ],
          'collection' => [],
          'container_profile' => {
            'ref' => '/container_profiles/2',
            '_resolved' => {'display_string' => 'Fallback Carton'},
          },
          'container_locations' => [
            {
              'ref' => '/locations/50',
              'status' => 'previous',
              '_resolved' => {'title' => 'Old Vault'},
            },
          ],
        },
      }
      archival_object = {
        'uri' => '/repositories/2/archival_objects/88',
        'record' => {
          'jsonmodel_type' => 'archival_object',
          'title' => 'Foldered item',
          'display_string' => 'Foldered item',
          'component_id' => 'AO-88',
          'ref_id' => 'ref-88',
          'resource' => {'ref' => '/repositories/2/resources/9'},
          'instances' => [
            {
              'sub_container' => {
                'top_container' => {
                  'ref' => '/repositories/2/top_containers/45',
                  '_resolved' => {
                    'container_locations' => [{'ref' => '/locations/50'}],
                  },
                },
                'type_2' => 'folder',
                'indicator_2' => '3',
                'barcode_2' => 'FOLD-3',
                'type_3' => 'item',
                'indicator_3' => '1',
              },
            },
          ],
        },
      }

      allow(indexer).to receive(:rust_index_documents).and_return(nil)

      indexer.index_records([top_container, archival_object])
      tc_doc = indexer.records.find {|d| d['primary_type'] == 'top_container'}
      ao_doc = indexer.records.find {|d| d['primary_type'] == 'archival_object'}

      expect(tc_doc['title']).to eq('Fallback Collection: Box 5 [FB-1]')
      expect(tc_doc['display_string']).to eq('Box 5 [FB-1]')
      expect(tc_doc['type_u_ssort']).to eq('box')
      expect(tc_doc['notes']).to eq('Fallback note')
      expect(tc_doc['series_identifier_u_stext']).to eq(['FB-01', 'FB 01', 'FB01', 'FB- 01'])
      expect(tc_doc).not_to have_key('published_series_uri_u_sstr')
      expect(tc_doc).not_to have_key('published_series_title_u_sstr')
      expect(tc_doc['collection_uri_u_sstr']).to eq([])
      expect(tc_doc['collection_display_string_u_sstr']).to eq([])
      expect(tc_doc['collection_identifier_stored_u_sstr']).to eq([])
      expect(tc_doc['collection_identifier_u_stext']).to eq([])
      expect(tc_doc['container_profile_uri_u_sstr']).to eq('/container_profiles/2')
      expect(tc_doc['container_profile_display_string_u_sstr']).to eq('Fallback Carton')
      expect(tc_doc['has_location_u_sbool']).to eq(true)
      expect(tc_doc['location_uri_u_sstr']).to eq([])
      expect(tc_doc['location_uris']).to eq([])
      expect(tc_doc['location_display_string_u_sstr']).to eq([])
      expect(tc_doc['location_profile_display_string_u_sstr']).to eq([])
      expect(tc_doc['exported_u_sbool']).to eq(true)
      expect(tc_doc['empty_u_sbool']).to eq(true)
      expect(tc_doc['indicator_u_icusort']).to eq('5')
      expect(tc_doc['top_container_u_typeahead_utext']).to eq('box5fb1')
      expect(tc_doc['top_container_u_icusort']).to eq('Box 5 [FB-1]')
      expect(tc_doc['barcode_u_sstr']).to eq('FB-1')
      expect(tc_doc['barcode_u_icusort']).to eq('FB-1')
      expect(tc_doc['ils_holding_id_u_sstr']).to eq('ILS-FB')
      expect(tc_doc['subcontainer_barcodes_u_sstr']).to eq(['FB-SUB'])
      expect(tc_doc['created_for_collection_u_sstr']).to eq('/repositories/2/resources/9')
      expect(tc_doc['publish']).to eq(false)
      expect(tc_doc['types']).to eq(['top_container'])
      expect(tc_doc).not_to have_key('_archivalist_rust_fields')

      expect(ao_doc['top_container_uri_u_sstr']).to eq(['/repositories/2/top_containers/45'])
      expect(ao_doc['child_container_u_sstr']).to eq(['folder 3 FOLD-3'])
      expect(ao_doc['grand_child_container_u_sstr']).to eq(['item 1'])
      expect(ao_doc).not_to have_key('location_uris')
    end

    it "retains Rust job fields, executes native I18n, time, HTTP, files, and job_data exactly once, strips job_metadata before downstream hooks, and bypasses PUI" do
      report_job = {
        'uri' => '/repositories/2/jobs/101',
        'record' => {
          'jsonmodel_type' => 'job',
          'job_type' => 'report_job',
          'status' => 'native_status',
          'owner' => 'native_owner',
          'queue_position' => 99,
          'time_submitted' => '2026-10-17 10:00:00 UTC',
          'time_started' => '2026-10-17 10:01:00 UTC',
          'time_finished' => '2026-10-17 10:05:00 UTC',
          'job' => {
            'jsonmodel_type' => 'report_job',
            'report_type' => 'assessment_list_report',
            'format' => 'pdf',
            'custom_param' => 'custom_val',
          },
        },
      }
      import_job = {
        'uri' => '/repositories/2/jobs/102',
        'record' => {
          'jsonmodel_type' => 'job',
          'job_type' => 'import_job',
          'status' => 'native_status',
          'owner' => 'native_owner',
          'queue_position' => 88,
          'job' => {
            'jsonmodel_type' => 'import_job',
            'filenames' => ['ead_sample.xml'],
            'import_type' => 'ead_xml',
          },
        },
      }

      allow(JSONModel::HTTP).to receive(:get_json).with('/repositories/2/jobs/101/output_files').and_return(['report.pdf'])
      allow(JSONModel::HTTP).to receive(:get_json).with('/repositories/2/jobs/102/output_files').and_return(['import.log'])

      marker_seen_in_downstream_hook = :uncalled
      indexer.add_document_prepare_hook do |doc, record|
        if record['record']['jsonmodel_type'] == 'job'
          marker_seen_in_downstream_hook = doc['_archivalist_rust_fields']
        end
      end

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, documents|
          doc = indexer.native_index_document(
            prepared[:uri], prepared[:record_type], prepared[:values]
          )
          doc['fullrecord_published'] = []
          doc['fullrecord'] = []
          doc['external_id'] = ['rust-job-ext']
          doc['ancestors'] = ['/repositories/2/resources/77']
          doc['langcode'] = ['eng']
          if prepared[:uri] == '/repositories/2/jobs/101'
            doc['types'] = ['job', 'report_job', 'assessment_list_report']
            doc['job_type'] = 'report_job'
            doc['report_type'] = 'assessment_list_report'
            doc['job_report_type'] = 'assessment_list_report'
            doc['status'] = 'queued'
            doc['owner'] = 'rust_admin'
            doc['queue_position'] = 2
          else
            doc['types'] = ['job', 'import_job', nil]
            doc['job_type'] = 'import_job'
            doc['report_type'] = nil
            doc['job_report_type'] = 'import_job'
            doc['status'] = 'running'
            doc['owner'] = 'rust_archivist'
            doc['queue_position'] = nil
          end
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'job_metadata',
          ]
          documents[prepared[:uri]] = doc
        end
      end

      indexer.index_records([report_job, import_job])
      rep_doc = indexer.records.find {|d| d['primary_type'] == 'job' && d['id'] == '/repositories/2/jobs/101'}
      imp_doc = indexer.records.find {|d| d['primary_type'] == 'job' && d['id'] == '/repositories/2/jobs/102'}

      expect(marker_seen_in_downstream_hook).to be_nil

      # Rust owns deterministic staff metadata:
      expect(rep_doc['job_type']).to eq('report_job')
      expect(rep_doc['report_type']).to eq('assessment_list_report')
      expect(rep_doc['job_report_type']).to eq('assessment_list_report')
      expect(rep_doc['status']).to eq('queued')
      expect(rep_doc['owner']).to eq('rust_admin')
      expect(rep_doc['queue_position']).to eq(2)
      expect(rep_doc['types']).to eq(['job', 'report_job', 'assessment_list_report'])

      expect(imp_doc['job_type']).to eq('import_job')
      expect(imp_doc['report_type']).to be_nil
      expect(imp_doc['job_report_type']).to eq('import_job')
      expect(imp_doc['status']).to eq('running')
      expect(imp_doc['owner']).to eq('rust_archivist')
      expect(imp_doc['queue_position']).to be_nil
      expect(imp_doc['types']).to eq(['job', 'import_job', nil])

      # Ruby retains translated title via I18n:
      expect(rep_doc['title']).to eq(I18n.t("reports.assessment_list_report.title", :default => 'assessment_list_report'))
      expect(imp_doc['title']).to eq(I18n.t("job.types.import_job"))

      # Ruby retains Time.parse(...).getlocal:
      expect(rep_doc['time_submitted']).to eq(Time.parse('2026-10-17 10:00:00 UTC').getlocal)
      expect(rep_doc['time_started']).to eq(Time.parse('2026-10-17 10:01:00 UTC').getlocal)
      expect(rep_doc['time_finished']).to eq(Time.parse('2026-10-17 10:05:00 UTC').getlocal)

      # Ruby retains JSONModel::HTTP output_files lookup and files/job_data construction:
      expect(rep_doc['files']).to eq(['/jobs/101/file/report.pdf'])
      expect(rep_doc['job_data']).to include('output_file --- /jobs/101/file/report.pdf')
      expect(rep_doc['job_data']).to include('format --- pdf')
      expect(rep_doc['job_data']).to include('custom_param --- custom_val')

      expect(imp_doc['files']).to eq(['/jobs/102/file/import.log'])
      expect(imp_doc['job_data']).to include('input_file --- ead_sample.xml')
      expect(imp_doc['job_data']).to include('import_type --- ead_xml')

      # Marker removed before Solr / downstream:
      expect(rep_doc).not_to have_key('_archivalist_rust_fields')
      expect(imp_doc).not_to have_key('_archivalist_rust_fields')

      # PUI bypass:
      expect(rep_doc['publish']).to eq(false)
      expect(rep_doc['types']).not_to include('pui', 'pui_record')
      expect(imp_doc['publish']).to eq(false)
      expect(imp_doc['types']).not_to include('pui', 'pui_record')
    end

    it "falls back to the native job hook when the sidecar returns nil or errors, preserving all native job fields and unrelated types" do
      report_job = {
        'uri' => '/repositories/2/jobs/103',
        'record' => {
          'jsonmodel_type' => 'job',
          'job_type' => 'report_job',
          'status' => 'completed',
          'owner' => 'fallback_user',
          'queue_position' => 5,
          'time_submitted' => '2026-10-17 11:00:00 UTC',
          'time_started' => '2026-10-17 11:01:00 UTC',
          'time_finished' => '2026-10-17 11:04:00 UTC',
          'job' => {
            'jsonmodel_type' => 'report_job',
            'report_type' => 'assessment_list_report',
            'format' => 'csv',
          },
        },
      }
      top_container = {
        'uri' => '/repositories/2/top_containers/46',
        'record' => {
          'jsonmodel_type' => 'top_container',
          'long_display_string' => 'Box 46',
          'display_string' => 'Box 46',
          'type' => 'box',
          'indicator' => '46',
          'barcode' => 'FB-46',
          'is_linked_to_published_record' => false,
        },
      }

      allow(JSONModel::HTTP).to receive(:get_json).with('/repositories/2/jobs/103/output_files').and_return(['report.csv'])

      allow(indexer).to receive(:rust_index_documents).and_return(nil)

      indexer.index_records([report_job, top_container])
      job_doc = indexer.records.find {|d| d['primary_type'] == 'job' && d['id'] == '/repositories/2/jobs/103'}
      tc_doc = indexer.records.find {|d| d['primary_type'] == 'top_container' && d['id'] == '/repositories/2/top_containers/46'}

      expect(job_doc['job_type']).to eq('report_job')
      expect(job_doc['report_type']).to eq('assessment_list_report')
      expect(job_doc['job_report_type']).to eq('assessment_list_report')
      expect(job_doc['status']).to eq('completed')
      expect(job_doc['owner']).to eq('fallback_user')
      expect(job_doc['queue_position']).to eq(5)
      expect(job_doc['types']).to eq(['job', 'report_job', 'assessment_list_report'])
      expect(job_doc['title']).to eq(I18n.t("reports.assessment_list_report.title", :default => 'assessment_list_report'))
      expect(job_doc['time_submitted']).to eq(Time.parse('2026-10-17 11:00:00 UTC').getlocal)
      expect(job_doc['files']).to eq(['/jobs/103/file/report.csv'])
      expect(job_doc['job_data']).to include('output_file --- /jobs/103/file/report.csv')
      expect(job_doc['job_data']).to include('format --- csv')
      expect(job_doc).not_to have_key('_archivalist_rust_fields')
      expect(job_doc['publish']).to eq(false)

      # Unrelated top container remains correctly indexed via its native fallback:
      expect(tc_doc['display_string']).to eq('Box 46')
      expect(tc_doc['barcode_u_sstr']).to eq('FB-46')
      expect(tc_doc).not_to have_key('_archivalist_rust_fields')
    end
  end

  describe "collection_management child document indexing" do
    let(:full_cm) do
      {
        'uri' => '/repositories/2/collection_management/10',
        'processing_priority' => ' high ',
        'processing_status' => ' completed ',
        'processing_hours_total' => ' 40 ',
        'processing_funding_source' => ' grant-funded ',
        'processors' => ' Alice and Bob ',
        'created_by' => 'admin',
        'last_modified_by' => 'archivist',
        'system_mtime' => '2026-01-01T00:00:00Z',
        'user_mtime' => '2026-01-01T00:00:00Z',
        'create_time' => '2026-01-01T00:00:00Z',
      }
    end

    let(:resource_with_cm) do
      {
        'uri' => '/repositories/2/resources/1',
        'record' => {
          'uri' => '/repositories/2/resources/1',
          'jsonmodel_type' => 'resource',
          'title' => ' <emph render="italic">Papers</emph> - Collection 1 ',
          'id_0' => 'COL-1',
          'level' => 'collection',
          'suppressed' => true,
          'collection_management' => full_cm,
        },
      }
    end

    let(:minimal_cm) do
      {
        'uri' => '/repositories/2/collection_management/11',
      }
    end

    let(:accession_with_cm) do
      {
        'uri' => '/repositories/2/accessions/2',
        'record' => {
          'uri' => '/repositories/2/accessions/2',
          'jsonmodel_type' => 'accession',
          'display_string' => 'Accession 2026-01',
          'title' => nil,
          'id_0' => 'ACC-2',
          'suppressed' => false,
          'collection_management' => minimal_cm,
        },
      }
    end

    let(:digital_object_with_cm) do
      {
        'uri' => '/repositories/2/digital_objects/3',
        'record' => {
          'uri' => '/repositories/2/digital_objects/3',
          'jsonmodel_type' => 'digital_object',
          'title' => 'Digital Object 3',
          'digital_object_id' => 'DO-3',
          'collection_management' => {
            'uri' => '/repositories/2/collection_management/12',
          },
        },
      }
    end

    it "indexes Rust-generated collection_management child documents, suppresses only the built-in hook, and executes third-party hooks exactly once" do
      third_party_calls = 0
      indexer.add_extra_documents_hook do |record|
        third_party_calls += 1
        [{ 'id' => "#{record['uri']}#plugin_extra", 'types' => ['plugin_doc'], 'primary_type' => 'plugin_doc' }]
      end

      marker_seen_in_prepare = nil
      indexer.add_document_prepare_hook do |doc, record|
        marker_seen_in_prepare = true if doc.has_key?('_archivalist_collection_management_documents')
      end

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        documents = {}
        prepared_records.each do |prepared|
          uri = prepared[:uri]
          rec = prepared[:values]
          doc = indexer.native_index_document(uri, prepared[:record_type], rec)
          if uri == '/repositories/2/resources/1'
            child = {
              'id' => '/repositories/2/collection_management/10',
              'uri' => '/repositories/2/collection_management/10',
              'parent_id' => '/repositories/2/resources/1',
              'parent_title' => '<emph render="italic">Papers</emph> - Collection 1',
              'parent_type' => 'resource',
              'title' => '<emph render="italic">Papers</emph> - Collection 1',
              'title_sort' => 'Papers Collection 1',
              'primary_type' => 'collection_management',
              'types' => ['collection_management'],
              'suppressed' => true,
              'repository' => 2,
              'processing_priority' => 'high',
              'processing_status' => 'completed',
              'processing_hours_total' => '40',
              'processing_funding_source' => 'grant-funded',
              'processors' => 'Alice and Bob',
              'created_by' => 'admin',
              'last_modified_by' => 'archivist',
              'system_mtime' => '2026-01-01T00:00:00Z',
              'user_mtime' => '2026-01-01T00:00:00Z',
              'create_time' => '2026-01-01T00:00:00Z',
              'json' => ASUtils.to_json(full_cm),
            }
            doc['_archivalist_collection_management_documents'] = [child]
          elsif uri == '/repositories/2/accessions/2'
            child = {
              'id' => '/repositories/2/collection_management/11',
              'uri' => '/repositories/2/collection_management/11',
              'parent_id' => '/repositories/2/accessions/2',
              'parent_title' => 'Accession 2026-01',
              'parent_type' => 'accession',
              'title' => 'Accession 2026-01',
              'title_sort' => 'Accession 2026 01',
              'primary_type' => 'collection_management',
              'types' => ['collection_management'],
              'suppressed' => false,
              'repository' => 2,
              'processing_priority' => nil,
              'processing_status' => nil,
              'processing_hours_total' => nil,
              'processing_funding_source' => nil,
              'processors' => nil,
              'created_by' => nil,
              'last_modified_by' => nil,
              'system_mtime' => nil,
              'user_mtime' => nil,
              'create_time' => nil,
              'json' => ASUtils.to_json(minimal_cm),
            }
            doc['_archivalist_collection_management_documents'] = [child]
          end
          documents[uri] = doc
        end
        documents
      end

      indexer.index_records([resource_with_cm, accession_with_cm])

      # Exactly one child per parent with CM
      cm_docs = indexer.records.select {|d| d['primary_type'] == 'collection_management'}
      expect(cm_docs.length).to eq(2)

      res_child = cm_docs.find {|d| d['parent_id'] == '/repositories/2/resources/1'}
      expect(res_child['id']).to eq('/repositories/2/collection_management/10')
      expect(res_child['parent_title']).to eq('<emph render="italic">Papers</emph> - Collection 1')
      expect(res_child['parent_type']).to eq('resource')
      expect(res_child['title_sort']).to eq('Papers Collection 1')
      expect(res_child['processing_priority']).to eq('high')
      expect(res_child['suppressed']).to eq(true)

      acc_child = cm_docs.find {|d| d['parent_id'] == '/repositories/2/accessions/2'}
      expect(acc_child['id']).to eq('/repositories/2/collection_management/11')
      expect(acc_child['parent_title']).to eq('Accession 2026-01')
      expect(acc_child['parent_type']).to eq('accession')
      expect(acc_child['title_sort']).to eq('Accession 2026 01')
      expect(acc_child['suppressed']).to eq(false)

      # Third-party extra documents hook called exactly once per record (2 records)
      expect(third_party_calls).to eq(2)
      plugin_docs = indexer.records.select {|d| d['primary_type'] == 'plugin_doc'}
      expect(plugin_docs.length).to eq(2)

      # Document ordering in batch: parent -> collection_management -> third-party
      res_idx = indexer.records.index {|d| d['id'] == '/repositories/2/resources/1'}
      cm_idx = indexer.records.index {|d| d['id'] == '/repositories/2/collection_management/10'}
      plugin_idx = indexer.records.index {|d| d['id'] == '/repositories/2/resources/1#plugin_extra'}
      expect(res_idx).to be < cm_idx
      expect(cm_idx).to be < plugin_idx

      # No private metadata in prepare hook or final batch
      expect(marker_seen_in_prepare).to be_nil
      indexer.records.each do |doc|
        expect(doc.keys.none? {|k| k.start_with?('_archivalist_')}).to be(true)
      end

      # records_with_children retains collection_management for stale-child deletion
      expect(indexer.records_with_children).to include('collection_management')

      # Proves no mutable cross-record instance state is retained on indexer
      expect(indexer.instance_variable_defined?(:@active_collection_management_children)).to be(false)
    end

    it "falls back to native child document generation when sidecar is unavailable or returns nil" do
      allow(indexer).to receive(:rust_index_documents).and_return(nil)

      indexer.index_records([resource_with_cm])

      cm_docs = indexer.records.select {|d| d['primary_type'] == 'collection_management'}
      expect(cm_docs.length).to eq(1)
      child = cm_docs.first
      expect(child['id']).to eq('/repositories/2/collection_management/10')
      expect(child['parent_id']).to eq('/repositories/2/resources/1')
      expect(child['parent_type']).to eq('resource')
      expect(child['suppressed']).to eq(true)
      expect(child.keys.none? {|k| k.start_with?('_archivalist_')}).to be(true)
    end

    it "falls back to native child document generation when enrichment is disabled" do
      allow(indexer).to receive(:rust_index_document_enrichment_enabled?).and_return(false)
      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        # When enrich is false, Rust returns envelope without child handoff
        prepared_records.each_with_object({}) do |prepared, docs|
          docs[prepared[:uri]] = indexer.native_index_document(prepared[:uri], prepared[:record_type], prepared[:values])
        end
      end

      indexer.index_records([resource_with_cm])

      cm_docs = indexer.records.select {|d| d['primary_type'] == 'collection_management'}
      expect(cm_docs.length).to eq(1)
      child = cm_docs.first
      expect(child['id']).to eq('/repositories/2/collection_management/10')
      expect(child['parent_id']).to eq('/repositories/2/resources/1')
    end

    it "falls back to native child document generation when Rust withholds child handoff" do
      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, docs|
          # Enriched doc returned, but child handoff omitted (withheld due to parity gate)
          doc = indexer.native_index_document(prepared[:uri], prepared[:record_type], prepared[:values])
          doc['_archivalist_rust_fields'] = ['common_metadata', 'fullrecord', 'resource_archival_object_metadata']
          docs[prepared[:uri]] = doc
        end
      end

      indexer.index_records([resource_with_cm])

      cm_docs = indexer.records.select {|d| d['primary_type'] == 'collection_management'}
      expect(cm_docs.length).to eq(1)
      child = cm_docs.first
      expect(child['id']).to eq('/repositories/2/collection_management/10')
      expect(child['parent_id']).to eq('/repositories/2/resources/1')
      expect(child.keys.none? {|k| k.start_with?('_archivalist_')}).to be(true)
    end

    it "does not emit child documents or suppression signal when collection_management is absent" do
      resource_no_cm = {
        'uri' => '/repositories/2/resources/99',
        'record' => {
          'uri' => '/repositories/2/resources/99',
          'jsonmodel_type' => 'resource',
          'title' => 'Resource without CM',
          'id_0' => 'R99',
          'level' => 'collection',
        },
      }

      indexer.index_records([resource_no_cm])

      cm_docs = indexer.records.select {|d| d['primary_type'] == 'collection_management'}
      expect(cm_docs).to be_empty
    end

    it "indexes digital_object parent with collection_management via native path" do
      indexer.index_records([digital_object_with_cm])

      cm_docs = indexer.records.select {|d| d['primary_type'] == 'collection_management'}
      expect(cm_docs.length).to eq(1)
      expect(cm_docs.first['parent_type']).to eq('digital_object')
      expect(cm_docs.first['parent_id']).to eq('/repositories/2/digital_objects/3')
    end

    it "ensures PUI indexer path uses no Rust child handoff and retains native behavior" do
      pui_idx = PeriodicIndexer.new
      def pui_idx.index_batch(batch, timing); @index_batch = batch; end
      def pui_idx.records; @index_batch ? @index_batch.to_a : []; end
      allow(pui_idx).to receive_message_chain(:class, :name).and_return('PUIIndexer')
      expect(pui_idx.rust_index_document_enrichment_enabled?).to be(false)

      pui_idx.index_records([resource_with_cm])

      cm_docs = pui_idx.records.select {|d| d['primary_type'] == 'collection_management'}
      expect(cm_docs.length).to eq(1)
      child = cm_docs.first
      expect(child['id']).to eq('/repositories/2/collection_management/10')
      expect(child['parent_id']).to eq('/repositories/2/resources/1')
      expect(child['parent_type']).to eq('resource')
      expect(child.keys.none? {|k| k.start_with?('_archivalist_')}).to be(true)
    end

    it "ensures no Rust child state can leak into a later record if a document-prepare hook raises or interrupts" do
      hook_should_raise = true
      indexer.add_document_prepare_hook do |doc, record|
        if hook_should_raise && record['uri'] == '/repositories/2/resources/1'
          raise "Simulated failure in document-prepare hook"
        end
      end

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        documents = {}
        prepared_records.each do |prepared|
          uri = prepared[:uri]
          rec = prepared[:values]
          doc = indexer.native_index_document(uri, prepared[:record_type], rec)
          if uri == '/repositories/2/resources/1'
            child = {
              'id' => '/repositories/2/collection_management/10',
              'uri' => '/repositories/2/collection_management/10',
              'parent_id' => '/repositories/2/resources/1',
              'parent_title' => 'Papers - Collection 1',
              'parent_type' => 'resource',
              'title' => 'Papers - Collection 1',
              'title_sort' => 'Papers Collection 1',
              'primary_type' => 'collection_management',
              'types' => ['collection_management'],
              'suppressed' => true,
              'repository' => 2,
              'processing_priority' => 'high',
              'processing_status' => 'completed',
              'processing_hours_total' => '40',
              'processing_funding_source' => 'grant-funded',
              'processors' => 'Alice and Bob',
              'created_by' => 'admin',
              'last_modified_by' => 'archivist',
              'system_mtime' => '2026-01-01T00:00:00Z',
              'user_mtime' => '2026-01-01T00:00:00Z',
              'create_time' => '2026-01-01T00:00:00Z',
              'json' => ASUtils.to_json(full_cm),
            }
            doc['_archivalist_collection_management_documents'] = [child]
          end
          documents[uri] = doc
        end
        documents
      end

      expect {
        indexer.index_records([resource_with_cm])
      }.to raise_error("Simulated failure in document-prepare hook")

      expect(indexer.instance_variable_defined?(:@active_collection_management_children)).to be(false)

      hook_should_raise = false
      resource_no_cm = {
        'uri' => '/repositories/2/resources/99',
        'record' => {
          'uri' => '/repositories/2/resources/99',
          'jsonmodel_type' => 'resource',
          'title' => 'Resource without CM',
          'id_0' => 'R99',
          'level' => 'collection',
        },
      }

      indexer.index_records([resource_no_cm])

      cm_docs = indexer.records.select {|d| d['primary_type'] == 'collection_management'}
      expect(cm_docs).to be_empty
    end

    it "falls back natively at the IndexerCommon integration boundary when Rust child is malformed" do
      corrupted_child = {
        'id' => '/repositories/2/collection_management/10',
        'uri' => '/repositories/2/collection_management/10',
        'parent_id' => '/repositories/2/resources/WRONG_PARENT',
        'parent_title' => 'Papers - Collection 1',
        'parent_type' => 'resource',
        'title' => 'Papers - Collection 1',
        'title_sort' => 'Papers Collection 1',
        'primary_type' => 'collection_management',
        'types' => ['collection_management'],
        'suppressed' => true,
        'repository' => 2,
        'processing_priority' => 'high',
        'processing_status' => 'completed',
        'processing_hours_total' => '40',
        'processing_funding_source' => 'grant-funded',
        'processors' => 'Alice and Bob',
        'created_by' => 'admin',
        'last_modified_by' => 'archivist',
        'system_mtime' => '2026-01-01T00:00:00Z',
        'user_mtime' => '2026-01-01T00:00:00Z',
        'create_time' => '2026-01-01T00:00:00Z',
        'json' => ASUtils.to_json(full_cm),
      }

      mock_response = Struct.new(:code, :body).new(
        '200',
        ASUtils.to_json(
          'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
          'documents' => [{
            'id' => '/repositories/2/resources/1',
            'uri' => '/repositories/2/resources/1',
            'title' => 'Papers - Collection 1',
            'primary_type' => 'resource',
            'types' => ['resource'],
            'json' => ASUtils.to_json(resource_with_cm['record']),
            'suppressed' => true,
            'publish' => false,
            'system_generated' => 'false',
            'repository' => 2,
            'fullrecord_published' => [],
            'fullrecord' => [],
            'finding_aid_title' => nil,
            'finding_aid_filing_title' => nil,
            'identifier' => 'COL-1',
            'resource_type' => nil,
            'level' => 'collection',
            'restrictions' => false,
            'ead_id' => nil,
            'finding_aid_status' => nil,
            'related_accession_uris' => [],
            'slug' => nil,
            'is_slug_auto' => false,
            'external_id' => [],
            'ancestors' => [],
            'langcode' => [],
            'four_part_id' => 'COL-1',
            'processing_priority' => 'high',
            'processors' => 'Alice and Bob',
            '_archivalist_rust_fields' => [
              'common_metadata',
              'fullrecord',
              'resource_archival_object_metadata',
            ],
            '_archivalist_collection_management_documents' => [corrupted_child],
          }]
        )
      )
      allow_any_instance_of(RustIndexDocumentBuilder).to receive(:perform_request).and_return(mock_response)

      indexer.index_records([resource_with_cm])

      cm_docs = indexer.records.select {|d| d['primary_type'] == 'collection_management'}
      expect(cm_docs.length).to eq(1)
      child = cm_docs.first
      expect(child['id']).to eq('/repositories/2/collection_management/10')
      expect(child['parent_id']).to eq('/repositories/2/resources/1')
      expect(child['parent_type']).to eq('resource')
      expect(child.keys.none? {|k| k.start_with?('_archivalist_')}).to be(true)
    end
  end

  describe "dynamic enum and ARK acceleration and fallback" do
    before(:each) do
      AppConfig[:rust_index_document_builder_enabled] = true
      AppConfig[:rust_index_document_enrichment_enabled] = true
      AppConfig[:arks_enabled] = true
    end

    after(:each) do
      AppConfig[:rust_index_document_builder_enabled] = false
      AppConfig[:rust_index_document_enrichment_enabled] = false
      AppConfig[:arks_enabled] = false
    end

    let(:resource_with_enums_and_arks) do
      {
        'uri' => '/repositories/2/resources/55',
        'record' => {
          'uri' => '/repositories/2/resources/55',
          'title' => 'Enum and ARK Resource',
          'display_string' => 'Enum and ARK Resource',
          'publish' => true,
          'status' => 'draft',
          'notes' => [
            { 'status' => 'reviewed', 'items' => { 'type' => 'container_summary' } },
            { 'status' => 'draft' },
            { '_resolved' => { 'status' => 'excluded_resolved' } },
          ],
          'ark_name' => {
            'current' => 'https://example.org/ark:/12345/curr55',
            'previous' => ['http://proxy/ark:/12345/ark:/prev55'],
          },
        },
      }
    end

    it "skips native enum traversal and add_arks when valid Rust projection and marker are present" do
      allow(indexer).to receive(:enum_fields).and_return(['status'])

      prepare_hook_calls = 0
      indexer.add_document_prepare_hook do |_doc, _rec|
        prepare_hook_calls += 1
      end

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, docs|
          doc = indexer.native_index_document(prepared[:uri], prepared[:record_type], prepared[:values])
          doc['status_enum_s'] = ['draft', 'reviewed']
          doc['type_enum_s'] = ['container_summary']
          doc['ark_name'] = ['ark:/12345/curr55', 'ark:/prev55']
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'resource_archival_object_metadata',
            'dynamic_enum_and_ark_metadata',
          ]
          docs[prepared[:uri]] = doc
        end
      end

      indexer.index_records([resource_with_enums_and_arks])

      doc = indexer.records.find {|d| d['id'] == '/repositories/2/resources/55'}
      expect(doc['status_enum_s']).to eq(['draft', 'reviewed'])
      expect(doc['type_enum_s']).to eq(['container_summary'])
      expect(doc['ark_name']).to eq(['ark:/12345/curr55', 'ark:/prev55'])
      expect(doc).not_to have_key('_archivalist_rust_fields')
      expect(prepare_hook_calls).to be > 0
    end

    it "falls back to complete native enum and ARK preparation when projection is invalid or corrupted" do
      allow(indexer).to receive(:enum_fields).and_return(['status'])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, docs|
          doc = indexer.native_index_document(prepared[:uri], prepared[:record_type], prepared[:values])
          # Corrupted: bogus enum values and bogus ark_name
          doc['status_enum_s'] = ['corrupted_val']
          doc['ark_name'] = ['ark:/bogus']
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'resource_archival_object_metadata',
            'dynamic_enum_and_ark_metadata',
          ]
          docs[prepared[:uri]] = doc
        end
      end

      indexer.index_records([resource_with_enums_and_arks])

      doc = indexer.records.find {|d| d['id'] == '/repositories/2/resources/55'}
      # Mismatch caused complete native preparation for both slices
      expect(doc['status_enum_s']).to eq(['draft', 'reviewed'])
      expect(doc['type_enum_s']).to eq(['container_summary'])
      expect(doc['ark_name']).to eq(['ark:/12345/curr55', 'ark:/prev55'])
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "omits ark_name when ARKs are disabled in AppConfig" do
      AppConfig[:arks_enabled] = false
      allow(indexer).to receive(:enum_fields).and_return(['status'])

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, docs|
          doc = indexer.native_index_document(prepared[:uri], prepared[:record_type], prepared[:values])
          doc['status_enum_s'] = ['draft', 'reviewed']
          doc['type_enum_s'] = ['container_summary']
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'resource_archival_object_metadata',
            'dynamic_enum_and_ark_metadata',
          ]
          docs[prepared[:uri]] = doc
        end
      end

      indexer.index_records([resource_with_enums_and_arks])

      doc = indexer.records.find {|d| d['id'] == '/repositories/2/resources/55'}
      expect(doc['status_enum_s']).to eq(['draft', 'reviewed'])
      expect(doc['type_enum_s']).to eq(['container_summary'])
      expect(doc).not_to have_key('ark_name')
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "falls back to native dynamic enum preparation when integer vs float type mismatch occurs" do
      allow(indexer).to receive(:enum_fields).and_return(['status'])

      typed_resource = {
        'uri' => '/repositories/2/resources/55',
        'record' => {
          'uri' => '/repositories/2/resources/55',
          'title' => 'Typed Enum Resource',
          'publish' => true,
          'status' => 1,
        },
      }

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, docs|
          doc = indexer.native_index_document(prepared[:uri], prepared[:record_type], prepared[:values])
          doc['status_enum_s'] = [1.0] # float instead of integer
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'resource_archival_object_metadata',
            'dynamic_enum_and_ark_metadata',
          ]
          docs[prepared[:uri]] = doc
        end
      end

      indexer.index_records([typed_resource])

      doc = indexer.records.find {|d| d['id'] == '/repositories/2/resources/55'}
      # Fell back to native, which extracted exact integer 1
      expect(doc['status_enum_s']).to eq([1])
      expect(doc['status_enum_s'].first).to be_a(Integer)
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end

    it "falls back to native dynamic enum preparation when boolean vs integer type mismatch occurs" do
      allow(indexer).to receive(:enum_fields).and_return(['status'])

      typed_resource = {
        'uri' => '/repositories/2/resources/55',
        'record' => {
          'uri' => '/repositories/2/resources/55',
          'title' => 'Typed Enum Resource',
          'publish' => true,
          'status' => true,
        },
      }

      allow(indexer).to receive(:rust_index_documents) do |prepared_records|
        prepared_records.each_with_object({}) do |prepared, docs|
          doc = indexer.native_index_document(prepared[:uri], prepared[:record_type], prepared[:values])
          doc['status_enum_s'] = [1] # integer instead of boolean true
          doc['_archivalist_rust_fields'] = [
            'common_metadata',
            'fullrecord',
            'resource_archival_object_metadata',
            'dynamic_enum_and_ark_metadata',
          ]
          docs[prepared[:uri]] = doc
        end
      end

      indexer.index_records([typed_resource])

      doc = indexer.records.find {|d| d['id'] == '/repositories/2/resources/55'}
      # Fell back to native, which extracted exact boolean true
      expect(doc['status_enum_s']).to eq([true])
      expect(doc['status_enum_s'].first).to be(true)
      expect(doc).not_to have_key('_archivalist_rust_fields')
    end
  end
end
