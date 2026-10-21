#!/usr/bin/env ruby
# frozen_string_literal: true

# Safety and invariant verification for integration_stage_harness.rb
# Tests that safety guards fail closed with zero mutations when:
# 1. Target core already exists in Solr.
# 2. Solr admin STATUS is unreachable or returns malformed response.
# 3. Protected or unapproved core names are supplied.
# 4. Solr URL has malicious host, scheme, credentials, query, or path tricks.
# 5. Process stop is attempted with unverified, missing, or mismatched PID.
# 6. Native equivalence comparator catches all negative perturbations (values, types, keys, order).

require 'minitest/autorun'
require 'net/http'
require 'uri'
require 'json'
require_relative 'integration_stage_harness'

class TestIntegrationStageHarnessSafety < Minitest::Test
  def test_url_validation_accepts_valid_loopback_and_container
    assert ArchivalistHarnessSafety.valid_solr_url?('http://127.0.0.1:8983/solr')
    assert ArchivalistHarnessSafety.valid_solr_url?('http://localhost:8985/solr')
    assert ArchivalistHarnessSafety.valid_solr_url?('http://solr:8983/solr')
    assert ArchivalistHarnessSafety.valid_solr_url?('http://solr-test:8984/solr/')
  end

  def test_url_validation_rejects_external_hosts
    refute ArchivalistHarnessSafety.valid_solr_url?('http://evil.com:8983/solr')
    refute ArchivalistHarnessSafety.valid_solr_url?('http://192.168.1.50:8983/solr')
  end

  def test_url_validation_rejects_credentials_and_query_and_paths
    refute ArchivalistHarnessSafety.valid_solr_url?('http://user:pass@127.0.0.1:8983/solr')
    refute ArchivalistHarnessSafety.valid_solr_url?('http://127.0.0.1:8983/solr?evil=1')
    refute ArchivalistHarnessSafety.valid_solr_url?('http://127.0.0.1:8983/solr/../other')
    refute ArchivalistHarnessSafety.valid_solr_url?('ftp://127.0.0.1:8983/solr')
  end

  def test_core_name_validation_accepts_stage_owned_unique_names
    run_id = ArchivalistHarnessSafety.generate_run_id
    valid_name = ArchivalistHarnessSafety.stage_core_name(run_id, 'native')
    assert ArchivalistHarnessSafety.valid_stage_core_name?(valid_name, run_id)
  end

  def test_core_name_validation_rejects_protected_and_unowned_names
    run_id = ArchivalistHarnessSafety.generate_run_id
    refute ArchivalistHarnessSafety.valid_stage_core_name?('archivesspace', run_id)
    refute ArchivalistHarnessSafety.valid_stage_core_name?('archivesspace_shadow', run_id)
    refute ArchivalistHarnessSafety.valid_stage_core_name?('stage_pui_native', run_id) # legacy hardcoded name
    refute ArchivalistHarnessSafety.valid_stage_core_name?("stage_pui_other_run_native", run_id)
    refute ArchivalistHarnessSafety.valid_stage_core_name?("stage_pui_#{run_id}_production", run_id)
  end

  def test_ensure_core_fails_closed_when_core_already_exists
    run_id = ArchivalistHarnessSafety.generate_run_id
    core_name = ArchivalistHarnessSafety.stage_core_name(run_id, 'candidate')

    mock_status = { 'responseHeader' => { 'status' => 0 }, 'status' => { core_name => {} } }
    mutation_called = false

    call_admin = lambda do |action, params = {}|
      if action == 'STATUS'
        mock_status
      else
        mutation_called = true
        raise "Mutation called unexpectedly: #{action}"
      end
    end

    err = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.ensure_stage_core(core_name, run_id, admin_caller: call_admin)
    end

    assert_match(/already exists/i, err.message)
    refute mutation_called, "No mutation (UNLOAD or CREATE) must occur when core already exists"
  end

  def test_ensure_core_fails_closed_when_admin_status_fails
    run_id = ArchivalistHarnessSafety.generate_run_id
    core_name = ArchivalistHarnessSafety.stage_core_name(run_id, 'candidate')
    mutation_called = false

    call_admin = lambda do |action, params = {}|
      if action == 'STATUS'
        nil # Network error or unreachable
      else
        mutation_called = true
        raise "Mutation called unexpectedly: #{action}"
      end
    end

    err = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.ensure_stage_core(core_name, run_id, admin_caller: call_admin)
    end

    assert_match(/failed to read solr admin status/i, err.message)
    refute mutation_called, "No mutation must occur when admin status is unreachable"
  end

  def test_ensure_core_fails_closed_when_admin_status_is_malformed
    run_id = ArchivalistHarnessSafety.generate_run_id
    core_name = ArchivalistHarnessSafety.stage_core_name(run_id, 'candidate')
    mutation_called = false

    call_admin = lambda do |action, params = {}|
      if action == 'STATUS'
        { 'responseHeader' => { 'status' => 1 } } # Solr error response
      else
        mutation_called = true
        raise "Mutation called unexpectedly: #{action}"
      end
    end

    err = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.ensure_stage_core(core_name, run_id, admin_caller: call_admin)
    end

    assert_match(/failed to read solr admin status/i, err.message)
    refute mutation_called, "No mutation must occur when admin status is malformed"
  end

  def test_ensure_core_fails_closed_when_status_missing_status_hash
    run_id = ArchivalistHarnessSafety.generate_run_id
    core_name = ArchivalistHarnessSafety.stage_core_name(run_id, 'candidate')
    mutation_called = false

    call_admin = lambda do |action, params = {}|
      if action == 'STATUS'
        { 'responseHeader' => { 'status' => 0 } } # Missing 'status' Hash
      else
        mutation_called = true
        raise "Mutation called unexpectedly: #{action}"
      end
    end

    err = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.ensure_stage_core(core_name, run_id, admin_caller: call_admin)
    end

    assert_match(/failed to read solr admin status/i, err.message)
    refute mutation_called, "Zero writes must occur when STATUS response lacks status Hash"
  end

  def test_ensure_core_fails_closed_on_protected_name
    run_id = ArchivalistHarnessSafety.generate_run_id
    mutation_called = false

    call_admin = lambda do |action, params = {}|
      mutation_called = true
      raise "Mutation called unexpectedly: #{action}"
    end

    ['archivesspace', 'archivesspace_shadow', 'production'].each do |protected_name|
      assert_raises(RuntimeError) do
        ArchivalistHarnessSafety.ensure_stage_core(protected_name, run_id, admin_caller: call_admin)
      end
    end
    refute mutation_called, "No mutation must occur for protected core names"
  end

  def test_native_equivalence_passes_on_identical_docs
    docs1 = [
      { 'id' => '/repositories/999/resources/1#pui', 'title' => 'Test', 'types' => ['pui', 'resource'], '_version_' => 123 },
      { 'id' => '/repositories/999/archival_objects/2#pui', 'title' => 'AO', 'types' => ['pui', 'ao'], '_version_' => 124 }
    ]
    docs2 = [
      { 'id' => '/repositories/999/resources/1#pui', 'title' => 'Test', 'types' => ['pui', 'resource'], '_version_' => 999 },
      { 'id' => '/repositories/999/archival_objects/2#pui', 'title' => 'AO', 'types' => ['pui', 'ao'], '_version_' => 1000 }
    ]
    assert ArchivalistHarnessSafety.verify_native_equivalence(docs1, docs2)
  end

  def test_native_equivalence_fails_on_negative_perturbations
    base1 = { 'id' => '1', 'title' => 'T', 'count' => 5, 'types' => ['a', 'b'] }
    base2 = { 'id' => '1', 'title' => 'T', 'count' => 5, 'types' => ['a', 'b'] }

    # Perturbation 1: Value mismatch
    p_val = { 'id' => '1', 'title' => 'Different', 'count' => 5, 'types' => ['a', 'b'] }
    err1 = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_native_equivalence([base1], [p_val]) }
    assert_match(/Value mismatch.*title/i, err1.message)

    # Perturbation 2: Type mismatch (Integer vs String)
    p_type = { 'id' => '1', 'title' => 'T', 'count' => '5', 'types' => ['a', 'b'] }
    err2 = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_native_equivalence([base1], [p_type]) }
    assert_match(/Type mismatch.*count/i, err2.message)

    # Perturbation 3: Missing field
    p_missing = { 'id' => '1', 'title' => 'T', 'types' => ['a', 'b'] }
    err3 = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_native_equivalence([base1], [p_missing]) }
    assert_match(/Field 'count' missing/i, err3.message)

    # Perturbation 4: Array order mismatch
    p_order = { 'id' => '1', 'title' => 'T', 'count' => 5, 'types' => ['b', 'a'] }
    err4 = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_native_equivalence([base1], [p_order]) }
    assert_match(/Value mismatch.*types/i, err4.message)

    # Perturbation 5: Extra leaked field
    p_extra = { 'id' => '1', 'title' => 'T', 'count' => 5, 'types' => ['a', 'b'], '_archivalist_marker' => true }
    err5 = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_native_equivalence([base1], [p_extra]) }
    assert_match(/Field '_archivalist_marker' missing in native/i, err5.message)
  end

  def test_native_equivalence_strictly_checks_nested_types
    # Nested Array Integer vs Float ([1] vs [1.0])
    doc_int = { 'id' => '1', 'nested' => [1] }
    doc_float = { 'id' => '1', 'nested' => [1.0] }
    err1 = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.verify_native_equivalence([doc_int], [doc_float])
    end
    assert_match(/Type mismatch at doc\[1\]\.nested\[0\]/i, err1.message)

    # Nested Hash Boolean vs Integer (false vs 0)
    doc_bool = { 'id' => '1', 'flags' => { 'active' => false } }
    doc_num = { 'id' => '1', 'flags' => { 'active' => 0 } }
    err2 = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.verify_native_equivalence([doc_bool], [doc_num])
    end
    assert_match(/Type mismatch at doc\[1\]\.flags\.active/i, err2.message)
  end

  def test_native_equivalence_rejects_duplicate_or_invalid_ids
    # Duplicate ID in native_docs
    dup_native = [{ 'id' => 'dup1', 'val' => 'a' }, { 'id' => 'dup1', 'val' => 'b' }]
    valid_fb = [{ 'id' => 'dup1', 'val' => 'a' }, { 'id' => 'other', 'val' => 'b' }]
    err1 = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.verify_native_equivalence(dup_native, valid_fb)
    end
    assert_match(/Duplicate IDs detected in native_docs/i, err1.message)

    # Duplicate ID in fallback_docs
    valid_nat = [{ 'id' => 'dup1', 'val' => 'a' }, { 'id' => 'other', 'val' => 'b' }]
    dup_fallback = [{ 'id' => 'dup1', 'val' => 'a' }, { 'id' => 'dup1', 'val' => 'b' }]
    err2 = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.verify_native_equivalence(valid_nat, dup_fallback)
    end
    assert_match(/Duplicate IDs detected in fallback_docs/i, err2.message)

    # Empty string ID
    empty_id = [{ 'id' => '', 'val' => 'a' }]
    err3 = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.verify_native_equivalence(empty_id, empty_id)
    end
    assert_match(/invalid or empty id/i, err3.message)

    # Non-string ID
    int_id = [{ 'id' => 12345, 'val' => 'a' }]
    err4 = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.verify_native_equivalence(int_id, int_id)
    end
    assert_match(/invalid or empty id/i, err4.message)

    # Missing ID in fallback (set mismatch)
    nat_set = [{ 'id' => 'doc1', 'val' => 'a' }, { 'id' => 'doc2', 'val' => 'b' }]
    fb_set = [{ 'id' => 'doc1', 'val' => 'a' }, { 'id' => 'doc3', 'val' => 'b' }]
    err5 = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.verify_native_equivalence(nat_set, fb_set)
    end
    assert_match(/Identity set mismatch/i, err5.message)
  end

  def test_safe_process_stop_refuses_missing_or_mismatched_pid
    # 1. Missing PID file
    err = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.stop_stage_process('/tmp/nonexistent-pid-file.json')
    end
    assert_match(/pid state file missing/i, err.message)

    # 2. Fake pid file pointing to non-sidecar process (e.g. init PID 1)
    fake_state_file = "/tmp/test-fake-pid-#{Process.pid}.json"
    File.write(fake_state_file, JSON.generate({
      'pid' => 1,
      'command' => 'aspace-shadow-sidecar',
      'binary' => '/archivesspace/rust-tools/target/debug/aspace-shadow-sidecar',
      'port' => 4599,
      'run_id' => 'test'
    }))

    err = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.stop_stage_process(fake_state_file)
    end
    assert_match(/ownership verification failed/i, err.message)
    # File is permanently preserved per anti-deletion policy
  end

  def test_safe_process_stop_refuses_stale_or_substituted_or_empty_commands
    # Stale already-stopped state
    stale_file = "/tmp/test-stale-pid-#{Process.pid}.json"
    File.write(stale_file, JSON.generate({
      'pid' => Process.pid,
      'command' => 'aspace-shadow-sidecar',
      'binary' => '/archivesspace/rust-tools/target/debug/aspace-shadow-sidecar',
      'port' => 4599,
      'run_id' => 'test',
      'status' => 'stopped'
    }))

    err_stale = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.stop_stage_process(stale_file)
    end
    assert_match(/ownership verification failed/i, err_stale.message)

    # Empty command
    empty_cmd_file = "/tmp/test-empty-cmd-#{Process.pid}.json"
    File.write(empty_cmd_file, JSON.generate({
      'pid' => Process.pid,
      'command' => '',
      'binary' => '/archivesspace/rust-tools/target/debug/aspace-shadow-sidecar',
      'port' => 4599,
      'run_id' => 'test'
    }))

    err_empty = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.stop_stage_process(empty_cmd_file)
    end
    assert_match(/ownership verification failed/i, err_empty.message)

    # Substituted command (e.g. bash or sh)
    sub_cmd_file = "/tmp/test-sub-cmd-#{Process.pid}.json"
    File.write(sub_cmd_file, JSON.generate({
      'pid' => Process.pid,
      'command' => 'bash',
      'binary' => '/bin/bash',
      'port' => 4599,
      'run_id' => 'test'
    }))

    err_sub = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.stop_stage_process(sub_cmd_file)
    end
    assert_match(/ownership verification failed/i, err_sub.message)
  end

  def test_port_in_use_detects_listening_and_closed_ports
    server = TCPServer.new('127.0.0.1', 0)
    bound_port = server.addr[1]
    assert ArchivalistHarnessSafety.port_in_use?(bound_port), "Bound port #{bound_port} must be detected as in use"
    server.close
    refute ArchivalistHarnessSafety.port_in_use?(bound_port), "Closed port #{bound_port} must be detected as free"
  end

  def test_find_unused_port_selects_free_port_and_fails_closed_when_exhausted
    port = ArchivalistHarnessSafety.find_unused_port(start_port: 4600, end_port: 4610)
    assert_includes 4600..4610, port
    refute ArchivalistHarnessSafety.port_in_use?(port)

    # Empty range raises safety violation
    err = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.find_unused_port(start_port: 4600, end_port: 4599)
    end
    assert_match(/No unused port available/i, err.message)
  end
end
