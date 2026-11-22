#!/usr/bin/env ruby
# frozen_string_literal: true

# Offline tests for archivalist_profiling_stack.rb (injected HTTP responses only).
# Run with MRI or JRuby: ruby rust-tools/scripts/test_archivalist_profiling_stack.rb

require 'minitest/autorun'
require_relative 'archivalist_profiling_stack'

class TestArchivalistProfilingStack < Minitest::Test
  ENV_OK = {
    'ARCHIVALIST_PROFILING_STACK_ID' => 'abcd1234',
    'ARCHIVALIST_PROFILING_STACK_NONCE' => 'n0nce',
    'ARCHIVALIST_PROFILING_STACK_IDENTITY_REPO_ID' => '7',
    'ARCHIVALIST_PROFILING_STACK_IDENTITY_CORE' => 'archivalist_profiling_identity_abcd1234'
  }.freeze
  BACKEND = 'http://archivalist-profiling-backend:4567'
  SOLR = 'http://archivalist-profiling-solr:8983/solr'

  def fake(repo: { 'name' => 'archivalist profiling identity n0nce', 'repo_code' => 'ARCHPROF-abcd1234' },
           core_status: { 'name' => 'archivalist_profiling_identity_abcd1234' })
    calls = []
    getter = lambda do |url|
      calls << url
      if url.include?('/repositories/')
        raise 'HTTP 404' if repo.nil?

        repo
      else
        { 'status' => { 'archivalist_profiling_identity_abcd1234' => core_status } }
      end
    end
    [getter, calls]
  end

  def test_accepts_matching_backend_and_solr
    getter, calls = fake
    id = ArchivalistProfilingStack.verify!(env: ENV_OK, backend_url: BACKEND, solr_url: SOLR, get_json: getter)
    assert_equal 'abcd1234', id['stack_id']
    assert_equal ["#{BACKEND}/repositories/7", "#{SOLR}/admin/cores?action=STATUS&core=archivalist_profiling_identity_abcd1234&wt=json"], calls
  end

  def test_rejects_missing_identity_without_any_request
    getter, calls = fake
    err = assert_raises(RuntimeError) { ArchivalistProfilingStack.verify!(env: {}, backend_url: BACKEND, solr_url: SOLR, get_json: getter) }
    assert_match(/identity missing/, err.message)
    assert_empty calls
  end

  def test_rejects_stale_or_foreign_nonce_and_wrong_repo_code
    getter, = fake(repo: { 'name' => 'archivalist profiling identity OTHER', 'repo_code' => 'ARCHPROF-abcd1234' })
    assert_raises(RuntimeError) { ArchivalistProfilingStack.verify!(env: ENV_OK, backend_url: BACKEND, solr_url: SOLR, get_json: getter) }
    getter, = fake(repo: { 'name' => 'archivalist profiling identity n0nce', 'repo_code' => 'test_repo' })
    err = assert_raises(RuntimeError) { ArchivalistProfilingStack.verify!(env: ENV_OK, backend_url: BACKEND, solr_url: SOLR, get_json: getter) }
    assert_match(/NOT connected/, err.message)
  end

  def test_rejects_backend_without_identity_record
    getter, = fake(repo: nil)
    err = assert_raises(RuntimeError) { ArchivalistProfilingStack.verify!(env: ENV_OK, backend_url: BACKEND, solr_url: SOLR, get_json: getter) }
    assert_match(/did not return the identity record/, err.message)
  end

  def test_rejects_right_backend_with_wrong_solr
    getter, = fake(core_status: {})
    err = assert_raises(RuntimeError) { ArchivalistProfilingStack.verify!(env: ENV_OK, backend_url: BACKEND, solr_url: SOLR, get_json: getter) }
    assert_match(/NOT the profiling stack's Solr/, err.message)
  end

  def test_rejects_malformed_identity_values
    getter, calls = fake
    bad_core = ENV_OK.merge('ARCHIVALIST_PROFILING_STACK_IDENTITY_CORE' => 'archivesspace')
    assert_raises(RuntimeError) { ArchivalistProfilingStack.verify!(env: bad_core, backend_url: BACKEND, solr_url: SOLR, get_json: getter) }
    bad_id = ENV_OK.merge('ARCHIVALIST_PROFILING_STACK_IDENTITY_REPO_ID' => '7; DROP')
    assert_raises(RuntimeError) { ArchivalistProfilingStack.verify!(env: bad_id, backend_url: BACKEND, solr_url: SOLR, get_json: getter) }
    assert_empty calls
  end
end
