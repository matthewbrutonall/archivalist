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
        'dynamic_enum_and_ark_metadata',
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
        'accession_date' => '2026-10-17',
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
      'accession_date' => '2026-10-17',
      'accession_date_year' => 2026,
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
        'accession_date_metadata',
        'dynamic_enum_and_ark_metadata',
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
        'dynamic_enum_and_ark_metadata',
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
        'dynamic_enum_and_ark_metadata',
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
        'dynamic_enum_and_ark_metadata',
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
        'dynamic_enum_and_ark_metadata',
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
        'dynamic_enum_and_ark_metadata',
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
        'dynamic_enum_and_ark_metadata',
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
        'dynamic_enum_and_ark_metadata',
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
        'dynamic_enum_and_ark_metadata',
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
        'dynamic_enum_and_ark_metadata',
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
        'dynamic_enum_and_ark_metadata',
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
        'dynamic_enum_and_ark_metadata',
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

  it 'validates top_container enrichment fields' do
    full_top_container = {
      'uri' => '/repositories/2/top_containers/10',
      'record_type' => 'top_container',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/top_containers/10',
        'jsonmodel_type' => 'top_container',
        'long_display_string' => 'Collection 1: Box 12A [BC-1]',
        'display_string' => 'Box 12A [BC-1]',
        'type' => 'box',
        'indicator' => '12A',
        'barcode' => 'BC-1',
        'ils_holding_id' => 'ILS-10',
        'internal_note' => 'Top container note',
        'exported_to_ils' => '2026-01-01T00:00:00Z',
        'subcontainer_barcodes' => ['SUB-1'],
        'created_for_collection' => '/repositories/2/resources/77',
        'series' => [
          {
            'ref' => '/repositories/2/archival_objects/100',
            'display_string' => 'Published Series',
            'level_display_string' => 'Series',
            'identifier' => 'S-01.A',
            'publish' => true,
          },
          {
            'ref' => '/repositories/2/archival_objects/101',
            'display_string' => 'Unpublished Series',
            'level_display_string' => 'Subseries',
            'identifier' => 'S-02',
            'publish' => false,
          },
        ],
        'collection' => [
          {
            'ref' => '/repositories/2/resources/77',
            'display_string' => 'Collection 1',
            'identifier' => 'MS-100',
          },
        ],
        'container_profile' => {
          'ref' => '/container_profiles/1',
          '_resolved' => {'display_string' => 'Standard Carton'},
        },
        'container_locations' => [
          {
            'ref' => '/locations/10',
            'status' => 'current',
            '_resolved' => {
              'title' => 'Vault A',
              'location_profile' => {
                '_resolved' => {'display_string' => 'Deep Shelf'},
              },
            },
          },
          {
            'ref' => '/locations/11',
            'status' => 'previous',
            '_resolved' => {
              'title' => 'Old Room',
              'location_profile' => {
                '_resolved' => {'display_string' => 'Old Shelf'},
              },
            },
          },
        ],
      },
    }
    full_document = {
      'id' => full_top_container['uri'],
      'uri' => full_top_container['uri'],
      'title' => 'Collection 1: Box 12A [BC-1]',
      'display_string' => 'Box 12A [BC-1]',
      'type_u_ssort' => 'box',
      'notes' => 'Top container note',
      'series_uri_u_sstr' => [
        '/repositories/2/archival_objects/100',
        '/repositories/2/archival_objects/101',
      ],
      'series_title_u_sstr' => ['Published Series', 'Unpublished Series'],
      'series_level_u_sstr' => ['Series', 'Subseries'],
      'series_identifier_stored_u_sstr' => ['S-01.A', 'S-02'],
      'series_identifier_u_stext' => [
        'S-01.A',
        'S 01 A',
        'S01A',
        'S- 01 .A',
        'S-02',
        'S 02',
        'S02',
        'S- 02',
      ],
      'published_series_uri_u_sstr' => ['/repositories/2/archival_objects/100'],
      'published_series_title_u_sstr' => ['Published Series'],
      'collection_uri_u_sstr' => ['/repositories/2/resources/77'],
      'collection_display_string_u_sstr' => ['Collection 1'],
      'collection_identifier_stored_u_sstr' => ['MS-100'],
      'collection_identifier_u_stext' => ['MS-100', 'MS 100', 'MS100', 'MS- 100'],
      'container_profile_uri_u_sstr' => '/container_profiles/1',
      'container_profile_display_string_u_sstr' => 'Standard Carton',
      'has_location_u_sbool' => true,
      'location_uri_u_sstr' => ['/locations/10'],
      'location_uris' => ['/locations/10'],
      'location_display_string_u_sstr' => ['Vault A'],
      'location_profile_display_string_u_sstr' => ['Deep Shelf'],
      'exported_u_sbool' => true,
      'empty_u_sbool' => false,
      'indicator_u_icusort' => '12A',
      'top_container_u_typeahead_utext' => 'box12abc1',
      'top_container_u_icusort' => 'Box 12A [BC-1]',
      'barcode_u_sstr' => 'BC-1',
      'barcode_u_icusort' => 'BC-1',
      'ils_holding_id_u_sstr' => 'ILS-10',
      'subcontainer_barcodes_u_sstr' => ['SUB-1'],
      'created_for_collection_u_sstr' => '/repositories/2/resources/77',
      'primary_type' => 'top_container',
      'types' => ['top_container'],
      'json' => ASUtils.to_json(full_top_container['record']),
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
        'top_container_metadata',
        'dynamic_enum_and_ark_metadata',
      ],
    }

    minimal_top_container = {
      'uri' => '/repositories/2/top_containers/11',
      'record_type' => 'top_container',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/top_containers/11',
        'jsonmodel_type' => 'top_container',
        'long_display_string' => 'Empty Box',
        'display_string' => 'Empty Box',
        'series' => [
          {
            'ref' => '/repositories/2/archival_objects/200',
            'display_string' => 'Hidden Series',
            'level_display_string' => 'Series',
            'identifier' => 'HS-1',
            'publish' => false,
          },
        ],
        'collection' => [],
        'container_locations' => [
          {
            'ref' => '/locations/20',
            'status' => 'previous',
            '_resolved' => {'title' => 'Old Room'},
          },
        ],
        'exported_to_ils' => false,
      },
    }
    minimal_document = {
      'id' => minimal_top_container['uri'],
      'uri' => minimal_top_container['uri'],
      'title' => 'Empty Box',
      'display_string' => 'Empty Box',
      'type_u_ssort' => nil,
      'notes' => nil,
      'series_uri_u_sstr' => ['/repositories/2/archival_objects/200'],
      'series_title_u_sstr' => ['Hidden Series'],
      'series_level_u_sstr' => ['Series'],
      'series_identifier_stored_u_sstr' => ['HS-1'],
      'series_identifier_u_stext' => ['HS-1', 'HS 1', 'HS1', 'HS- 1'],
      'collection_uri_u_sstr' => [],
      'collection_display_string_u_sstr' => [],
      'collection_identifier_stored_u_sstr' => [],
      'collection_identifier_u_stext' => [],
      'has_location_u_sbool' => true,
      'location_uri_u_sstr' => [],
      'location_uris' => [],
      'location_display_string_u_sstr' => [],
      'location_profile_display_string_u_sstr' => [],
      'exported_u_sbool' => true,
      'empty_u_sbool' => true,
      'top_container_u_typeahead_utext' => 'emptybox',
      'top_container_u_icusort' => 'Empty Box',
      'barcode_u_sstr' => nil,
      'barcode_u_icusort' => nil,
      'ils_holding_id_u_sstr' => nil,
      'subcontainer_barcodes_u_sstr' => nil,
      'created_for_collection_u_sstr' => nil,
      'primary_type' => 'top_container',
      'types' => ['top_container'],
      'json' => ASUtils.to_json(minimal_top_container['record']),
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
        'top_container_metadata',
        'dynamic_enum_and_ark_metadata',
      ],
    }

    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [full_document, minimal_document])
    )

    expect(builder.build([full_top_container, minimal_top_container])).to eq(
      full_top_container['uri'] => full_document,
      minimal_top_container['uri'] => minimal_document
    )

    [
      'series_identifier_u_stext',
      'published_series_uri_u_sstr',
      'collection_identifier_u_stext',
      'container_profile_display_string_u_sstr',
      'location_profile_display_string_u_sstr',
      'indicator_u_icusort',
    ].each do |required_field|
      missing_doc = full_document.reject {|k, _| k == required_field}
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [missing_doc])
      )
      expect { builder.build([full_top_container]) }.to raise_error(
        /enrichment fields missing.*#{Regexp.escape(required_field)}/
      )
    end
  end

  it 'validates job enrichment fields, types ordering, and job_metadata marker' do
    report_job = {
      'uri' => '/repositories/2/jobs/31',
      'record_type' => 'job',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/jobs/31',
        'jsonmodel_type' => 'job',
        'job_type' => 'report_job',
        'status' => 'completed',
        'owner' => 'admin',
        'queue_position' => 0,
        'job' => {
          'jsonmodel_type' => 'report_job',
          'report_type' => 'assessment_list_report',
          'format' => 'pdf',
        },
      },
    }
    report_job_document = {
      'id' => report_job['uri'],
      'uri' => report_job['uri'],
      'title' => nil,
      'primary_type' => 'job',
      'types' => ['job', 'report_job', 'assessment_list_report'],
      'job_type' => 'report_job',
      'report_type' => 'assessment_list_report',
      'job_report_type' => 'assessment_list_report',
      'status' => 'completed',
      'owner' => 'admin',
      'queue_position' => 0,
      'json' => ASUtils.to_json(report_job['record']),
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
        'job_metadata',
        'dynamic_enum_and_ark_metadata',
      ],
    }

    import_job = {
      'uri' => '/repositories/2/jobs/32',
      'record_type' => 'job',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/jobs/32',
        'jsonmodel_type' => 'job',
        'job_type' => 'import_job',
        'status' => 'queued',
        'owner' => 'archivist',
        'queue_position' => 3,
        'job' => {
          'jsonmodel_type' => 'import_job',
          'import_type' => 'ead_xml',
          'filenames' => ['collection.xml'],
        },
      },
    }
    import_job_document = {
      'id' => import_job['uri'],
      'uri' => import_job['uri'],
      'title' => nil,
      'primary_type' => 'job',
      'types' => ['job', 'import_job', nil],
      'job_type' => 'import_job',
      'report_type' => nil,
      'job_report_type' => 'import_job',
      'status' => 'queued',
      'owner' => 'archivist',
      'queue_position' => 3,
      'json' => ASUtils.to_json(import_job['record']),
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
        'job_metadata',
        'dynamic_enum_and_ark_metadata',
      ],
    }

    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [report_job_document, import_job_document])
    )

    expect(builder.build([report_job, import_job])).to eq(
      report_job['uri'] => report_job_document,
      import_job['uri'] => import_job_document
    )

    bad_types_doc = import_job_document.merge('types' => ['job', 'import_job'])
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [bad_types_doc])
    )
    expect { builder.build([import_job]) }.to raise_error(/response types mismatch/)

    bad_marker_doc = report_job_document.merge(
      '_archivalist_rust_fields' => ['common_metadata', 'fullrecord']
    )
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [bad_marker_doc])
    )
    expect { builder.build([report_job]) }.to raise_error(/response enrichment marker mismatch/)

    [
      'job_type',
      'report_type',
      'job_report_type',
      'status',
      'owner',
      'queue_position',
      'external_id',
      'ancestors',
      'langcode',
    ].each do |required_field|
      missing_doc = report_job_document.reject {|k, _| k == required_field}
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [missing_doc])
      )
      expect { builder.build([report_job]) }.to raise_error(
        /enrichment fields missing.*#{Regexp.escape(required_field)}/
      )
    end
  end

  it 'accepts unenriched job documents without job_metadata marker and rejects unexpected markers' do
    unenriched_job = {
      'uri' => '/repositories/2/jobs/33',
      'record_type' => 'job',
      'repository' => 2,
      'enrich' => false,
      'record' => {
        'uri' => '/repositories/2/jobs/33',
        'jsonmodel_type' => 'job',
        'title' => 'Envelope Job Title',
        'job_type' => 'import_job',
        'status' => 'queued',
        'owner' => 'archivist',
        'job' => {
          'jsonmodel_type' => 'import_job',
        },
      },
    }
    unenriched_doc = {
      'id' => unenriched_job['uri'],
      'uri' => unenriched_job['uri'],
      'title' => 'Envelope Job Title',
      'primary_type' => 'job',
      'types' => ['job'],
      'json' => ASUtils.to_json(unenriched_job['record']),
      'suppressed' => false,
      'publish' => false,
      'system_generated' => 'false',
      'repository' => 2,
    }

    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [unenriched_doc])
    )
    expect(builder.build([unenriched_job])).to eq(unenriched_job['uri'] => unenriched_doc)

    bad_marker_doc = unenriched_doc.merge('_archivalist_rust_fields' => ['common_metadata', 'fullrecord', 'job_metadata'])
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [bad_marker_doc])
    )
    expect { builder.build([unenriched_job]) }.to raise_error(/unexpected response enrichment marker/)
  end

  it 'rejects job_metadata marker on unrelated record types' do
    resource_input = {
      'uri' => '/repositories/2/resources/99',
      'record_type' => 'resource',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/resources/99',
        'title' => 'Test Resource',
        'id_0' => 'RES-99',
        'level' => 'collection',
      },
    }
    unrelated_doc_with_job_marker = {
      'id' => resource_input['uri'],
      'uri' => resource_input['uri'],
      'title' => 'Test Resource',
      'primary_type' => 'resource',
      'types' => ['resource'],
      'json' => ASUtils.to_json(resource_input['record']),
      'suppressed' => false,
      'publish' => false,
      'system_generated' => 'false',
      'repository' => 2,
      'fullrecord_published' => [],
      'fullrecord' => [],
      'finding_aid_title' => nil,
      'finding_aid_filing_title' => nil,
      'identifier' => 'RES-99',
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
      'four_part_id' => 'RES-99',
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'resource_archival_object_metadata',
        'job_metadata',
        'dynamic_enum_and_ark_metadata',
      ],
    }

    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [unrelated_doc_with_job_marker])
    )
    expect { builder.build([resource_input]) }.to raise_error(/response enrichment marker mismatch/)
  end

  it 'validates collection_management child document on enriched resource and accession parents' do
    full_cm = {
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
    resource_input = {
      'uri' => '/repositories/2/resources/1',
      'record_type' => 'resource',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/resources/1',
        'title' => ' <emph render="italic">Papers</emph> - Collection 1 ',
        'suppressed' => true,
        'id_0' => 'COL-1',
        'level' => 'collection',
        'collection_management' => full_cm,
      },
    }
    child_doc = {
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
      'json' => ASUtils.to_json(builder.send(:reconstruct_expected_collection_management_json, full_cm)),
    }
    resource_doc = {
      'id' => resource_input['uri'],
      'uri' => resource_input['uri'],
      'title' => ' <emph render="italic">Papers</emph> - Collection 1 ',
      'primary_type' => 'resource',
      'types' => ['resource'],
      'json' => ASUtils.to_json(resource_input['record']),
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
        'dynamic_enum_and_ark_metadata',
      ],
      '_archivalist_collection_management_documents' => [child_doc],
    }

    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [resource_doc])
    )
    result = builder.build([resource_input])
    expect(result[resource_input['uri']]).to eq(resource_doc)

    # Minimal accession parent
    minimal_cm = {
      'uri' => '/repositories/2/collection_management/11',
    }
    accession_input = {
      'uri' => '/repositories/2/accessions/2',
      'record_type' => 'accession',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/accessions/2',
        'suppressed' => false,
        'display_string' => 'Accession 2026-01',
        'title' => nil,
        'accession_date' => '2026-01-15',
        'id_0' => 'ACC-2',
        'acquisition_type' => nil,
        'resource_type' => nil,
        'restrictions_apply' => false,
        'access_restrictions' => false,
        'use_restrictions' => false,
        'related_resources' => [],
        'related_accessions' => [],
        'slug' => nil,
        'is_slug_auto' => false,
        'collection_management' => minimal_cm,
      },
    }
    minimal_child = {
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
    accession_doc = {
      'id' => accession_input['uri'],
      'uri' => accession_input['uri'],
      'title' => 'Accession 2026-01',
      'primary_type' => 'accession',
      'types' => ['accession'],
      'json' => ASUtils.to_json(accession_input['record']),
      'suppressed' => false,
      'publish' => false,
      'system_generated' => 'false',
      'repository' => 2,
      'fullrecord_published' => [],
      'fullrecord' => [],
      'accession_date' => '2026-01-15',
      'accession_date_year' => 2026,
      'identifier' => 'ACC-2',
      'acquisition_type' => nil,
      'resource_type' => nil,
      'restrictions_apply' => false,
      'access_restrictions' => false,
      'use_restrictions' => false,
      'related_resource_uris' => [],
      'related_accession_uris' => [],
      'slug' => nil,
      'is_slug_auto' => false,
      'external_id' => [],
      'ancestors' => [],
      'langcode' => [],
      'four_part_id' => 'ACC-2',
      'processing_priority' => nil,
      'processors' => nil,
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'accession_digital_object_metadata',
        'accession_date_metadata',
        'dynamic_enum_and_ark_metadata',
      ],
      '_archivalist_collection_management_documents' => [minimal_child],
    }
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [accession_doc])
    )
    result = builder.build([accession_input])
    expect(result[accession_input['uri']]).to eq(accession_doc)
  end

  it 'rejects mismatched or malformed collection_management child document metadata' do
    cm = { 'uri' => '/repositories/2/collection_management/10' }
    valid_input = {
      'uri' => '/repositories/2/resources/1',
      'record_type' => 'resource',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/resources/1',
        'title' => 'Resource 1',
        'id_0' => 'R1',
        'level' => 'collection',
        'collection_management' => cm,
      },
    }
    base_child = {
      'id' => '/repositories/2/collection_management/10',
      'uri' => '/repositories/2/collection_management/10',
      'parent_id' => '/repositories/2/resources/1',
      'parent_title' => 'Resource 1',
      'parent_type' => 'resource',
      'title' => 'Resource 1',
      'title_sort' => 'Resource 1',
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
      'json' => ASUtils.to_json(cm),
    }
    build_doc = lambda do |child|
      {
        'id' => valid_input['uri'],
        'uri' => valid_input['uri'],
        'title' => 'Resource 1',
        'primary_type' => 'resource',
        'types' => ['resource'],
        'json' => ASUtils.to_json(valid_input['record']),
        'suppressed' => false,
        'publish' => false,
        'system_generated' => 'false',
        'repository' => 2,
        'notes' => [],
        'notes_published' => [],
        'fullrecord_published' => [],
        'fullrecord' => [],
        'finding_aid_title' => nil,
        'finding_aid_filing_title' => nil,
        'identifier' => 'R1',
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
        'four_part_id' => 'R1',
        'processing_priority' => nil,
        'processors' => nil,
        '_archivalist_rust_fields' => [
          'common_metadata',
          'fullrecord',
          'resource_archival_object_metadata',
          'dynamic_enum_and_ark_metadata',
        ],
        '_archivalist_collection_management_documents' => child ? [child] : [],
      }
    end

    # Mismatched child id
    bad_id_doc = build_doc.call(base_child.merge('id' => '/repositories/2/collection_management/999'))
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [bad_id_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child id mismatch/)

    # Mismatched child uri
    bad_uri_doc = build_doc.call(base_child.merge('uri' => '/repositories/2/collection_management/999'))
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [bad_uri_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child uri mismatch/)

    # Mismatched parent_id
    bad_parent_id_doc = build_doc.call(base_child.merge('parent_id' => '/repositories/2/resources/999'))
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [bad_parent_id_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child parent_id mismatch/)

    # Mismatched parent_type
    bad_parent_type_doc = build_doc.call(base_child.merge('parent_type' => 'accession'))
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [bad_parent_type_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child parent_type mismatch/)

    # Mismatched title / parent_title
    bad_title_doc = build_doc.call(base_child.merge('title' => 'Wrong Title'))
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [bad_title_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child title mismatch/)

    # Mismatched primary_type
    bad_type_doc = build_doc.call(base_child.merge('primary_type' => 'resource'))
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [bad_type_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child primary_type mismatch/)

    # Mismatched types
    bad_types_doc = build_doc.call(base_child.merge('types' => ['wrong_type']))
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [bad_types_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child types mismatch/)

    # Mismatched repository
    bad_repo_doc = build_doc.call(base_child.merge('repository' => 999))
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [bad_repo_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child repository mismatch/)

    # Mismatched suppression
    bad_suppressed_doc = build_doc.call(base_child.merge('suppressed' => true))
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [bad_suppressed_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child suppression mismatch/)

    # Child documents count mismatch (0 children)
    empty_children_doc = build_doc.call(nil)
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [empty_children_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/collection management documents count mismatch/)

    # Child document contains private handoff field
    private_child_doc = build_doc.call(base_child.merge('_archivalist_private' => 'leaked'))
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [private_child_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child document contains private handoff field/)

    # Missing nil-valued optional key (e.g. 'processors' omitted from child hash)
    missing_key_child = base_child.reject {|k, _| k == 'processors'}
    expect(base_child['processors']).to be_nil
    missing_key_doc = build_doc.call(missing_key_child)
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [missing_key_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child document missing required fields.*processors/)

    # Extra ordinary Solr field (not a private handoff field)
    extra_field_child = base_child.merge('extra_solr_field' => 'unexpected_value')
    extra_field_doc = build_doc.call(extra_field_child)
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [extra_field_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child document contains unexpected fields.*extra_solr_field/)

    # Unexpected collection management documents when record has no collection_management
    no_cm_input = valid_input.merge('record' => valid_input['record'].reject {|k, _| k == 'collection_management'})
    doc_with_unwanted_child = build_doc.call(base_child)
    doc_with_unwanted_child['json'] = ASUtils.to_json(no_cm_input['record'])
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [doc_with_unwanted_child])
    )
    expect { builder.build([no_cm_input]) }.to raise_error(/unexpected response collection management documents/)
  end

  it 'accepts enriched parent document when collection_management child is withheld by Rust' do
    cm = { 'uri' => '/repositories/2/collection_management/10' }
    valid_input = {
      'uri' => '/repositories/2/resources/1',
      'record_type' => 'resource',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/resources/1',
        'title' => 'Resource Withheld CM',
        'id_0' => 'R1',
        'level' => 'collection',
        'collection_management' => cm,
      },
    }
    withheld_doc = {
      'id' => valid_input['uri'],
      'uri' => valid_input['uri'],
      'title' => 'Resource Withheld CM',
      'primary_type' => 'resource',
      'types' => ['resource'],
      'json' => ASUtils.to_json(valid_input['record']),
      'suppressed' => false,
      'publish' => false,
      'system_generated' => 'false',
      'repository' => 2,
      'fullrecord_published' => [],
      'fullrecord' => [],
      'finding_aid_title' => nil,
      'finding_aid_filing_title' => nil,
      'identifier' => 'R1',
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
      'four_part_id' => 'R1',
      'processing_priority' => nil,
      'processors' => nil,
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'resource_archival_object_metadata',
        'dynamic_enum_and_ark_metadata',
      ],
      # Notice: _archivalist_collection_management_documents is absent because Rust withheld it
    }
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [withheld_doc])
    )
    result = builder.build([valid_input])
    expect(result[valid_input['uri']]).to eq(withheld_doc)
  end

  it 'deeply validates collection_management child JSON with native stripping and rejects corrupted JSON' do
    full_cm = {
      'uri' => '/repositories/2/collection_management/10',
      'processing_priority' => ' high ',
      'processing_status' => ' completed ',
      'processing_hours_total' => ' 40 ',
      'processing_funding_source' => ' grant-funded ',
      'processors' => ' Alice and Bob ',
      'created_by' => ' admin ',
      'last_modified_by' => ' archivist ',
      'system_mtime' => ' 2026-01-01T00:00:00Z ',
      'user_mtime' => ' 2026-01-01T00:00:00Z ',
      'create_time' => ' 2026-01-01T00:00:00Z ',
      'user_defined_enum_s' => ' unstripped_enum ',
      'external_ids' => [
        {
          'external_id' => ' ext-1 ',
          'source' => ' local ',
        }
      ],
      'nested' => {
        'title' => ' nested title ',
        'created_by' => ' unstripped_audit ',
      },
    }

    valid_input = {
      'uri' => '/repositories/2/resources/1',
      'record_type' => 'resource',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/resources/1',
        'title' => 'Resource 1',
        'id_0' => 'R1',
        'level' => 'collection',
        'suppressed' => false,
        'collection_management' => full_cm,
      },
    }

    expected_cm_json = {
      'uri' => '/repositories/2/collection_management/10',
      'processing_priority' => 'high',
      'processing_status' => 'completed',
      'processing_hours_total' => '40',
      'processing_funding_source' => 'grant-funded',
      'processors' => 'Alice and Bob',
      'created_by' => ' admin ',
      'last_modified_by' => ' archivist ',
      'system_mtime' => ' 2026-01-01T00:00:00Z ',
      'user_mtime' => ' 2026-01-01T00:00:00Z ',
      'create_time' => ' 2026-01-01T00:00:00Z ',
      'user_defined_enum_s' => ' unstripped_enum ',
      'external_ids' => [
        {
          'external_id' => 'ext-1',
          'source' => ' local ',
        }
      ],
      'nested' => {
        'title' => 'nested title',
        'created_by' => ' unstripped_audit ',
      },
    }

    valid_child = {
      'id' => '/repositories/2/collection_management/10',
      'uri' => '/repositories/2/collection_management/10',
      'parent_id' => '/repositories/2/resources/1',
      'parent_title' => 'Resource 1',
      'parent_type' => 'resource',
      'title' => 'Resource 1',
      'title_sort' => 'Resource 1',
      'primary_type' => 'collection_management',
      'types' => ['collection_management'],
      'suppressed' => false,
      'repository' => 2,
      'processing_priority' => 'high',
      'processing_status' => 'completed',
      'processing_hours_total' => '40',
      'processing_funding_source' => 'grant-funded',
      'processors' => 'Alice and Bob',
      'created_by' => ' admin ',
      'last_modified_by' => ' archivist ',
      'system_mtime' => ' 2026-01-01T00:00:00Z ',
      'user_mtime' => ' 2026-01-01T00:00:00Z ',
      'create_time' => ' 2026-01-01T00:00:00Z ',
      'json' => ASUtils.to_json(expected_cm_json),
    }

    build_doc = lambda do |child|
      {
        'id' => valid_input['uri'],
        'uri' => valid_input['uri'],
        'title' => 'Resource 1',
        'primary_type' => 'resource',
        'types' => ['resource'],
        'json' => ASUtils.to_json(valid_input['record']),
        'suppressed' => false,
        'publish' => false,
        'system_generated' => 'false',
        'repository' => 2,
        'notes' => [],
        'notes_published' => [],
        'fullrecord_published' => [],
        'fullrecord' => [],
        'finding_aid_title' => nil,
        'finding_aid_filing_title' => nil,
        'identifier' => 'R1',
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
        'four_part_id' => 'R1',
        'processing_priority' => 'high',
        'processors' => 'Alice and Bob',
        '_archivalist_rust_fields' => [
          'common_metadata',
          'fullrecord',
          'resource_archival_object_metadata',
          'dynamic_enum_and_ark_metadata',
        ],
        '_archivalist_collection_management_documents' => [child],
      }
    end

    # 1. Exact expected JSON is accepted, and input record is NOT mutated
    full_cm_before = ASUtils.json_parse(ASUtils.to_json(full_cm))
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [build_doc.call(valid_child)])
    )
    result = builder.build([valid_input])
    expect(result[valid_input['uri']]).not_to be_nil
    expect(valid_input['record']['collection_management']).to eq(full_cm_before)
    expect(result[valid_input['uri']]['_archivalist_collection_management_documents'].first['json']).to eq(ASUtils.to_json(expected_cm_json))

    test_rejection = lambda do |corrupt_json, error_match|
      corrupt_child = valid_child.merge('json' => ASUtils.to_json(corrupt_json))
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [build_doc.call(corrupt_child)])
      )
      expect { builder.build([valid_input]) }.to raise_error(error_match)
    end

    # 2. Same-URI but missing field
    missing_field_json = expected_cm_json.reject {|k, _| k == 'processing_status'}
    test_rejection.call(missing_field_json, /keys mismatch.*missing.*processing_status/)

    # 3. Same-URI but extra field
    extra_field_json = expected_cm_json.merge('unexpected_extra_field' => 'bogus')
    test_rejection.call(extra_field_json, /keys mismatch.*extra.*unexpected_extra_field/)

    # 4. Same-URI but altered processing value
    altered_processing_json = expected_cm_json.merge('processing_priority' => 'low')
    test_rejection.call(altered_processing_json, /processing_priority value mismatch/)

    # 5. Same-URI but altered audit value
    altered_audit_json = expected_cm_json.merge('created_by' => 'altered_admin')
    test_rejection.call(altered_audit_json, /created_by value mismatch/)

    # 6. Same-URI but altered nested value
    corrupt_nested = expected_cm_json.dup
    corrupt_nested['nested'] = expected_cm_json['nested'].merge('title' => 'corrupted title')
    test_rejection.call(corrupt_nested, /nested\.title value mismatch/)

    # 7. Same-URI but type change
    type_change_json = expected_cm_json.merge('processing_hours_total' => 40)
    test_rejection.call(type_change_json, /processing_hours_total type mismatch/)

    # 8. Same parsed JSON object but different key order
    reordered_cm_hash = expected_cm_json.to_a.reverse.to_h
    expect(reordered_cm_hash).to eq(expected_cm_json)
    reordered_json_string = ASUtils.to_json(reordered_cm_hash)
    expect(reordered_json_string).not_to eq(ASUtils.to_json(expected_cm_json))
    corrupt_child = valid_child.merge('json' => reordered_json_string)
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [build_doc.call(corrupt_child)])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child json serialization mismatch/)
  end

  it 'requires byte-for-byte exact stored JSON string equality for collection_management child document' do
    full_cm = {
      'uri' => '/repositories/2/collection_management/10',
      'processing_priority' => ' high ',
      'processing_status' => ' completed ',
    }
    valid_input = {
      'uri' => '/repositories/2/resources/1',
      'record_type' => 'resource',
      'repository' => 2,
      'enrich' => true,
      'record' => {
        'uri' => '/repositories/2/resources/1',
        'title' => 'Resource 1',
        'id_0' => 'R1',
        'level' => 'collection',
        'suppressed' => false,
        'collection_management' => full_cm,
      },
    }
    expected_cm_json = {
      'uri' => '/repositories/2/collection_management/10',
      'processing_priority' => 'high',
      'processing_status' => 'completed',
    }
    expected_json_string = ASUtils.to_json(expected_cm_json)
    valid_child = {
      'id' => '/repositories/2/collection_management/10',
      'uri' => '/repositories/2/collection_management/10',
      'parent_id' => '/repositories/2/resources/1',
      'parent_title' => 'Resource 1',
      'parent_type' => 'resource',
      'title' => 'Resource 1',
      'title_sort' => 'Resource 1',
      'primary_type' => 'collection_management',
      'types' => ['collection_management'],
      'suppressed' => false,
      'repository' => 2,
      'processing_priority' => 'high',
      'processing_status' => 'completed',
      'processing_hours_total' => nil,
      'processing_funding_source' => nil,
      'processors' => nil,
      'created_by' => nil,
      'last_modified_by' => nil,
      'system_mtime' => nil,
      'user_mtime' => nil,
      'create_time' => nil,
      'json' => expected_json_string,
    }
    doc = {
      'id' => valid_input['uri'],
      'uri' => valid_input['uri'],
      'title' => 'Resource 1',
      'primary_type' => 'resource',
      'types' => ['resource'],
      'json' => ASUtils.to_json(valid_input['record']),
      'suppressed' => false,
      'publish' => false,
      'system_generated' => 'false',
      'repository' => 2,
      'fullrecord_published' => [],
      'fullrecord' => [],
      'finding_aid_title' => nil,
      'finding_aid_filing_title' => nil,
      'identifier' => 'R1',
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
      'four_part_id' => 'R1',
      'processing_priority' => 'high',
      'processors' => nil,
      '_archivalist_rust_fields' => [
        'common_metadata',
        'fullrecord',
        'resource_archival_object_metadata',
        'dynamic_enum_and_ark_metadata',
      ],
      '_archivalist_collection_management_documents' => [valid_child],
    }

    # Positive exact-string test
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [doc])
    )
    result = builder.build([valid_input])
    emitted_child = result[valid_input['uri']]['_archivalist_collection_management_documents'].first
    expect(emitted_child['json']).to eq(expected_json_string)

    # Key order variation with identical parsed hash is rejected
    reordered_cm_hash = expected_cm_json.to_a.reverse.to_h
    expect(reordered_cm_hash).to eq(expected_cm_json)
    reordered_doc = doc.merge(
      '_archivalist_collection_management_documents' => [
        valid_child.merge('json' => ASUtils.to_json(reordered_cm_hash))
      ]
    )
    allow(builder).to receive(:perform_request).and_return(
      response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                      'documents' => [reordered_doc])
    )
    expect { builder.build([valid_input]) }.to raise_error(/child json serialization mismatch/)
  end

  describe "dynamic_enum_and_ark_metadata projection validation" do
    let(:base_resource_input) do
      {
        'uri' => '/repositories/2/resources/99',
        'record_type' => 'resource',
        'repository' => 2,
        'enrich' => true,
        'dynamic_enum_fields' => ['status', 'genre'],
        'arks_enabled' => true,
        'record' => {
          'uri' => '/repositories/2/resources/99',
          'title' => 'Validation Resource',
          'publish' => true,
          'status' => 'draft',
          'genre' => 'korespondencja_rękopiśmienna',
          'notes' => [
            { 'status' => 'reviewed', 'items' => { 'type' => 'ordered_list' } },
            { 'status' => 'draft' }, # duplicate
            { '_resolved' => { 'status' => 'should_be_excluded' } },
          ],
          'ark_name' => {
            'current' => 'https://example.org/ark:/12345/curr_żółw',
            'previous' => ['http://proxy/ark:/12345/ark:/prev_final'],
          },
        },
      }
    end

    def valid_doc_for(input)
      {
        'id' => input['uri'],
        'uri' => input['uri'],
        'title' => input['record']['title'],
        'primary_type' => 'resource',
        'types' => ['resource'],
        'json' => ASUtils.to_json(input['record']),
        'suppressed' => false,
        'publish' => true,
        'system_generated' => 'false',
        'repository' => 2,
        'notes' => [],
        'notes_published' => [],
        'fullrecord_published' => [],
        'fullrecord' => [],
        'finding_aid_title' => nil,
        'finding_aid_filing_title' => nil,
        'identifier' => 'VAL-99',
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
        'four_part_id' => 'VAL-99',
        'status_enum_s' => ['draft', 'reviewed'],
        'genre_enum_s' => ['korespondencja_rękopiśmienna'],
        'type_enum_s' => ['ordered_list'],
        'ark_name' => ['ark:/12345/curr_żółw', 'ark:/prev_final'],
        '_archivalist_rust_fields' => [
          'common_metadata',
          'fullrecord',
          'resource_archival_object_metadata',
          'dynamic_enum_and_ark_metadata',
        ],
      }
    end

    it 'accepts exact dynamic enum and ARK projection matching native semantics' do
      doc = valid_doc_for(base_resource_input)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      result = builder.build([base_resource_input])
      expect(result[base_resource_input['uri']]).to eq(doc)
    end

    it 'rejects unexpected dynamic enum keys' do
      doc = valid_doc_for(base_resource_input)
      doc['unexpected_field_enum_s'] = ['intruder']
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_resource_input]) }.to raise_error(/unexpected dynamic enum keys projected/)
    end

    it 'rejects missing dynamic enum keys' do
      doc = valid_doc_for(base_resource_input)
      doc.delete('genre_enum_s')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_resource_input]) }.to raise_error(/missing dynamic enum keys/)
    end

    it 'rejects non-array dynamic enum values' do
      doc = valid_doc_for(base_resource_input)
      doc['status_enum_s'] = 'draft' # not an array
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_resource_input]) }.to raise_error(/dynamic enum field status_enum_s must be an array/)
    end

    it 'rejects mismatched dynamic enum values or ordering' do
      doc = valid_doc_for(base_resource_input)
      doc['status_enum_s'] = ['reviewed', 'draft'] # inverted order
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_resource_input]) }.to raise_error(/dynamic enum values mismatch/)
    end

    it 'rejects ark_name when ARKs are disabled in input' do
      disabled_input = base_resource_input.merge('arks_enabled' => false)
      doc = valid_doc_for(base_resource_input) # doc still has ark_name
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([disabled_input]) }.to raise_error(/unexpected ark_name projected when ARKs are disabled/)
    end

    it 'rejects ark_name when absent in record but present in response' do
      no_ark_record = base_resource_input['record'].dup
      no_ark_record.delete('ark_name')
      no_ark_input = base_resource_input.merge('record' => no_ark_record)
      doc = valid_doc_for(no_ark_input)
      doc['ark_name'] = ['ark:/12345/curr_żółw', 'ark:/prev_final']
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([no_ark_input]) }.to raise_error(/unexpected ark_name projected when absent in record/)
    end

    it 'rejects non-array ark_name or mismatched ARK values' do
      doc = valid_doc_for(base_resource_input)
      doc['ark_name'] = 'ark:/12345/curr_żółw' # non-array
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_resource_input]) }.to raise_error(/response ark_name must be an array/)

      doc['ark_name'] = ['ark:/wrong_val']
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_resource_input]) }.to raise_error(/response ark_name mismatch/)
    end

    it 'rejects integer vs float dynamic enum value type mismatches' do
      int_input = base_resource_input.merge(
        'record' => base_resource_input['record'].merge('status' => 1)
      )
      doc = valid_doc_for(int_input)
      doc['status_enum_s'] = [1.0, 'reviewed', 'draft'] # float instead of integer
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([int_input]) }.to raise_error(/dynamic enum values mismatch.*type mismatch/)
    end

    it 'rejects boolean vs integer dynamic enum value type mismatches' do
      bool_input = base_resource_input.merge(
        'record' => base_resource_input['record'].merge('status' => true)
      )
      doc = valid_doc_for(bool_input)
      doc['status_enum_s'] = [1, 'reviewed', 'draft'] # integer instead of boolean
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([bool_input]) }.to raise_error(/dynamic enum values mismatch.*type mismatch/)
    end

    it 'validates ARK values with newline cases matching native Ruby /\A.*ark:/ semantics' do
      newline_ark_input = base_resource_input.merge(
        'record' => base_resource_input['record'].merge(
          'ark_name' => {
            'current' => "prefix\nark:/later",
            'previous' => ["pre ark:/first\nark:/second", "pre ark:/one ark:/two\nark:/three"],
          }
        )
      )
      doc = valid_doc_for(newline_ark_input)
      doc['ark_name'] = [
        "prefix\nark:/later",
        "ark:/first\nark:/second",
        "ark:/two\nark:/three",
      ]
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      result = builder.build([newline_ark_input])
      expect(result[newline_ark_input['uri']]['ark_name']).to eq(doc['ark_name'])
    end

    it 'rejects ARK projection if Rust incorrectly trims across newlines' do
      newline_ark_input = base_resource_input.merge(
        'record' => base_resource_input['record'].merge(
          'ark_name' => {
            'current' => "prefix\nark:/later",
            'previous' => [],
          }
        )
      )
      doc = valid_doc_for(newline_ark_input)
      doc['ark_name'] = ["ark:/later"]
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([newline_ark_input]) }.to raise_error(/response ark_name mismatch/)
    end
  end

  describe 'accession date enrichment validation' do
    let(:base_accession_input) do
      {
        'uri' => '/repositories/2/accessions/88',
        'record_type' => 'accession',
        'repository' => 2,
        'enrich' => true,
        'accession_date_unknown_label' => 'незнаний',
        'record' => {
          'uri' => '/repositories/2/accessions/88',
          'title' => 'Accession 88',
          'display_string' => 'ACC-88: Accession 88',
          'accession_date' => '2026-10-17',
          'publish' => true,
          'id_0' => 'ACC-88',
          'acquisition_type' => 'deposit',
          'resource_type' => 'records',
          'restrictions_apply' => false,
          'access_restrictions' => false,
          'use_restrictions' => false,
          'related_resources' => [],
          'related_accessions' => [],
          'slug' => 'acc-88',
          'is_slug_auto' => true,
        },
      }
    end

    def valid_accession_doc_for(input)
      rec = input['record']
      raw_date = rec['accession_date']
      is_sentinel = (raw_date == '9999-12-31')
      unknown_label = input['accession_date_unknown_label']
      acc_date = is_sentinel ? unknown_label : raw_date
      acc_year = is_sentinel ? 9999 : Date.parse(raw_date).year
      fullrecord = is_sentinel ? ["record content", "#{unknown_label} "] : ["record content"]

      {
        'id' => input['uri'],
        'uri' => input['uri'],
        'title' => rec['display_string'],
        'primary_type' => 'accession',
        'types' => ['accession'],
        'json' => ASUtils.to_json(rec),
        'suppressed' => false,
        'publish' => true,
        'system_generated' => 'false',
        'repository' => 2,
        'notes' => [],
        'notes_published' => [],
        'fullrecord_published' => [],
        'fullrecord' => fullrecord,
        'accession_date' => acc_date,
        'accession_date_year' => acc_year,
        'identifier' => 'ACC-88',
        'acquisition_type' => 'deposit',
        'resource_type' => 'records',
        'restrictions_apply' => false,
        'access_restrictions' => false,
        'use_restrictions' => false,
        'related_resource_uris' => [],
        'related_accession_uris' => [],
        'slug' => 'acc-88',
        'is_slug_auto' => true,
        'external_id' => [],
        'ancestors' => [],
        'langcode' => [],
        'four_part_id' => 'ACC-88',
        '_archivalist_rust_fields' => [
          'common_metadata',
          'fullrecord',
          'accession_digital_object_metadata',
          'accession_date_metadata',
          'dynamic_enum_and_ark_metadata',
        ],
      }
    end

    it 'accepts ordinary accession date and year with exact integer type' do
      doc = valid_accession_doc_for(base_accession_input)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      result = builder.build([base_accession_input])
      expect(result[base_accession_input['uri']]['accession_date']).to eq('2026-10-17')
      expect(result[base_accession_input['uri']]['accession_date_year']).to eq(2026)
    end

    it 'accepts sentinel accession date with Unicode localized label and array fullrecord' do
      sentinel_input = base_accession_input.merge(
        'record' => base_accession_input['record'].merge('accession_date' => '9999-12-31')
      )
      doc = valid_accession_doc_for(sentinel_input)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      result = builder.build([sentinel_input])
      expect(result[sentinel_input['uri']]['accession_date']).to eq('незнаний')
      expect(result[sentinel_input['uri']]['accession_date_year']).to eq(9999)
      expect(result[sentinel_input['uri']]['fullrecord'].last).to eq('незнаний ')
    end

    it 'accepts sentinel accession date with string fullrecord' do
      sentinel_input = base_accession_input.merge(
        'record' => base_accession_input['record'].merge('accession_date' => '9999-12-31')
      )
      doc = valid_accession_doc_for(sentinel_input)
      doc['fullrecord'] = "some initial text незнаний "
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      result = builder.build([sentinel_input])
      expect(result[sentinel_input['uri']]['fullrecord']).to eq("some initial text незнаний ")
    end

    it 'rejects string accession_date_year (anti-string-coercion)' do
      doc = valid_accession_doc_for(base_accession_input)
      doc['accession_date_year'] = '2026'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_accession_input]) }.to raise_error(/response accession_date_year must be an integer/)
    end

    it 'rejects float accession_date_year' do
      doc = valid_accession_doc_for(base_accession_input)
      doc['accession_date_year'] = 2026.0
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_accession_input]) }.to raise_error(/response accession_date_year must be an integer/)
    end

    it 'rejects mismatched accession_date_year' do
      doc = valid_accession_doc_for(base_accession_input)
      doc['accession_date_year'] = 2025
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_accession_input]) }.to raise_error(/response accession_date_year mismatch/)
    end

    it 'rejects mismatched accession_date value' do
      doc = valid_accession_doc_for(base_accession_input)
      doc['accession_date'] = '2026-10-18'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_accession_input]) }.to raise_error(/response accession_date mismatch/)
    end

    it 'rejects missing accession_date_metadata marker on accession' do
      doc = valid_accession_doc_for(base_accession_input)
      doc['_archivalist_rust_fields'].delete('accession_date_metadata')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_accession_input]) }.to raise_error(/response enrichment marker mismatch/)
    end

    it 'rejects missing accession_date field in response' do
      doc = valid_accession_doc_for(base_accession_input)
      doc.delete('accession_date')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_accession_input]) }.to raise_error(/response (enrichment fields missing:.*accession_date|accession_date)/)
    end

    it 'rejects missing accession_date_year field in response' do
      doc = valid_accession_doc_for(base_accession_input)
      doc.delete('accession_date_year')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_accession_input]) }.to raise_error(/response (enrichment fields missing:.*accession_date_year|accession_date_year)/)
    end

    it 'rejects sentinel accession date when accession_date_unknown_label is missing in input' do
      sentinel_input = base_accession_input.merge(
        'accession_date_unknown_label' => nil,
        'record' => base_accession_input['record'].merge('accession_date' => '9999-12-31')
      )
      doc = valid_accession_doc_for(sentinel_input.merge('accession_date_unknown_label' => 'unknown'))
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([sentinel_input]) }.to raise_error(/accession_date_unknown_label missing or empty/)
    end

    it 'rejects sentinel accession date when fullrecord does not end with localized label' do
      sentinel_input = base_accession_input.merge(
        'record' => base_accession_input['record'].merge('accession_date' => '9999-12-31')
      )
      doc = valid_accession_doc_for(sentinel_input)
      doc['fullrecord'] = ["without localized label"]
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([sentinel_input]) }.to raise_error(/response fullrecord array does not end with sentinel label/)
    end

    it 'accepts ordinary accession date when fullrecord legitimately ends with unknown label string' do
      doc = valid_accession_doc_for(base_accession_input)
      doc['fullrecord'] = ["content", "незнаний "]
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      result = builder.build([base_accession_input])
      expect(result[base_accession_input['uri']]['fullrecord']).to eq(["content", "незнаний "])
    end

    it 'accepts non-canonical schema-valid accession dates (year 0, unpadded month, whitespace)' do
      examples = [
        { 'date' => '0000-01-01', 'expected_year' => 0 },
        { 'date' => '2026-2-03', 'expected_year' => 2026 },
        { 'date' => '  2026-10-17  ', 'expected_year' => 2026 },
      ]

      examples.each_with_index do |ex, i|
        input = base_accession_input.merge(
          'uri' => "/repositories/2/accessions/#{100 + i}",
          'parsed_accession_date_year' => ex['expected_year'],
          'record' => base_accession_input['record'].merge(
            'uri' => "/repositories/2/accessions/#{100 + i}",
            'accession_date' => ex['date']
          )
        )
        doc = valid_accession_doc_for(input)
        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                          'documents' => [doc])
        )
        result = builder.build([input])
        expect(result[input['uri']]['accession_date']).to eq(ex['date'])
        expect(result[input['uri']]['accession_date_year']).to eq(ex['expected_year'])
      end
    end

    it 'preserves and validates Unicode unknown label with intentional surrounding whitespace byte-for-byte' do
      ws_label = "  незнаний  "
      sentinel_input = base_accession_input.merge(
        'accession_date_unknown_label' => ws_label,
        'parsed_accession_date_year' => 9999,
        'record' => base_accession_input['record'].merge('accession_date' => '9999-12-31')
      )
      doc = valid_accession_doc_for(sentinel_input)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      result = builder.build([sentinel_input])
      expect(result[sentinel_input['uri']]['accession_date']).to eq(ws_label)
      expect(result[sentinel_input['uri']]['fullrecord'].last).to eq("#{ws_label} ")
    end
  end

  describe 'PUI final document privacy preparation' do
    let(:pui_input) do
      candidate = {
        'uri' => '/repositories/2/archival_objects/42',
        'jsonmodel_type' => 'archival_object',
        'title' => 'Letters to Bro. Paul',
        'display_string' => 'Letters to Bro. Paul, 1880',
        'publish' => true,
        'ancestors' => [
          {
            'ref' => '/repositories/2/resources/1',
            'title' => 'Secret Resource Title'
          }
        ],
        'instances' => [
          {
            'sub_container' => {
              'top_container' => {
                'ref' => '/repositories/2/top_containers/10',
                '_resolved' => {
                  'indicator' => 'Box 1',
                  'internal_note' => 'RESTRICTED_TC_INTERNAL_NOTE',
                  'container_profile' => {
                    'ref' => '/container_profiles/5',
                    '_resolved' => {
                      'name' => 'Hollinger Box',
                      'notes' => 'RESTRICTED_PROFILE_NOTE'
                    }
                  }
                }
              }
            }
          }
        ],
        'notes' => [
          {
            'jsonmodel_type' => 'note_singlepart',
            'type' => 'abstract',
            'publish' => true,
            'content' => ['Public abstract content']
          }
        ]
      }

      {
        'uri' => '/repositories/2/archival_objects/42',
        'record_type' => 'archival_object',
        'repository' => 2,
        'record' => {
          'uri' => '/repositories/2/archival_objects/42',
          'jsonmodel_type' => 'archival_object',
          'title' => 'Letters to Bro. Paul',
          'publish' => true
        },
        'prepare_pui_final' => true,
        'pui_candidate_record' => candidate
      }
    end

    def valid_clean_candidate_for(candidate)
      clean = ASUtils.json_parse(ASUtils.to_json(candidate))
      clean.delete('ancestors')
      clean['instances'][0]['sub_container']['top_container']['_resolved'].delete('internal_note')
      clean['instances'][0]['sub_container']['top_container']['_resolved']['container_profile']['_resolved'].delete('notes')
      clean
    end

    def valid_pui_doc_for(input)
      clean = valid_clean_candidate_for(input['pui_candidate_record'])
      published_strings = builder.send(:extract_published_strings, clean)
      notes_published = if input['record']['notes']
                          builder.send(:extract_published_strings, input['record']['notes'])
                        else
                          nil
                        end
      {
        'id' => input['uri'],
        'uri' => input['uri'],
        'title' => input['record']['title'],
        'primary_type' => input['record_type'],
        'types' => [input['record_type']],
        'json' => ASUtils.to_json(input['record']),
        'suppressed' => false,
        'publish' => true,
        'system_generated' => 'false',
        'repository' => input['repository'],
        '_archivalist_rust_fields' => [
          'pui_final_document_metadata',
          'pui_routing_metadata',
          'pui_notes_metadata'
        ],
        '_archivalist_pui_final' => {
          'marker' => 'pui_final_document_metadata',
          'json' => ASUtils.to_json(clean),
          'fullrecord_published' => published_strings
        },
        '_archivalist_pui_routing' => {
          'marker' => 'pui_routing_metadata',
          'id' => "#{input['uri']}#pui",
          'pui_parent_id' => input['uri'],
          'types' => ['pui', "pui_#{input['record_type']}", 'pui_record', 'pui_only']
        },
        '_archivalist_pui_notes' => {
          'marker' => 'pui_notes_metadata',
          'notes_published' => notes_published
        }
      }
    end

    it 'accepts a valid PUI final projection matching privacy-clean candidate' do
      doc = valid_pui_doc_for(pui_input)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      result = builder.build([pui_input])
      expect(result[pui_input['uri']]).to eq(doc)
    end

    it 'rejects wrong marker in _archivalist_rust_fields' do
      doc = valid_pui_doc_for(pui_input)
      doc['_archivalist_rust_fields'] = ['wrong_marker']
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([pui_input]) }.to raise_error(/response pui final marker mismatch/)
    end

    it 'rejects missing or wrong marker in _archivalist_pui_final' do
      doc = valid_pui_doc_for(pui_input)
      doc['_archivalist_pui_final']['marker'] = 'bad_marker'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui final payload marker mismatch/)
    end

    it 'rejects malformed JSON string in pui_final[json]' do
      doc = valid_pui_doc_for(pui_input)
      doc['_archivalist_pui_final']['json'] = '{bad json'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([pui_input]) }.to raise_error(StandardError)
    end

    it 'rejects wrong types in pui_final' do
      doc = valid_pui_doc_for(pui_input)
      doc['_archivalist_pui_final']['fullrecord_published'] = 'not an array'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([pui_input]) }.to raise_error(/fullrecord_published must be an Array/)
    end

    it 'rejects altered public data in pui_final[json]' do
      doc = valid_pui_doc_for(pui_input)
      clean = valid_clean_candidate_for(pui_input['pui_candidate_record'])
      clean['title'] = 'Tampered Title'
      doc['_archivalist_pui_final']['json'] = ASUtils.to_json(clean)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([pui_input]) }.to raise_error(/value mismatch/)
    end

    it 'rejects leaked ancestor in pui_final[json]' do
      doc = valid_pui_doc_for(pui_input)
      clean = valid_clean_candidate_for(pui_input['pui_candidate_record'])
      clean['ancestors'] = [{'ref' => '/repositories/2/resources/1'}]
      doc['_archivalist_pui_final']['json'] = ASUtils.to_json(clean)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([pui_input]) }.to raise_error(/must not contain ancestors/)
    end

    it 'rejects leaked top_container internal_note in pui_final[json]' do
      doc = valid_pui_doc_for(pui_input)
      clean = valid_clean_candidate_for(pui_input['pui_candidate_record'])
      clean['instances'][0]['sub_container']['top_container']['_resolved']['internal_note'] = 'LEAKED'
      doc['_archivalist_pui_final']['json'] = ASUtils.to_json(clean)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([pui_input]) }.to raise_error(/contains top_container internal_note/)
    end

    it 'rejects leaked container_profile notes in pui_final[json]' do
      doc = valid_pui_doc_for(pui_input)
      clean = valid_clean_candidate_for(pui_input['pui_candidate_record'])
      clean['instances'][0]['sub_container']['top_container']['_resolved']['container_profile']['_resolved']['notes'] = 'LEAKED'
      doc['_archivalist_pui_final']['json'] = ASUtils.to_json(clean)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([pui_input]) }.to raise_error(/contains container_profile notes/)
    end

    it 'rejects unexpected unpublished fullrecord field in PUI final document' do
      doc = valid_pui_doc_for(pui_input)
      doc['fullrecord'] = ['UNPUBLISHED TEXT']
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([pui_input]) }.to raise_error(/unexpected unpublished fullrecord field/)
    end

    it 'rejects unexpected _archivalist_pui_final when prepare_pui_final is false' do
      unenriched_input = pui_input.dup
      unenriched_input.delete('prepare_pui_final')
      unenriched_input.delete('pui_candidate_record')
      unenriched_input['record'] = unenriched_input['record'].merge('display_string' => unenriched_input['record']['title'])
      doc = valid_pui_doc_for(pui_input)
      doc['json'] = ASUtils.to_json(unenriched_input['record'])
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([unenriched_input]) }.to raise_error(/(unexpected response pui projection marker|unexpected response enrichment marker)/)
    end

    it 'rejects mismatched fullrecord_published' do
      doc = valid_pui_doc_for(pui_input)
      doc['_archivalist_pui_final']['fullrecord_published'] = ['tampered string']
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([pui_input]) }.to raise_error(/fullrecord_published mismatch/)
    end

    it 'rejects missing, wrong-marker, or mismatched pui_routing projection' do
      # 1. Missing _archivalist_pui_routing
      doc_no_routing = valid_pui_doc_for(pui_input)
      doc_no_routing.delete('_archivalist_pui_routing')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_no_routing])
      )
      expect { builder.build([pui_input]) }.to raise_error(/missing _archivalist_pui_routing/)

      # 2. Bad marker
      doc_bad_marker = valid_pui_doc_for(pui_input)
      doc_bad_marker['_archivalist_pui_routing']['marker'] = 'bad_marker'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_bad_marker])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui routing marker mismatch/)

      # 3. Mismatched id
      doc_bad_id = valid_pui_doc_for(pui_input)
      doc_bad_id['_archivalist_pui_routing']['id'] = "#{pui_input['uri']}#wrong"
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_bad_id])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui routing id mismatch/)

      # 4. Mismatched pui_parent_id
      doc_bad_parent = valid_pui_doc_for(pui_input)
      doc_bad_parent['_archivalist_pui_routing']['pui_parent_id'] = '/repositories/2/resources/999'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_bad_parent])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui routing pui_parent_id mismatch/)

      # 5. Mismatched types
      doc_bad_types = valid_pui_doc_for(pui_input)
      doc_bad_types['_archivalist_pui_routing']['types'] = ['pui', 'unauthorized_type']
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_bad_types])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui routing types mismatch/)
    end

    it 'rejects extra, missing, or non-string keys in pui_routing projection' do
      # Extra string key (e.g. admin_override)
      doc_extra = valid_pui_doc_for(pui_input)
      doc_extra['_archivalist_pui_routing']['admin_override'] = true
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_extra])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui routing contains unexpected fields.*admin_override/)

      # Non-string / symbol key (e.g. :admin_override)
      doc_sym = valid_pui_doc_for(pui_input)
      doc_sym['_archivalist_pui_routing'][:admin_override] = true
      mock_body_sym = '__MOCK_BODY_SYM_ROUTING__'
      allow(builder).to receive(:perform_request).and_return(Struct.new(:code, :body).new('200', mock_body_sym))
      allow(ASUtils).to receive(:json_parse).and_wrap_original do |m, s, *args|
        s == mock_body_sym ? { 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER, 'documents' => [doc_sym] } : m.call(s, *args)
      end
      expect { builder.build([pui_input]) }.to raise_error(/pui routing contains non-string keys/)

      # Missing key (types)
      doc_missing_types = valid_pui_doc_for(pui_input)
      doc_missing_types['_archivalist_pui_routing'].delete('types')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_missing_types])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui routing missing required fields.*types/)

      # Missing key (id)
      doc_missing_id = valid_pui_doc_for(pui_input)
      doc_missing_id['_archivalist_pui_routing'].delete('id')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_missing_id])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui routing missing required fields.*id/)
    end

    it 'rejects missing, wrong-marker, or mismatched pui_notes projection' do
      # 1. Missing _archivalist_pui_notes
      doc_no_notes = valid_pui_doc_for(pui_input)
      doc_no_notes.delete('_archivalist_pui_notes')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_no_notes])
      )
      expect { builder.build([pui_input]) }.to raise_error(/missing _archivalist_pui_notes/)

      # 2. Bad marker
      doc_bad_marker = valid_pui_doc_for(pui_input)
      doc_bad_marker['_archivalist_pui_notes']['marker'] = 'bad_marker'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_bad_marker])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui notes marker mismatch/)

      # 3. Present notes with mismatched notes_published
      input_with_notes = pui_input.dup
      input_with_notes['record'] = pui_input['record'].dup
      input_with_notes['record']['notes'] = [
        {
          'jsonmodel_type' => 'note_singlepart',
          'type' => 'abstract',
          'publish' => true,
          'content' => ['Authentic note']
        }
      ]
      doc_with_notes = valid_pui_doc_for(input_with_notes)
      doc_with_notes['_archivalist_pui_notes']['notes_published'] = ['Injected unpublished note']
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_with_notes])
      )
      expect { builder.build([input_with_notes]) }.to raise_error(/pui notes_published mismatch/)

      # 4. Absent notes with non-nil notes_published
      doc_absent_notes = valid_pui_doc_for(pui_input)
      doc_absent_notes['_archivalist_pui_notes']['notes_published'] = ['Should be nil']
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_absent_notes])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui notes_published must be nil/)
    end

    it 'rejects extra, missing, or non-string keys in pui_notes projection' do
      # Extra string key (e.g. private_payload)
      doc_extra = valid_pui_doc_for(pui_input)
      doc_extra['_archivalist_pui_notes']['private_payload'] = 'CONFIDENTIAL_NOTE'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_extra])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui notes contains unexpected fields.*private_payload/)

      # Non-string / symbol key (e.g. :private_payload)
      doc_sym = valid_pui_doc_for(pui_input)
      doc_sym['_archivalist_pui_notes'][:private_payload] = 'CONFIDENTIAL_NOTE'
      mock_body_notes_sym = '__MOCK_BODY_SYM_NOTES__'
      allow(builder).to receive(:perform_request).and_return(Struct.new(:code, :body).new('200', mock_body_notes_sym))
      allow(ASUtils).to receive(:json_parse).and_wrap_original do |m, s, *args|
        s == mock_body_notes_sym ? { 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER, 'documents' => [doc_sym] } : m.call(s, *args)
      end
      expect { builder.build([pui_input]) }.to raise_error(/pui notes contains non-string keys/)

      # Missing key (notes_published)
      doc_missing_notes = valid_pui_doc_for(pui_input)
      doc_missing_notes['_archivalist_pui_notes'].delete('notes_published')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_missing_notes])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui notes missing required fields.*notes_published/)

      # Missing key (marker)
      doc_missing_marker = valid_pui_doc_for(pui_input)
      doc_missing_marker['_archivalist_pui_notes'].delete('marker')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_missing_marker])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui notes missing required fields.*marker/)
    end

    it 'rejects missing, null, wrong-type, or mismatched input record identity' do
      doc = valid_pui_doc_for(pui_input)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )

      # 1. Missing record uri
      bad_rec1 = pui_input.dup
      bad_rec1['record'] = pui_input['record'].dup
      bad_rec1['record'].delete('uri')
      expect { builder.build([bad_rec1]) }.to raise_error(/record.uri must be a non-empty string/)

      # 2. Mismatched record uri
      bad_rec2 = pui_input.dup
      bad_rec2['record'] = pui_input['record'].dup
      bad_rec2['record']['uri'] = '/repositories/2/archival_objects/999'
      expect { builder.build([bad_rec2]) }.to raise_error(/record uri mismatch/)

      # 3. Missing record jsonmodel_type
      bad_rec3 = pui_input.dup
      bad_rec3['record'] = pui_input['record'].dup
      bad_rec3['record'].delete('jsonmodel_type')
      expect { builder.build([bad_rec3]) }.to raise_error(/record.jsonmodel_type must be a non-empty string/)

      # 4. Mismatched record jsonmodel_type
      bad_rec4 = pui_input.dup
      bad_rec4['record'] = pui_input['record'].dup
      bad_rec4['record']['jsonmodel_type'] = 'resource'
      expect { builder.build([bad_rec4]) }.to raise_error(/record jsonmodel_type mismatch/)
    end

    it 'rejects missing, null, wrong-type, or mismatched candidate identity' do
      doc = valid_pui_doc_for(pui_input)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )

      # 1. Missing candidate uri
      bad_input = pui_input.dup
      bad_input['pui_candidate_record'] = pui_input['pui_candidate_record'].dup
      bad_input['pui_candidate_record'].delete('uri')
      expect { builder.build([bad_input]) }.to raise_error(/pui_candidate_record.uri must be a non-empty string/)

      # 2. Null candidate uri
      bad_input['pui_candidate_record']['uri'] = nil
      expect { builder.build([bad_input]) }.to raise_error(/pui_candidate_record.uri must be a non-empty string/)

      # 3. Wrong-type (integer) candidate uri
      bad_input['pui_candidate_record']['uri'] = 123
      expect { builder.build([bad_input]) }.to raise_error(/pui_candidate_record.uri must be a non-empty string/)

      # 4. Mismatched candidate uri
      bad_input['pui_candidate_record']['uri'] = '/repositories/2/archival_objects/999'
      expect { builder.build([bad_input]) }.to raise_error(/pui_candidate_record uri mismatch/)

      # 5. Missing candidate jsonmodel_type
      bad_input2 = pui_input.dup
      bad_input2['pui_candidate_record'] = pui_input['pui_candidate_record'].dup
      bad_input2['pui_candidate_record'].delete('jsonmodel_type')
      expect { builder.build([bad_input2]) }.to raise_error(/pui_candidate_record.jsonmodel_type must be a non-empty string/)

      # 6. Null candidate jsonmodel_type
      bad_input2['pui_candidate_record']['jsonmodel_type'] = nil
      expect { builder.build([bad_input2]) }.to raise_error(/pui_candidate_record.jsonmodel_type must be a non-empty string/)

      # 7. Mismatched candidate jsonmodel_type
      bad_input2['pui_candidate_record']['jsonmodel_type'] = 'resource'
      expect { builder.build([bad_input2]) }.to raise_error(/pui_candidate_record jsonmodel_type mismatch/)
    end

    it 'rejects missing, null, wrong-type, or mismatched parsed_json identity' do
      # 1. Mismatched parsed_json uri
      doc_mismatch_uri = valid_pui_doc_for(pui_input)
      clean = valid_clean_candidate_for(pui_input['pui_candidate_record'])
      clean['uri'] = '/repositories/2/archival_objects/999'
      doc_mismatch_uri['_archivalist_pui_final']['json'] = ASUtils.to_json(clean)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_mismatch_uri])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui final json uri mismatch/)

      # 2. Missing parsed_json uri
      clean_no_uri = valid_clean_candidate_for(pui_input['pui_candidate_record'])
      clean_no_uri.delete('uri')
      doc_no_uri = valid_pui_doc_for(pui_input)
      doc_no_uri['_archivalist_pui_final']['json'] = ASUtils.to_json(clean_no_uri)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_no_uri])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui final json uri must be a non-empty string/)

      # 3. Mismatched parsed_json jsonmodel_type
      clean_mismatch_type = valid_clean_candidate_for(pui_input['pui_candidate_record'])
      clean_mismatch_type['jsonmodel_type'] = 'resource'
      doc_mismatch_type = valid_pui_doc_for(pui_input)
      doc_mismatch_type['_archivalist_pui_final']['json'] = ASUtils.to_json(clean_mismatch_type)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_mismatch_type])
      )
      expect { builder.build([pui_input]) }.to raise_error(/pui final json jsonmodel_type mismatch/)
    end
  end

  describe 'PUI availability preparation' do
    let(:avail_input) do
      {
        'uri' => '/repositories/2/resources/77',
        'record_type' => 'resource',
        'repository' => 2,
        'prepare_pui_availability' => true,
        'record_inheritance_has_type' => false,
        'record' => {
          'uri' => '/repositories/2/resources/77',
          'jsonmodel_type' => 'resource',
          'title' => 'Avail Resource',
          'publish' => true,
        },
      }
    end

    def valid_avail_doc_for(inp)
      {
        'id' => inp['uri'],
        'uri' => inp['uri'],
        'title' => inp['record']['title'],
        'primary_type' => inp['record_type'],
        'types' => [inp['record_type']],
        'json' => ASUtils.to_json(inp['record']),
        'suppressed' => false,
        'publish' => true,
        'system_generated' => 'false',
        'repository' => inp['repository'],
        '_archivalist_rust_fields' => ['pui_availability_metadata'],
        '_archivalist_pui_availability' => {
          'marker' => 'pui_availability_metadata',
          'uri' => inp['uri'],
          'primary_type' => inp['record_type'],
          'record_inheritance_has_type' => inp['record_inheritance_has_type'],
          'conditional_additions' => RustIndexDocumentBuilder.compute_pui_conditional_additions(
            inp['record_type'],
            inp['record_inheritance_has_type']
          ),
          'top_container_override' => inp['record_type'] == 'top_container' ? {
            'publish' => inp['record']['is_linked_to_published_record'],
            'additions' => ['pui_container', 'pui'],
          } : nil,
        },
      }
    end

    it 'accepts a valid PUI availability projection for resource' do
      doc = valid_avail_doc_for(avail_input)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      res = builder.build([avail_input])
      expect(res).to eq(avail_input['uri'] => doc)
    end

    it 'accepts a valid PUI availability projection for top_container' do
      tc_input = {
        'uri' => '/repositories/2/top_containers/88',
        'record_type' => 'top_container',
        'repository' => 2,
        'prepare_pui_availability' => true,
        'record_inheritance_has_type' => false,
        'record' => {
          'uri' => '/repositories/2/top_containers/88',
          'jsonmodel_type' => 'top_container',
          'title' => 'Box 88',
          'publish' => true,
          'is_linked_to_published_record' => true,
        },
      }
      doc = valid_avail_doc_for(tc_input)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      res = builder.build([tc_input])
      expect(res).to eq(tc_input['uri'] => doc)
    end

    it 'computes exact conditional additions and order for all types' do
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('resource', false))
        .to eq(['pui_collection', 'pui'])
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('classification', false))
        .to eq(['pui_record_group', 'pui'])
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('accession', false))
        .to eq(['pui_record', 'pui_accession', 'pui'])
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('digital_object', false))
        .to eq(['pui_record', 'pui_digital_object', 'pui'])
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('digital_object_component', false))
        .to eq(['pui_record', 'pui_digital_object_component', 'pui'])
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('agent_person', false))
        .to eq(['pui_agent', 'pui_person', 'pui'])
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('agent_corporate_entity', false))
        .to eq(['pui_agent', 'pui_agent_corporate_entity', 'pui'])
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('subject', false))
        .to eq(['pui_subject', 'pui'])
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('top_container', false))
        .to eq(['pui_top_container', 'pui'])

      # When record_inheritance_has_type is true:
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('archival_object', true))
        .to eq([])
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('resource', true))
        .to eq([])
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('accession', true))
        .to eq(['pui_record'])
      expect(RustIndexDocumentBuilder.compute_pui_conditional_additions('agent_person', true))
        .to eq(['pui_agent'])
    end

    it 'rejects combining prepare_pui_final and prepare_pui_availability in same input' do
      conflict_input = avail_input.merge('prepare_pui_final' => true)
      expect { builder.build([conflict_input]) }
        .to raise_error(/cannot combine prepare_pui_final and prepare_pui_availability/)
    end

    it 'rejects missing, null, wrong-type, empty, or mismatched record uri and jsonmodel_type, plus non-boolean inheritance flag' do
      # Missing uri
      bad1 = avail_input.dup
      bad1['record'] = avail_input['record'].dup
      bad1['record'].delete('uri')
      expect { builder.build([bad1]) }.to raise_error(/record.uri must be a non-empty string/)

      # Null uri
      bad1_null = avail_input.dup
      bad1_null['record'] = avail_input['record'].dup
      bad1_null['record']['uri'] = nil
      expect { builder.build([bad1_null]) }.to raise_error(/record.uri must be a non-empty string/)

      # Numeric uri
      bad1_num = avail_input.dup
      bad1_num['record'] = avail_input['record'].dup
      bad1_num['record']['uri'] = 123
      expect { builder.build([bad1_num]) }.to raise_error(/record.uri must be a non-empty string/)

      # Array uri
      bad1_arr = avail_input.dup
      bad1_arr['record'] = avail_input['record'].dup
      bad1_arr['record']['uri'] = ['/repositories/2/resources/77']
      expect { builder.build([bad1_arr]) }.to raise_error(/record.uri must be a non-empty string/)

      # Hash uri
      bad1_hash = avail_input.dup
      bad1_hash['record'] = avail_input['record'].dup
      bad1_hash['record']['uri'] = {'ref' => '/repositories/2/resources/77'}
      expect { builder.build([bad1_hash]) }.to raise_error(/record.uri must be a non-empty string/)

      # Empty uri
      bad1_empty = avail_input.dup
      bad1_empty['record'] = avail_input['record'].dup
      bad1_empty['record']['uri'] = ''
      expect { builder.build([bad1_empty]) }.to raise_error(/record.uri must be a non-empty string/)

      # Whitespace uri
      bad1_ws = avail_input.dup
      bad1_ws['record'] = avail_input['record'].dup
      bad1_ws['record']['uri'] = '  '
      expect { builder.build([bad1_ws]) }.to raise_error(/record.uri must be a non-empty string/)

      # Mismatched uri
      bad1_mismatch = avail_input.dup
      bad1_mismatch['record'] = avail_input['record'].dup
      bad1_mismatch['record']['uri'] = '/repositories/2/resources/999'
      expect { builder.build([bad1_mismatch]) }.to raise_error(/record uri mismatch/)

      # Missing jsonmodel_type
      bad2_miss = avail_input.dup
      bad2_miss['record'] = avail_input['record'].dup
      bad2_miss['record'].delete('jsonmodel_type')
      expect { builder.build([bad2_miss]) }.to raise_error(/record.jsonmodel_type must be a non-empty string/)

      # Null jsonmodel_type
      bad2_null = avail_input.dup
      bad2_null['record'] = avail_input['record'].dup
      bad2_null['record']['jsonmodel_type'] = nil
      expect { builder.build([bad2_null]) }.to raise_error(/record.jsonmodel_type must be a non-empty string/)

      # Numeric jsonmodel_type
      bad2_num = avail_input.dup
      bad2_num['record'] = avail_input['record'].dup
      bad2_num['record']['jsonmodel_type'] = 456
      expect { builder.build([bad2_num]) }.to raise_error(/record.jsonmodel_type must be a non-empty string/)

      # Array jsonmodel_type
      bad2_arr = avail_input.dup
      bad2_arr['record'] = avail_input['record'].dup
      bad2_arr['record']['jsonmodel_type'] = ['resource']
      expect { builder.build([bad2_arr]) }.to raise_error(/record.jsonmodel_type must be a non-empty string/)

      # Hash jsonmodel_type
      bad2_hash = avail_input.dup
      bad2_hash['record'] = avail_input['record'].dup
      bad2_hash['record']['jsonmodel_type'] = {'name' => 'resource'}
      expect { builder.build([bad2_hash]) }.to raise_error(/record.jsonmodel_type must be a non-empty string/)

      # Empty jsonmodel_type
      bad2_empty = avail_input.dup
      bad2_empty['record'] = avail_input['record'].dup
      bad2_empty['record']['jsonmodel_type'] = ''
      expect { builder.build([bad2_empty]) }.to raise_error(/record.jsonmodel_type must be a non-empty string/)

      # Whitespace jsonmodel_type
      bad2_ws = avail_input.dup
      bad2_ws['record'] = avail_input['record'].dup
      bad2_ws['record']['jsonmodel_type'] = '   '
      expect { builder.build([bad2_ws]) }.to raise_error(/record.jsonmodel_type must be a non-empty string/)

      # Mismatched jsonmodel_type
      bad2_mismatch = avail_input.dup
      bad2_mismatch['record'] = avail_input['record'].dup
      bad2_mismatch['record']['jsonmodel_type'] = 'archival_object'
      expect { builder.build([bad2_mismatch]) }.to raise_error(/record jsonmodel_type mismatch/)

      # Non-hash direct record
      bad3 = avail_input.dup
      bad3['record'] = 'not a hash'
      expect { builder.build([bad3]) }.to raise_error(/record must be a Hash/)

      # Missing record_inheritance_has_type
      bad4 = avail_input.dup
      bad4.delete('record_inheritance_has_type')
      expect { builder.build([bad4]) }.to raise_error(/record_inheritance_has_type must be explicitly boolean/)

      # Null record_inheritance_has_type
      bad4_null = avail_input.dup
      bad4_null['record_inheritance_has_type'] = nil
      expect { builder.build([bad4_null]) }.to raise_error(/record_inheritance_has_type must be explicitly boolean/)

      # String record_inheritance_has_type
      bad4_str = avail_input.dup
      bad4_str['record_inheritance_has_type'] = 'false'
      expect { builder.build([bad4_str]) }.to raise_error(/record_inheritance_has_type must be explicitly boolean/)

      # Integer record_inheritance_has_type
      bad4_int = avail_input.dup
      bad4_int['record_inheritance_has_type'] = 0
      expect { builder.build([bad4_int]) }.to raise_error(/record_inheritance_has_type must be explicitly boolean/)
    end

    it 'rejects projection with non-string keys, missing keys, or extra keys' do
      # Non-string key
      doc_sym = valid_avail_doc_for(avail_input)
      proj = doc_sym['_archivalist_pui_availability'].dup
      proj[:bad_sym] = 'val'
      doc_sym['_archivalist_pui_availability'] = proj
      mock_body_avail_sym = '__MOCK_BODY_SYM_AVAIL__'
      allow(builder).to receive(:perform_request).and_return(Struct.new(:code, :body).new('200', mock_body_avail_sym))
      allow(ASUtils).to receive(:json_parse).and_wrap_original do |m, s, *args|
        s == mock_body_avail_sym ? { 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER, 'documents' => [doc_sym] } : m.call(s, *args)
      end
      expect { builder.build([avail_input]) }.to raise_error(/pui availability contains non-string keys/)

      # Missing key
      doc_miss = valid_avail_doc_for(avail_input)
      doc_miss['_archivalist_pui_availability'].delete('conditional_additions')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_miss])
      )
      expect { builder.build([avail_input]) }.to raise_error(/pui availability missing required fields/)

      # Extra key
      doc_extra = valid_avail_doc_for(avail_input)
      doc_extra['_archivalist_pui_availability']['malicious_extra'] = 'payload'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_extra])
      )
      expect { builder.build([avail_input]) }.to raise_error(/pui availability contains unexpected fields/)
    end

    it 'rejects projection with mismatched marker, uri, type, or inheritance flag' do
      # Marker mismatch
      doc1 = valid_avail_doc_for(avail_input)
      doc1['_archivalist_pui_availability']['marker'] = 'pui_wrong_metadata'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc1])
      )
      expect { builder.build([avail_input]) }.to raise_error(/pui availability marker mismatch/)

      # URI mismatch
      doc2 = valid_avail_doc_for(avail_input)
      doc2['_archivalist_pui_availability']['uri'] = '/repositories/2/resources/WRONG'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc2])
      )
      expect { builder.build([avail_input]) }.to raise_error(/pui availability uri mismatch/)

      # Primary type mismatch
      doc3 = valid_avail_doc_for(avail_input)
      doc3['_archivalist_pui_availability']['primary_type'] = 'accession'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc3])
      )
      expect { builder.build([avail_input]) }.to raise_error(/pui availability primary_type mismatch/)

      # Inheritance flag mismatch
      doc4 = valid_avail_doc_for(avail_input)
      doc4['_archivalist_pui_availability']['record_inheritance_has_type'] = true
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc4])
      )
      expect { builder.build([avail_input]) }.to raise_error(/pui availability record_inheritance_has_type mismatch/)
    end

    it 'rejects projection with mismatched conditional additions' do
      doc = valid_avail_doc_for(avail_input)
      doc['_archivalist_pui_availability']['conditional_additions'] = ['pui', 'pui_collection'] # wrong order
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([avail_input]) }.to raise_error(/pui availability conditional_additions mismatch/)
    end

    it 'rejects malformed or mismatched top_container_override' do
      tc_input = {
        'uri' => '/repositories/2/top_containers/88',
        'record_type' => 'top_container',
        'repository' => 2,
        'prepare_pui_availability' => true,
        'record_inheritance_has_type' => false,
        'record' => {
          'uri' => '/repositories/2/top_containers/88',
          'jsonmodel_type' => 'top_container',
          'title' => 'Box 88',
          'publish' => true,
          'is_linked_to_published_record' => true,
        },
      }

      # 1. Non-top-container has non-nil top_container_override
      doc_res = valid_avail_doc_for(avail_input)
      doc_res['_archivalist_pui_availability']['top_container_override'] = {'publish' => true, 'additions' => []}
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_res])
      )
      expect { builder.build([avail_input]) }.to raise_error(/top_container_override must be nil for non-top_container/)

      # 2. Top-container has nil top_container_override
      doc_tc_nil = valid_avail_doc_for(tc_input)
      doc_tc_nil['_archivalist_pui_availability']['top_container_override'] = nil
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_tc_nil])
      )
      expect { builder.build([tc_input]) }.to raise_error(/top_container_override must be a hash/)

      # 3. Top-container has mismatched publish value
      doc_tc_pub = valid_avail_doc_for(tc_input)
      doc_tc_pub['_archivalist_pui_availability']['top_container_override']['publish'] = false # record says true
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_tc_pub])
      )
      expect { builder.build([tc_input]) }.to raise_error(/top_container_override publish mismatch/)

      # 4. Top-container has mismatched additions
      doc_tc_add = valid_avail_doc_for(tc_input)
      doc_tc_add['_archivalist_pui_availability']['top_container_override']['additions'] = ['pui', 'pui_container']
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_tc_add])
      )
      expect { builder.build([tc_input]) }.to raise_error(/top_container_override additions mismatch/)

      # 5. Top-container has extra key in top_container_override
      doc_tc_extra = valid_avail_doc_for(tc_input)
      doc_tc_extra['_archivalist_pui_availability']['top_container_override']['extra'] = true
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_tc_extra])
      )
      expect { builder.build([tc_input]) }.to raise_error(/top_container_override contains unexpected fields/)
    end

    it 'rejects unexpected PUI availability projection when prepare_pui_availability is not requested' do
      plain_input = {
        'uri' => '/repositories/2/resources/99',
        'record_type' => 'resource',
        'repository' => 2,
        'record' => {
          'uri' => '/repositories/2/resources/99',
          'title' => 'Plain Res',
          'publish' => true,
        },
      }
      leaked_doc = valid_avail_doc_for(avail_input).merge('id' => plain_input['uri'], 'uri' => plain_input['uri'], 'title' => plain_input['record']['title'], 'json' => ASUtils.to_json(plain_input['record']))
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [leaked_doc])
      )
      expect { builder.build([plain_input]) }.to raise_error(/(unexpected response pui availability projection marker|unexpected response enrichment marker)/)
    end
  end

  describe '#build_pui_auxiliary' do
    let(:aux_input) do
      rec = {'uris' => ['/repositories/2/archival_objects/1']}
      {
        'uri' => '/repositories/2/resources/10/ordered_records',
        'record_type' => 'resource_ordered_records',
        'pui_parent_id' => '/repositories/2/resources/10',
        'record' => rec,
        'pui_auxiliary_json' => ASUtils.to_json(rec),
        'prepare_pui_auxiliary' => true,
      }
    end

    def valid_aux_doc_for(inp)
      {
        'id' => inp['uri'],
        'uri' => inp['uri'],
        'pui_parent_id' => inp['pui_parent_id'],
        'publish' => 'true',
        'primary_type' => inp['record_type'],
        'types' => ['pui'],
        'json' => inp['pui_auxiliary_json'] || ASUtils.to_json(inp['record']),
        '_archivalist_pui_auxiliary' => {
          'marker' => 'pui_auxiliary_document_metadata',
          'auxiliary_type' => inp['record_type'],
        },
      }
    end

    it 'returns empty array when inputs are empty without calling HTTP' do
      expect(builder).not_to receive(:perform_request)
      expect(builder.build_pui_auxiliary([])).to eq([])
    end

    it 'accepts valid auxiliary documents for all 5 kinds and strips marker' do
      inputs = [
        {
          'uri' => '/repositories/2/resources/10/ordered_records',
          'record_type' => 'resource_ordered_records',
          'pui_parent_id' => '/repositories/2/resources/10',
          'record' => {'uris' => ['/repositories/2/archival_objects/1']},
          'prepare_pui_auxiliary' => true,
        },
        {
          'uri' => '/repositories/2/resources/10/tree/root',
          'record_type' => 'tree_root',
          'pui_parent_id' => '/repositories/2/resources/10',
          'record' => {'title' => 'Root'},
          'prepare_pui_auxiliary' => true,
        },
        {
          'uri' => '/repositories/2/resources/10/tree/waypoint__0',
          'record_type' => 'tree_waypoint',
          'pui_parent_id' => '/repositories/2/resources/10',
          'record' => [{'uri' => '/repositories/2/archival_objects/1'}],
          'prepare_pui_auxiliary' => true,
        },
        {
          'uri' => '/repositories/2/resources/10/tree/node_/repositories/2/archival_objects/1',
          'record_type' => 'tree_node',
          'pui_parent_id' => '/repositories/2/archival_objects/1',
          'record' => {'title' => 'Node'},
          'prepare_pui_auxiliary' => true,
        },
        {
          'uri' => '/repositories/2/resources/10/tree/node_from_root_42',
          'record_type' => 'tree_node_from_root',
          'pui_parent_id' => '/repositories/2/archival_objects/42',
          'record' => {'42' => ['path']},
          'prepare_pui_auxiliary' => true,
        },
      ]
      inputs.each { |inp| inp['pui_auxiliary_json'] = ASUtils.to_json(inp['record']) }
      docs = inputs.map { |inp| valid_aux_doc_for(inp) }

      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => docs)
      )

      results = builder.build_pui_auxiliary(inputs)
      expect(results.length).to eq(5)

      results.each_with_index do |res, idx|
        inp = inputs[idx]
        expect(res.keys.sort).to eq(RustIndexDocumentBuilder::PUI_AUXILIARY_FIELDS.sort)
        expect(res['id']).to eq(inp['uri'])
        expect(res['uri']).to eq(inp['uri'])
        expect(res['pui_parent_id']).to eq(inp['pui_parent_id'])
        expect(res['publish']).to eq('true')
        expect(res['publish']).to be_a(String)
        expect(res['publish']).not_to be(true)
        expect(res['primary_type']).to eq(inp['record_type'])
        expect(res['types']).to eq(['pui'])
        expect(ASUtils.json_parse(res['json'])).to eq(inp['record'])
        expect(res.has_key?('_archivalist_pui_auxiliary')).to be(false)
      end
    end

    it 'strictly rejects non-string or non-"true" publish values' do
      # boolean true
      doc1 = valid_aux_doc_for(aux_input).merge('publish' => true)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc1])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/publish mismatch/)

      # integer 1
      doc2 = valid_aux_doc_for(aux_input).merge('publish' => 1)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc2])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/publish mismatch/)

      # string "false"
      doc3 = valid_aux_doc_for(aux_input).merge('publish' => 'false')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc3])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/publish mismatch/)
    end

    it 'strictly rejects type and value mismatches in types, id, uri, pui_parent_id, and primary_type' do
      # types is not ['pui']
      doc_types = valid_aux_doc_for(aux_input).merge('types' => ['pui', 'extra'])
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_types])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/types mismatch/)

      # id mismatch
      doc_id = valid_aux_doc_for(aux_input).merge('id' => '/other/id')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_id])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/id mismatch/)

      # uri mismatch
      doc_uri = valid_aux_doc_for(aux_input).merge('uri' => '/other/uri')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_uri])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/uri mismatch/)

      # pui_parent_id mismatch
      doc_parent = valid_aux_doc_for(aux_input).merge('pui_parent_id' => '/other/parent')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_parent])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/pui_parent_id mismatch/)

      # primary_type mismatch
      doc_pt = valid_aux_doc_for(aux_input).merge('primary_type' => 'tree_root')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_pt])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/primary_type mismatch/)
    end

    it 'strictly rejects missing fields and unexpected extra fields' do
      # Missing pui_parent_id
      doc_missing = valid_aux_doc_for(aux_input)
      doc_missing.delete('pui_parent_id')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_missing])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/missing required fields/)

      # Extra rogue field
      doc_extra = valid_aux_doc_for(aux_input).merge('rogue_field' => 'injected_leak')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_extra])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/unexpected fields/)
    end

    it 'strictly validates auxiliary metadata marker and auxiliary_type' do
      # Missing marker
      doc_no_meta = valid_aux_doc_for(aux_input)
      doc_no_meta.delete('_archivalist_pui_auxiliary')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_no_meta])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/missing required fields/)

      # Wrong marker name
      doc_wrong_marker = valid_aux_doc_for(aux_input)
      doc_wrong_marker['_archivalist_pui_auxiliary']['marker'] = 'bad_marker'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_wrong_marker])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/marker mismatch/)

      # Wrong auxiliary_type
      doc_wrong_type = valid_aux_doc_for(aux_input)
      doc_wrong_type['_archivalist_pui_auxiliary']['auxiliary_type'] = 'tree_node'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_wrong_type])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/type mismatch/)

      # Extra field in metadata
      doc_meta_extra = valid_aux_doc_for(aux_input)
      doc_meta_extra['_archivalist_pui_auxiliary']['injected'] = true
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_meta_extra])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/auxiliary metadata contains unexpected fields/)
    end

    it 'requires byte-for-byte exact JSON serialization parity and rejects noncanonical key ordering' do
      rec = {'alpha' => 1, 'beta' => 2}
      inp = {
        'uri' => '/repositories/2/resources/10/tree/root',
        'record_type' => 'tree_root',
        'pui_parent_id' => '/repositories/2/resources/10',
        'record' => rec,
        'pui_auxiliary_json' => ASUtils.to_json(rec),
        'prepare_pui_auxiliary' => true,
      }
      # Noncanonical key ordering with same semantics is strictly rejected
      doc_noncanonical = valid_aux_doc_for(inp).merge('json' => '{"beta":2,"alpha":1}')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_noncanonical])
      )
      expect { builder.build_pui_auxiliary([inp]) }.to raise_error(/json serialization mismatch/)

      # Mismatched payload fails
      doc_mismatch = valid_aux_doc_for(inp).merge('json' => ASUtils.to_json({'alpha' => 1, 'beta' => 999}))
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_mismatch])
      )
      expect { builder.build_pui_auxiliary([inp]) }.to raise_error(/json serialization mismatch/)
    end

    it 'enforces recursive type-strict semantic JSON comparison and preserves exact Unicode and nested structures' do
      nested_record = {
        'title' => 'Muintir na hÉireann & Sánchez Papers / 記録 ☘️',
        'numbers' => [1, 2, 3],
        'flags' => {'active' => true, 'archived' => false},
        'ratio' => 3.14,
        'nested' => {
          'levels' => [{'id' => 10, 'empty' => nil}]
        }
      }
      inp = {
        'uri' => '/repositories/2/resources/10/tree/root',
        'record_type' => 'tree_root',
        'pui_parent_id' => '/repositories/2/resources/10',
        'record' => nested_record,
        'pui_auxiliary_json' => ASUtils.to_json(nested_record),
        'prepare_pui_auxiliary' => true,
      }
      doc = valid_aux_doc_for(inp)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      res = builder.build_pui_auxiliary([inp])
      expect(res.first['json']).to eq(ASUtils.to_json(nested_record))
      expect(ASUtils.json_parse(res.first['json'])).to eq(nested_record)

      # Type mismatch: boolean vs integer
      bad_type_record = {
        'title' => 'Muintir na hÉireann & Sánchez Papers / 記録 ☘️',
        'numbers' => [1, 2, 3],
        'flags' => {'active' => 1, 'archived' => false},
        'ratio' => 3.14,
        'nested' => {
          'levels' => [{'id' => 10, 'empty' => nil}]
        }
      }
      doc_type_err = valid_aux_doc_for(inp).merge('json' => ASUtils.to_json(bad_type_record))
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_type_err])
      )
      expect { builder.build_pui_auxiliary([inp]) }.to raise_error(/type mismatch|json serialization mismatch/)

      # Type mismatch: integer vs float
      bad_num_record = {
        'title' => 'Muintir na hÉireann & Sánchez Papers / 記録 ☘️',
        'numbers' => [1.0, 2, 3],
        'flags' => {'active' => true, 'archived' => false},
        'ratio' => 3.14,
        'nested' => {
          'levels' => [{'id' => 10, 'empty' => nil}]
        }
      }
      doc_num_err = valid_aux_doc_for(inp).merge('json' => ASUtils.to_json(bad_num_record))
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc_num_err])
      )
      expect { builder.build_pui_auxiliary([inp]) }.to raise_error(/type mismatch|json serialization mismatch/)
    end

    it 'preserves multi-record ordering and duplicate input entries' do
      inp1 = aux_input
      inp2_rec = {'title' => 'Duplicate item'}
      inp2 = {
        'uri' => '/repositories/2/resources/10/tree/root',
        'record_type' => 'tree_root',
        'pui_parent_id' => '/repositories/2/resources/10',
        'record' => inp2_rec,
        'pui_auxiliary_json' => ASUtils.to_json(inp2_rec),
        'prepare_pui_auxiliary' => true,
      }
      inputs = [inp1, inp2, inp2] # duplicate inp2
      docs = [valid_aux_doc_for(inp1), valid_aux_doc_for(inp2), valid_aux_doc_for(inp2)]

      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => docs)
      )

      res = builder.build_pui_auxiliary(inputs)
      expect(res.length).to eq(3)
      expect(res[0]['uri']).to eq(inp1['uri'])
      expect(res[1]['uri']).to eq(inp2['uri'])
      expect(res[2]['uri']).to eq(inp2['uri'])
    end

    it 'rejects response document count mismatch' do
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [valid_aux_doc_for(aux_input), valid_aux_doc_for(aux_input)])
      )
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/document count mismatch/)
    end

    it 'raises error on sidecar failure (HTTP 500, network error)' do
      allow(builder).to receive(:perform_request).and_return(response('500', 'error' => 'Internal Server Error'))
      expect { builder.build_pui_auxiliary([aux_input]) }.to raise_error(/HTTP 500/)
    end

    it 'pre-validates inputs in Ruby before making HTTP request' do
      expect(builder).not_to receive(:perform_request)

      # Not a hash
      expect { builder.build_pui_auxiliary(['invalid']) }.to raise_error(/input must be a Hash/)

      # Missing prepare_pui_auxiliary
      expect { builder.build_pui_auxiliary([aux_input.merge('prepare_pui_auxiliary' => false)]) }.to raise_error(/prepare_pui_auxiliary must be explicitly true/)

      # Empty uri
      expect { builder.build_pui_auxiliary([aux_input.merge('uri' => '')]) }.to raise_error(/uri must be a non-empty string/)

      # Invalid record_type
      expect { builder.build_pui_auxiliary([aux_input.merge('record_type' => 'unknown')]) }.to raise_error(/invalid record_type/)

      # Empty pui_parent_id
      expect { builder.build_pui_auxiliary([aux_input.merge('pui_parent_id' => '  ')]) }.to raise_error(/pui_parent_id must be a non-empty string/)

      # Nil record
      expect { builder.build_pui_auxiliary([aux_input.merge('record' => nil)]) }.to raise_error(/record must not be nil/)

      # Missing pui_auxiliary_json
      expect { builder.build_pui_auxiliary([aux_input.merge('pui_auxiliary_json' => nil)]) }.to raise_error(/pui_auxiliary_json must be a non-empty string/)

      # Empty pui_auxiliary_json
      expect { builder.build_pui_auxiliary([aux_input.merge('pui_auxiliary_json' => '   ')]) }.to raise_error(/pui_auxiliary_json must be a non-empty string/)
    end
  end

  describe 'record inheritance candidate preparation' do
    let(:plan_input) do
      rec = {
        'uri' => '/repositories/2/archival_objects/42',
        'jsonmodel_type' => 'archival_object',
        'title' => 'AO 42',
        'publish' => true,
        'suppressed' => false,
        'ancestors' => []
      }
      plan = {
        'record_type' => 'archival_object',
        'rules' => [
          {
            'ancestor_element_indices' => nil,
            'ancestor_index' => 0,
            'property' => 'title',
            'inherit_directly' => true,
            'inherited_value' => 'AO 42',
            'inherited_props' => {
              'ref' => '/repositories/2/archival_objects/41',
              'level' => 'series',
              'direct' => true,
            },
          }
        ],
        'composite_identifiers' => nil,
      }
      {
        'uri' => '/repositories/2/archival_objects/42',
        'record_type' => 'archival_object',
        'repository' => 2,
        'record' => rec,
        'prepare_pui_final' => true,
        'record_inheritance_plan' => plan,
      }
    end

    it 'validates record_inheritance_plan input structure' do
      expect(builder).not_to receive(:perform_request)

      # Non-hash plan
      expect {
        builder.build([plan_input.merge('record_inheritance_plan' => 'invalid')])
      }.to raise_error(/record_inheritance_plan must be a Hash/)

      # Non-string keys in plan
      expect {
        bad_plan = plan_input['record_inheritance_plan'].dup
        bad_plan[:symbol_key] = 'value'
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/record_inheritance_plan contains non-string keys/)

      # Missing required fields in plan
      expect {
        bad_plan = plan_input['record_inheritance_plan'].dup
        bad_plan.delete('rules')
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/record_inheritance_plan missing required fields/)

      # Unexpected fields in plan
      expect {
        bad_plan = plan_input['record_inheritance_plan'].merge('extra_field' => 'bad')
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/record_inheritance_plan contains unexpected fields/)

      # Empty record_type in plan
      expect {
        bad_plan = plan_input['record_inheritance_plan'].merge('record_type' => '')
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/record_inheritance_plan\.record_type must be a non-empty string/)

      # Mismatched record_type in plan
      expect {
        bad_plan = plan_input['record_inheritance_plan'].merge('record_type' => 'resource')
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/record_inheritance_plan\.record_type 'resource' does not match input\.record_type 'archival_object'/)

      # Non-array rules
      expect {
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => 'invalid')
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/record_inheritance_plan\.rules must be an Array/)

      # Non-hash rule entry
      expect {
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => ['invalid'])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule in record_inheritance_plan\.rules must be a Hash/)

      # Non-string keys in rule
      expect {
        bad_rule = { :property => 'title', 'ancestor_index' => 0, 'ancestor_element_indices' => nil, 'inherit_directly' => true, 'inherited_value' => 'AO', 'inherited_props' => { 'ref' => 'r', 'level' => nil, 'direct' => true } }
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule in record_inheritance_plan\.rules contains non-string keys/)

      # Missing required fields in rule
      expect {
        bad_rule = { 'property' => 'title', 'inherit_directly' => true }
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule in record_inheritance_plan\.rules missing required fields/)

      # Unexpected fields in rule
      expect {
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('unexpected' => 1)
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule in record_inheritance_plan\.rules contains unexpected fields/)

      # Empty rule property
      expect {
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('property' => '')
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule\.property must be a non-empty string/)

      # Non-boolean inherit_directly
      expect {
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('inherit_directly' => 'yes')
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule\.inherit_directly must be a Boolean/)

      # Invalid ancestor_index
      expect {
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('ancestor_index' => 'not_an_int')
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule\.ancestor_index must be a non-negative Integer/)

      expect {
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('ancestor_index' => -1)
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule\.ancestor_index must be a non-negative Integer/)

      # Invalid ancestor_element_indices
      expect {
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('ancestor_element_indices' => 'not_an_array')
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule\.ancestor_element_indices must be nil or an Array of non-negative Integers/)

      expect {
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('ancestor_element_indices' => [-1])
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule\.ancestor_element_indices must be nil or an Array of non-negative Integers/)

      # Nil inherited_value
      expect {
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('inherited_value' => nil)
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule\.inherited_value must not be nil/)

      # Non-hash inherited_props
      expect {
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('inherited_props' => 'invalid')
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule\.inherited_props must be a Hash/)

      # Non-string keys in inherited_props
      expect {
        bad_props = { :ref => 'r', 'level' => nil, 'direct' => true }
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('inherited_props' => bad_props)
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule\.inherited_props contains non-string keys/)

      # Missing required fields in inherited_props
      expect {
        bad_props = { 'ref' => 'r', 'level' => nil }
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('inherited_props' => bad_props)
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule\.inherited_props missing required fields/)

      # Unexpected fields in inherited_props
      expect {
        bad_props = plan_input['record_inheritance_plan']['rules'][0]['inherited_props'].merge('unknown' => true)
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('inherited_props' => bad_props)
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule\.inherited_props contains unexpected fields/)

      # Non-boolean direct in inherited_props
      expect {
        bad_props = plan_input['record_inheritance_plan']['rules'][0]['inherited_props'].merge('direct' => 1)
        bad_rule = plan_input['record_inheritance_plan']['rules'][0].merge('inherited_props' => bad_props)
        bad_plan = plan_input['record_inheritance_plan'].merge('rules' => [bad_rule])
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/rule\.inherited_props\.direct must be a Boolean/)

      # Invalid composite_identifiers structure
      expect {
        bad_plan = plan_input['record_inheritance_plan'].merge('composite_identifiers' => 'invalid')
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/composite_identifiers must be a Hash/)

      expect {
        bad_comp = { :pieces => ['a'], 'delimiter' => '.' }
        bad_plan = plan_input['record_inheritance_plan'].merge('composite_identifiers' => bad_comp)
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/composite_identifiers contains non-string keys/)

      expect {
        bad_comp = { 'pieces' => ['a'] }
        bad_plan = plan_input['record_inheritance_plan'].merge('composite_identifiers' => bad_comp)
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/composite_identifiers missing required fields/)

      expect {
        bad_comp = { 'pieces' => ['a'], 'delimiter' => '.', 'extra' => 1 }
        bad_plan = plan_input['record_inheritance_plan'].merge('composite_identifiers' => bad_comp)
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/composite_identifiers contains unexpected fields/)

      expect {
        bad_comp = { 'pieces' => 'not_array', 'delimiter' => '.' }
        bad_plan = plan_input['record_inheritance_plan'].merge('composite_identifiers' => bad_comp)
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/composite_identifiers\.pieces must be an Array/)

      expect {
        bad_comp = { 'pieces' => [123], 'delimiter' => '.' }
        bad_plan = plan_input['record_inheritance_plan'].merge('composite_identifiers' => bad_comp)
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/composite_identifiers\.pieces (elements must be Strings|must contain only strings)/)

      expect {
        bad_comp = { 'pieces' => ['a'], 'delimiter' => 123 }
        bad_plan = plan_input['record_inheritance_plan'].merge('composite_identifiers' => bad_comp)
        builder.build([plan_input.merge('record_inheritance_plan' => bad_plan)])
      }.to raise_error(/composite_identifiers\.delimiter must be a (String|string)/)
    end

    def valid_cand_doc_for(input)
      cand_record = RecordInheritance.merge(ASUtils.json_parse(ASUtils.to_json(input['record'])))
      clean = builder.send(:reconstruct_expected_pui_final_json, cand_record)
      published_strings = builder.send(:extract_published_strings, clean)
      {
        'id' => input['uri'],
        'uri' => input['uri'],
        'title' => input['record']['title'],
        'primary_type' => input['record_type'],
        'types' => [input['record_type']],
        'json' => ASUtils.to_json(input['record']),
        'suppressed' => false,
        'publish' => true,
        'system_generated' => 'false',
        'repository' => input['repository'],
        '_archivalist_rust_fields' => [
          'pui_final_document_metadata',
          'pui_routing_metadata',
          'pui_notes_metadata',
          'pui_record_inheritance_candidate_metadata',
        ],
        '_archivalist_pui_final' => {
          'marker' => 'pui_final_document_metadata',
          'json' => ASUtils.to_json(clean),
          'fullrecord_published' => published_strings,
        },
        '_archivalist_pui_routing' => {
          'marker' => 'pui_routing_metadata',
          'id' => "#{input['uri']}#pui",
          'pui_parent_id' => input['uri'],
          'types' => ['pui', "pui_#{input['record_type']}", 'pui_record', 'pui_only'],
        },
        '_archivalist_pui_notes' => {
          'marker' => 'pui_notes_metadata',
          'notes_published' => nil,
        },
        '_archivalist_pui_candidate' => {
          'marker' => 'pui_record_inheritance_candidate_metadata',
          'record' => cand_record,
        },
      }
    end

    it 'validates _archivalist_pui_candidate response projection' do
      doc = valid_cand_doc_for(plan_input)
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )

      res = builder.build([plan_input])
      expect(res.length).to eq(1)
      expect(res[plan_input['uri']]['_archivalist_pui_candidate']).to be_a(Hash)
      expect(res[plan_input['uri']]['_archivalist_pui_candidate']['record']['title']).to eq('AO 42')
    end

    it 'rejects _archivalist_pui_candidate with non-string keys' do
      doc = valid_cand_doc_for(plan_input)
      doc['_archivalist_pui_candidate'][:symbol_key] = 'invalid'
      mock_body_cand_sym = '__MOCK_BODY_SYM_CAND__'
      allow(builder).to receive(:perform_request).and_return(Struct.new(:code, :body).new('200', mock_body_cand_sym))
      allow(ASUtils).to receive(:json_parse).and_wrap_original do |m, s, *args|
        s == mock_body_cand_sym ? { 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER, 'documents' => [doc] } : m.call(s, *args)
      end

      expect { builder.build([plan_input]) }.to raise_error(/pui candidate contains non-string keys/)
    end

    it 'rejects _archivalist_pui_candidate with missing required fields' do
      doc = valid_cand_doc_for(plan_input)
      doc['_archivalist_pui_candidate'].delete('record')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )

      expect { builder.build([plan_input]) }.to raise_error(/pui candidate missing required fields/)
    end

    it 'rejects _archivalist_pui_candidate with unexpected extra fields' do
      doc = valid_cand_doc_for(plan_input)
      doc['_archivalist_pui_candidate']['extra_field'] = 'bad'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )

      expect { builder.build([plan_input]) }.to raise_error(/pui candidate contains unexpected fields/)
    end

    it 'rejects _archivalist_pui_candidate with mismatched marker' do
      doc = valid_cand_doc_for(plan_input)
      doc['_archivalist_pui_candidate']['marker'] = 'wrong_marker'
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )

      expect { builder.build([plan_input]) }.to raise_error(/marker mismatch/)
    end

    it 'rejects _archivalist_pui_candidate with mismatched uri' do
      doc = valid_cand_doc_for(plan_input)
      doc['_archivalist_pui_candidate']['record'] = doc['_archivalist_pui_candidate']['record'].merge('uri' => '/repositories/2/archival_objects/999')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )

      expect { builder.build([plan_input]) }.to raise_error(/pui candidate record uri mismatch/)
    end

    it 'rejects _archivalist_pui_candidate with mismatched jsonmodel_type' do
      doc = valid_cand_doc_for(plan_input)
      doc['_archivalist_pui_candidate']['record'] = doc['_archivalist_pui_candidate']['record'].merge('jsonmodel_type' => 'resource')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )

      expect { builder.build([plan_input]) }.to raise_error(/pui candidate record jsonmodel_type mismatch/)
    end

    it 'rejects _archivalist_pui_candidate when candidate record fails parity validation' do
      doc = valid_cand_doc_for(plan_input)
      doc['_archivalist_pui_candidate']['record'] = doc['_archivalist_pui_candidate']['record'].merge('title' => 'Tampered Merged Title')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )

      expect { builder.build([plan_input]) }.to raise_error(/pui candidate record for/)
    end

    it 'rejects unexpected _archivalist_pui_candidate when record_inheritance_plan was not provided' do
      cand_input = plan_input.dup
      cand_input.delete('record_inheritance_plan')
      cand_input['pui_candidate_record'] = plan_input['record']

      doc = valid_cand_doc_for(plan_input)
      doc['_archivalist_rust_fields'] = [
        'pui_final_document_metadata',
        'pui_routing_metadata',
        'pui_notes_metadata',
      ]
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )

      expect { builder.build([cand_input]) }.to raise_error(/unexpected _archivalist_pui_candidate when record_inheritance_plan is absent/)
    end

    describe 'RecordInheritance error and coercion parity' do
      let(:custom_config) do
        {
          :archival_object => {
            :inherited_fields => [
              {
                :property => 'notes',
                :inherit_directly => true,
              },
              {
                :property => 'language',
                :inherit_directly => false,
              }
            ],
            :composite_identifiers => {
              :include_level => true,
              :identifier_delimiter => '/',
            }
          }
        }
      end

      it 'raises identical NoMethodError on missing ancestors between merge and build_plan' do
        rec_missing_ancestors = {
          'jsonmodel_type' => 'archival_object',
          'uri' => '/repositories/2/archival_objects/500',
          'notes' => []
        }
        merge_err = nil
        plan_err = nil
        begin
          RecordInheritance.new(custom_config).merge(rec_missing_ancestors)
        rescue => e
          merge_err = e
        end
        begin
          RecordInheritance.new(custom_config).build_plan(rec_missing_ancestors)
        rescue => e
          plan_err = e
        end
        expect(merge_err).to be_a(NoMethodError)
        expect(plan_err).to be_a(NoMethodError)
        expect(merge_err.class).to eq(plan_err.class)
      end

      it 'raises identical NoMethodError on malformed ancestor _resolved between merge and build_plan' do
        rec_malformed_resolved = {
          'jsonmodel_type' => 'archival_object',
          'uri' => '/repositories/2/archival_objects/501',
          'notes' => [],
          'ancestors' => [
            {
              'ref' => '/repositories/2/resources/1',
              'level' => 'collection',
              '_resolved' => nil
            }
          ]
        }
        merge_err = nil
        plan_err = nil
        begin
          RecordInheritance.new(custom_config).merge(rec_malformed_resolved)
        rescue => e
          merge_err = e
        end
        begin
          RecordInheritance.new(custom_config).build_plan(rec_malformed_resolved)
        rescue => e
          plan_err = e
        end
        expect(merge_err).to be_a(NoMethodError)
        expect(plan_err).to be_a(NoMethodError)
        expect(merge_err.class).to eq(plan_err.class)
      end

      it 'raises identical error on wrong-shaped filtered ancestor values between merge and build_plan' do
        cfg_with_bad_filter = {
          :archival_object => {
            :inherited_fields => [
              {
                :property => 'notes',
                :inherit_directly => true,
                :inherit_if => proc { |_v| 12345 } # integer does not respond to .empty?
              }
            ]
          }
        }
        rec_bad_filter = {
          'jsonmodel_type' => 'archival_object',
          'uri' => '/repositories/2/archival_objects/502',
          'notes' => [],
          'ancestors' => [
            {
              'ref' => '/repositories/2/resources/1',
              'level' => 'collection',
              '_resolved' => {
                'notes' => [{ 'jsonmodel_type' => 'note_singlepart', 'type' => 'abstract' }]
              }
            }
          ]
        }
        merge_err = nil
        plan_err = nil
        begin
          RecordInheritance.new(cfg_with_bad_filter).merge(rec_bad_filter)
        rescue => e
          merge_err = e
        end
        begin
          RecordInheritance.new(cfg_with_bad_filter).build_plan(rec_bad_filter)
        rescue => e
          plan_err = e
        end
        expect(merge_err).to be_a(NoMethodError)
        expect(plan_err).to be_a(NoMethodError)
        expect(merge_err.class).to eq(plan_err.class)
      end

      it 'preserves exact property key semantics when configured with symbols vs strings' do
        cfg_sym = {
          :archival_object => {
            :inherited_fields => [
              {
                :property => :notes,
                :inherit_directly => true,
              }
            ]
          }
        }
        rec_with_symbol_key = {
          'jsonmodel_type' => 'archival_object',
          'uri' => '/repositories/2/archival_objects/503',
          :notes => [],
          'ancestors' => [
            {
              'ref' => '/repositories/2/resources/1',
              'level' => 'collection',
              '_resolved' => {
                :notes => [{ 'type' => 'abstract', 'content' => ['Symbol key note'] }]
              }
            }
          ]
        }
        plan = RecordInheritance.new(cfg_sym).build_plan(rec_with_symbol_key)
        expect(plan['rules'][0]['property']).to eq('notes')
        expect(plan['rules'][0]['ancestor_index']).to eq(0)
        expect(plan['rules'][0]['ancestor_element_indices']).to eq([0])
      end

      it 'preserves exact translated level value and composite delimiter conversion' do
        custom_delimiter_obj = Class.new do
          def to_s
            ' :: '
          end
          def to_str
            ' :: '
          end
        end.new

        cfg_custom_delim = {
          :archival_object => {
            :inherited_fields => [],
            :composite_identifiers => {
              :include_level => false,
              :identifier_delimiter => custom_delimiter_obj
            }
          }
        }
        rec_delim = {
          'jsonmodel_type' => 'archival_object',
          'uri' => '/repositories/2/archival_objects/504',
          'component_id' => 'c504',
          'ancestors' => [
            {
              'ref' => '/repositories/2/resources/1',
              'level' => 'collection',
              '_resolved' => {
                'id_0' => 'COLL',
                'id_1' => '100'
              }
            }
          ]
        }
        merged = RecordInheritance.new(cfg_custom_delim).merge(rec_delim)
        plan = RecordInheritance.new(cfg_custom_delim).build_plan(rec_delim)
        expect(merged['_composite_identifier']).to eq('COLL :: 100 :: c504')
        expect(plan['composite_identifiers']['pieces']).to eq(['COLL :: 100', 'c504'])
        expect(plan['composite_identifiers']['delimiter']).to eq(' :: ')
      end
    end

    describe 'end-to-end candidate projection with native validation and unmutated source input' do
      it 'validates candidate with ancestor mutations against RecordInheritance.merge and leaves source unmutated' do
        ao_source = {
          'jsonmodel_type' => 'archival_object',
          'uri' => '/repositories/2/archival_objects/777',
          'title' => 'Child AO 777',
          'publish' => true,
          'suppressed' => false,
          'notes' => [],
          'linked_agents' => [],
          'ancestors' => [
            {
              'ref' => '/repositories/2/archival_objects/700',
              'level' => 'series',
              '_resolved' => {
                'notes' => [
                  {
                    'jsonmodel_type' => 'note_singlepart',
                    'type' => 'accessrestrict',
                    'publish' => true,
                    'content' => ['Direct series restriction']
                  }
                ],
                'linked_agents' => []
              }
            },
            {
              'ref' => '/repositories/2/resources/50',
              'level' => 'collection',
              '_resolved' => {
                'notes' => [
                  {
                    'jsonmodel_type' => 'note_singlepart',
                    'type' => 'scopecontent',
                    'publish' => true,
                    'content' => ['Collection scope note']
                  }
                ],
                'linked_agents' => [
                  {
                    'ref' => '/agents/people/99',
                    'role' => 'creator',
                    'terms' => []
                  }
                ]
              }
            }
          ]
        }

        # Snapshot source before anything runs
        source_snapshot = ASUtils.json_parse(ASUtils.to_json(ao_source))

        # 1. Build plan using native RecordInheritance
        plan = RecordInheritance.build_plan(ao_source)
        expect(plan).to be_a(Hash)
        expect(plan['rules'].length).to be >= 2

        # Verify source was not mutated by build_plan
        expect(ao_source).to eq(source_snapshot)

        # 2. Compute expected native candidate using RecordInheritance.merge on a clone
        # Note: RecordInheritance.merge shallow-clones the root and mutates ancestors in place!
        expected_native_candidate = RecordInheritance.merge(ASUtils.json_parse(ASUtils.to_json(ao_source)))

        # Assert expected native candidate has _inherited in BOTH destination AND inside ancestors
        expect(expected_native_candidate['notes'][0]['_inherited']).to be_a(Hash)
        expect(expected_native_candidate['ancestors'][0]['_resolved']['notes'][0]['_inherited']).to be_a(Hash)
        expect(expected_native_candidate['linked_agents'][0]['_inherited']).to be_a(Hash)
        expect(expected_native_candidate['ancestors'][1]['_resolved']['linked_agents'][0]['_inherited']).to be_a(Hash)

        # 3. Simulate the sidecar response returning this candidate
        cand_input = {
          'uri' => ao_source['uri'],
          'record_type' => 'archival_object',
          'repository' => 2,
          'record' => ao_source,
          'prepare_pui_final' => true,
          'record_inheritance_plan' => plan,
        }

        clean_expected = ASUtils.json_parse(ASUtils.to_json(expected_native_candidate))
        clean_expected.delete('ancestors')

        sidecar_doc = {
          'id' => ao_source['uri'],
          'uri' => ao_source['uri'],
          'publish' => true,
          'suppressed' => false,
          'title' => ao_source['title'],
          'primary_type' => 'archival_object',
          'types' => ['archival_object'],
          'repository' => 2,
          'system_generated' => 'false',
          'json' => ASUtils.to_json(ao_source),
          '_archivalist_rust_fields' => [
            'pui_final_document_metadata',
            'pui_routing_metadata',
            'pui_notes_metadata',
            'pui_record_inheritance_candidate_metadata',
          ],
          '_archivalist_pui_final' => {
            'marker' => 'pui_final_document_metadata',
            'json' => ASUtils.to_json(clean_expected),
            'fullrecord_published' => builder.send(:extract_published_strings, clean_expected),
          },
          '_archivalist_pui_routing' => {
            'marker' => 'pui_routing_metadata',
            'id' => "#{ao_source['uri']}#pui",
            'pui_parent_id' => ao_source['uri'],
            'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only'],
          },
          '_archivalist_pui_notes' => {
            'marker' => 'pui_notes_metadata',
            'notes_published' => builder.send(:extract_published_strings, ao_source['notes']),
          },
          '_archivalist_pui_candidate' => {
            'marker' => 'pui_record_inheritance_candidate_metadata',
            'record' => expected_native_candidate,
          },
        }

        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                          'documents' => [sidecar_doc])
        )

        # 4. Builder should accept candidate via strict equality check against RecordInheritance.merge(ao_source)
        res = builder.build([cand_input])
        expect(res.length).to eq(1)
        expect(res[cand_input['uri']]['_archivalist_pui_candidate']['record']).to eq(expected_native_candidate)

        # 5. Verify source input is STILL completely unmutated!
        expect(ao_source).to eq(source_snapshot)
        expect(ao_source['notes']).to eq([])
        expect(ao_source['linked_agents']).to eq([])
        expect(ao_source['ancestors'][0]['_resolved']['notes'][0]['_inherited']).to be_nil
        expect(ao_source['ancestors'][1]['_resolved']['linked_agents'][0]['_inherited']).to be_nil
      end

      it "correctly handles custom inherit_if returning deep-copied equal Hashes without mutating ancestor _resolved" do
        custom_config = {
          :archival_object => {
            :inherited_fields => [
              {
                :property => 'notes',
                :inherit_directly => true,
                :inherit_if => proc { |notes|
                  # Return newly allocated deep copies with equal values
                  notes.map { |n| RecordInheritance.new.deep_copy_record(n) }
                }
              }
            ]
          }
        }

        inheritance = RecordInheritance.new(custom_config)

        raw_note = {
          'jsonmodel_type' => 'note_singlepart',
          'type' => 'scopecontent',
          'publish' => true,
          'content' => ['Scope content from ancestor']
        }

        ao_source = {
          'uri' => '/repositories/2/archival_objects/999',
          'jsonmodel_type' => 'archival_object',
          'title' => 'AO Custom Inherit If Test',
          'publish' => true,
          'suppressed' => false,
          'notes' => [],
          'ancestors' => [
            {
              'ref' => '/repositories/2/resources/55',
              'level' => 'collection',
              '_resolved' => {
                'notes' => [raw_note]
              }
            }
          ]
        }

        source_snapshot = ASUtils.json_parse(ASUtils.to_json(ao_source))

        # 1. Plan generation
        plan = inheritance.build_plan(ao_source)
        expect(plan).to be_a(Hash)
        expect(plan['rules'].length).to eq(1)
        notes_rule = plan['rules'][0]
        expect(notes_rule['property']).to eq('notes')
        # Identity matching did NOT match raw_val items because elements were newly allocated deep copies!
        expect(notes_rule['ancestor_element_indices']).to eq([])

        # 2. Native merge
        expected_native_candidate = inheritance.merge(inheritance.deep_copy_record(ao_source))

        # Expected native candidate MUST have _inherited on destination copies:
        expect(expected_native_candidate['notes'][0]['_inherited']).to be_a(Hash)
        expect(expected_native_candidate['notes'][0]['_inherited']['ref']).to eq('/repositories/2/resources/55')

        # Expected native candidate MUST NOT have _inherited inside ancestor _resolved:
        expect(expected_native_candidate['ancestors'][0]['_resolved']['notes'][0]['_inherited']).to be_nil

        # 3. Simulate sidecar response with candidate matching expected native candidate
        clean_expected = ASUtils.json_parse(ASUtils.to_json(expected_native_candidate))
        clean_expected.delete('ancestors')

        sidecar_doc = {
          'id' => ao_source['uri'],
          'uri' => ao_source['uri'],
          'publish' => true,
          'suppressed' => false,
          'title' => ao_source['title'],
          'primary_type' => 'archival_object',
          'types' => ['archival_object'],
          'repository' => 2,
          'system_generated' => 'false',
          'json' => ASUtils.to_json(ao_source),
          '_archivalist_rust_fields' => [
            'pui_final_document_metadata',
            'pui_routing_metadata',
            'pui_notes_metadata',
            'pui_record_inheritance_candidate_metadata',
          ],
          '_archivalist_pui_final' => {
            'marker' => 'pui_final_document_metadata',
            'json' => ASUtils.to_json(clean_expected),
            'fullrecord_published' => builder.send(:extract_published_strings, clean_expected),
          },
          '_archivalist_pui_routing' => {
            'marker' => 'pui_routing_metadata',
            'id' => "#{ao_source['uri']}#pui",
            'pui_parent_id' => ao_source['uri'],
            'types' => ['pui', 'pui_archival_object', 'pui_record', 'pui_only'],
          },
          '_archivalist_pui_notes' => {
            'marker' => 'pui_notes_metadata',
            'notes_published' => builder.send(:extract_published_strings, ao_source['notes']),
          },
          '_archivalist_pui_candidate' => {
            'marker' => 'pui_record_inheritance_candidate_metadata',
            'record' => expected_native_candidate,
          },
        }

        cand_input = {
          'uri' => ao_source['uri'],
          'record_type' => 'archival_object',
          'repository' => 2,
          'record' => ao_source,
          'prepare_pui_final' => true,
          'record_inheritance_plan' => plan,
        }

        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                          'documents' => [sidecar_doc])
        )

        # In builder.build:
        # validate_json_equal! validates expected_merged against cand_rec.
        # Allow RecordInheritance.merge to use the custom config for this test:
        allow(RecordInheritance).to receive(:merge) do |rec|
          inheritance.merge(rec)
        end

        res = builder.build([cand_input])
        expect(res.length).to eq(1)
        cand_record = res[cand_input['uri']]['_archivalist_pui_candidate']['record']
        expect(cand_record).to eq(expected_native_candidate)
        expect(cand_record['notes'][0]['_inherited']).to be_a(Hash)
        expect(cand_record['ancestors'][0]['_resolved']['notes'][0]['_inherited']).to be_nil

        # Source unmutated
        expect(ao_source).to eq(source_snapshot)
      end
    end

    describe "prepare_pui_common_metadata" do
      let(:common_record) do
        {
          'uri' => '/repositories/2/resources/555',
          'jsonmodel_type' => 'resource',
          'title' => 'Common Builder Spec Resource',
          'publish' => true,
          'suppressed' => false,
          'level' => 'collection',
          'created_by' => 'archivist',
          'subjects' => [
            { 'ref' => '/subjects/1', '_resolved' => { 'title' => 'Local History' } }
          ],
          'linked_agents' => [
            {
              'ref' => '/agents/people/1',
              'role' => 'creator',
              '_resolved' => {
                'display_name' => { 'sort_name' => 'Doe, Jane' },
                'publish' => true
              }
            }
          ],
          'dates' => [
            { 'date_type' => 'single', 'begin' => '1920' }
          ],
          'notes' => [
            { 'type' => 'abstract', 'content' => ['Summary note'] }
          ],
          'extents' => [
            { 'portion' => 'whole', 'number' => '1', 'extent_type' => 'box' }
          ]
        }
      end

      let(:valid_common_proj) do
        {
          'marker' => 'pui_common_metadata',
          'uri' => '/repositories/2/resources/555',
          'primary_type' => 'resource',
          'subjects' => {
            'subjects' => ['Local History'],
            'subject_uris' => ['/subjects/1']
          },
          'agents' => {
            'agents' => ['Doe, Jane'],
            'agent_uris' => ['/agents/people/1'],
            'published_agents' => ['Doe, Jane'],
            'published_agent_uris' => ['/agents/people/1'],
            'creators' => ['Doe, Jane']
          },
          'audit_info' => {
            'created_by' => 'archivist'
          },
          'years' => {
            'years' => ['1920'],
            'year_sort' => '19201920',
            'dates' => ['1920']
          },
          'level' => {
            'level' => 'collection'
          },
          'summary' => {
            'summary' => 'Summary note'
          },
          'extents' => {
            'extents' => ['1 --- box']
          }
        }
      end

      it "validates and accepts trusted common metadata projection in common-only request" do
        doc = {
          'id' => '/repositories/2/resources/555',
          'uri' => '/repositories/2/resources/555',
          'publish' => true,
          'suppressed' => false,
          'primary_type' => 'resource',
          'title' => 'Common Builder Spec Resource',
          'types' => ['resource'],
          'repository' => 2,
          'system_generated' => 'false',
          'json' => ASUtils.to_json(common_record),
          '_archivalist_rust_fields' => ['pui_common_metadata'],
          '_archivalist_pui_common' => valid_common_proj,
        }

        inp = {
          'uri' => '/repositories/2/resources/555',
          'record_type' => 'resource',
          'repository' => 2,
          'record' => common_record,
          'prepare_pui_common_metadata' => true,
        }

        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                          'documents' => [doc])
        )

        res = builder.build([inp])
        expect(res.length).to eq(1)
        expect(res[inp['uri']]['_archivalist_pui_common']['marker']).to eq('pui_common_metadata')
        expect(res[inp['uri']]['_archivalist_rust_fields']).to eq(['pui_common_metadata'])
      end

      it "validates and accepts combined prepare_pui_final and prepare_pui_common_metadata request" do
        clean_rec = common_record.dup
        doc = {
          'id' => '/repositories/2/resources/555',
          'uri' => '/repositories/2/resources/555',
          'publish' => true,
          'suppressed' => false,
          'primary_type' => 'resource',
          'title' => 'Common Builder Spec Resource',
          'types' => ['resource'],
          'repository' => 2,
          'system_generated' => 'false',
          'json' => ASUtils.to_json(clean_rec),
          '_archivalist_rust_fields' => [
            'pui_final_document_metadata',
            'pui_routing_metadata',
            'pui_notes_metadata',
            'pui_common_metadata'
          ],
          '_archivalist_pui_final' => {
            'marker' => 'pui_final_document_metadata',
            'json' => ASUtils.to_json(clean_rec),
            'fullrecord_published' => IndexerCommon.extract_string_values(clean_rec, :published_only)
          },
          '_archivalist_pui_routing' => {
            'marker' => 'pui_routing_metadata',
            'id' => '/repositories/2/resources/555#pui',
            'pui_parent_id' => '/repositories/2/resources/555',
            'types' => ['pui', 'pui_resource', 'pui_record', 'pui_only']
          },
          '_archivalist_pui_notes' => {
            'marker' => 'pui_notes_metadata',
            'notes_published' => IndexerCommon.extract_string_values(clean_rec['notes'], :published_only)
          },
          '_archivalist_pui_common' => valid_common_proj,
        }

        inp = {
          'uri' => '/repositories/2/resources/555',
          'record_type' => 'resource',
          'repository' => 2,
          'record' => common_record,
          'pui_candidate_record' => common_record,
          'prepare_pui_final' => true,
          'prepare_pui_common_metadata' => true,
        }

        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                          'documents' => [doc])
        )

        res = builder.build([inp])
        expect(res.length).to eq(1)
        expect(res[inp['uri']]['_archivalist_pui_common']).to be_a(Hash)
        expect(res[inp['uri']]['_archivalist_pui_final']).to be_a(Hash)
      end

      it "rejects incompatible mode combinations with prepare_pui_common_metadata" do
        inp_enrich = {
          'uri' => '/repositories/2/resources/555',
          'record_type' => 'resource',
          'repository' => 2,
          'record' => common_record,
          'enrich' => true,
          'prepare_pui_common_metadata' => true,
        }
        expect { builder.build([inp_enrich]) }.to raise_error(/cannot combine enrich and prepare_pui_common_metadata/)

        inp_avail = {
          'uri' => '/repositories/2/resources/555',
          'record_type' => 'resource',
          'repository' => 2,
          'record' => common_record,
          'prepare_pui_availability' => true,
          'record_inheritance_has_type' => false,
          'prepare_pui_common_metadata' => true,
        }
        expect { builder.build([inp_avail]) }.to raise_error(/cannot combine prepare_pui_availability and prepare_pui_common_metadata/)

        inp_aux = {
          'uri' => '/repositories/2/resources/555/ordered_records',
          'record_type' => 'resource_ordered_records',
          'pui_parent_id' => '/repositories/2/resources/555',
          'record' => common_record,
          'pui_auxiliary_json' => '{}',
          'prepare_pui_auxiliary' => true,
          'prepare_pui_common_metadata' => true,
        }
        expect { builder.build([inp_aux]) }.to raise_error(/cannot combine prepare_pui_auxiliary and prepare_pui_common_metadata/)
      end

      it "rejects non-boolean prepare_pui_common_metadata capability flag" do
        inp = {
          'uri' => '/repositories/2/resources/555',
          'record_type' => 'resource',
          'repository' => 2,
          'record' => common_record,
          'prepare_pui_common_metadata' => 'true', # non-boolean
        }
        expect { builder.build([inp]) }.to raise_error(/prepare_pui_common_metadata must be explicitly boolean/)
      end

      it "rejects unexpected _archivalist_pui_common when capability is not requested" do
        clean_rec = common_record.dup
        doc = {
          'id' => '/repositories/2/resources/555',
          'uri' => '/repositories/2/resources/555',
          'publish' => true,
          'suppressed' => false,
          'primary_type' => 'resource',
          'title' => 'Common Builder Spec Resource',
          'types' => ['resource'],
          'repository' => 2,
          'system_generated' => 'false',
          'json' => ASUtils.to_json(clean_rec),
          '_archivalist_rust_fields' => [
            'pui_final_document_metadata',
            'pui_routing_metadata',
            'pui_notes_metadata',
          ],
          '_archivalist_pui_final' => {
            'marker' => 'pui_final_document_metadata',
            'json' => ASUtils.to_json(clean_rec),
            'fullrecord_published' => IndexerCommon.extract_string_values(clean_rec, :published_only)
          },
          '_archivalist_pui_routing' => {
            'marker' => 'pui_routing_metadata',
            'id' => '/repositories/2/resources/555#pui',
            'pui_parent_id' => '/repositories/2/resources/555',
            'types' => ['pui', 'pui_resource', 'pui_record', 'pui_only']
          },
          '_archivalist_pui_notes' => {
            'marker' => 'pui_notes_metadata',
            'notes_published' => IndexerCommon.extract_string_values(clean_rec['notes'], :published_only)
          },
          '_archivalist_pui_common' => valid_common_proj, # unexpected
        }

        inp = {
          'uri' => '/repositories/2/resources/555',
          'record_type' => 'resource',
          'repository' => 2,
          'record' => common_record,
          'pui_candidate_record' => common_record,
          'prepare_pui_final' => true,
          # prepare_pui_common_metadata NOT requested
        }

        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                          'documents' => [doc])
        )

        expect { builder.build([inp]) }.to raise_error(/unexpected response pui common projection marker/)
      end

      it "rejects missing required fields, unknown keys, or value mismatches in _archivalist_pui_common" do
        doc_bad_marker = {
          'id' => '/repositories/2/resources/555',
          'uri' => '/repositories/2/resources/555',
          'publish' => true,
          'suppressed' => false,
          'primary_type' => 'resource',
          'title' => 'Common Builder Spec Resource',
          'types' => ['resource'],
          'repository' => 2,
          'system_generated' => 'false',
          'json' => ASUtils.to_json(common_record),
          '_archivalist_rust_fields' => ['pui_common_metadata'],
          '_archivalist_pui_common' => valid_common_proj.merge('marker' => 'corrupt_marker'),
        }

        inp = {
          'uri' => '/repositories/2/resources/555',
          'record_type' => 'resource',
          'repository' => 2,
          'record' => common_record,
          'prepare_pui_common_metadata' => true,
        }

        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                          'documents' => [doc_bad_marker])
        )
        expect { builder.build([inp]) }.to raise_error(/pui common marker mismatch/)

        # Unknown extra key
        doc_extra_key = {
          'id' => '/repositories/2/resources/555',
          'uri' => '/repositories/2/resources/555',
          'publish' => true,
          'suppressed' => false,
          'primary_type' => 'resource',
          'title' => 'Common Builder Spec Resource',
          'types' => ['resource'],
          'repository' => 2,
          'system_generated' => 'false',
          'json' => ASUtils.to_json(common_record),
          '_archivalist_rust_fields' => ['pui_common_metadata'],
          '_archivalist_pui_common' => valid_common_proj.merge('evil_extra' => true),
        }
        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                          'documents' => [doc_extra_key])
        )
        expect { builder.build([inp]) }.to raise_error(/pui common contains unexpected fields/)
      end

      it "validates exact allowed top-level document key sets and rejects injected notes, fullrecord, extra fields, and missing base fields" do
        base_valid_doc = {
          'id' => '/repositories/2/resources/555',
          'uri' => '/repositories/2/resources/555',
          'publish' => true,
          'suppressed' => false,
          'primary_type' => 'resource',
          'title' => 'Common Builder Spec Resource',
          'types' => ['resource'],
          'repository' => 2,
          'system_generated' => 'false',
          'json' => ASUtils.to_json(common_record),
          '_archivalist_rust_fields' => ['pui_common_metadata'],
          '_archivalist_pui_common' => valid_common_proj,
        }

        inp = {
          'uri' => '/repositories/2/resources/555',
          'record_type' => 'resource',
          'repository' => 2,
          'record' => common_record,
          'prepare_pui_common_metadata' => true,
        }

        # 1. Fault injection: injected top-level notes
        doc_with_notes = base_valid_doc.merge('notes' => ['injected note'])
        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER, 'documents' => [doc_with_notes])
        )
        expect { builder.build([inp]) }.to raise_error(/document contains unexpected fields.*notes/)

        # 2. Fault injection: injected top-level fullrecord
        doc_with_fullrecord = base_valid_doc.merge('fullrecord' => ['injected fullrecord'])
        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER, 'documents' => [doc_with_fullrecord])
        )
        expect { builder.build([inp]) }.to raise_error(/document contains unexpected fields.*fullrecord/)

        # 3. Fault injection: injected arbitrary public field
        doc_with_arbitrary = base_valid_doc.merge('arbitrary_public_field' => 'should_fail')
        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER, 'documents' => [doc_with_arbitrary])
        )
        expect { builder.build([inp]) }.to raise_error(/document contains unexpected fields.*arbitrary_public_field/)

        # 4. Fault injection: injected private marker
        doc_with_marker = base_valid_doc.merge('_archivalist_private_data' => { 'leak' => true })
        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER, 'documents' => [doc_with_marker])
        )
        expect { builder.build([inp]) }.to raise_error(/document contains unexpected fields.*_archivalist_private_data/)

        # 5. Fault injection: missing base envelope field (e.g. types)
        doc_missing_base = base_valid_doc.dup
        doc_missing_base.delete('types')
        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER, 'documents' => [doc_missing_base])
        )
        expect { builder.build([inp]) }.to raise_error(/(document missing required fields.*types|response types mismatch)/)

        # 6. Fault injection: common-only request containing unrequested pui_final projection
        doc_unrequested_final = base_valid_doc.merge(
          '_archivalist_pui_final' => { 'marker' => 'pui_final_document_metadata' }
        )
        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER, 'documents' => [doc_unrequested_final])
        )
        expect { builder.build([inp]) }.to raise_error(/document contains unexpected fields.*_archivalist_pui_final/)

        # 7. Combined request missing required pui_final
        combined_inp = {
          'uri' => '/repositories/2/resources/555',
          'record_type' => 'resource',
          'repository' => 2,
          'record' => common_record,
          'pui_candidate_record' => common_record,
          'prepare_pui_final' => true,
          'prepare_pui_common_metadata' => true,
        }
        # In a combined request, base_valid_doc lacks pui_final/routing/notes
        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER, 'documents' => [base_valid_doc])
        )
        expect { builder.build([combined_inp]) }.to raise_error(/(document missing required fields.*_archivalist_pui_final|response pui final marker mismatch|missing _archivalist_pui_final)/)

        # 8. Combined request without inheritance plan containing unrequested pui_candidate
        valid_combined_doc = {
          'id' => '/repositories/2/resources/555',
          'uri' => '/repositories/2/resources/555',
          'publish' => true,
          'suppressed' => false,
          'primary_type' => 'resource',
          'title' => 'Common Builder Spec Resource',
          'types' => ['resource'],
          'repository' => 2,
          'system_generated' => 'false',
          'json' => ASUtils.to_json(common_record),
          '_archivalist_rust_fields' => [
            'pui_final_document_metadata',
            'pui_routing_metadata',
            'pui_notes_metadata',
            'pui_common_metadata'
          ],
          '_archivalist_pui_final' => {
            'marker' => 'pui_final_document_metadata',
            'json' => ASUtils.to_json(common_record),
            'fullrecord_published' => IndexerCommon.extract_string_values(common_record, :published_only)
          },
          '_archivalist_pui_routing' => {
            'marker' => 'pui_routing_metadata',
            'id' => '/repositories/2/resources/555#pui',
            'pui_parent_id' => '/repositories/2/resources/555',
            'types' => ['pui', 'pui_resource', 'pui_record', 'pui_only']
          },
          '_archivalist_pui_notes' => {
            'marker' => 'pui_notes_metadata',
            'notes_published' => IndexerCommon.extract_string_values(common_record['notes'], :published_only)
          },
          '_archivalist_pui_common' => valid_common_proj,
          '_archivalist_pui_candidate' => { 'marker' => 'pui_record_inheritance_candidate_metadata' } # unrequested!
        }
        allow(builder).to receive(:perform_request).and_return(
          response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER, 'documents' => [valid_combined_doc])
        )
        expect { builder.build([combined_inp]) }.to raise_error(/(document contains unexpected fields.*_archivalist_pui_candidate|unexpected _archivalist_pui_candidate when record_inheritance_plan is absent)/)
      end

      it "strictly validates recursive JSON types including integer vs float, boolean vs int, ordering and duplicate keys" do
        # 1 vs 1.0 (Integer vs Float)
        expect {
          RustIndexDocumentBuilder.validate_json_equal!({ 'count' => 1 }, { 'count' => 1.0 })
        }.to raise_error(/type mismatch: expected (Integer|Fixnum), got Float/)

        # false vs 0 (Boolean vs Integer)
        expect {
          RustIndexDocumentBuilder.validate_json_equal!({ 'flag' => false }, { 'flag' => 0 })
        }.to raise_error(/type mismatch: expected Boolean, got (Integer|Fixnum)/)

        # Array ordering mismatch
        expect {
          RustIndexDocumentBuilder.validate_json_equal!({ 'arr' => ['a', 'b'] }, { 'arr' => ['b', 'a'] })
        }.to raise_error(/value mismatch/)

        # Array duplicate mismatch
        expect {
          RustIndexDocumentBuilder.validate_json_equal!({ 'arr' => ['a', 'a'] }, { 'arr' => ['a'] })
        }.to raise_error(/array length mismatch/)

        # Missing recursive key
        expect {
          RustIndexDocumentBuilder.validate_json_equal!({ 'nested' => { 'k1' => 'v1', 'k2' => 'v2' } }, { 'nested' => { 'k1' => 'v1' } })
        }.to raise_error(/keys mismatch.*missing: \["k2"\]/)

        # Extra recursive key
        expect {
          RustIndexDocumentBuilder.validate_json_equal!({ 'nested' => { 'k1' => 'v1' } }, { 'nested' => { 'k1' => 'v1', 'k_extra' => 'v2' } })
        }.to raise_error(/keys mismatch.*extra: \["k_extra"\]/)
      end
    end
  end
end
