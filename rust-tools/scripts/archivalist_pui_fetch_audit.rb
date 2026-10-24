# frozen_string_literal: true

# Opt-in PUI archival_object backend batch GET fetch accounting (PUI_FETCH_AUDIT=1).
#
# Observational only: quantifies actual backend `/repositories/:repo_id/archival_objects`
# batch GET calls (with non-empty `id_set`, excluding `all_ids`), response-body bytes,
# summed call duration, and overlap of ID sets requested by successful HTTP 200 calls
# between PUI ordinary indexing (`PUIIndexer#fetch_records`) and PUI tree discovery
# (`PUIIndexer#index_round_complete`). It does not parse JSON response bodies to verify
# which requested records were returned (for example, an HTTP 200 response with `[]` or
# omitted deleted/unpublished IDs still counts those IDs as requested by a successful call).
#
# Caveat: this accounting is strictly observational and CANNOT imply safe cross-phase cache
# reuse or deduplication in production indexing, because records and ancestor trees may change
# in the backend between ordinary indexing and tree discovery or across checkpoints.
#
# Privacy and safety:
# - Default off installs no wrappers and introduces no measurable path change.
# - Only runs during an explicitly bracketed PUI pass (`start!` .. `stop!`), never during
#   pre-pass inventory or STAFF passes.
# - Classifies phase by execution context (`PUIIndexer#fetch_records` vs
#   `PUIIndexer#index_round_complete`), not merely `resolve[]` query values.
# - Thread-safe across worker threads; scopes ID sets per round and per repository.
# - Accounts for successful (HTTP 200) and failed/retried calls separately; failed calls
#   are excluded from `unique_successful_requested_ids` and overlap sets.
# - Retains only integer IDs and numeric aggregates; never retains or logs record bodies,
#   session tokens, headers, or credentials.

require 'set'
require 'uri'

module ArchivalistPuiFetchAudit
  PHASE_ORDINARY = 'ordinary_indexing'
  PHASE_TREE_DISCOVERY = 'tree_discovery'
  PHASES = [PHASE_ORDINARY, PHASE_TREE_DISCOVERY].freeze

  ARCHIVAL_OBJECTS_PATH_RE = %r{\A/repositories/(\d+)/archival_objects\z}.freeze

  STATE = {
    active: false,
    installed: false,
    repositories: {}
  }
  LOCK = Mutex.new

  def self.now
    Process.clock_gettime(Process::CLOCK_MONOTONIC)
  end

  def self.enabled?(env = ENV)
    env['PUI_FETCH_AUDIT'].to_s.strip == '1'
  end

  def self.active?
    STATE[:active]
  end

  def self.current_phase
    stack = Thread.current[:archivalist_pui_fetch_audit_phases]
    stack && !stack.empty? ? stack.last : nil
  end

  def self.with_phase(phase)
    return yield unless STATE[:active]

    stack = (Thread.current[:archivalist_pui_fetch_audit_phases] ||= [])
    stack.push(phase)
    begin
      yield
    ensure
      stack.pop
    end
  end

  module PuiIndexerHook
    def fetch_records(*args, **kwargs, &blk)
      return super unless ArchivalistPuiFetchAudit.active?

      ArchivalistPuiFetchAudit.with_phase(ArchivalistPuiFetchAudit::PHASE_ORDINARY) do
        super(*args, **kwargs, &blk)
      end
    end

    def index_round_complete(*args, **kwargs, &blk)
      return super unless ArchivalistPuiFetchAudit.active?

      ArchivalistPuiFetchAudit.with_phase(ArchivalistPuiFetchAudit::PHASE_TREE_DISCOVERY) do
        super(*args, **kwargs, &blk)
      end
    end
  end

  module JsonModelHttpHook
    def get_response(url, *rest, &blk)
      phase = ArchivalistPuiFetchAudit.current_phase
      return super unless ArchivalistPuiFetchAudit.active? && phase

      target = ArchivalistPuiFetchAudit.extract_archival_object_batch_target(url)
      return super unless target

      reentry_key = :archivalist_pui_fetch_audit_in_get_response
      return super if Thread.current[reentry_key]

      Thread.current[reentry_key] = true
      started = ArchivalistPuiFetchAudit.now
      begin
        response = super
      rescue Exception
        elapsed_ms = (ArchivalistPuiFetchAudit.now - started) * 1000.0
        ArchivalistPuiFetchAudit.record_call(
          phase: phase,
          repo_id: target[:repo_id],
          requested_ids: target[:ids],
          success: false,
          response_bytes: 0,
          duration_ms: elapsed_ms
        )
        raise
      ensure
        Thread.current[reentry_key] = false
      end

      elapsed_ms = (ArchivalistPuiFetchAudit.now - started) * 1000.0
      ok = ArchivalistPuiFetchAudit.successful_response?(response)
      body_bytes = ArchivalistPuiFetchAudit.response_body_bytes(response)
      ArchivalistPuiFetchAudit.record_call(
        phase: phase,
        repo_id: target[:repo_id],
        requested_ids: target[:ids],
        success: ok,
        response_bytes: body_bytes,
        duration_ms: elapsed_ms
      )
      response
    end
  end

  def self.install!(pui_indexer:, jsonmodel_http:)
    pui_indexer.prepend(PuiIndexerHook) unless pui_indexer.ancestors.include?(PuiIndexerHook)
    jsonmodel_http.singleton_class.prepend(JsonModelHttpHook) unless jsonmodel_http.singleton_class.ancestors.include?(JsonModelHttpHook)
    LOCK.synchronize { STATE[:installed] = true }
    {
      'PUIIndexer' => %w[fetch_records index_round_complete],
      'JSONModel::HTTP' => %w[get_response]
    }
  end

  def self.successful_response?(response)
    return false if response.nil?
    return true if defined?(Net::HTTPSuccess) && response.is_a?(Net::HTTPSuccess)

    response.respond_to?(:code) && response.code.to_s == '200'
  end

  def self.response_body_bytes(response)
    return 0 unless response.respond_to?(:body)

    body = response.body
    body.is_a?(String) ? body.bytesize : 0
  end

  # Parses `url` (URI or String) and returns `{ repo_id: Integer, ids: Array<Integer> }`
  # ONLY when the path is `/repositories/:repo_id/archival_objects`, `all_ids` is not set,
  # and `id_set` contains at least one integer ID.
  def self.extract_archival_object_batch_target(url)
    uri = case url
          when URI then url
          when String
            begin
              URI(url)
            rescue URI::InvalidURIError
              return nil
            end
          else
            return nil
          end

    path = uri.path.to_s
    match = ARCHIVAL_OBJECTS_PATH_RE.match(path)
    return nil unless match

    query = uri.query.to_s
    return nil if query.empty?

    params = URI.decode_www_form(query)
    has_all_ids = params.any? do |k, v|
      k == 'all_ids' && !v.to_s.strip.empty? && !%w[false 0].include?(v.to_s.strip.downcase)
    end
    return nil if has_all_ids

    id_set_values = params.select { |k, _| k == 'id_set' }.map { |_, v| v.to_s }
    return nil if id_set_values.empty?

    ids = []
    id_set_values.each do |raw|
      raw.split(',').each do |token|
        stripped = token.strip
        next if stripped.empty?
        return nil unless /\A\d+\z/.match?(stripped)

        ids << stripped.to_i
      end
    end
    return nil if ids.empty?

    repo_id = match[1].to_i
    { repo_id: repo_id, ids: ids }
  rescue ArgumentError
    nil
  end

  def self.new_phase_bucket
    {
      calls: 0,
      successful_calls: 0,
      failed_calls: 0,
      requested_ids_total: 0,
      successful_requested_ids_total: 0,
      failed_requested_ids_total: 0,
      unique_successful_requested_ids: Set.new,
      unique_failed_requested_ids: Set.new,
      response_bytes: 0,
      successful_response_bytes: 0,
      failed_response_bytes: 0,
      call_duration_sum_ms: 0.0,
      successful_call_duration_sum_ms: 0.0,
      failed_call_duration_sum_ms: 0.0
    }
  end

  def self.record_call(phase:, repo_id:, requested_ids:, success:, response_bytes:, duration_ms:)
    LOCK.synchronize do
      return unless STATE[:active]

      repo_buckets = (STATE[:repositories][repo_id] ||= PHASES.to_h { |p| [p, new_phase_bucket] })
      bucket = (repo_buckets[phase] ||= new_phase_bucket)
      count = requested_ids.length
      bytes = response_bytes.to_i
      dur = duration_ms.to_f

      bucket[:calls] += 1
      bucket[:requested_ids_total] += count
      bucket[:response_bytes] += bytes
      bucket[:call_duration_sum_ms] += dur

      if success
        bucket[:successful_calls] += 1
        bucket[:successful_requested_ids_total] += count
        bucket[:successful_response_bytes] += bytes
        bucket[:successful_call_duration_sum_ms] += dur
        requested_ids.each { |id| bucket[:unique_successful_requested_ids].add(id) }
      else
        bucket[:failed_calls] += 1
        bucket[:failed_requested_ids_total] += count
        bucket[:failed_response_bytes] += bytes
        bucket[:failed_call_duration_sum_ms] += dur
        requested_ids.each { |id| bucket[:unique_failed_requested_ids].add(id) }
      end
    end
  end

  def self.start!
    LOCK.synchronize do
      STATE[:repositories] = {}
      STATE[:active] = true
    end
  end

  def self.stop!
    repos = LOCK.synchronize do
      STATE[:active] = false
      taken = STATE[:repositories]
      STATE[:repositories] = {}
      taken
    end
    summarize(repos)
  end

  def self.capture
    start!
    result = yield
    [result, stop!]
  ensure
    stop! if STATE[:active]
  end

  def self.format_phase_bucket(bucket)
    unique_count = bucket[:unique_successful_requested_ids].length
    succ_req = bucket[:successful_requested_ids_total]
    {
      'calls' => bucket[:calls],
      'successful_calls' => bucket[:successful_calls],
      'failed_calls' => bucket[:failed_calls],
      'requested_ids_total' => bucket[:requested_ids_total],
      'successful_requested_ids_total' => succ_req,
      'failed_requested_ids_total' => bucket[:failed_requested_ids_total],
      'unique_successful_requested_ids' => unique_count,
      'repeated_successful_requested_ids' => succ_req - unique_count,
      'unique_failed_requested_ids' => bucket[:unique_failed_requested_ids].length,
      'response_bytes' => bucket[:response_bytes],
      'successful_response_bytes' => bucket[:successful_response_bytes],
      'failed_response_bytes' => bucket[:failed_response_bytes],
      'call_duration_sum_ms' => bucket[:call_duration_sum_ms].round(3),
      'successful_call_duration_sum_ms' => bucket[:successful_call_duration_sum_ms].round(3),
      'failed_call_duration_sum_ms' => bucket[:failed_call_duration_sum_ms].round(3),
      'duration_kind' => 'summed call time across threads (not wall time)'
    }
  end

  def self.compute_overlap(ord_set, tree_set)
    inter = (ord_set & tree_set).length
    union = (ord_set | tree_set).length
    ord_only = (ord_set - tree_set).length
    tree_only = (tree_set - ord_set).length
    {
      'ordinary_unique_successful_requested_ids' => ord_set.length,
      'tree_discovery_unique_successful_requested_ids' => tree_set.length,
      'intersection_successful_requested_ids' => inter,
      'union_unique_successful_requested_ids' => union,
      'ordinary_only_successful_requested_ids' => ord_only,
      'tree_discovery_only_successful_requested_ids' => tree_only,
      'tree_discovery_overlap_ratio' => tree_set.empty? ? 0.0 : (inter.to_f / tree_set.length).round(4),
      'ordinary_overlap_ratio' => ord_set.empty? ? 0.0 : (inter.to_f / ord_set.length).round(4)
    }
  end

  def self.merge_phase_buckets(buckets)
    merged = new_phase_bucket
    buckets.each do |b|
      merged[:calls] += b[:calls]
      merged[:successful_calls] += b[:successful_calls]
      merged[:failed_calls] += b[:failed_calls]
      merged[:requested_ids_total] += b[:requested_ids_total]
      merged[:successful_requested_ids_total] += b[:successful_requested_ids_total]
      merged[:failed_requested_ids_total] += b[:failed_requested_ids_total]
      merged[:unique_successful_requested_ids].merge(b[:unique_successful_requested_ids])
      merged[:unique_failed_requested_ids].merge(b[:unique_failed_requested_ids])
      merged[:response_bytes] += b[:response_bytes]
      merged[:successful_response_bytes] += b[:successful_response_bytes]
      merged[:failed_response_bytes] += b[:failed_response_bytes]
      merged[:call_duration_sum_ms] += b[:call_duration_sum_ms]
      merged[:successful_call_duration_sum_ms] += b[:successful_call_duration_sum_ms]
      merged[:failed_call_duration_sum_ms] += b[:failed_call_duration_sum_ms]
    end
    merged
  end

  def self.summarize(repos)
    sorted_repo_ids = repos.keys.sort
    by_repo = {}
    ord_buckets = []
    tree_buckets = []
    round_intersection = 0
    round_union = 0
    round_ord_only = 0
    round_tree_only = 0
    round_ord_unique = 0
    round_tree_unique = 0

    sorted_repo_ids.each do |repo_id|
      phase_map = repos[repo_id]
      ord_b = phase_map[PHASE_ORDINARY] || new_phase_bucket
      tree_b = phase_map[PHASE_TREE_DISCOVERY] || new_phase_bucket
      ord_buckets << ord_b
      tree_buckets << tree_b

      overlap = compute_overlap(ord_b[:unique_successful_requested_ids], tree_b[:unique_successful_requested_ids])
      round_intersection += overlap['intersection_successful_requested_ids']
      round_union += overlap['union_unique_successful_requested_ids']
      round_ord_only += overlap['ordinary_only_successful_requested_ids']
      round_tree_only += overlap['tree_discovery_only_successful_requested_ids']
      round_ord_unique += overlap['ordinary_unique_successful_requested_ids']
      round_tree_unique += overlap['tree_discovery_unique_successful_requested_ids']

      by_repo[repo_id.to_s] = {
        'repository_id' => repo_id,
        'phases' => {
          PHASE_ORDINARY => format_phase_bucket(ord_b),
          PHASE_TREE_DISCOVERY => format_phase_bucket(tree_b)
        },
        'overlap' => overlap
      }
    end

    merged_ord = merge_phase_buckets(ord_buckets)
    merged_tree = merge_phase_buckets(tree_buckets)
    ord_formatted = format_phase_bucket(merged_ord)
    tree_formatted = format_phase_bucket(merged_tree)
    # Use per-repository scoped sets for round totals when IDs are repository-scoped,
    # and also merge sets directly (archival_object IDs are globally unique in ASpace DB,
    # while summing per-repository set sizes preserves strict repository scoping).
    ord_formatted['unique_successful_requested_ids'] = round_ord_unique
    ord_formatted['repeated_successful_requested_ids'] = ord_formatted['successful_requested_ids_total'] - round_ord_unique
    tree_formatted['unique_successful_requested_ids'] = round_tree_unique
    tree_formatted['repeated_successful_requested_ids'] = tree_formatted['successful_requested_ids_total'] - round_tree_unique

    round_overlap = {
      'ordinary_unique_successful_requested_ids' => round_ord_unique,
      'tree_discovery_unique_successful_requested_ids' => round_tree_unique,
      'intersection_successful_requested_ids' => round_intersection,
      'union_unique_successful_requested_ids' => round_union,
      'ordinary_only_successful_requested_ids' => round_ord_only,
      'tree_discovery_only_successful_requested_ids' => round_tree_only,
      'tree_discovery_overlap_ratio' => round_tree_unique.zero? ? 0.0 : (round_intersection.to_f / round_tree_unique).round(4),
      'ordinary_overlap_ratio' => round_ord_unique.zero? ? 0.0 : (round_intersection.to_f / round_ord_unique).round(4)
    }

    {
      'record_type' => 'archival_object',
      'id_accounting_basis' => 'requested_by_http_200_calls (response bodies are not parsed; HTTP 200 with [] or omitted records still counts requested IDs)',
      'note' => 'Observational accounting of backend archival_objects batch GETs (id_set) only. ' \
                'ID counts and overlap reflect IDs requested by successful HTTP 200 calls (failed calls excluded), ' \
                'not parsed response-body records. Cannot imply safe cache reuse or deduplication across phases ' \
                'because records and ancestor trees may change between ordinary indexing and tree discovery or across checkpoints. ' \
                'call_duration_sum_ms is summed call time across threads, not wall time.',
      'phases' => {
        PHASE_ORDINARY => ord_formatted,
        PHASE_TREE_DISCOVERY => tree_formatted
      },
      'overlap' => round_overlap,
      'by_repository' => by_repo
    }
  end

  def self.median_scalar(values)
    sorted = values.sort
    mid = sorted.length / 2
    sorted.length.odd? ? sorted[mid] : ((sorted[mid - 1] + sorted[mid]) / 2.0).round(4)
  end

  def self.median_phase(phase_hashes)
    numeric_keys = %w[
      calls successful_calls failed_calls
      requested_ids_total successful_requested_ids_total failed_requested_ids_total
      unique_successful_requested_ids repeated_successful_requested_ids unique_failed_requested_ids
      response_bytes successful_response_bytes failed_response_bytes
      call_duration_sum_ms successful_call_duration_sum_ms failed_call_duration_sum_ms
    ]
    out = numeric_keys.to_h do |k|
      vals = phase_hashes.map { |h| h[k] }
      med = median_scalar(vals)
      [k, k.end_with?('_ms') ? med.round(3) : med]
    end
    out['duration_kind'] = 'summed call time across threads (not wall time)'
    out
  end

  def self.median_overlap(overlaps)
    keys = %w[
      ordinary_unique_successful_requested_ids tree_discovery_unique_successful_requested_ids
      intersection_successful_requested_ids union_unique_successful_requested_ids
      ordinary_only_successful_requested_ids tree_discovery_only_successful_requested_ids
      tree_discovery_overlap_ratio ordinary_overlap_ratio
    ]
    keys.to_h do |k|
      [k, median_scalar(overlaps.map { |o| o[k] })]
    end
  end

  def self.median_summary(summaries)
    return nil if summaries.empty?

    {
      'record_type' => 'archival_object',
      'id_accounting_basis' => summaries.first['id_accounting_basis'],
      'note' => summaries.first['note'],
      'phases' => {
        PHASE_ORDINARY => median_phase(summaries.map { |s| s.dig('phases', PHASE_ORDINARY) }),
        PHASE_TREE_DISCOVERY => median_phase(summaries.map { |s| s.dig('phases', PHASE_TREE_DISCOVERY) })
      },
      'overlap' => median_overlap(summaries.map { |s| s['overlap'] })
    }
  end
end
