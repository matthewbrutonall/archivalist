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
  end
end
