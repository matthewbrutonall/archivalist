#!/usr/bin/env ruby
# frozen_string_literal: true

# Archivalist Authorized Integration Stage Harness
# Validates:
# 1. Full JRuby indexer execution inside container runtime with live Solr cores.
# 2. Sidecar lifecycle (spawn, health, identity, and request metric recording).
# 3. Isolated multi-core Solr reindexing (native baseline vs Rust candidate vs outage fallback).
# 4. Canonical type-strict parity using compare_pui_index_parity.py and privacy hardening.
# 5. Outage fallback behavior upon verified stage-owned sidecar stop with full native equivalence.
# 6. Measured performance timings (warmup, multiple repeats, raw times, median wall-clock time).
#
# Scope & Transparency Disclosure:
# This harness exercises fixture-fed index_records execution through PUIIndexer
# with synthetic records across Resources and Archival Objects (including published,
# unpublished, and suppressed records). It demonstrates candidate preparation, strict validation,
# privacy hardening, and native fallback on outage. It DOES NOT claim full backend
# repository reindexing, STAFF mode coverage, or tree document traversal.

require 'rubygems'
if defined?(JRUBY_VERSION) && File.directory?('/archivesspace/build/gems/jruby/3.1.0')
  ENV['GEM_HOME'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  ENV['GEM_PATH'] ||= '/archivesspace/build/gems/jruby/3.1.0'
  Gem.paths = {
    'GEM_HOME' => '/archivesspace/build/gems/jruby/3.1.0',
    'GEM_PATH' => '/archivesspace/build/gems/jruby/3.1.0'
  }
end

require 'json'
require 'net/http'
require 'uri'
require 'fileutils'
require 'time'
require 'securerandom'
require 'socket'

module ArchivalistHarnessSafety
  ALLOWED_HOSTS = ['127.0.0.1', 'localhost', 'solr', 'solr-test'].freeze
  ALLOWED_PORTS = [8983, 8984, 8985].freeze
  ALLOWED_BACKEND_HOSTS = ['127.0.0.1', 'localhost', 'backend', 'app'].freeze
  ALLOWED_BACKEND_PORTS = [4567, 8089, 3000, 3002, 3003].freeze
  ALLOWED_ROLES = ['native', 'candidate', 'fallback'].freeze
  ALLOWED_MODES = ['pui', 'staff'].freeze
  FORBIDDEN_KEYWORDS = ['archivesspace', 'archivesspace_shadow', 'production', 'primary', 'live'].freeze

  def self.generate_run_id
    "#{Time.now.utc.strftime('%Y%m%d_%H%M%S')}_#{Process.pid}_#{SecureRandom.hex(4)}"
  end

  def self.valid_backend_url?(url_str)
    return false unless url_str.is_a?(String)
    uri = URI.parse(url_str)
    return false unless uri.is_a?(URI::HTTP) || uri.is_a?(URI::HTTPS)
    return false unless ALLOWED_BACKEND_HOSTS.include?(uri.hostname)
    return false unless ALLOWED_BACKEND_PORTS.include?(uri.port)
    return false unless uri.user.nil? && uri.password.nil?
    true
  rescue URI::InvalidURIError
    false
  end

  def self.stage_core_name(run_id, role, mode = 'pui')
    unless ALLOWED_ROLES.include?(role)
      raise "Invalid stage role '#{role}'; must be one of: #{ALLOWED_ROLES.join(', ')}"
    end
    unless ALLOWED_MODES.include?(mode.to_s)
      raise "Invalid stage mode '#{mode}'; must be one of: #{ALLOWED_MODES.join(', ')}"
    end
    "stage_#{mode}_#{run_id}_#{role}"
  end

  def self.valid_solr_url?(url_str)
    uri = URI.parse(url_str)
    return false unless uri.is_a?(URI::HTTP) || uri.is_a?(URI::HTTPS)
    return false unless ALLOWED_HOSTS.include?(uri.hostname)
    return false unless ALLOWED_PORTS.include?(uri.port)
    return false unless uri.user.nil? && uri.password.nil?
    return false unless uri.query.nil? || uri.query.empty?
    return false unless uri.path == '/solr' || uri.path == '/solr/'
    true
  rescue URI::InvalidURIError
    false
  end

  def self.valid_stage_core_name?(core_name, expected_run_id)
    return false unless core_name.is_a?(String)
    FORBIDDEN_KEYWORDS.each do |kw|
      return false if core_name.downcase == kw ||
                      core_name.downcase == "stage_pui_#{kw}" ||
                      core_name.downcase == "stage_staff_#{kw}"
    end
    pattern = /\Astage_(pui|staff)_#{Regexp.escape(expected_run_id)}_(native|candidate|fallback)\z/
    !(core_name =~ pattern).nil?
  end

  def self.solr_admin_request(base_url, action, params = {})
    unless valid_solr_url?(base_url)
      raise "Safety violation: Invalid or unapproved Solr URL '#{base_url}'"
    end

    uri = URI("#{base_url}/admin/cores")
    query_params = { 'action' => action, 'wt' => 'json' }.merge(params)
    uri.query = URI.encode_www_form(query_params)
    req = Net::HTTP::Get.new(uri)
    res = Net::HTTP.start(uri.hostname, uri.port, read_timeout: 10, open_timeout: 5) do |http|
      http.request(req)
    end

    raise "Solr admin #{action} returned HTTP #{res.code}" unless res.is_a?(Net::HTTPSuccess)
    parsed = JSON.parse(res.body)
    raise "Solr admin #{action} failed: #{parsed.inspect}" unless parsed.dig('responseHeader', 'status') == 0
    parsed
  end

  def self.ensure_stage_core(core_name, run_id, solr_base: nil, admin_caller: nil)
    unless valid_stage_core_name?(core_name, run_id)
      raise "Safety violation: Target core '#{core_name}' is not an authorized stage name for run '#{run_id}'"
    end

    caller_fn = admin_caller || lambda { |action, params = {}| solr_admin_request(solr_base, action, params) }

    status = caller_fn.call('STATUS', {})
    unless status.is_a?(Hash) &&
           status.dig('responseHeader', 'status') == 0 &&
           status.key?('status') &&
           status['status'].is_a?(Hash)
      raise "Failed to read Solr admin status before core creation: expected status object with responseHeader status 0 and status Hash"
    end

    existing_cores = status['status'].keys
    if existing_cores.include?(core_name)
      raise "Safety violation: Stage core '#{core_name}' already exists in Solr. Harness strictly refuses to mutate, overwrite, or unload existing cores."
    end

    create_res = caller_fn.call('CREATE', 'name' => core_name, 'configSet' => 'archivesspace')
    unless create_res && create_res.dig('responseHeader', 'status') == 0
      raise "Failed to create isolated Solr core #{core_name}: #{create_res.inspect}"
    end
    true
  end

  def self.assert_strict_equal(v1, v2, path = 'root')
    unless v1.class == v2.class
      raise "Native equivalence failure: Type mismatch at #{path} (v1: #{v1.class}, v2: #{v2.class})"
    end

    case v1
    when Hash
      keys1 = v1.keys.sort
      keys2 = v2.keys.sort
      unless keys1 == keys2
        diff_missing = keys1 - keys2
        diff_extra = keys2 - keys1
        raise "Native equivalence failure: Key mismatch at #{path} (missing in v2: #{diff_missing.inspect}, extra in v2: #{diff_extra.inspect})"
      end
      keys1.each do |k|
        assert_strict_equal(v1[k], v2[k], "#{path}.#{k}")
      end
    when Array
      unless v1.length == v2.length
        raise "Native equivalence failure: Array length mismatch at #{path} (v1: #{v1.length}, v2: #{v2.length})"
      end
      v1.each_with_index do |item, idx|
        assert_strict_equal(item, v2[idx], "#{path}[#{idx}]")
      end
    when Numeric
      unless v1 == v2
        raise "Native equivalence failure: Numeric value mismatch at #{path} (v1: #{v1.inspect}, v2: #{v2.inspect})"
      end
    else
      unless v1 == v2
        raise "Native equivalence failure: Value mismatch at #{path} (v1: #{v1.inspect}, v2: #{v2.inspect})"
      end
    end
    true
  end

  def self.verify_native_equivalence(native_docs, fallback_docs, ignored_fields: ['_version_', 'score'])
    unless native_docs.is_a?(Array) && fallback_docs.is_a?(Array)
      raise "Native equivalence failure: Both native_docs and fallback_docs must be Arrays"
    end
    unless native_docs.length == fallback_docs.length
      raise "Native equivalence failure: Doc count mismatch (native: #{native_docs.length}, fallback: #{fallback_docs.length})"
    end

    # Validate IDs in native_docs: assert unique nonempty string IDs
    native_ids = []
    native_docs.each_with_index do |doc, idx|
      unless doc.is_a?(Hash)
        raise "Native equivalence failure: Native doc at index #{idx} is not a Hash"
      end
      id = doc['id']
      unless id.is_a?(String) && !id.strip.empty?
        raise "Native equivalence failure: Native doc at index #{idx} has invalid or empty id: #{id.inspect}"
      end
      native_ids << id
    end
    if native_ids.length != native_ids.uniq.length
      duplicates = native_ids.select { |id| native_ids.count(id) > 1 }.uniq
      raise "Native equivalence failure: Duplicate IDs detected in native_docs: #{duplicates.inspect}"
    end

    # Validate IDs in fallback_docs: assert unique nonempty string IDs
    fallback_ids = []
    fallback_docs.each_with_index do |doc, idx|
      unless doc.is_a?(Hash)
        raise "Native equivalence failure: Fallback doc at index #{idx} is not a Hash"
      end
      id = doc['id']
      unless id.is_a?(String) && !id.strip.empty?
        raise "Native equivalence failure: Fallback doc at index #{idx} has invalid or empty id: #{id.inspect}"
      end
      fallback_ids << id
    end
    if fallback_ids.length != fallback_ids.uniq.length
      duplicates = fallback_ids.select { |id| fallback_ids.count(id) > 1 }.uniq
      raise "Native equivalence failure: Duplicate IDs detected in fallback_docs: #{duplicates.inspect}"
    end

    # Assert exact identity sets match
    unless native_ids.sort == fallback_ids.sort
      missing_in_fb = native_ids - fallback_ids
      extra_in_fb = fallback_ids - native_ids
      raise "Native equivalence failure: Identity set mismatch (missing in fallback: #{missing_in_fb.inspect}, extra in fallback: #{extra_in_fb.inspect})"
    end

    nat_by_id = native_docs.each_with_object({}) { |d, h| h[d['id']] = d }
    fallback_docs.each do |fb_d|
      id = fb_d['id']
      nat_d = nat_by_id[id]

      all_keys = (nat_d.keys + fb_d.keys).uniq - ignored_fields
      all_keys.sort.each do |k|
        raise "Native equivalence failure: Field '#{k}' missing in fallback doc #{id}" unless fb_d.key?(k)
        raise "Native equivalence failure: Field '#{k}' missing in native doc #{id}" unless nat_d.key?(k)

        assert_strict_equal(nat_d[k], fb_d[k], "doc[#{id}].#{k}")
      end
    end
    true
  end

  def self.verify_auxiliary_parity(baseline_docs, candidate_docs, ignored_fields: ['_version_', 'score'])
    unless baseline_docs.is_a?(Array) && candidate_docs.is_a?(Array)
      raise "Auxiliary parity failure: Both baseline_docs and candidate_docs must be Arrays"
    end
    unless baseline_docs.length == candidate_docs.length
      raise "Auxiliary parity failure: Doc count mismatch (baseline: #{baseline_docs.length}, candidate: #{candidate_docs.length})"
    end

    # Validate IDs in baseline_docs: assert unique nonempty string IDs
    base_ids = []
    baseline_docs.each_with_index do |doc, idx|
      unless doc.is_a?(Hash)
        raise "Auxiliary parity failure: Baseline doc at index #{idx} is not a Hash"
      end
      id = doc['id']
      unless id.is_a?(String) && !id.strip.empty?
        raise "Auxiliary parity failure: Baseline doc at index #{idx} has invalid or empty id: #{id.inspect}"
      end
      base_ids << id
    end
    if base_ids.length != base_ids.uniq.length
      duplicates = base_ids.select { |id| base_ids.count(id) > 1 }.uniq
      raise "Auxiliary parity failure: Duplicate IDs detected in baseline_docs: #{duplicates.inspect}"
    end

    # Validate IDs in candidate_docs
    cand_ids = []
    candidate_docs.each_with_index do |doc, idx|
      unless doc.is_a?(Hash)
        raise "Auxiliary parity failure: Candidate doc at index #{idx} is not a Hash"
      end
      id = doc['id']
      unless id.is_a?(String) && !id.strip.empty?
        raise "Auxiliary parity failure: Candidate doc at index #{idx} has invalid or empty id: #{id.inspect}"
      end
      cand_ids << id
    end
    if cand_ids.length != cand_ids.uniq.length
      duplicates = cand_ids.select { |id| cand_ids.count(id) > 1 }.uniq
      raise "Auxiliary parity failure: Duplicate IDs detected in candidate_docs: #{duplicates.inspect}"
    end

    unless base_ids.sort == cand_ids.sort
      missing_in_cand = base_ids - cand_ids
      extra_in_cand = cand_ids - base_ids
      raise "Auxiliary parity failure: Identity set mismatch (missing in candidate: #{missing_in_cand.inspect}, extra in candidate: #{extra_in_cand.inspect})"
    end

    base_by_id = baseline_docs.each_with_object({}) { |d, h| h[d['id']] = d }
    candidate_docs.each do |cand_d|
      id = cand_d['id']
      base_d = base_by_id[id]

      all_keys = (base_d.keys + cand_d.keys).uniq - ignored_fields
      all_keys.sort.each do |k|
        raise "Auxiliary parity failure: Field '#{k}' missing in candidate doc #{id}" unless cand_d.key?(k)
        raise "Auxiliary parity failure: Field '#{k}' missing in baseline doc #{id}" unless base_d.key?(k)

        if k == 'json'
          base_json_raw = base_d[k]
          cand_json_raw = cand_d[k]
          unless base_json_raw.is_a?(String) && cand_json_raw.is_a?(String)
            raise "Auxiliary parity failure: 'json' field must be a String in doc #{id}"
          end
          begin
            base_parsed = JSON.parse(base_json_raw)
          rescue => e
            raise "Auxiliary parity failure: Failed to parse baseline JSON in doc #{id}: #{e.message}"
          end
          begin
            cand_parsed = JSON.parse(cand_json_raw)
          rescue => e
            raise "Auxiliary parity failure: Failed to parse candidate JSON in doc #{id}: #{e.message}"
          end
          assert_strict_equal(base_parsed, cand_parsed, "doc[#{id}].json")
        else
          assert_strict_equal(base_d[k], cand_d[k], "doc[#{id}].#{k}")
        end
      end
    end
    true
  end

  def self.freeze_dynamic_enum_strings!(doc)
    return unless doc.is_a?(Hash)
    doc.each do |k, v|
      if k.is_a?(String) && k.end_with?('_enum_s') && v.is_a?(Array)
        v.each { |item| item.freeze if item.is_a?(String) }
      end
    end
  end

  def self.clean_whitespace(doc)
    if doc.is_a?(String) && !doc.frozen?
      doc.strip!
    elsif doc.is_a?(Hash)
      doc.values.each { |v| clean_whitespace(v) }
    elsif doc.is_a?(Array)
      doc.each { |v| clean_whitespace(v) }
    end
    doc
  end

  def self.verify_process_identity(pid, state)
    return false unless pid.is_a?(Integer) && pid > 1
    return false unless state.is_a?(Hash)

    # Reject stale state (already stopped)
    return false if state['status'] == 'stopped'

    # Strict command validation: must not be empty or substituted
    command = state['command']
    return false unless command.is_a?(String) && command == 'aspace-shadow-sidecar'

    binary = state['binary']
    return false unless binary.is_a?(String) && !binary.strip.empty?

    port = state['port']
    return false unless port.is_a?(Integer) || (port.is_a?(String) && port =~ /\A\d+\z/)

    run_id = state['run_id']
    return false unless run_id.is_a?(String) && !run_id.strip.empty?

    # Verify process existence via signal 0
    begin
      Process.kill(0, pid)
    rescue Errno::ESRCH, Errno::EPERM
      return false
    end

    # Verify cmdline on Linux via /proc
    proc_cmdline = "/proc/#{pid}/cmdline"
    if File.exist?(proc_cmdline)
      raw = File.read(proc_cmdline)
      args = raw.split("\0")
      return false if args.empty?

      # Arg 0 must match binary path or basename
      cmd_bin = args[0]
      return false unless cmd_bin == binary || File.basename(cmd_bin) == 'aspace-shadow-sidecar'

      # Port and collection must be in cmdline arguments
      return false unless args.include?('--port') && args[args.index('--port') + 1] == port.to_s
      return false unless args.include?('--collection')

      # Start identity verification if recorded
      if state['start_identity']
        proc_stat = "/proc/#{pid}/stat"
        if File.exist?(proc_stat)
          current_start = File.read(proc_stat).split(/\s+/)[21] rescue nil
          return false if current_start && current_start != state['start_identity']
        end
      end

      return true
    end

    # Fallback if /proc not available (e.g. non-Linux / macOS dev)
    cmd = `ps -p #{pid} -o args= 2>/dev/null`.strip
    return false if cmd.empty?
    return false unless cmd.include?('aspace-shadow-sidecar') && cmd.include?("--port #{port}")

    true
  end

  def self.port_in_use?(port, host: '127.0.0.1')
    s = Socket.tcp(host, port, connect_timeout: 0.2)
    s.close
    true
  rescue Errno::ECONNREFUSED, Errno::EHOSTUNREACH, Errno::ETIMEDOUT
    false
  end

  def self.find_unused_port(start_port: 4590, end_port: 4690)
    (start_port..end_port).each do |p|
      return p unless port_in_use?(p)
    end
    raise "Safety violation: No unused port available in range #{start_port}..#{end_port}"
  end

  def self.start_stage_sidecar(port:, solr_url:, collection:, run_id:, binary_path: nil)
    # Check that port is not already occupied before spawning
    port_in_use = false
    begin
      s = Socket.tcp('127.0.0.1', port, connect_timeout: 0.5)
      s.close
      port_in_use = true
    rescue Errno::ECONNREFUSED, Errno::EHOSTUNREACH, Errno::ETIMEDOUT
      port_in_use = false
    end
    if port_in_use
      raise "Safety violation: Port #{port} is already occupied before spawning stage sidecar"
    end

    bin = binary_path || '/archivesspace/rust-tools/target/debug/aspace-shadow-sidecar'
    unless File.executable?(bin)
      raise "Sidecar binary missing or not executable at #{bin}"
    end

    log_path = "/tmp/stage-sidecar-#{run_id}.log"
    full_cmd = [
      bin,
      '--port', port.to_s,
      '--solr-url', solr_url,
      '--collection', collection
    ]

    pid = Process.spawn(
      *full_cmd,
      out: log_path,
      err: [:child, :out]
    )

    start_identity = nil
    proc_stat = "/proc/#{pid}/stat"
    if File.exist?(proc_stat)
      start_identity = File.read(proc_stat).split(/\s+/)[21] rescue nil
    end

    state = {
      'pid' => pid,
      'command' => 'aspace-shadow-sidecar',
      'binary' => bin,
      'full_command' => full_cmd,
      'port' => port,
      'run_id' => run_id,
      'start_identity' => start_identity,
      'started_at' => Time.now.utc.iso8601,
      'log_path' => log_path,
      'status' => 'running'
    }

    state_file = "/tmp/stage-sidecar-#{run_id}.json"
    File.write(state_file, JSON.generate(state))

    # Bounded wait for /health (up to 3 seconds)
    health_uri = URI("http://127.0.0.1:#{port}/health")
    healthy = false
    15.times do
      sleep 0.2
      begin
        res = Net::HTTP.get_response(health_uri)
        if res.code == '200'
          body = JSON.parse(res.body)
          if body['served_by'] == 'aspace-shadow-sidecar'
            healthy = true
            break
          end
        end
      rescue Errno::ECONNREFUSED, Errno::EADDRNOTAVAIL
        # wait next iteration
      end
    end

    unless healthy
      # Terminate spawned PID only if process identity is verified to prevent killing unrelated process
      if verify_process_identity(pid, state)
        begin
          Process.kill('TERM', pid)
          Process.waitpid(pid, Process::WNOHANG) rescue nil
        rescue
          nil
        end
      end
      raise "Stage sidecar failed to report healthy on port #{port} within timeout"
    end

    state_file
  end

  def self.stop_stage_process(state_file_path)
    unless File.exist?(state_file_path)
      raise "Process stop error: PID state file missing at #{state_file_path}. Cannot verify stage ownership."
    end

    state = JSON.parse(File.read(state_file_path))
    pid = state['pid'].to_i

    unless verify_process_identity(pid, state)
      raise "Process stop error: PID #{pid} ownership verification failed. Refusing to send signal to unverified process."
    end

    # Send SIGTERM to verified stage process
    Process.kill('TERM', pid)

    # Bounded wait for process termination and reap child to avoid zombie
    stopped = false
    50.times do
      wpid = Process.waitpid(pid, Process::WNOHANG) rescue nil
      if wpid == pid
        stopped = true
        break
      end
      begin
        Process.getpgid(pid)
        sleep 0.1
      rescue Errno::ESRCH, Errno::ECHILD
        stopped = true
        break
      end
    end

    unless stopped
      raise "Process stop error: Stage process PID #{pid} did not terminate within bounded wait timeout."
    end

    # Preserve PID state file; do NOT delete it. Update status.
    state['status'] = 'stopped'
    state['stopped_at'] = Time.now.utc.iso8601
    File.write(state_file_path, JSON.pretty_generate(state))
    true
  end
end

# Main Execution Guard
if __FILE__ == $PROGRAM_NAME
  SOLR_BASE = ENV['SOLR_BASE_URL'] || 'http://solr:8983/solr'
  SIDECAR_PORT = if ENV['SIDECAR_PORT']
                   ENV['SIDECAR_PORT'].to_i
                 else
                   ArchivalistHarnessSafety.find_unused_port(start_port: 4590, end_port: 4690)
                 end
  SIDECAR_URL = "http://127.0.0.1:#{SIDECAR_PORT}"
  REPORT_DIR = File.expand_path('../reports/stage_runs', __dir__)
  FileUtils.mkdir_p(REPORT_DIR)

  RUN_ID = ArchivalistHarnessSafety.generate_run_id
  CORE_NATIVE = ArchivalistHarnessSafety.stage_core_name(RUN_ID, 'native')
  CORE_CANDIDATE = ArchivalistHarnessSafety.stage_core_name(RUN_ID, 'candidate')
  CORE_FALLBACK = ArchivalistHarnessSafety.stage_core_name(RUN_ID, 'fallback')

  puts "=========================================================="
  puts "Archivalist Authorized Integration Stage Harness"
  puts "Stage Run ID: #{RUN_ID}"
  puts "Runtime: #{RUBY_DESCRIPTION}"
  puts "Solr Base: #{SOLR_BASE}"
  puts "Sidecar URL: #{SIDECAR_URL}"
  puts "Target Cores: #{CORE_NATIVE}, #{CORE_CANDIDATE}, #{CORE_FALLBACK}"
  puts "=========================================================="

  # Require ArchivesSpace libraries from container classpath
  $LOAD_PATH.unshift('/archivesspace/common')
  $LOAD_PATH.unshift('/archivesspace/indexer')
  $LOAD_PATH.unshift('/archivesspace/indexer/app/lib')

  require 'active_support/core_ext/string/inflections'
  require 'config/config-distribution'
  AppConfig.reload
  AppConfig[:backend_url] ||= 'http://localhost:8089'
  AppConfig[:solr_url] ||= SOLR_BASE
  require 'index_state'
  require 'asutils'
  require 'aspace_i18n'
  require 'ashttp'
  require 'record_inheritance'
  require 'rust_index_document_builder'
  require 'pui_indexer'

  def commit_solr(core_name)
    unless ArchivalistHarnessSafety.valid_solr_url?(SOLR_BASE)
      raise "Safety violation: Invalid Solr URL '#{SOLR_BASE}'"
    end
    unless ArchivalistHarnessSafety.valid_stage_core_name?(core_name, RUN_ID)
      raise "Safety violation: Refusing commit to unapproved core '#{core_name}'"
    end

    uri = URI("#{SOLR_BASE}/#{core_name}/update?commit=true")
    req = Net::HTTP::Post.new(uri)
    req.content_type = 'application/json'
    req.body = '{}'
    res = Net::HTTP.start(uri.hostname, uri.port, read_timeout: 10, open_timeout: 5) { |h| h.request(req) }
    raise "Solr commit failed: HTTP #{res.code}" unless res.is_a?(Net::HTTPSuccess)
  end

  def fetch_all_solr_docs(core_name)
    unless ArchivalistHarnessSafety.valid_solr_url?(SOLR_BASE)
      raise "Safety violation: Invalid Solr URL '#{SOLR_BASE}'"
    end
    unless ArchivalistHarnessSafety.valid_stage_core_name?(core_name, RUN_ID)
      raise "Safety violation: Refusing fetch from unapproved core '#{core_name}'"
    end

    # First get total count without truncation
    count_uri = URI("#{SOLR_BASE}/#{core_name}/select?q=*:*&rows=0&wt=json")
    count_res = Net::HTTP.start(count_uri.hostname, count_uri.port, read_timeout: 10, open_timeout: 5) { |h| h.request(Net::HTTP::Get.new(count_uri)) }
    raise "Failed to query Solr doc count" unless count_res.is_a?(Net::HTTPSuccess)
    total_found = JSON.parse(count_res.body).dig('response', 'numFound') || 0

    return [] if total_found == 0

    # Fetch all documents with exact count
    uri = URI("#{SOLR_BASE}/#{core_name}/select?q=*:*&rows=#{total_found}&sort=id+asc&wt=json")
    res = Net::HTTP.start(uri.hostname, uri.port, read_timeout: 10, open_timeout: 5) { |h| h.request(Net::HTTP::Get.new(uri)) }
    raise "Failed to fetch Solr docs" unless res.is_a?(Net::HTTPSuccess)
    parsed = JSON.parse(res.body)
    docs = parsed.dig('response', 'docs') || []
    raise "Fetched count #{docs.length} does not match numFound #{total_found}" unless docs.length == total_found
    docs
  end

  def solr_search(core_name, query, fq: nil)
    params = { 'q' => query, 'rows' => '100', 'sort' => 'id asc', 'wt' => 'json' }
    params['fq'] = fq if fq
    uri = URI("#{SOLR_BASE}/#{core_name}/select?#{URI.encode_www_form(params)}")
    req = Net::HTTP::Get.new(uri)
    res = Net::HTTP.start(uri.hostname, uri.port, read_timeout: 10, open_timeout: 5) { |h| h.request(req) }
    raise "Search request failed" unless res.is_a?(Net::HTTPSuccess)
    parsed = JSON.parse(res.body)
    {
      'numFound' => parsed.dig('response', 'numFound') || 0,
      'ids' => (parsed.dig('response', 'docs') || []).map { |d| d['id'] }
    }
  end

  # Step 1: Initialize uniquely-named cores
  puts "\n--- Step 1: Initializing Stage-Owned Unique Solr Cores ---"
  [CORE_NATIVE, CORE_CANDIDATE, CORE_FALLBACK].each do |c|
    ArchivalistHarnessSafety.ensure_stage_core(c, RUN_ID, solr_base: SOLR_BASE)
    puts "Verified isolated stage core created: #{c}"
  end

  # Step 2: Spawn & Verify Stage Sidecar Lifecycle
  puts "\n--- Step 2: Spawning Stage-Owned Sidecar & Verifying Health ---"
  stage_state_file = ArchivalistHarnessSafety.start_stage_sidecar(
    port: SIDECAR_PORT,
    solr_url: SOLR_BASE,
    collection: CORE_CANDIDATE,
    run_id: RUN_ID
  )
  puts "Stage sidecar spawned on port #{SIDECAR_PORT} (State: #{stage_state_file})"

  health_uri = URI("#{SIDECAR_URL}/health")
  health_res = Net::HTTP.get_response(health_uri)
  raise "Sidecar health check failed: HTTP #{health_res.code}" unless health_res.code == '200'
  health_json = JSON.parse(health_res.body)
  puts "Sidecar Health: #{health_json.inspect}"
  raise "Unexpected server identity: #{health_json['served_by']}" unless health_json['served_by'] == 'aspace-shadow-sidecar'

  def get_sidecar_metrics
    metrics_uri = URI("#{SIDECAR_URL}/metrics")
    metrics_res = Net::HTTP.get_response(metrics_uri)
    return {} unless metrics_res.code == '200'
    out = {}
    metrics_res.body.each_line do |line|
      next if line.start_with?('#')
      parts = line.strip.split(/\s+/)
      out[parts[0]] = parts[1].to_f if parts.length == 2
    end
    out
  rescue
    {}
  end

  initial_metrics = get_sidecar_metrics
  initial_req_total = initial_metrics['archivalist_sidecar_requests_total'] || 0
  puts "Initial sidecar requests total: #{initial_req_total}"

  # Step 3: Build Synthetic Archival Records with Full Schema Fields
  puts "\n--- Step 3: Generating Synthetic Archival Records ---"
  repo_uri = '/repositories/999'
  resource_uri = "#{repo_uri}/resources/10"
  ao1_uri = "#{repo_uri}/archival_objects/101"
  ao2_uri = "#{repo_uri}/archival_objects/102"
  ao_unpub_uri = "#{repo_uri}/archival_objects/103"
  ao_suppressed_uri = "#{repo_uri}/archival_objects/104"

  synthetic_records = [
    {
      'uri' => resource_uri,
      'jsonmodel_type' => 'resource',
      'title' => 'Synthetic Pioneer Collection of Scientific Manuscripts',
      'publish' => true,
      'suppressed' => false,
      'repository' => { 'ref' => repo_uri },
      'id_0' => 'SCI-10',
      'level' => 'collection',
      'related_accessions' => [],
      'classifications' => [],
      'dates' => [
        { 'label' => 'creation', 'date_type' => 'inclusive', 'begin' => '1842-01-01', 'end' => '1882-12-31', 'expression' => '1842-1882' }
      ],
      'extents' => [
        { 'portion' => 'whole', 'number' => '12', 'extent_type' => 'boxes' }
      ],
      'notes' => [
        { 'jsonmodel_type' => 'note_singlepart', 'type' => 'scopecontent', 'publish' => true, 'content' => ['Collection level scope and content note.'] },
        { 'jsonmodel_type' => 'note_singlepart', 'type' => 'accessrestrict', 'publish' => true, 'content' => ['Open for research access.'] }
      ]
    },
    {
      'uri' => ao1_uri,
      'jsonmodel_type' => 'archival_object',
      'title' => 'Series 1: Correspondence with Royal Astronomical Society',
      'publish' => true,
      'suppressed' => false,
      'repository' => { 'ref' => repo_uri },
      'resource' => { 'ref' => resource_uri },
      'dates' => [],
      'extents' => [],
      'ancestors' => [
        { 'ref' => resource_uri, 'level' => 'collection', '_resolved' => {
          'title' => 'Synthetic Pioneer Collection of Scientific Manuscripts',
          'dates' => [{ 'label' => 'creation', 'date_type' => 'inclusive', 'begin' => '1842-01-01', 'end' => '1882-12-31', 'expression' => '1842-1882' }],
          'extents' => [{ 'portion' => 'whole', 'number' => '12', 'extent_type' => 'boxes' }],
          'notes' => [
            { 'jsonmodel_type' => 'note_singlepart', 'type' => 'accessrestrict', 'publish' => true, 'content' => ['Open for research access.'] }
          ]
        }}
      ],
      'instances' => [
        {
          'instance_type' => 'mixed_materials',
          'sub_container' => {
            'top_container' => {
              'ref' => "#{repo_uri}/top_containers/501",
              '_resolved' => {
                'container_locations' => [],
                'collection' => [],
                'display_string' => 'Box 501',
                'internal_note' => 'RESTRICTED_INTERNAL_BOX_NOTE',
                'container_profile' => {
                  '_resolved' => {
                    'notes' => 'INTERNAL_PROFILE_SPEC_NOTE'
                  }
                }
              }
            }
          }
        }
      ],
      'notes' => [
        { 'jsonmodel_type' => 'note_singlepart', 'type' => 'scopecontent', 'publish' => true, 'content' => ['Correspondence files concerning astronomical observations.'] }
      ]
    },
    {
      'uri' => ao2_uri,
      'jsonmodel_type' => 'archival_object',
      'title' => 'Sub-series 1.1: Letters from Dublin Observatory',
      'publish' => true,
      'suppressed' => false,
      'repository' => { 'ref' => repo_uri },
      'resource' => { 'ref' => resource_uri },
      'dates' => [],
      'extents' => [],
      'ancestors' => [
        { 'ref' => ao1_uri, 'level' => 'series', '_resolved' => { 'dates' => [], 'extents' => [], 'notes' => [] } },
        { 'ref' => resource_uri, 'level' => 'collection', '_resolved' => {
          'dates' => [{ 'label' => 'creation', 'date_type' => 'inclusive', 'begin' => '1842-01-01', 'end' => '1882-12-31', 'expression' => '1842-1882' }],
          'extents' => [{ 'portion' => 'whole', 'number' => '12', 'extent_type' => 'boxes' }],
          'notes' => [
            { 'jsonmodel_type' => 'note_singlepart', 'type' => 'accessrestrict', 'publish' => true, 'content' => ['Open for research access.'] }
          ]
        }}
      ],
      'notes' => [
        { 'jsonmodel_type' => 'note_singlepart', 'type' => 'scopecontent', 'publish' => true, 'content' => ['Detailed transit telescope recordings.'] }
      ]
    },
    {
      'uri' => ao_unpub_uri,
      'jsonmodel_type' => 'archival_object',
      'title' => 'Draft Unpublished Personal Diary',
      'publish' => false,
      'suppressed' => false,
      'repository' => { 'ref' => repo_uri },
      'resource' => { 'ref' => resource_uri },
      'dates' => [],
      'extents' => [],
      'ancestors' => [
        { 'ref' => resource_uri, 'level' => 'collection', '_resolved' => { 'dates' => [], 'extents' => [], 'notes' => [] } }
      ],
      'notes' => [
        { 'jsonmodel_type' => 'note_singlepart', 'type' => 'scopecontent', 'publish' => false, 'content' => ['Private draft thoughts not for publication.'] }
      ]
    },
    {
      'uri' => ao_suppressed_uri,
      'jsonmodel_type' => 'archival_object',
      'title' => 'Suppressed Secret Document',
      'publish' => false,
      'suppressed' => true,
      'repository' => { 'ref' => repo_uri },
      'resource' => { 'ref' => resource_uri },
      'dates' => [],
      'extents' => [],
      'ancestors' => [
        { 'ref' => resource_uri, 'level' => 'collection', '_resolved' => { 'dates' => [], 'extents' => [], 'notes' => [] } }
      ],
      'notes' => [
        { 'jsonmodel_type' => 'note_singlepart', 'type' => 'scopecontent', 'publish' => false, 'content' => ['Suppressed internal information.'] }
      ]
    }
  ]
  puts "Generated #{synthetic_records.length} synthetic records."

  def run_indexing_pass(target_core, records, options = {})
    indexer = PUIIndexer.new(AppConfig[:backend_url], nil, 'PUIIndexer')
    core_solr_url = "#{SOLR_BASE}/#{target_core}"
    AppConfig[:solr_url] = core_solr_url
    indexer.instance_variable_set(:@solr_url, URI.parse(core_solr_url))

    AppConfig[:rust_index_document_builder_enabled] = options.fetch(:builder_enabled, false)
    AppConfig[:rust_index_document_builder_url] = options.fetch(:sidecar_url, SIDECAR_URL)
    AppConfig[:rust_pui_document_preparation_enabled] = options.fetch(:pui_prep_enabled, false)
    AppConfig[:rust_pui_common_metadata_enabled] = options.fetch(:common_meta_enabled, false)
    AppConfig[:rust_pui_record_inheritance_candidate_enabled] = options.fetch(:candidate_enabled, false)
    AppConfig[:rust_pui_auxiliary_documents_enabled] = options.fetch(:aux_enabled, false)

    records_input = records.map do |r|
      clean_r = ASUtils.json_parse(ASUtils.to_json(r))
      clean_r['display_string'] ||= clean_r['title']
      if RecordInheritance.has_type?(clean_r['jsonmodel_type'])
        clean_r = RecordInheritance.merge(clean_r, :direct_only => true)
      end
      { 'record' => clean_r, 'uri' => clean_r['uri'] }
    end

    # Metrics Instrumentation Counters
    instrumentation = {
      'builder_calls' => 0,
      'builder_projections_received' => 0,
      'accepted_routing' => 0,
      'accepted_notes' => 0,
      'accepted_final' => 0,
      'accepted_common_helpers' => Hash.new(0),
      'rejections' => 0
    }

    orig_builder_build = RustIndexDocumentBuilder.instance_method(:build)
    if options[:builder_enabled]
      RustIndexDocumentBuilder.define_method(:build) do |inputs|
        instrumentation['builder_calls'] += 1
        res = orig_builder_build.bind(self).call(inputs)
        if res.is_a?(Hash)
          instrumentation['builder_projections_received'] += res.length
        end
        res
      end

      # Instrument apply_rust_pui_routing_projection
      orig_routing = indexer.method(:apply_rust_pui_routing_projection)
      indexer.define_singleton_method(:apply_rust_pui_routing_projection) do |doc, record|
        res = orig_routing.call(doc, record)
        if res
          instrumentation['accepted_routing'] += 1
        else
          instrumentation['rejections'] += 1
        end
        res
      end

      # Instrument apply_rust_pui_notes_projection
      orig_notes = indexer.method(:apply_rust_pui_notes_projection)
      indexer.define_singleton_method(:apply_rust_pui_notes_projection) do |doc, record|
        res = orig_notes.call(doc, record)
        if res
          instrumentation['accepted_notes'] += 1
        else
          instrumentation['rejections'] += 1
        end
        res
      end

      # Instrument apply_rust_pui_final_projection
      orig_final = indexer.method(:apply_rust_pui_final_projection)
      indexer.define_singleton_method(:apply_rust_pui_final_projection) do |doc, record|
        res = orig_final.call(doc, record)
        if res
          instrumentation['accepted_final'] += 1
        else
          instrumentation['rejections'] += 1
        end
        res
      end

      # Instrument apply_rust_pui_common_helper (forwarding &block is required because original helper yields scratch_doc)
      orig_helper = indexer.method(:apply_rust_pui_common_helper)
      indexer.define_singleton_method(:apply_rust_pui_common_helper) do |helper_name, doc, record, &block|
        res = orig_helper.call(helper_name, doc, record, &block)
        if res
          instrumentation['accepted_common_helpers'][helper_name] += 1
        else
          instrumentation['rejections'] += 1
        end
        res
      end
    end

    start_wall = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    begin
      indexer.send(:index_records, records_input)
    ensure
      RustIndexDocumentBuilder.define_method(:build, orig_builder_build)
    end
    end_wall = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    wall_ms = ((end_wall - start_wall) * 1000.0).round(2)

    commit_solr(target_core)
    docs = fetch_all_solr_docs(target_core)

    {
      'core' => target_core,
      'wall_ms' => wall_ms,
      'doc_count' => docs.length,
      'docs' => docs,
      'instrumentation' => instrumentation
    }
  end

  def benchmark_mode(mode_name, core_name, records, options, repeats: 3)
    # Warmup pass
    run_indexing_pass(core_name, records, options)

    timings = []
    last_res = nil
    repeats.times do
      res = run_indexing_pass(core_name, records, options)
      timings << res['wall_ms']
      last_res = res
    end

    timings.sort!
    median_time = timings[timings.length / 2]
    {
      'mode' => mode_name,
      'core' => core_name,
      'raw_timings' => timings,
      'median_ms' => median_time,
      'repeats' => repeats,
      'sample_size' => records.length,
      'last_result' => last_res
    }
  end

  puts "\n--- Step 4: Pass 1 - Native Ruby Baseline Indexing ---"
  native_bench = benchmark_mode('native', CORE_NATIVE, synthetic_records, { builder_enabled: false }, repeats: 3)
  native_res = native_bench['last_result']
  puts "Native Baseline Median: #{native_bench['median_ms']} ms (Raw: #{native_bench['raw_timings'].inspect})"

  puts "\n--- Step 5: Pass 2 - Rust Candidate Indexing ---"
  candidate_options = {
    builder_enabled: true,
    sidecar_url: SIDECAR_URL,
    pui_prep_enabled: true,
    common_meta_enabled: true,
    candidate_enabled: true,
    aux_enabled: true
  }
  candidate_bench = benchmark_mode('candidate', CORE_CANDIDATE, synthetic_records, candidate_options, repeats: 3)
  candidate_res = candidate_bench['last_result']
  puts "Rust Candidate Median: #{candidate_bench['median_ms']} ms (Raw: #{candidate_bench['raw_timings'].inspect})"

  # Metric & Acceptance Verification
  post_cand_metrics = get_sidecar_metrics
  post_cand_req_total = post_cand_metrics['archivalist_sidecar_requests_total'] || 0
  delta = post_cand_req_total - initial_req_total
  puts "Sidecar request counter delta: +#{delta}"
  raise "Verification failure: Sidecar received zero requests during candidate passes!" unless delta > 0

  inst = candidate_res['instrumentation']
  puts "Candidate Projections Instrumentation:"
  puts "  Builder Calls: #{inst['builder_calls']}"
  puts "  Builder Projections Received: #{inst['builder_projections_received']}"
  puts "  Accepted Routing Projections: #{inst['accepted_routing']}"
  puts "  Accepted Notes Projections: #{inst['accepted_notes']}"
  puts "  Accepted Final Projections: #{inst['accepted_final']}"
  puts "  Accepted Common Helpers: #{inst['accepted_common_helpers'].inspect}"
  puts "  Rejections / Fallbacks: #{inst['rejections']}"

  raise "Silent fallback: Zero builder calls!" unless inst['builder_calls'] > 0
  raise "Silent fallback: Zero routing projections accepted!" unless inst['accepted_routing'] > 0
  raise "Silent fallback: Zero notes projections accepted!" unless inst['accepted_notes'] > 0
  raise "Silent fallback: Zero final projections accepted!" unless inst['accepted_final'] > 0
  raise "Silent fallback: Zero common helper projections accepted!" unless inst['accepted_common_helpers'].values.sum > 0
  raise "Projection rejection detected in candidate pass!" unless inst['rejections'] == 0

  # Step 6: Verified Process Stop & Outage Fallback Pass
  puts "\n--- Step 6: Pass 3 - Stopping Verified Sidecar & Testing Outage Fallback ---"
  ArchivalistHarnessSafety.stop_stage_process(stage_state_file)
  puts "Verified stage sidecar stopped successfully."

  # Verify sidecar port is closed
  port_closed = false
  begin
    Net::HTTP.get_response(health_uri)
  rescue Errno::ECONNREFUSED
    port_closed = true
  end
  raise "Outage verification failure: Sidecar is still accepting connections on #{health_uri}!" unless port_closed
  puts "Outage verification confirmed: Sidecar is unreachable (ECONNREFUSED)."

  fallback_bench = benchmark_mode('fallback', CORE_FALLBACK, synthetic_records, candidate_options, repeats: 3)
  fallback_res = fallback_bench['last_result']
  puts "Outage Fallback Median: #{fallback_bench['median_ms']} ms (Raw: #{fallback_bench['raw_timings'].inspect})"

  # Step 7: Parity, Privacy, and Search Verifications
  puts "\n--- Step 7: Parity, Privacy, and Search Verifications ---"
  raise "Document count mismatch" unless native_res['doc_count'] == candidate_res['doc_count'] && native_res['doc_count'] == fallback_res['doc_count']
  puts "Document counts match across all 3 cores: #{native_res['doc_count']} docs."

  # 1. Candidate Parity Verification via Canonical compare_pui_index_parity.py
  puts "\nRunning Canonical compare_pui_index_parity.py against candidate..."
  comparator_script = File.expand_path('compare_pui_index_parity.py', __dir__)
  comp_cmd = [
    'python3', comparator_script,
    '--baseline-url', "#{SOLR_BASE}/#{CORE_NATIVE}",
    '--candidate-url', "#{SOLR_BASE}/#{CORE_CANDIDATE}",
    '--repository', '999'
  ]
  comp_success = system(*comp_cmd)
  raise "Canonical compare_pui_index_parity.py failed on candidate core!" unless comp_success
  puts "Canonical Candidate Parity Check: PASS"

  # 2. Candidate Privacy Invariants
  candidate_res['docs'].each do |d|
    uri = d['id']
    raise "Privacy leak: candidate doc #{uri} has root ancestors" if d.key?('ancestors')
    if d['json']
      parsed = JSON.parse(d['json'])
      raise "Privacy leak: candidate doc #{uri} json has ancestors" if parsed.key?('ancestors')
      raise "Privacy leak: candidate doc #{uri} leaked RESTRICTED_INTERNAL_BOX_NOTE" if d['json'].include?('RESTRICTED_INTERNAL_BOX_NOTE')
      raise "Privacy leak: candidate doc #{uri} leaked INTERNAL_PROFILE_SPEC_NOTE" if d['json'].include?('INTERNAL_PROFILE_SPEC_NOTE')
    end
    if d['publish'] == false
      raise "Privacy leak: unpublished doc #{uri} has published fullrecord" if d['fullrecord_published'] && !d['fullrecord_published'].empty?
    end
    d.keys.each do |k|
      raise "Privacy leak: candidate doc #{uri} leaked internal marker #{k}" if k.start_with?('_archivalist_')
    end
  end
  puts "Candidate Privacy Invariants: PASS"

  # 3. Fallback Full Native Equivalence Check (must match native baseline type-strict and field-exact)
  ArchivalistHarnessSafety.verify_native_equivalence(native_res['docs'], fallback_res['docs'])
  puts "Fallback Full Native Equivalence Check: PASS"

  # 4. Search Queries (positive & negative & unpublished/suppressed absences)
  positive_scenarios = [
    { term: 'Correspondence', expected_count: 1, expected_ids: ["#{ao1_uri}#pui"] },
    { term: 'Astronomical',   expected_count: 1, expected_ids: ["#{ao1_uri}#pui"] },
    { term: 'Observatory',    expected_count: 1, expected_ids: ["#{ao2_uri}#pui"] },
    { term: 'Dublin',         expected_count: 1, expected_ids: ["#{ao2_uri}#pui"] }
  ]

  positive_scenarios.each do |scenario|
    term = scenario[:term]
    expected_count = scenario[:expected_count]
    expected_ids = scenario[:expected_ids]

    sn = solr_search(CORE_NATIVE, term)
    sc = solr_search(CORE_CANDIDATE, term)
    sf = solr_search(CORE_FALLBACK, term)
    puts "Search '#{term}': Native=#{sn['numFound']}, Candidate=#{sc['numFound']}, Fallback=#{sf['numFound']}"
    raise "Search count zero for expected hit '#{term}'" if sn['numFound'] == 0
    raise "Search count mismatch for '#{term}' (expected #{expected_count}, got Native=#{sn['numFound']})" unless sn['numFound'] == expected_count
    raise "Search count mismatch across cores for '#{term}'" unless sn['numFound'] == sc['numFound'] && sn['numFound'] == sf['numFound']
    raise "Search IDs mismatch for Native on '#{term}'" unless sn['ids'] == expected_ids
    raise "Search IDs mismatch for Candidate on '#{term}'" unless sc['ids'] == expected_ids
    raise "Search IDs mismatch for Fallback on '#{term}'" unless sf['ids'] == expected_ids
  end

  # Negative query assertion: 'Scientific' exists only on unindexed resource fixture, so must yield 0 across all 3 cores
  ['Scientific', 'NonexistentArchivalTerm'].each do |neg_term|
    sn = solr_search(CORE_NATIVE, neg_term)
    sc = solr_search(CORE_CANDIDATE, neg_term)
    sf = solr_search(CORE_FALLBACK, neg_term)
    puts "Negative Search '#{neg_term}': Native=#{sn['numFound']}, Candidate=#{sc['numFound']}, Fallback=#{sf['numFound']}"
    raise "Expected zero hits for '#{neg_term}', but Native got #{sn['numFound']}" unless sn['numFound'] == 0
    raise "Expected zero hits for '#{neg_term}', but Candidate got #{sc['numFound']}" unless sc['numFound'] == 0
    raise "Expected zero hits for '#{neg_term}', but Fallback got #{sf['numFound']}" unless sf['numFound'] == 0
  end

  # Negative Search / Absence checks with public filter: publish:true AND suppressed:false
  public_fq = 'publish:true AND suppressed:false'
  all_public_native = solr_search(CORE_NATIVE, '*:*', fq: public_fq)
  all_public_cand = solr_search(CORE_CANDIDATE, '*:*', fq: public_fq)
  all_public_fallback = solr_search(CORE_FALLBACK, '*:*', fq: public_fq)

  # Check that unpublished & suppressed IDs are absent from public search
  unpub_id = "#{ao_unpub_uri}#pui"
  suppressed_id = "#{ao_suppressed_uri}#pui"

  [all_public_native, all_public_cand, all_public_fallback].each_with_index do |search_res, idx|
    core_label = ['Native', 'Candidate', 'Fallback'][idx]
    raise "Unpublished record leaked in public search for #{core_label}" if search_res['ids'].include?(unpub_id)
    raise "Suppressed record leaked in public search for #{core_label}" if search_res['ids'].include?(suppressed_id)
  end

  # Direct term queries for unpublished and suppressed terms under public filter
  ['Diary', 'Secret'].each do |term|
    [CORE_NATIVE, CORE_CANDIDATE, CORE_FALLBACK].each do |core|
      res = solr_search(core, term, fq: public_fq)
      raise "Negative search violation: Public search for '#{term}' on #{core} returned #{res['numFound']} results (expected 0)" unless res['numFound'] == 0
    end
  end
  puts "Search Invariants & Suppressed/Unpublished Absences: PASS"

  # Step 8: Save Final Verified Evidence
  evidence = {
    'run_id' => RUN_ID,
    'timestamp' => Time.now.utc.iso8601,
    'jruby_description' => RUBY_DESCRIPTION,
    'solr_base' => SOLR_BASE,
    'cores' => {
      'native' => CORE_NATIVE,
      'candidate' => CORE_CANDIDATE,
      'fallback' => CORE_FALLBACK
    },
    'benchmark' => {
      'sample_size' => synthetic_records.length,
      'repeats' => 3,
      'native_median_ms' => native_bench['median_ms'],
      'native_raw_timings' => native_bench['raw_timings'],
      'candidate_median_ms' => candidate_bench['median_ms'],
      'candidate_raw_timings' => candidate_bench['raw_timings'],
      'fallback_median_ms' => fallback_bench['median_ms'],
      'fallback_raw_timings' => fallback_bench['raw_timings'],
      'candidate_to_native_ratio' => (candidate_bench['median_ms'] / native_bench['median_ms']).round(2)
    },
    'sidecar_verification' => {
      'requests_delta' => delta,
      'builder_calls' => inst['builder_calls'],
      'builder_projections_received' => inst['builder_projections_received'],
      'accepted_routing' => inst['accepted_routing'],
      'accepted_notes' => inst['accepted_notes'],
      'accepted_final' => inst['accepted_final'],
      'accepted_common_helpers' => inst['accepted_common_helpers'],
      'rejections' => inst['rejections']
    },
    'conversions_exercised' => {
      'pui_routing_projection' => true,
      'pui_notes_projection' => true,
      'pui_final_document_preparation' => true,
      'pui_common_metadata' => true,
      'pui_record_inheritance_candidate' => true,
      'staff_conversions' => 'NOT_EXERCISED (STAFF indexing is out of scope for PUI stage)',
      'backend_db_reindex' => 'NOT_EXERCISED (Fixture-fed PUIIndexer#index_records milestone)'
    },
    'status' => 'fixture_fed_pui_stage_verified',
    'scope_notes' => 'PUIIndexer fixture-fed integration verified with live Solr cores and spawned sidecar. Full backend database reindexing remains separate milestone.'
  }

  evidence_path = File.join(REPORT_DIR, "stage_run_#{RUN_ID}.json")
  File.write(evidence_path, JSON.pretty_generate(evidence))
  puts "\nEvidence saved to: #{evidence_path}"
end
