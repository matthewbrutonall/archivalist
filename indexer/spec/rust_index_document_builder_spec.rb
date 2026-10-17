require 'spec_helper'
require_relative '../app/lib/rust_index_document_builder'

describe RustIndexDocumentBuilder do
  let(:builder) { RustIndexDocumentBuilder.new('http://127.0.0.1:4570', 1) }
  let(:input) do
    {
      'uri' => '/repositories/2/resources/7',
      'record_type' => 'resource',
      'repository' => 2,
      'record' => {
        'uri' => '/repositories/2/resources/7',
        'title' => 'Sánchez papers',
        'publish' => true,
      },
    }
  end

  def response(code, body)
    Struct.new(:code, :body).new(code, ASUtils.to_json(body))
  end

  it 'accepts a complete, identified batch response' do
    document = {
      'id' => input['uri'],
      'uri' => input['uri'],
      'title' => 'Sánchez papers',
      'primary_type' => 'resource',
      'types' => ['resource'],
      'json' => ASUtils.to_json(input['record']),
      'suppressed' => false,
      'publish' => true,
      'system_generated' => 'false',
      'repository' => 2,
    }
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [document])
    )

    expect(builder.build([input])).to eq(input['uri'] => document)
  end

  it 'rejects a response with the wrong identity or document contract' do
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => 'unexpected', 'documents' => [])
    )
    expect { builder.build([input]) }.to raise_error(/unexpected response identity/)

    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [{'uri' => input['uri']}])
    )
    expect { builder.build([input]) }.to raise_error(/response id mismatch/)
  end

  it 'rejects non-success HTTP responses' do
    allow(builder).to receive(:perform_request).and_return(response('503', 'error' => 'offline'))
    expect { builder.build([input]) }.to raise_error(/HTTP 503/)
  end

  it 'rejects an incomplete enrichment response' do
    input['enrich'] = true
    document = {
      'id' => input['uri'],
      'uri' => input['uri'],
      'title' => 'Sánchez papers',
      'primary_type' => 'resource',
      'types' => ['resource'],
      'json' => ASUtils.to_json(input['record']),
      'suppressed' => false,
      'publish' => true,
      'system_generated' => 'false',
      'repository' => 2,
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'resource_archival_object_metadata',
      ],
    }
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [document])
    )

    expect { builder.build([input]) }.to raise_error(/enrichment fields missing.*fullrecord_published/)
  end

  it 'does not require resource-specific enrichment fields from accessions' do
    accession = {
      'uri' => '/repositories/2/accessions/4',
      'record_type' => 'accession',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/accessions/4',
        'title' => 'New records',
        'display_string' => '2026-004: New records',
        'publish' => true,
        'id_0' => '2026',
        'acquisition_type' => 'gift',
        'resource_type' => 'papers',
        'restrictions_apply' => false,
        'access_restrictions' => false,
        'use_restrictions' => false,
        'related_resources' => [],
        'related_accessions' => [],
        'slug' => '2026-004-new-records',
        'is_slug_auto' => true,
      },
    }
    document = {
      'id' => accession['uri'],
      'uri' => accession['uri'],
      'title' => '2026-004: New records',
      'primary_type' => 'accession',
      'types' => ['accession'],
      'json' => ASUtils.to_json(accession['record']),
      'suppressed' => false,
      'publish' => true,
      'system_generated' => 'false',
      'repository' => 2,
      'fullrecord_published' => [],
      'fullrecord' => [],
      'identifier' => '2026',
      'acquisition_type' => 'gift',
      'resource_type' => 'papers',
      'restrictions_apply' => false,
      'access_restrictions' => false,
      'use_restrictions' => false,
      'related_resource_uris' => [],
      'related_accession_uris' => [],
      'slug' => '2026-004-new-records',
      'is_slug_auto' => true,
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      'four_part_id' => '2026',
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'accession_digital_object_metadata',
      ],
    }
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [document])
    )

    expect(builder.build([accession])).to eq(accession['uri'] => document)
  end

  it 'accepts authority enrichment for agents' do
    agent = {
      'uri' => '/agents/people/9',
      'record_type' => 'agent_person',
      'repository' => nil,
      'enrich' => true,
      'record' => {
        'uri' => '/agents/people/9',
        'title' => 'Original title',
        'publish' => true,
        'display_name' => {'sort_name' => "O'Connor, Maeve"},
        'names' => [],
        'agent_record_identifiers' => [],
        'agent_conventions_declarations' => [],
        'linked_agent_roles' => ['creator'],
        'related_agents' => [],
        'slug' => 'oconnor-maeve',
        'is_slug_auto' => true,
        'is_user' => false,
      },
    }
    document = {
      'id' => agent['uri'],
      'uri' => agent['uri'],
      'title' => "O'Connor, Maeve",
      'primary_type' => 'agent_person',
      'types' => ['agent_person', 'agent'],
      'json' => ASUtils.to_json(agent['record']),
      'suppressed' => false,
      'publish' => true,
      'system_generated' => 'false',
      'repository' => nil,
      'fullrecord_published' => [],
      'fullrecord' => [],
      'linked_agent_roles' => ['creator'],
      'related_agent_uris' => [],
      'slug' => 'oconnor-maeve',
      'is_slug_auto' => true,
      'is_user' => false,
      'subjects' => [],
      'subject_uris' => [],
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'agent_subject_metadata',
      ],
    }
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [document])
    )

    expect(builder.build([agent])).to eq(agent['uri'] => document)
  end

  it 'accepts enrichment for digital object components' do
    component = {
      'uri' => '/repositories/2/digital_object_components/4',
      'record_type' => 'digital_object_component',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/digital_object_components/4',
        'title' => 'Original title',
        'display_string' => 'Chapter one',
        'publish' => true,
        'component_id' => 'DOC-4',
        'digital_object' => {'ref' => '/repositories/2/digital_objects/1'},
        'slug' => 'chapter-one',
        'is_slug_auto' => true,
      },
    }
    document = {
      'id' => component['uri'],
      'uri' => component['uri'],
      'title' => 'Chapter one',
      'primary_type' => 'digital_object_component',
      'types' => ['digital_object_component'],
      'json' => ASUtils.to_json(component['record']),
      'suppressed' => false,
      'publish' => true,
      'system_generated' => 'false',
      'repository' => 2,
      'fullrecord_published' => [],
      'fullrecord' => [],
      'digital_object' => '/repositories/2/digital_objects/1',
      'digital_object_id' => 'DOC-4',
      'identifier' => 'DOC-4',
      'slug' => 'chapter-one',
      'is_slug_auto' => true,
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'component_classification_metadata',
      ],
    }
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [document])
    )

    expect(builder.build([component])).to eq(component['uri'] => document)
  end

  it 'accepts enrichment for classifications' do
    classification = {
      'uri' => '/repositories/2/classifications/9',
      'record_type' => 'classification',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/classifications/9',
        'title' => 'Records groups',
        'publish' => true,
        'identifier' => 'RG9',
        'path_from_root' => [],
        'creator' => nil,
        'repository' => {'_resolved' => {'display_string' => 'Example Archives'}},
        'has_classification_terms' => false,
        'slug' => 'records-groups',
        'is_slug_auto' => true,
      },
    }
    document = {
      'id' => classification['uri'],
      'uri' => classification['uri'],
      'title' => 'Records groups',
      'primary_type' => 'classification',
      'types' => ['classification'],
      'json' => ASUtils.to_json(classification['record']),
      'suppressed' => false,
      'publish' => true,
      'system_generated' => 'false',
      'repository' => 2,
      'fullrecord_published' => [],
      'fullrecord' => [],
      'classification_path' => '[]',
      'agent_uris' => [],
      'published_agent_uris' => [],
      'agents' => [],
      'identifier_sort' => 'rust-sort',
      'repo_sort' => 'Example Archives',
      'has_classification_terms' => false,
      'slug' => 'records-groups',
      'is_slug_auto' => true,
      'identifier' => 'RG9',
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'component_classification_metadata',
      ],
    }
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [document])
    )

    expect(builder.build([classification])).to eq(classification['uri'] => document)
  end

  it 'accepts enrichment for repositories and events' do
    repository = {
      'uri' => '/repositories/2',
      'record_type' => 'repository',
      'repository' => 'global',
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2',
        'repo_code' => 'ARCH-2',
        'display_string' => 'Example Archives',
        'publish' => true,
        'slug' => 'example-archives',
        'is_slug_auto' => true,
        'position' => 1,
      },
    }
    repository_document = {
      'id' => repository['uri'],
      'uri' => repository['uri'],
      'title' => 'ARCH-2',
      'title_sort' => 'Example Archives',
      'repo_sort' => 'Example Archives',
      'primary_type' => 'repository',
      'types' => ['repository'],
      'json' => ASUtils.to_json(repository['record']),
      'suppressed' => false,
      'publish' => true,
      'system_generated' => 'false',
      'repository' => '/repositories/2',
      'fullrecord_published' => [],
      'fullrecord' => [],
      'slug' => 'example-archives',
      'is_slug_auto' => true,
      'position_int_sort' => 1,
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'repository_event_location_metadata',
      ],
    }

    event = {
      'uri' => '/repositories/2/events/11',
      'record_type' => 'event',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/events/11',
        'event_type' => 'custody_transfer',
        'outcome' => 'pass',
        'publish' => false,
        'linked_records' => [
          {
            'ref' => '/repositories/2/resources/7',
            '_resolved' => {'jsonmodel_type' => 'resource', 'title' => 'Sánchez papers'},
          },
        ],
      },
    }
    event_document = {
      'id' => event['uri'],
      'uri' => event['uri'],
      'title' => 'custody_transfer',
      'event_type' => 'custody_transfer',
      'outcome' => 'pass',
      'primary_type' => 'event',
      'types' => ['event'],
      'json' => ASUtils.to_json(event['record']),
      'suppressed' => false,
      'publish' => false,
      'system_generated' => 'false',
      'repository' => 2,
      'fullrecord_published' => [],
      'fullrecord' => [],
      'linked_record_uris' => ['/repositories/2/resources/7'],
      'linked_record_titles' => ['Sánchez papers'],
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'repository_event_location_metadata',
      ],
    }

    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [repository_document, event_document])
    )

    expect(builder.build([repository, event])).to eq(
      repository['uri'] => repository_document,
      event['uri'] => event_document
    )
  end

  it 'accepts location enrichment with resolved or unresolved owner_repo and location_profile' do
    resolved_location = {
      'uri' => '/locations/15',
      'record_type' => 'location',
      'repository' => 'global',
      'enrich' => true,
      'record' => {
        'uri' => '/locations/15',
        'title' => 'Main Vault, Bay 3',
        'publish' => false,
        'temporary' => 'conservation',
        'building' => 'Main Vault',
        'floor' => '1',
        'room' => 'A',
        'area' => 'Bay 3',
        'owner_repo' => {
          'ref' => '/repositories/2',
          '_resolved' => {'repo_code' => 'ARCH-2'},
        },
        'location_profile' => {
          'ref' => '/location_profiles/4',
          '_resolved' => {'display_string' => 'Standard folio shelf'},
        },
      },
    }
    resolved_document = {
      'id' => resolved_location['uri'],
      'uri' => resolved_location['uri'],
      'title' => 'Main Vault, Bay 3',
      'primary_type' => 'location',
      'types' => ['location'],
      'json' => ASUtils.to_json(resolved_location['record']),
      'suppressed' => false,
      'publish' => false,
      'system_generated' => 'false',
      'repository' => 'global',
      'fullrecord_published' => [],
      'fullrecord' => [],
      'temporary' => 'conservation',
      'building' => 'Main Vault',
      'floor' => '1',
      'room' => 'A',
      'area' => 'Bay 3',
      'owner_repo_uri_u_sstr' => '/repositories/2',
      'owner_repo_display_string_u_ssort' => 'ARCH-2',
      'location_profile_uri_u_sstr' => '/location_profiles/4',
      'location_profile_display_string_u_ssort' => 'Standard folio shelf',
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'repository_event_location_metadata',
      ],
    }

    unresolved_location = {
      'uri' => '/locations/16',
      'record_type' => 'location',
      'repository' => 'global',
      'enrich' => true,
      'record' => {
        'uri' => '/locations/16',
        'title' => 'Annex, Room 2',
        'publish' => false,
        'building' => 'Annex',
        'floor' => nil,
        'room' => '2',
        'area' => nil,
        'owner_repo' => {'ref' => '/repositories/2'},
      },
    }
    unresolved_document = {
      'id' => unresolved_location['uri'],
      'uri' => unresolved_location['uri'],
      'title' => 'Annex, Room 2',
      'primary_type' => 'location',
      'types' => ['location'],
      'json' => ASUtils.to_json(unresolved_location['record']),
      'suppressed' => false,
      'publish' => false,
      'system_generated' => 'false',
      'repository' => 'global',
      'fullrecord_published' => [],
      'fullrecord' => [],
      'building' => 'Annex',
      'floor' => nil,
      'room' => '2',
      'area' => nil,
      'owner_repo_uri_u_sstr' => '/repositories/2',
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'repository_event_location_metadata',
      ],
    }

    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [resolved_document, unresolved_document])
    )

    expect(builder.build([resolved_location, unresolved_location])).to eq(
      resolved_location['uri'] => resolved_document,
      unresolved_location['uri'] => unresolved_document
    )
  end

  it 'accepts enrichment for container_profile and location_profile' do
    container_profile = {
      'uri' => '/container_profiles/3',
      'record_type' => 'container_profile',
      'repository' => 'global',
      'enrich' => true,
      'record' => {
        'uri' => '/container_profiles/3',
        'name' => 'Standard Hollinger',
        'display_string' => 'Standard Hollinger [10x12x15] (inches)',
        'note' => 'Acid-free archival carton',
        'width' => '10',
        'height' => '12',
        'depth' => '15',
        'dimension_units' => 'inches',
      },
    }
    container_profile_document = {
      'id' => container_profile['uri'],
      'uri' => container_profile['uri'],
      'title' => 'Standard Hollinger [10x12x15] (inches)',
      'title_sort' => 'Standard Hollinger 10x12x15 inches',
      'display_string' => 'Standard Hollinger [10x12x15] (inches)',
      'note' => 'Acid-free archival carton',
      'container_profile_width_u_sstr' => '10',
      'container_profile_height_u_sstr' => '12',
      'container_profile_depth_u_sstr' => '15',
      'container_profile_dimension_units_u_sstr' => 'inches',
      'typeahead_sort_key_u_sort' => 'Standard Hollinger [10x12x15] (inches)',
      'primary_type' => 'container_profile',
      'types' => ['container_profile'],
      'json' => ASUtils.to_json(container_profile['record']),
      'suppressed' => false,
      'publish' => false,
      'system_generated' => 'false',
      'repository' => 'global',
      'fullrecord_published' => [],
      'fullrecord' => [],
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'profile_assessment_metadata',
      ],
    }

    location_profile = {
      'uri' => '/location_profiles/4',
      'record_type' => 'location_profile',
      'repository' => 'global',
      'enrich' => true,
      'record' => {
        'uri' => '/location_profiles/4',
        'name' => 'Deep Folio Shelf',
        'display_string' => 'Deep Folio Shelf [120x40x60] (cm)',
        'width' => '120',
        'height' => '40',
        'depth' => '60',
        'dimension_units' => 'centimeters',
      },
    }
    location_profile_document = {
      'id' => location_profile['uri'],
      'uri' => location_profile['uri'],
      'title' => 'Deep Folio Shelf [120x40x60] (cm)',
      'title_sort' => 'Deep Folio Shelf 120x40x60 cm',
      'display_string' => 'Deep Folio Shelf [120x40x60] (cm)',
      'location_profile_width_u_sstr' => '120',
      'location_profile_height_u_sstr' => '40',
      'location_profile_depth_u_sstr' => '60',
      'location_profile_dimension_units_u_sstr' => 'centimeters',
      'typeahead_sort_key_u_sort' => 'Deep Folio Shelf [120x40x60] (cm)',
      'primary_type' => 'location_profile',
      'types' => ['location_profile'],
      'json' => ASUtils.to_json(location_profile['record']),
      'suppressed' => false,
      'publish' => false,
      'system_generated' => 'false',
      'repository' => 'global',
      'fullrecord_published' => [],
      'fullrecord' => [],
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'profile_assessment_metadata',
      ],
    }

    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [container_profile_document, location_profile_document])
    )

    expect(builder.build([container_profile, location_profile])).to eq(
      container_profile['uri'] => container_profile_document,
      location_profile['uri'] => location_profile_document
    )
  end

  it 'accepts assessment enrichment for full and minimal records' do
    full_assessment = {
      'uri' => '/repositories/2/assessments/42',
      'record_type' => 'assessment',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/assessments/42',
        'display_string' => 'Assessment 42',
        'records' => [
          {
            'ref' => '/repositories/2/resources/7',
            '_resolved' => {'jsonmodel_type' => 'resource', 'title' => 'Sánchez papers'},
          },
        ],
        'surveyed_by' => [
          {'ref' => '/agents/people/1', '_resolved' => {'title' => 'Surveyor One'}},
        ],
        'survey_begin' => '2024-03-15',
        'survey_end' => '2026-04-01',
        'review_required' => true,
        'sensitive_material' => false,
        'inactive' => false,
        'reviewer' => [
          {'ref' => '/agents/people/9', '_resolved' => {'title' => 'Reviewer Nine'}},
        ],
        'collections' => [
          {
            'ref' => '/repositories/2/resources/7',
            '_resolved' => {'display_string' => 'Sánchez Display'},
          },
        ],
        'formats' => [{'label' => 'Architectural Drawings', 'value' => 'true'}],
        'ratings' => [{'label' => 'Research Value', 'note' => 'High'}],
        'conservation_issues' => [{'label' => 'Brittle Paper', 'value' => 'true'}],
      },
    }
    full_document = {
      'id' => full_assessment['uri'],
      'uri' => full_assessment['uri'],
      'assessment_id' => 42,
      'title' => 'Assessment 42',
      'display_string' => 'Assessment 42',
      'assessment_record_uris' => ['/repositories/2/resources/7'],
      'assessment_records' => ['Sánchez papers'],
      'assessment_record_types' => ['resource'],
      'assessment_surveyor_uris' => ['/agents/people/1'],
      'assessment_surveyors' => ['Surveyor One'],
      'assessment_survey_begin' => '2024-03-15T00:00:00Z',
      'assessment_survey_end' => '2026-04-01T00:00:00Z',
      'assessment_review_required' => true,
      'assessment_sensitive_material' => false,
      'assessment_inactive' => false,
      'assessment_reviewer_uris' => ['/agents/people/9'],
      'assessment_reviewers' => ['Reviewer Nine'],
      'assessment_survey_year' => ['2024', '2025', '2026'],
      'assessment_collection_uris' => ['/repositories/2/resources/7'],
      'assessment_collections' => ['Sánchez Display'],
      'assessment_completed' => true,
      'assessment_formats' => ['Architectural Drawings'],
      'assessment_ratings' => ['Research Value'],
      'assessment_conservation_issues' => ['Brittle Paper'],
      'title_sort' => '0000000042',
      'primary_type' => 'assessment',
      'types' => ['assessment'],
      'json' => ASUtils.to_json(full_assessment['record']),
      'suppressed' => false,
      'publish' => false,
      'system_generated' => 'false',
      'repository' => 2,
      'fullrecord_published' => [],
      'fullrecord' => [],
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'profile_assessment_metadata',
      ],
    }

    minimal_assessment = {
      'uri' => '/repositories/2/assessments/7',
      'record_type' => 'assessment',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/assessments/7',
        'display_string' => 'Assessment 7',
        'records' => [],
        'surveyed_by' => [],
        'survey_begin' => '2025-11-01',
        'reviewer' => [],
        'collections' => [],
        'formats' => [],
        'ratings' => [],
        'conservation_issues' => [],
      },
    }
    minimal_document = {
      'id' => minimal_assessment['uri'],
      'uri' => minimal_assessment['uri'],
      'assessment_id' => 7,
      'title' => 'Assessment 7',
      'display_string' => 'Assessment 7',
      'assessment_record_uris' => [],
      'assessment_records' => [],
      'assessment_record_types' => [],
      'assessment_surveyor_uris' => [],
      'assessment_surveyors' => [],
      'assessment_survey_begin' => '2025-11-01T00:00:00Z',
      'assessment_review_required' => false,
      'assessment_sensitive_material' => false,
      'assessment_inactive' => false,
      'assessment_survey_year' => ['2025'],
      'assessment_collection_uris' => [],
      'assessment_collections' => [],
      'assessment_completed' => false,
      'assessment_formats' => [],
      'assessment_ratings' => [],
      'assessment_conservation_issues' => [],
      'title_sort' => '0000000007',
      'primary_type' => 'assessment',
      'types' => ['assessment'],
      'json' => ASUtils.to_json(minimal_assessment['record']),
      'suppressed' => false,
      'publish' => false,
      'system_generated' => 'false',
      'repository' => 2,
      'fullrecord_published' => [],
      'fullrecord' => [],
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'profile_assessment_metadata',
      ],
    }

    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [full_document, minimal_document])
    )

    expect(builder.build([full_assessment, minimal_assessment])).to eq(
      full_assessment['uri'] => full_document,
      minimal_assessment['uri'] => minimal_document
    )

    missing_reviewer_doc = full_document.reject {|k, _| k == 'assessment_reviewers'}
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [missing_reviewer_doc])
    )
    expect { builder.build([full_assessment]) }.to raise_error(
      /enrichment fields missing.*assessment_reviewers/
    )
  end
end
