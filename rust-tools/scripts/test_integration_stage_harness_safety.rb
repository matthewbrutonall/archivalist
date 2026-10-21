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

  def test_stage_core_name_supports_staff_and_pui
    run_id = ArchivalistHarnessSafety.generate_run_id
    assert_equal "stage_pui_#{run_id}_native", ArchivalistHarnessSafety.stage_core_name(run_id, 'native')
    assert_equal "stage_pui_#{run_id}_candidate", ArchivalistHarnessSafety.stage_core_name(run_id, 'candidate', 'pui')
    assert_equal "stage_staff_#{run_id}_fallback", ArchivalistHarnessSafety.stage_core_name(run_id, 'fallback', 'staff')
    assert_equal "stage_staff_#{run_id}_native", ArchivalistHarnessSafety.stage_core_name(run_id, 'native', :staff)
  end

  def test_stage_core_name_rejects_invalid_mode
    run_id = ArchivalistHarnessSafety.generate_run_id
    err = assert_raises(RuntimeError) do
      ArchivalistHarnessSafety.stage_core_name(run_id, 'native', 'invalid_mode')
    end
    assert_match(/Invalid stage mode/i, err.message)
  end

  def test_valid_stage_core_name_accepts_both_modes
    run_id = ArchivalistHarnessSafety.generate_run_id
    pui_name = ArchivalistHarnessSafety.stage_core_name(run_id, 'native', 'pui')
    staff_name = ArchivalistHarnessSafety.stage_core_name(run_id, 'candidate', 'staff')
    assert ArchivalistHarnessSafety.valid_stage_core_name?(pui_name, run_id)
    assert ArchivalistHarnessSafety.valid_stage_core_name?(staff_name, run_id)
  end

  def test_valid_stage_core_name_rejects_protected_staff_names
    run_id = ArchivalistHarnessSafety.generate_run_id
    refute ArchivalistHarnessSafety.valid_stage_core_name?('stage_staff_production', run_id)
    refute ArchivalistHarnessSafety.valid_stage_core_name?('stage_staff_archivesspace', run_id)
    refute ArchivalistHarnessSafety.valid_stage_core_name?("stage_staff_#{run_id}_production", run_id)
  end

  def test_verify_auxiliary_parity_passes_on_identical_docs
    doc1 = {
      'id' => '/repositories/4/resources/9/tree/root',
      'uri' => '/repositories/4/resources/9/tree/root',
      'pui_parent_id' => '/repositories/4/resources/9',
      'primary_type' => 'tree_root',
      'types' => ['pui'],
      'publish' => 'true',
      'json' => JSON.generate({ 'child_count' => 4, 'waypoints' => 1, 'items' => [1, 2, 3] }),
      '_version_' => 100
    }
    doc2 = {
      'id' => '/repositories/4/resources/9/tree/root',
      'uri' => '/repositories/4/resources/9/tree/root',
      'pui_parent_id' => '/repositories/4/resources/9',
      'primary_type' => 'tree_root',
      'types' => ['pui'],
      'publish' => 'true',
      'json' => JSON.generate({ 'child_count' => 4, 'waypoints' => 1, 'items' => [1, 2, 3] }),
      '_version_' => 200
    }
    assert ArchivalistHarnessSafety.verify_auxiliary_parity([doc1], [doc2])
  end

  def test_verify_auxiliary_parity_fails_on_json_value_and_type_perturbations
    base = {
      'id' => '/repositories/4/resources/9/tree/root',
      'uri' => '/repositories/4/resources/9/tree/root',
      'pui_parent_id' => '/repositories/4/resources/9',
      'primary_type' => 'tree_root',
      'types' => ['pui'],
      'publish' => 'true',
      'json' => JSON.generate({ 'child_count' => 4, 'title' => 'Original', 'items' => ['a', 'b'] })
    }

    # Perturbation 1: Value mismatch in parsed JSON
    p_val = base.merge('json' => JSON.generate({ 'child_count' => 5, 'title' => 'Original', 'items' => ['a', 'b'] }))
    err1 = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_auxiliary_parity([base], [p_val]) }
    assert_match(/Value mismatch at doc\[.*\]\.json\.child_count/i, err1.message)

    # Perturbation 2: Type mismatch in parsed JSON (Integer vs String)
    p_type = base.merge('json' => JSON.generate({ 'child_count' => '4', 'title' => 'Original', 'items' => ['a', 'b'] }))
    err2 = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_auxiliary_parity([base], [p_type]) }
    assert_match(/Type mismatch at doc\[.*\]\.json\.child_count/i, err2.message)

    # Perturbation 3: Array order mismatch in parsed JSON
    p_order = base.merge('json' => JSON.generate({ 'child_count' => 4, 'title' => 'Original', 'items' => ['b', 'a'] }))
    err3 = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_auxiliary_parity([base], [p_order]) }
    assert_match(/Value mismatch at doc\[.*\]\.json\.items\[0\]/i, err3.message)

    # Perturbation 4: Key mismatch in parsed JSON
    p_key = base.merge('json' => JSON.generate({ 'child_count' => 4, 'title' => 'Original', 'extra_key' => true }))
    err4 = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_auxiliary_parity([base], [p_key]) }
    assert_match(/Key mismatch at doc\[.*\]\.json/i, err4.message)

    # Perturbation 5: Malformed JSON string
    p_malformed = base.merge('json' => "{invalid json string")
    err5 = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_auxiliary_parity([base], [p_malformed]) }
    assert_match(/Failed to parse candidate JSON/i, err5.message)

    # Perturbation 6: Scalar field mismatch outside JSON
    p_field = base.merge('primary_type' => 'tree_node')
    err6 = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_auxiliary_parity([base], [p_field]) }
    assert_match(/Value mismatch at doc\[.*\]\.primary_type/i, err6.message)
  end

  def test_backend_url_validation_accepts_valid_local_endpoints
    assert ArchivalistHarnessSafety.valid_backend_url?('http://localhost:4567')
    assert ArchivalistHarnessSafety.valid_backend_url?('http://127.0.0.1:4567')
    assert ArchivalistHarnessSafety.valid_backend_url?('http://backend:8089')
    assert ArchivalistHarnessSafety.valid_backend_url?('http://app:3000')
  end

  def test_backend_url_validation_rejects_external_hosts_and_credentials
    refute ArchivalistHarnessSafety.valid_backend_url?('http://evil.com:4567')
    refute ArchivalistHarnessSafety.valid_backend_url?('http://192.168.1.50:4567')
    refute ArchivalistHarnessSafety.valid_backend_url?('http://user:pass@127.0.0.1:4567')
    refute ArchivalistHarnessSafety.valid_backend_url?('http://127.0.0.1:9999')
    refute ArchivalistHarnessSafety.valid_backend_url?('not-a-url')
  end

  def test_verify_auxiliary_parity_catches_perturbations_on_array_json
    base_array_doc = {
      'id' => '/repositories/4/resources/9/ordered_records',
      'uri' => '/repositories/4/resources/9/ordered_records',
      'pui_parent_id' => '/repositories/4/resources/9',
      'primary_type' => 'resource_ordered_records',
      'types' => ['pui'],
      'publish' => 'true',
      'json' => JSON.generate(['/repositories/4/resources/9', '/repositories/4/archival_objects/1'])
    }

    # Value perturbation on array
    p_arr_val = base_array_doc.merge(
      'json' => JSON.generate(['/repositories/4/resources/9', '/repositories/4/archival_objects/2'])
    )
    err = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_auxiliary_parity([base_array_doc], [p_arr_val]) }
    assert_match(/Value mismatch at doc\[.*\]\.json\[1\]/i, err.message)

    # Length perturbation on array
    p_arr_len = base_array_doc.merge(
      'json' => JSON.generate(['/repositories/4/resources/9'])
    )
    err_len = assert_raises(RuntimeError) { ArchivalistHarnessSafety.verify_auxiliary_parity([base_array_doc], [p_arr_len]) }
    assert_match(/Array length mismatch at doc\[.*\]\.json/i, err_len.message)
  end

  def test_pui_documents_partition_covers_all_docs_exactly_once
    aux_types = ['resource_ordered_records', 'tree_root', 'tree_waypoint', 'tree_node', 'tree_node_from_root']
    sample_docs = [
      { 'id' => '/repositories/4/resources/1#pui', 'types' => ['pui', 'pui_record'], 'primary_type' => 'resource' },
      { 'id' => '/repositories/4/archival_objects/1#pui', 'types' => ['pui', 'pui_record'], 'primary_type' => 'archival_object' },
      { 'id' => '/repositories/4/resources/1/ordered_records', 'types' => ['pui'], 'primary_type' => 'resource_ordered_records' },
      { 'id' => '/repositories/4/resources/1/tree/root', 'types' => ['pui'], 'primary_type' => 'tree_root' },
      { 'id' => '/repositories/4/resources/1/tree/waypoint_0_0', 'types' => ['pui'], 'primary_type' => 'tree_waypoint' },
      { 'id' => '/repositories/4/resources/1/tree/node_1', 'types' => ['pui'], 'primary_type' => 'tree_node' },
      { 'id' => '/repositories/4/resources/1/tree/node_from_root_1', 'types' => ['pui'], 'primary_type' => 'tree_node_from_root' },
      { 'id' => '/repositories/4', 'types' => ['repository'], 'primary_type' => 'repository' }
    ]

    ordinary_docs = sample_docs.select { |d| Array(d['types']).include?('pui_record') }
    auxiliary_docs = sample_docs.select { |d| aux_types.include?(d['primary_type']) }
    other_ordinary = sample_docs.reject { |d| Array(d['types']).include?('pui_record') || aux_types.include?(d['primary_type']) }

    assert_equal 2, ordinary_docs.length
    assert_equal 5, auxiliary_docs.length
    assert_equal 1, other_ordinary.length
    assert_equal sample_docs.length, (ordinary_docs.length + auxiliary_docs.length + other_ordinary.length)
    assert_empty((ordinary_docs.map { |d| d['id'] } & auxiliary_docs.map { |d| d['id'] }))
    assert_empty((ordinary_docs.map { |d| d['id'] } & other_ordinary.map { |d| d['id'] }))
    assert_empty((auxiliary_docs.map { |d| d['id'] } & other_ordinary.map { |d| d['id'] }))
  end

  # ============================================================================
  # REGRESSION: Dynamic Enum Whitespace Preservation & Frozen-String Pipeline
  # ============================================================================
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
  def test_dynamic_enum_whitespace_frozen_pipeline_preserves_exact_values
    doc_rust_unfrozen = {
      'title' => '  Uncleaned Title  ',
      'description' => "\tOrdinary description\n",
      'label_enum_s' => ['  leading_space_enum', 'trailing_space_enum  ', 'clean_enum'],
      'type_enum_s' => ['  both_spaces_enum  '],
      'unrelated_array' => ['  should_be_stripped  ']
    }

    # Verify unfrozen strings would be mutated by clean_whitespace (the original bug)
    unfrozen_copy = Marshal.load(Marshal.dump(doc_rust_unfrozen))
    ArchivalistHarnessSafety.clean_whitespace(unfrozen_copy)
    assert_equal 'leading_space_enum', unfrozen_copy['label_enum_s'][0], 'unfrozen would be stripped'

    # Now verify with freeze_dynamic_enum_strings!
    candidate_doc = Marshal.load(Marshal.dump(doc_rust_unfrozen))
    ArchivalistHarnessSafety.freeze_dynamic_enum_strings!(candidate_doc)

    # Confirm only *_enum_s strings are frozen
    assert candidate_doc['label_enum_s'][0].frozen?, 'label_enum_s[0] must be frozen'
    assert candidate_doc['label_enum_s'][1].frozen?, 'label_enum_s[1] must be frozen'
    assert candidate_doc['label_enum_s'][2].frozen?, 'label_enum_s[2] must be frozen'
    assert candidate_doc['type_enum_s'][0].frozen?, 'type_enum_s[0] must be frozen'

    # Confirm NO extra unrelated fields are frozen
    refute candidate_doc['title'].frozen?, 'title must NOT be frozen'
    refute candidate_doc['description'].frozen?, 'description must NOT be frozen'
    refute candidate_doc['unrelated_array'][0].frozen?, 'unrelated_array items must NOT be frozen'

    # Execute final document strip pipeline
    ArchivalistHarnessSafety.clean_whitespace(candidate_doc)

    # Exact stored enum values must be preserved exactly as native
    assert_equal ['  leading_space_enum', 'trailing_space_enum  ', 'clean_enum'], candidate_doc['label_enum_s']
    assert_equal ['  both_spaces_enum  '], candidate_doc['type_enum_s']

    # Ordinary fields must be cleaned and stripped
    assert_equal 'Uncleaned Title', candidate_doc['title']
    assert_equal 'Ordinary description', candidate_doc['description']
    assert_equal ['should_be_stripped'], candidate_doc['unrelated_array']
  end

  def test_freeze_dynamic_enum_strings_handles_empty_and_non_array_safely
    doc = {
      'title' => 'Sample',
      'empty_enum_s' => [],
      'nil_enum_s' => nil,
      'numeric_enum_s' => [123, nil]
    }
    ArchivalistHarnessSafety.freeze_dynamic_enum_strings!(doc)
    assert_equal [], doc['empty_enum_s']
    assert_nil doc['nil_enum_s']
    assert_equal [123, nil], doc['numeric_enum_s']
  end
end
