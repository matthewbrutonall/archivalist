# frozen_string_literal: true

# Installs opt-in ArchivalistProfiler wrappers on the real indexing path. Only the stage harness
# requires this (ARCHIVALIST_PROFILE=1). Requires ArchivesSpace indexer/common libs to be loaded.
#
# Attribution categories (exclusive time, nested calls subtracted):
#   sidecar_http_incl_rust_compute  - Net::HTTP round trip to the sidecar (loopback transport,
#                                     socket, server-side Rust compute); NOT separable from each
#                                     other without server-side timing -> reported together.
#   json_encode / json_decode       - ASUtils.to_json / ASUtils.json_parse (all callers).
#   clone_copy                      - deep copies of records/candidates/JSON values.
#   native_reference_recompute      - Ruby recomputation of expected values used to validate Rust
#                                     output (inheritance merge/plan, expected enums, subjects...).
#   strict_validation               - strict key/type/value validators and recursive JSON equality.
#   privacy_scan                    - published-string extraction used for privacy checks.
#   rust_input_construction         - rust_index_documents Ruby work excluding wrapped callees.
#   tree_traversal_auxiliary        - add_infscroll_docs / LargeTreeDocIndexer (excl. backend HTTP).
#   backend_http                    - JSONModel::HTTP.get_json calls (source backend reads).
#   solr_http                       - IndexerCommon#do_http_request (Solr update/delete posts).
#   indexer_orchestration           - index_records/index_batch/apply_* exclusive remainder.
# Anything else inside the timed region lands in `unwrapped_remainder_in_region` and is NOT
# attributed (native document rules, IndexBatch streaming, GC, JIT, etc.).

require_relative 'archivalist_profiler'

module ArchivalistProfileWrappers
  CATEGORIES = {
    'sidecar_http' => 'sidecar_http_incl_rust_compute',
    'json_encode' => 'json_encode',
    'json_decode' => 'json_decode',
    'clone_record_candidate' => 'clone_copy',
    'clone_inheritance_record' => 'clone_copy',
    'clone_builder_value' => 'clone_copy',
    'inheritance_merge' => 'native_reference_recompute',
    'inheritance_build_plan' => 'native_reference_recompute',
    'expected_recompute' => 'native_reference_recompute',
    'strict_validation' => 'strict_validation',
    'privacy_scan' => 'privacy_scan',
    'sanitize_json' => 'rust_input_construction',
    'rust_input_construction' => 'rust_input_construction',
    'builder_build' => 'builder_build_exclusive_prelude_and_response_handling',
    'builder_build_pui_auxiliary' => 'builder_build_exclusive_prelude_and_response_handling',
    'tree_traversal' => 'tree_traversal_auxiliary',
    'backend_http' => 'backend_http',
    'solr_http' => 'solr_http',
    'indexer_orchestration' => 'indexer_orchestration'
  }.freeze

  def self.install!(log: ->(m) { puts m })
    p = ArchivalistProfiler
    skipped = []
    attempt = lambda do |target, meth, label, **opts|
      begin
        p.wrap(target, meth, label, **opts)
      rescue ArgumentError, NameError => e
        skipped << "#{target}##{meth} (#{e.class})"
      end
    end

    str_bytes = ->(s) { s.respond_to?(:bytesize) ? s.bytesize : 0 }

    attempt.call(ASUtils, :to_json, 'json_encode', singleton: true, bytes: ->(_a, r) { str_bytes.call(r) })
    attempt.call(ASUtils, :json_parse, 'json_decode', singleton: true, bytes: ->(a, _r) { str_bytes.call(a[0]) })
    attempt.call(JSONModel::HTTP, :get_json, 'backend_http', singleton: true) if defined?(JSONModel::HTTP)

    attempt.call(IndexerCommon, :index_records, 'indexer_orchestration', outermost_only: true)
    attempt.call(IndexerCommon, :index_batch, 'indexer_orchestration', outermost_only: true)
    attempt.call(IndexerCommon, :rust_index_documents, 'rust_input_construction')
    attempt.call(IndexerCommon, :sanitize_json, 'sanitize_json', outermost_only: true)
    attempt.call(IndexerCommon, :deep_copy_record_candidate, 'clone_record_candidate', outermost_only: true)
    attempt.call(IndexerCommon, :do_http_request, 'solr_http',
                 bytes: ->(a, _r) { req = a[1]; (req['Content-Length'] || req.body.to_s.bytesize).to_i })
    %i[valid_dynamic_enum_and_ark_projection? valid_accession_date_projection?].each do |m|
      attempt.call(IndexerCommon, m, 'strict_validation')
    end
    attempt.call(IndexerCommon, :strict_json_equal?, 'strict_validation', outermost_only: true)
    attempt.call(IndexerCommon, :compute_expected_dynamic_enums, 'expected_recompute')
    attempt.call(IndexerCommon, :apply_rust_pui_availability_plan, 'indexer_orchestration')

    %i[apply_rust_pui_routing_projection apply_rust_pui_notes_projection
       apply_rust_pui_final_projection apply_rust_pui_common_helper].each do |m|
      attempt.call(PUIIndexer, m, 'indexer_orchestration')
    end
    attempt.call(PUIIndexer, :add_infscroll_docs, 'tree_traversal')
    attempt.call(LargeTreeDocIndexer, :add_largetree_docs, 'tree_traversal')

    attempt.call(RecordInheritance, :merge, 'inheritance_merge')
    attempt.call(RecordInheritance, :build_plan, 'inheritance_build_plan')
    attempt.call(RecordInheritance, :deep_copy_record, 'clone_inheritance_record', outermost_only: true)

    b = RustIndexDocumentBuilder
    # build/build_pui_auxiliary are deliberately NOT prepended: the stage harness captures
    # `instance_method(:build)` and re-defines it, and a prepended wrapper would make that
    # capture recurse. The harness measures them via ArchivalistProfileWrappers.measure instead.
    attempt.call(b, :perform_request, 'sidecar_http',
                 bytes: ->(a, r) { a[0].body.to_s.bytesize + (r.respond_to?(:body) ? r.body.to_s.bytesize : 0) })
    names = b.private_instance_methods(false) + b.public_instance_methods(false)
    names.uniq.sort.each do |m|
      s = m.to_s
      case s
      when /\Avalidate_/ then attempt.call(b, m, 'strict_validation', outermost_only: true)
      when /\Acompute_expected_/, /\Areconstruct_/ then attempt.call(b, m, 'expected_recompute')
      when 'deep_copy_json_value', /\Astrip_fullrecord_/ then attempt.call(b, m, 'clone_builder_value', outermost_only: true)
      when /\Aextract_published_strings\z/, /\Aextract_string_values_published_only\z/ then attempt.call(b, m, 'privacy_scan')
      end
    end

    log.call("[Profiler] wrappers installed: #{p.installed.length}; skipped (undefined): #{skipped.inspect}")
    { 'installed' => p.installed.map { |i| "#{i[:target]}#{i[:singleton] ? '.' : '#'}#{i[:method]}=>#{i[:label]}" },
      'skipped' => skipped }
  end

  def self.measure(label)
    return yield unless ArchivalistProfiler.active?

    ArchivalistProfiler.aggregator.measure(label) { yield }
  end

  def self.median(values)
    s = values.sort
    s.empty? ? nil : s[s.length / 2]
  end
end
