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
      'json' => ASUtils.to_json(full_cm),
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
      doc = valid_doc_for(base_resource_input) # doc still has ark_name
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
      doc['status_enum_s'] = [1.0] # float instead of integer
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
      doc['status_enum_s'] = [1] # integer instead of boolean
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
      expect { builder.build([base_accession_input]) }.to raise_error(/response enrichment fields missing.*accession_date/)
    end

    it 'rejects missing accession_date_year field in response' do
      doc = valid_accession_doc_for(base_accession_input)
      doc.delete('accession_date_year')
      allow(builder).to receive(:perform_request).and_return(
        response('200', 'served_by' => RustIndexDocumentBuilder::EXPECTED_SERVER,
                        'documents' => [doc])
      )
      expect { builder.build([base_accession_input]) }.to raise_error(/response enrichment fields missing.*accession_date_year/)
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
end
