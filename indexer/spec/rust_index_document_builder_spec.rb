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
end
