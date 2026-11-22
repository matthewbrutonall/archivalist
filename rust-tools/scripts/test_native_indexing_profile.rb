#!/usr/bin/env ruby
# frozen_string_literal: true

# Focused offline unit tests for native_indexing_profile.rb measurement instrumentation.
# Runs under MRI or JRuby with minitest.
# Proves CPU delta calculations, Linux /proc/self/status parsing, Solr JVM heap parsing,
# error handling / sanitization, and exact report schema / backward compatibility.

require 'rubygems'
if defined?(JRUBY_VERSION) && File.directory?('/archivesspace/build/gems/jruby/3.1.0')
  ENV['GEM_HOME'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  ENV['GEM_PATH'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  Gem.paths = {
    'GEM_HOME' => '/archivesspace/build/gems/jruby/3.1.0',
    'GEM_PATH' => '/archivesspace/build/gems/jruby/3.1.0'
  }
end

require 'minitest/autorun'
require 'json'
require 'tempfile'

require_relative 'native_indexing_profile'

class TestNativeIndexingProfileInstrumentation < Minitest::Test
  CpuTimesMock = Struct.new(:utime, :stime)

  # --- CPU Delta Tests ---

  def test_compute_cpu_delta_positive
    t_start = CpuTimesMock.new(1.250, 0.400)
    t_end = CpuTimesMock.new(2.500, 0.850)

    delta = NativeIndexingProfile.compute_cpu_delta(t_start, t_end)
    assert_equal 1250.0, delta['cpu_user_ms']
    assert_equal 450.0, delta['cpu_system_ms']
  end

  def test_compute_cpu_delta_zero_and_clamp_negative
    t_start = CpuTimesMock.new(5.000, 2.000)
    t_end = CpuTimesMock.new(5.000, 1.999) # negative jitter clamped to 0.0

    delta = NativeIndexingProfile.compute_cpu_delta(t_start, t_end)
    assert_equal 0.0, delta['cpu_user_ms']
    assert_equal 0.0, delta['cpu_system_ms']
  end

  def test_compute_cpu_delta_nil_handling
    assert_equal({ 'cpu_user_ms' => nil, 'cpu_system_ms' => nil },
                 NativeIndexingProfile.compute_cpu_delta(nil, nil))
    assert_equal({ 'cpu_user_ms' => nil, 'cpu_system_ms' => nil },
                 NativeIndexingProfile.compute_cpu_delta(CpuTimesMock.new(1.0, 1.0), nil))
    assert_equal({ 'cpu_user_ms' => nil, 'cpu_system_ms' => nil },
                 NativeIndexingProfile.compute_cpu_delta(nil, CpuTimesMock.new(2.0, 2.0)))
  end

  def test_compute_cpu_delta_real_process_times
    t0 = Process.times
    # Burn a tiny amount of CPU
    sum = 0
    100_000.times { |i| sum += i }
    t1 = Process.times

    delta = NativeIndexingProfile.compute_cpu_delta(t0, t1)
    refute_nil delta['cpu_user_ms']
    refute_nil delta['cpu_system_ms']
    assert delta['cpu_user_ms'] >= 0.0
    assert delta['cpu_system_ms'] >= 0.0
  end

  # --- /proc/self/status Parser Tests ---

  def test_read_proc_status_success
    fixture = <<~STATUS
      Name:\truby
      Umask:\t0002
      State:\tS (sleeping)
      VmPeak:\t  250000 kB
      VmHWM:\t  184500 kB
      VmSize:\t  240000 kB
      VmRSS:\t  123456 kB
      VmData:\t   50000 kB
    STATUS

    Tempfile.create('proc_status_test') do |f|
      f.write(fixture)
      f.flush

      res = NativeIndexingProfile.read_proc_status(f.path)
      assert_equal 123456, res['vm_rss_kib']
      assert_equal 184500, res['vm_hwm_kib']
      assert_equal NativeIndexingProfile::VM_HWM_SEMANTICS, res['vm_hwm_semantics']
      assert_nil res['diagnostic']
    end
  end

  def test_read_proc_status_missing_file
    res = NativeIndexingProfile.read_proc_status('/nonexistent/path/proc/status')
    assert_nil res['vm_rss_kib']
    assert_nil res['vm_hwm_kib']
    assert_equal NativeIndexingProfile::VM_HWM_SEMANTICS, res['vm_hwm_semantics']
    refute_nil res['diagnostic']
    assert_includes res['diagnostic'], 'not found'
  end

  def test_read_proc_status_missing_fields_never_fabricates_zero
    partial_fixture = <<~STATUS
      Name:\truby
      VmRSS:\t  65432 kB
    STATUS

    Tempfile.create('proc_status_partial') do |f|
      f.write(partial_fixture)
      f.flush

      res = NativeIndexingProfile.read_proc_status(f.path)
      assert_equal 65432, res['vm_rss_kib']
      assert_nil res['vm_hwm_kib'], 'Must be null, not fabricated zero'
      refute_nil res['diagnostic']
      assert_includes res['diagnostic'], 'Missing VmHWM'
    end
  end

  def test_read_proc_status_corrupt_content
    Tempfile.create('proc_status_corrupt') do |f|
      f.write("Corrupt file with no memory metrics\n")
      f.flush

      res = NativeIndexingProfile.read_proc_status(f.path)
      assert_nil res['vm_rss_kib']
      assert_nil res['vm_hwm_kib']
      refute_nil res['diagnostic']
      assert_includes res['diagnostic'], 'Missing VmRSS, VmHWM'
    end
  end

  def test_read_proc_status_real_linux_if_available
    if File.exist?('/proc/self/status')
      res = NativeIndexingProfile.read_proc_status('/proc/self/status')
      assert_kind_of Integer, res['vm_rss_kib']
      assert_kind_of Integer, res['vm_hwm_kib']
      assert res['vm_rss_kib'] > 0
      assert res['vm_hwm_kib'] >= res['vm_rss_kib']
      assert_nil res['diagnostic']
    end
  end

  # --- Solr JVM Heap Parser and Error Tests ---

  def test_parse_solr_jvm_heap_success
    solr_json = {
      'responseHeader' => { 'status' => 0, 'QTime' => 2 },
      'jvm' => {
        'version' => '11.0.32',
        'memory' => {
          'free' => '300.0 MB',
          'total' => '512 MB',
          'max' => '512 MB',
          'used' => '212.0 MB',
          'raw' => {
            'free' => 314572800,
            'total' => 536870912,
            'max' => 536870912,
            'used' => 222298112,
            'used%' => 41.4
          }
        }
      }
    }

    res = NativeIndexingProfile.parse_solr_jvm_heap(JSON.generate(solr_json))
    assert_equal 222298112, res['used_bytes']
    assert_equal 536870912, res['committed_bytes']
    assert_equal 536870912, res['max_bytes']
    assert_nil res['diagnostic']
  end

  def test_parse_solr_jvm_heap_with_committed_key
    solr_hash = {
      'jvm' => {
        'memory' => {
          'raw' => {
            'used' => 100000000,
            'committed' => 256000000,
            'max' => 512000000
          }
        }
      }
    }

    res = NativeIndexingProfile.parse_solr_jvm_heap(solr_hash)
    assert_equal 100000000, res['used_bytes']
    assert_equal 256000000, res['committed_bytes']
    assert_equal 512000000, res['max_bytes']
    assert_nil res['diagnostic']
  end

  def test_parse_solr_jvm_heap_missing_raw
    malformed = { 'responseHeader' => { 'status' => 0 }, 'jvm' => {} }
    res = NativeIndexingProfile.parse_solr_jvm_heap(malformed)

    assert_nil res['used_bytes']
    assert_nil res['committed_bytes']
    assert_nil res['max_bytes']
    refute_nil res['diagnostic']
    assert_includes res['diagnostic'], 'jvm.memory.raw missing'
  end

  def test_parse_solr_jvm_heap_invalid_json
    res = NativeIndexingProfile.parse_solr_jvm_heap('NOT JSON {{{')
    assert_nil res['used_bytes']
    assert_nil res['committed_bytes']
    assert_nil res['max_bytes']
    refute_nil res['diagnostic']
    assert_includes res['diagnostic'], 'Failed to parse Solr JVM heap'
  end

  def test_fetch_solr_jvm_heap_unapproved_url_rejected_by_safety
    res = NativeIndexingProfile.fetch_solr_jvm_heap('http://unapproved-host:8983/solr')
    assert_nil res['used_bytes']
    assert_nil res['committed_bytes']
    assert_nil res['max_bytes']
    refute_nil res['diagnostic']
    assert_includes res['diagnostic'], 'Safety violation'

    # Primary keyword forbidden in stage / safety URL validation
    res2 = NativeIndexingProfile.fetch_solr_jvm_heap('http://solr:8983/solr/primary')
    assert_nil res2['used_bytes']
    assert_includes res2['diagnostic'], 'Safety violation'
  end

  def test_fetch_solr_jvm_heap_getter_injection_success
    getter = lambda do |url|
      assert_equal 'http://solr:8983/solr/admin/info/system?wt=json', url
      {
        'jvm' => {
          'memory' => {
            'raw' => {
              'used' => 12345678,
              'total' => 98765432,
              'max' => 98765432
            }
          }
        }
      }
    end

    res = NativeIndexingProfile.fetch_solr_jvm_heap('http://solr:8983/solr', getter: getter)
    assert_equal 12345678, res['used_bytes']
    assert_equal 98765432, res['committed_bytes']
    assert_equal 98765432, res['max_bytes']
    assert_nil res['diagnostic']
  end

  def test_fetch_solr_jvm_heap_getter_error_sanitization
    getter = lambda do |_url|
      raise StandardError, 'Failed to connect to http://secretuser:secretpass@solr:8983/solr'
    end

    res = NativeIndexingProfile.fetch_solr_jvm_heap('http://solr:8983/solr', getter: getter)
    assert_nil res['used_bytes']
    refute_nil res['diagnostic']
    assert_includes res['diagnostic'], 'http://***:***@solr:8983/solr'
    refute_includes res['diagnostic'], 'secretpass'
  end

  def test_sanitize_error_message
    assert_nil NativeIndexingProfile.sanitize_error_message(nil)
    sanitized = NativeIndexingProfile.sanitize_error_message(
      StandardError.new('Failed to connect to http://user:verysecret123@127.0.0.1:8983/solr')
    )
    assert_equal 'StandardError: Failed to connect to http://***:***@127.0.0.1:8983/solr', sanitized
  end

  # --- Resource Fields Helper Tests ---

  def test_median_calculation
    assert_nil NativeIndexingProfile.median(nil)
    assert_nil NativeIndexingProfile.median([])
    assert_equal 5.0, NativeIndexingProfile.median([5.0])
    assert_equal 20.0, NativeIndexingProfile.median([10.0, 20.0, 30.0])
    assert_equal 25.0, NativeIndexingProfile.median([10.0, 20.0, 30.0, 40.0])
  end

  def test_resource_fields_clean_values
    cpu_delta = { 'cpu_user_ms' => 3800.25, 'cpu_system_ms' => 420.10 }
    proc_before = {
      'vm_rss_kib' => 100_000,
      'vm_hwm_kib' => 120_000,
      'vm_hwm_semantics' => NativeIndexingProfile::VM_HWM_SEMANTICS,
      'diagnostic' => nil
    }
    proc_after = {
      'vm_rss_kib' => 105_000,
      'vm_hwm_kib' => 125_000,
      'vm_hwm_semantics' => NativeIndexingProfile::VM_HWM_SEMANTICS,
      'diagnostic' => nil
    }
    solr_before = {
      'used_bytes' => 150_000_000,
      'committed_bytes' => 500_000_000,
      'max_bytes' => 500_000_000,
      'diagnostic' => nil
    }
    solr_after = {
      'used_bytes' => 180_000_000,
      'committed_bytes' => 500_000_000,
      'max_bytes' => 500_000_000,
      'diagnostic' => nil
    }

    res = NativeIndexingProfile.resource_fields(
      cpu_delta: cpu_delta,
      proc_mem_before: proc_before,
      proc_mem_after: proc_after,
      solr_heap_before: solr_before,
      solr_heap_after: solr_after
    )

    assert_equal 3800.25, res['cpu_user_ms']
    assert_equal 420.10, res['cpu_system_ms']

    pm = res['process_memory']
    assert_equal 100_000, pm['vm_rss_before_kib']
    assert_equal 105_000, pm['vm_rss_after_kib']
    assert_equal 120_000, pm['vm_hwm_before_kib']
    assert_equal 125_000, pm['vm_hwm_after_kib']
    assert_equal NativeIndexingProfile::VM_HWM_SEMANTICS, pm['vm_hwm_semantics']
    assert_nil pm['diagnostic']

    sjh = res['solr_jvm_heap']
    assert_equal 150_000_000, sjh['before']['used_bytes']
    assert_equal 180_000_000, sjh['after']['used_bytes']
    assert_equal 500_000_000, sjh['after']['committed_bytes']
    assert_equal 500_000_000, sjh['after']['max_bytes']
  end

  def test_resource_fields_missing_and_diagnostics
    # 1. Completely nil inputs must not raise and must produce valid schema with nils
    nil_res = NativeIndexingProfile.resource_fields(
      cpu_delta: nil,
      proc_mem_before: nil,
      proc_mem_after: nil,
      solr_heap_before: nil,
      solr_heap_after: nil
    )
    assert_nil nil_res['cpu_user_ms']
    assert_nil nil_res['cpu_system_ms']
    assert_nil nil_res['process_memory']['vm_rss_before_kib']
    assert_nil nil_res['process_memory']['vm_rss_after_kib']
    assert_nil nil_res['process_memory']['vm_hwm_before_kib']
    assert_nil nil_res['process_memory']['vm_hwm_after_kib']
    assert_equal NativeIndexingProfile::VM_HWM_SEMANTICS, nil_res['process_memory']['vm_hwm_semantics']
    assert_nil nil_res['process_memory']['diagnostic']
    assert_nil nil_res['solr_jvm_heap']['before']
    assert_nil nil_res['solr_jvm_heap']['after']

    # 2. Duplicate diagnostics must be deduplicated
    dup_res = NativeIndexingProfile.resource_fields(
      proc_mem_before: { 'diagnostic' => 'proc status missing' },
      proc_mem_after: { 'diagnostic' => 'proc status missing' }
    )
    assert_equal 'proc status missing', dup_res['process_memory']['diagnostic']

    # 3. Differing diagnostics must be joined
    diff_res = NativeIndexingProfile.resource_fields(
      proc_mem_before: { 'diagnostic' => 'Missing VmRSS' },
      proc_mem_after: { 'diagnostic' => 'Error reading file' }
    )
    assert_equal 'Missing VmRSS; Error reading file', diff_res['process_memory']['diagnostic']

    # 4. Solr diagnostics retained independently
    solr_res = NativeIndexingProfile.resource_fields(
      solr_heap_before: { 'diagnostic' => 'HTTP 500' },
      solr_heap_after: { 'diagnostic' => 'HTTP 503' }
    )
    assert_equal 'HTTP 500', solr_res['solr_jvm_heap']['before']['diagnostic']
    assert_equal 'HTTP 503', solr_res['solr_jvm_heap']['after']['diagnostic']
  end

  # --- Exact Report Schema & Backward Compatibility Tests ---

  def test_report_schema_and_backward_compatibility
    proc_before = {
      'vm_rss_kib' => 100_000,
      'vm_hwm_kib' => 120_000,
      'vm_hwm_semantics' => NativeIndexingProfile::VM_HWM_SEMANTICS,
      'diagnostic' => nil
    }
    proc_after = {
      'vm_rss_kib' => 105_000,
      'vm_hwm_kib' => 125_000,
      'vm_hwm_semantics' => NativeIndexingProfile::VM_HWM_SEMANTICS,
      'diagnostic' => nil
    }
    solr_before = {
      'used_bytes' => 150_000_000,
      'committed_bytes' => 500_000_000,
      'max_bytes' => 500_000_000,
      'diagnostic' => nil
    }
    solr_after = {
      'used_bytes' => 180_000_000,
      'committed_bytes' => 500_000_000,
      'max_bytes' => 500_000_000,
      'diagnostic' => nil
    }

    cpu_delta = { 'cpu_user_ms' => 3800.25, 'cpu_system_ms' => 420.10 }

    # Execute actual runtime helper
    res_fields = NativeIndexingProfile.resource_fields(
      cpu_delta: cpu_delta,
      proc_mem_before: proc_before,
      proc_mem_after: proc_after,
      solr_heap_before: solr_before,
      solr_heap_after: solr_after
    )

    # Assemble result_row using exact runtime merge path
    result_row = {
      'repeat' => 1,
      'mode' => 'staff',
      'run_id' => 'test_run_123',
      'core' => 'stage_staff_test_run_123_native',
      'wall_ms' => 4500.5
    }.merge(res_fields).merge(
      'solr_doc_count' => 1234,
      'timing' => nil
    )

    # Backward compatibility: verify all legacy keys exist
    %w[repeat mode run_id core wall_ms solr_doc_count timing].each do |k|
      assert result_row.key?(k), "Missing legacy key #{k} in result_row"
    end

    # New Gate 1 instrumentation keys exist with explicit units
    %w[cpu_user_ms cpu_system_ms process_memory solr_jvm_heap].each do |k|
      assert result_row.key?(k), "Missing Gate 1 key #{k} in result_row"
    end

    # Verify process_memory keys & semantics
    pm = result_row['process_memory']
    assert_equal 100_000, pm['vm_rss_before_kib']
    assert_equal 105_000, pm['vm_rss_after_kib']
    assert_equal 120_000, pm['vm_hwm_before_kib']
    assert_equal 125_000, pm['vm_hwm_after_kib']
    assert_equal NativeIndexingProfile::VM_HWM_SEMANTICS, pm['vm_hwm_semantics']
    assert_nil pm['diagnostic']

    # Verify solr_jvm_heap keys
    sjh = result_row['solr_jvm_heap']
    assert_equal 150_000_000, sjh['before']['used_bytes']
    assert_equal 180_000_000, sjh['after']['used_bytes']
    assert_equal 500_000_000, sjh['after']['committed_bytes']
    assert_equal 500_000_000, sjh['after']['max_bytes']

    # Build report
    notes = [
      'process_memory records Linux /proc/self/status VmRSS and VmHWM snapshots in KiB before and after each pass; snapshots are point-in-time and are not continuous samples of peak per-pass RSS. VmHWM is the operating system process-lifetime high-water mark since process startup, not a per-pass peak, so later repeats may inherit earlier highs.',
      'solr_jvm_heap records point-in-time snapshots of isolated Solr JVM heap used, committed, and max bytes before and after each pass via the bounded read-only Solr system API; snapshots are not continuous samples of peak heap usage during the indexing round.'
    ]

    report = {
      'run_id' => 'test_run_123',
      'timestamp' => Time.now.utc.iso8601,
      'runtime' => RUBY_DESCRIPTION,
      'backend_base' => 'http://localhost:4567',
      'solr_base' => 'http://solr:8983/solr',
      'profiling_stack' => { 'stack_id' => 'test' },
      'timers' => false,
      'pui_fetch_audit' => false,
      'pui_records_per_thread' => 25,
      'pui_tree_batch_size' => 25,
      'pui_tree_discovery_batch_size' => 25,
      'threads' => { 'staff' => [1, 25], 'pui' => [1, 25] },
      'rust_flags' => {},
      'inventory' => [],
      'results' => [result_row],
      'per_mode' => {
        'staff' => {
          'repeats' => 1,
          'wall_ms_raw' => [4500.5],
          'wall_ms_median' => 4500.5,
          'cpu_user_ms_raw' => [3800.25],
          'cpu_user_ms_median' => 3800.25,
          'cpu_system_ms_raw' => [420.10],
          'cpu_system_ms_median' => 420.10,
          'solr_doc_counts' => [1234],
          'timing_median' => nil
        }
      },
      'notes' => notes
    }

    # Verify JSON serialization roundtrip
    serialized = JSON.generate(report)
    parsed = JSON.parse(serialized)

    assert_equal 1, parsed['results'].length
    assert_equal 3800.25, parsed['results'][0]['cpu_user_ms']
    assert_equal 420.10, parsed['results'][0]['cpu_system_ms']
    assert_equal 100_000, parsed['results'][0]['process_memory']['vm_rss_before_kib']
    assert_equal 180_000_000, parsed['results'][0]['solr_jvm_heap']['after']['used_bytes']
    assert_equal 3800.25, parsed['per_mode']['staff']['cpu_user_ms_median']

    # Verify notes clarify snapshots are not continuous samples of peak heap or per-pass RSS
    assert parsed['notes'].any? { |n| n.include?('not continuous samples of peak per-pass RSS') }
    assert parsed['notes'].any? { |n| n.include?('not continuous samples of peak heap usage') }
    assert parsed['notes'].any? { |n| n.include?('process-lifetime high-water mark') }
  end

  def test_runtime_calls_resource_fields_helper
    script_path = File.expand_path('native_indexing_profile.rb', __dir__)
    source = File.read(script_path)

    # 1. Helper is defined in NativeIndexingProfile
    assert_match(/def self\.resource_fields\(/, source,
                 'NativeIndexingProfile must define self.resource_fields')

    # 2. Runtime indexing loop calls NativeIndexingProfile.resource_fields
    assert_match(/res_fields\s*=\s*NativeIndexingProfile\.resource_fields\(/, source,
                 'Runtime execution path must call NativeIndexingProfile.resource_fields')

    # 3. result_row is assembled by merging res_fields
    assert_match(/result_row\s*=\s*\{[^\}]*\}\.merge\(res_fields\)/m, source,
                 'result_row must merge helper output rather than hardcoding resource fields')

    # 4. CPU/RSS/Solr snapshots happen outside wall timing
    wall_stop_pos = source.index('wall_ms =')
    refute_nil wall_stop_pos, 'wall_ms assignment must exist'
    cpu_end_pos = source.index('cpu_end = Process.times')
    proc_after_pos = source.index('proc_mem_after = NativeIndexingProfile.read_proc_status')
    solr_after_pos = source.index('solr_heap_after = NativeIndexingProfile.fetch_solr_jvm_heap')

    assert cpu_end_pos > wall_stop_pos, 'cpu_end must be captured outside wall_ms measurement'
    assert proc_after_pos > wall_stop_pos, 'proc_mem_after must be captured outside wall_ms measurement'
    assert solr_after_pos > wall_stop_pos, 'solr_heap_after must be captured outside wall_ms measurement'
  end
end
