# frozen_string_literal: true

# In-container half of the profiling-stack identity check (see profiling_stack.py).
#
# profiling_stack.py verify writes a fresh nonce directly into the profiling database and proves
# the stack's backend returns it; the wrapper then passes the nonce here. Before a tool writes or
# indexes anything, this re-checks the SAME nonce through the backend URL the tool will actually
# use (an anonymous GET, so nothing is written to whatever backend that URL reaches) and requires
# the stack's identity core in the Solr the tool will write to. Any mismatch raises.

require 'json'
require 'net/http'
require 'uri'

module ArchivalistProfilingStack
  IDENTITY_NAME_PREFIX = 'archivalist profiling identity '
  ENV_KEYS = %w[
    ARCHIVALIST_PROFILING_STACK_ID ARCHIVALIST_PROFILING_STACK_NONCE
    ARCHIVALIST_PROFILING_STACK_IDENTITY_REPO_ID ARCHIVALIST_PROFILING_STACK_IDENTITY_CORE
  ].freeze

  def self.http_get_json(url)
    res = Net::HTTP.get_response(URI(url))
    raise "GET #{url} returned HTTP #{res.code}" unless res.is_a?(Net::HTTPSuccess)

    JSON.parse(res.body)
  end

  # Returns the verified identity; raises unless backend_url and solr_url belong to the stack.
  # `get_json` is injectable for tests.
  def self.verify!(env:, backend_url:, solr_url:, get_json: method(:http_get_json))
    missing = ENV_KEYS.reject { |k| env[k].to_s.strip != '' }
    unless missing.empty?
      raise "Safety violation: profiling-stack identity missing (#{missing.join(', ')}); " \
            'run through run_native_indexing_profile.sh with STACK_STATE from profiling_stack.py setup'
    end
    stack_id = env['ARCHIVALIST_PROFILING_STACK_ID']
    nonce = env['ARCHIVALIST_PROFILING_STACK_NONCE']
    repo_id = env['ARCHIVALIST_PROFILING_STACK_IDENTITY_REPO_ID']
    core = env['ARCHIVALIST_PROFILING_STACK_IDENTITY_CORE']
    raise 'Safety violation: malformed identity repo id' unless repo_id =~ /\A[1-9]\d*\z/
    raise 'Safety violation: malformed identity core' unless core == "archivalist_profiling_identity_#{stack_id}"

    begin
      repo = get_json.call("#{backend_url.chomp('/')}/repositories/#{repo_id}")
    rescue StandardError => e
      raise "Safety violation: backend #{backend_url} did not return the identity record (#{e.class}: #{e.message})"
    end
    unless repo['name'] == IDENTITY_NAME_PREFIX + nonce && repo['repo_code'] == "ARCHPROF-#{stack_id}"
      raise "Safety violation: backend #{backend_url} is NOT connected to profiling stack #{stack_id} (identity mismatch)"
    end

    begin
      status = get_json.call("#{solr_url.chomp('/')}/admin/cores?action=STATUS&core=#{core}&wt=json")
    rescue StandardError => e
      raise "Safety violation: Solr #{solr_url} unreachable (#{e.class}: #{e.message})"
    end
    core_status = (status['status'] || {})[core]
    if core_status.nil? || core_status.empty?
      raise "Safety violation: Solr #{solr_url} is NOT the profiling stack's Solr (identity core #{core} missing)"
    end

    { 'stack_id' => stack_id, 'identity_repo_id' => repo_id.to_i, 'identity_core' => core, 'backend_url' => backend_url, 'solr_url' => solr_url }
  end
end
