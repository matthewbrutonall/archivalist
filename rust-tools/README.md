# Archivalist Rust Tools

This workspace contains command-line and sidecar tools for validating, measuring,
and accelerating selected archival-management operations.

## Components

- `aspace-client`: typed HTTP client, validation models, and shadow-index query support.
- `aspace-cli`: command-line validation, performance probes, shadow-index utilities,
  and the `aspace-shadow-sidecar` binary.

The application API remains authoritative. The tools do not write directly to the
application database or primary Solr collection. The index-document endpoint only
returns prepared JSON to the Ruby indexer, which remains the sole Solr writer.

## Build and test

From this directory:

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

## CLI configuration

Commands that access the application API accept a base URL and authentication
parameters. Use environment-specific credentials; never commit passwords or session
tokens.

```bash
cargo run -p aspace-cli -- --url http://localhost:4567 --help
cargo run -p aspace-cli -- --url http://localhost:4567 validate --help
cargo run -p aspace-cli -- --url http://localhost:4567 perf --help
cargo run -p aspace-cli -- --url http://localhost:4567 shadow-index --help
cargo run -p aspace-cli -- shadow-query --help
```

Validation commands support human-readable output, JSON, and CSV. CSV output can be
written to a file with `--output` where the selected command supports it.

Use your own repository and resource identifiers in commands:

```bash
cargo run -p aspace-cli -- --url http://localhost:4567 \
  validate resources --repo REPOSITORY_ID --json

cargo run -p aspace-cli -- --url http://localhost:4567 \
  validate archival-objects --repo REPOSITORY_ID --csv --output issues.csv

cargo run -p aspace-cli -- --url http://localhost:4567 \
  perf health --repo REPOSITORY_ID --json
```

## Shadow index

Shadow-index commands use a separate Solr collection. The default query endpoint is
`http://localhost:8985/solr`, and the default shadow collection is
`archivesspace_shadow`.

```bash
cargo run -p aspace-cli -- shadow-query search \
  --q SEARCH_TERM --resource-uri /repositories/REPOSITORY_ID/resources/RESOURCE_ID \
  --rows 10 --json

cargo run -p aspace-cli -- shadow-query descendants \
  --resource-uri /repositories/REPOSITORY_ID/resources/RESOURCE_ID \
  --rows 10 --tree --include-ancestors
```

The query layer escapes user-controlled identifiers and filters, limits requested
row counts, and rejects the primary collection before issuing a network request.

## Sidecar

Start the long-running loopback service with:

```bash
cargo run -p aspace-cli --bin aspace-shadow-sidecar -- \
  --host 127.0.0.1 --port 4570
```

Endpoints:

- `GET /health`: process and configuration liveness.
- `GET /ready`: bounded dependency check against the configured shadow collection.
- `GET /metrics`: Prometheus-format uptime and request counters.
- `GET /shadow/search`: keyword search with optional resource and date filters.
- `GET /shadow/descendants`: descendant retrieval with optional tree assembly.
- `GET /shadow/resource-tree/waypoint`: resource-tree waypoint lookup.
- `GET /shadow/resource-tree/node-from-root`: batched root-to-node path lookup.
- `POST /index/documents`: bounded preparation of common Solr document fields and,
  when enrichment is requested, staff-index mappings for resources, archival
  objects, accessions, digital objects and components, subjects, classifications,
  repositories, events, locations, container profiles, location profiles,
  assessments, top containers, jobs (deterministic staff metadata), synthetic
  collection-management child documents for valid parent records, all
  agent authority types, staff-index dynamic-enum projection (`<field>_enum_s`
  and `type_enum_s`), ARK field preparation (`ark_name`), and accession-date
  staff enrichment (`accession_date` and `accession_date_year` with sentinel
  `9999-12-31` handling).

The example systemd unit in `deploy/` binds to loopback by default. Put authentication
and transport security in front of the sidecar before exposing it beyond the local
host.

`--index-timing` (or `ARCHIVALIST_SIDECAR_INDEX_TIMING=1`) adds a `Server-Timing` header to
successful `/index/documents` responses with `receive`, `parse`, `build`, `serialize` and
`total` durations in milliseconds. `total` runs from the start of reading the request to just
before the response is written, so it excludes writing the response. It is off by default and
never changes the JSON body.

Release archives can be built reproducibly with
`scripts/package_sidecar_release.sh`. See `deploy/CANARY.md` for the staged
non-production rollout, parity gate, outage fallback test, and rollback procedure.
The search, resource-tree, and index-document gates are automated by
`scripts/canary_shadow_search.sh`, `scripts/canary_resource_tree.sh`, and
`scripts/canary_index_document_builder.sh`. Staff-index enrichment parity is
checked at the resource and accession level by `scripts/canary_index_document_enrichment.sh`,
and across complete disposable-repository reindexes by the full STAFF index
parity comparator, `scripts/compare_staff_index_parity.py`. PUI document preparation is gated separately under `AppConfig[:rust_pui_document_preparation_enabled]`
(disabled by default). Inheritance semantics, top-container publication decisions
(`is_linked_to_published_record` / `pui_container`), LargeTree/ordered-record indexing,
and Solr writes remain Ruby-owned. When trusted under capability markers
(`pui_final_document_metadata`, `pui_routing_metadata`, and `pui_notes_metadata`),
Rust prepares deterministic final public JSON privacy preparation (removing ancestors,
top-container `internal_note`, and container-profile `notes` across nested instances)
and published-only fullrecord extraction (`fullrecord_published`), PUI identity/routing metadata
(`id: "#{parent_id}#pui"`, `pui_parent_id: parent_id`, and exact types additions in native order),
and direct-only published note extraction (`notes_published`). Ruby strictly validates exact
values, types, recursive JSON equality, serialization, and published string values at the
corresponding logical hook positions (`configure_doc_rules`, `add_notes`, `final_doc_rules`)
before trusting each projection, falling back completely to unchanged native PUIIndexer
logic on missing, malformed, or mismatched sidecar responses. Job-specific dynamic I18n
translation, local timestamps, and output-file discovery remain native because
localization and external I/O are handled by Ruby. Dynamic-enum schema derivation
and ARK enablement configuration remain in Ruby; Rust handles deterministic recursive
traversal, `_resolved` exclusion, ordering, deduplication, and trim semantics under
strict Ruby validation and complete native fallback. Accession-date staff preparation
for ordinary dates and the sentinel date `9999-12-31` is owned by Rust when trusted
under the distinct `accession_date_metadata` capability marker; localization remains
Ruby-owned (Ruby passes only the minimal localized unknown label `I18n.t('accession.accession_date_unknown')`
byte-for-byte, and Rust preserves it without trimming or translating). Exact native
date parsing semantics are preserved by Ruby computing `Date.parse(date).year` and
passing it as `parsed_accession_date_year` (accepting non-canonical schema-valid dates
such as year 0 and strings with whitespace); Rust requires this integer within valid
signed 32-bit range and projects `accession_date` and `accession_date_year`. Ruby strictly
validates exact fullrecord parity against native extraction before trusting the projection,
falling back completely to native preparation with deterministic `build_fullrecord`
reconstruction (preventing fullrecord pollution or duplication) on missing, malformed,
or mismatched sidecar responses. Collection-management child documents are generated
exclusively for staff enrichment; PUI indexing remains native, and Ruby retains sole
ownership of Solr writes and stale-child deletions. Main index PUI availability
preparation is gated separately under `AppConfig[:rust_pui_availability_preparation_enabled]`
(disabled by default) and applies to the common/main index path (`IndexerCommon#apply_pui_fields`),
remaining completely isolated from `PUIIndexer`. Ruby computes `RecordInheritance.has_type?(record_type)`
and passes it in the batched `/index/documents` payload. Rust emits a narrowly scoped
PUI-availability plan under `_archivalist_pui_availability` with capability marker
`pui_availability_metadata`. `IndexerCommon` strips the projection and marker into a per-URI
batch map before document prepare hooks and plugins run. At the final `apply_pui_fields`
location, Ruby independently validates the plan and reads `doc['publish']` at apply time.
On truthy publish, conditional type additions are appended; for `top_container`, `doc['publish']`
is overridden by `is_linked_to_published_record` and `['pui_container', 'pui']` appended if
truthy. Exact native append order and duplicates (e.g. published top_container linked to a
published record acquiring `['pui_top_container', 'pui', 'pui_container', 'pui']`) and Ruby
truthiness semantics are strictly preserved. If absent, malformed, extra-keyed, or mismatched,
`IndexerCommon` discards the projection and executes unchanged native `apply_pui_fields` with
zero partial mutation. Deterministic PUI auxiliary document preparation is gated separately
under `AppConfig[:rust_pui_auxiliary_documents_enabled]` (disabled by default), migrating
ordered-records synthetic documents (`PUIIndexer#add_infscroll_docs`) and LargeTree synthetic
document envelopes (`tree_root`, `tree_waypoint`, `tree_node`, and `tree_node_from_root`) into
Rust while keeping HTTP traversal, tree recursion, node/path discovery, delete calculations,
batch state, and Solr writes entirely in Ruby. Rust receives already-fetched JSON payloads and
authoritative Ruby-computed identity/parent inputs, returning deterministic candidate documents
with capability marker `pui_auxiliary_document_metadata` (`_archivalist_pui_auxiliary`). Ruby
strictly validates exact key sets, identity, type-strict values (including string publish `"true"`,
`types: ['pui']`), JSON semantic parity, response completeness, duplicate preservation, and ordering
before stripping internal markers and appending to `batch`. On any sidecar unavailability, timeout,
malformed response, or validation failure, Ruby cleanly falls back to native document construction
with zero partial mutation. Deterministic PUI RecordInheritance merge candidate construction for
`PUIIndexer` final-document preparation is gated separately under
`AppConfig[:rust_pui_record_inheritance_candidate_enabled]` (disabled by default). Ruby remains
authoritative for dynamic configuration, evaluating `skip_if` and `inherit_if` Procs with exact native
inputs and truthiness, localized `translate_level` values via I18n, custom/plugin-modified configuration,
repeated rules, direct flags, composite identifier delimiter semantics, and native exception timing
while compiling a serializable plan (`RecordInheritance.build_plan`). Rust executes the deterministic
merge on a deep JSON copy (scalar/array inheritance, first qualifying ancestor selection, array
concatenation/flattening, `_inherited` metadata on hashes vs `<property>_inherited` on scalars, repeated
rules, and composite identifier construction). The merged candidate is projected under
`_archivalist_pui_candidate` (`pui_record_inheritance_candidate_metadata`), retaining `ancestors`, and
feeds the existing PUI final privacy/fullrecord projection (`_archivalist_pui_final`) in the same
`/index/documents` batch request. Ruby independently computes native `RecordInheritance.merge` and
type-strictly validates the Rust candidate; any mismatch, missing/extra key, or sidecar error falls back
cleanly to native path with zero partial mutation or private marker leakage. Non-PUI/direct-only
inheritance (`fetch_records`, backend/OAI, native `RecordInheritance.merge`) remains untouched.
Deterministic common PUI search metadata preparation is gated separately under
`AppConfig[:rust_pui_common_metadata_enabled]` (disabled by default). It migrates deterministic
search metadata for the seven native helpers (`IndexerCommon#add_subjects`, `#add_agents`, `#add_audit_info`,
`#add_years`, `#add_level`, `#add_summary`, `#add_extents`) when invoked by `PUIIndexer` into the existing
batched Rust `POST /index/documents` path. The input is strictly the direct-only record seen by these helpers,
never the fully inherited final candidate. Rust emits a narrow projection under `_archivalist_pui_common` with
marker `pui_common_metadata` containing per-helper field patches across strictly enumerated keys. Ruby
independently verifies wrapper identity, record identity/type, markers, recursive key sets, and type-strict
JSON equality against native execution on a scratch doc at each helper's original native call position before
applying the patch. Plugin hooks, pre-existing doc fields, absence/no-write semantics, and prior record
mutations are strictly preserved. Any mismatch, unknown key, outage, or exception falls back cleanly to native
helper execution without partial data or leaked markers.

## Test and benchmark helpers

Scripts under `scripts/` are code-only harnesses:

- `compare_staff_index_parity.py`: read-only Staff Solr collection comparator that
  verifies full document and field parity, array ordering, and absence of private
  handoff markers between native baseline and candidate reindexes.
- `compare_pui_index_parity.py`: read-only PUI Solr collection comparator that
  verifies privacy-normalized PUI parity, public document parity, type strictness,
  and canonical structural JSON equality against an unhardened native baseline by
  normalizing ONLY documented native-private fields after strictly verifying zero
  candidate privacy leaks (absence of root/JSON ancestors, internal notes, container-profile
  notes, unpublished fullrecord, and private handoff metadata). Non-private fields are
  exact; four documented private baseline fields are normalized.
- `canary_pui_document_preparation.sh`: isolated non-production canary script for
  low-level PUI document preparation that verifies sidecar /health liveness and response
  privacy cleaning against the Rust sidecar while permanently refusing primary production Solr collections.
- `repository_integration_stage_harness.rb`: full backend-driven repository integration gate
  that exercises real ArchivesSpace API record snapshots, STAFF and PUI indexers, and tree/auxiliary
  traversal against isolated, stage-owned unique Solr cores.
  - **Invocation**:
    ```bash
    docker exec -e REPO_ID=4 -e REPEATS=3 -w /archivesspace archivesspaceai-app-1 \
      /archivesspace/scripts/jruby /archivesspace/rust-tools/scripts/repository_integration_stage_harness.rb
    ```
  - **Safety Guarantees**:
    - Pre-flight fail-closed validation of backend URL, Solr URL, and repository existence before any core or network mutations.
    - Permanent prohibition on production or primary Solr cores (`archivesspace`, `archivesspace_shadow`, `production`, `primary`, `live`). Cores are uniquely named `stage_{staff,pui}_<run-id>_{native,candidate,fallback}`.
    - Sidecars are spawned on dynamically verified unused loopback ports and strictly stopped using verified ownership; other running processes and cores are never blindly killed.
    - Source database, service configurations, and existing Solr cores remain strictly read-only and intact.
    - All created cores, benchmark logs, and reports are retained as permanent evidence; no automatic deletes.
    - All feature flags (`AppConfig[:rust_*]`) default to false and remain disabled by default.
  - **Truthful Coverage Disclosure**:
    - Evaluates all record types present in the specified repository (e.g. resources, digital objects, accessions, top containers, container profiles, location profiles, archival objects, jobs).
    - Unexercised global authority types (`agent_person`, `agent_software`, `agent_family`, `agent_corporate_entity`, `location`, `subject`) and absent repository types (e.g. `assessment`, `classification`, `event`) are explicitly logged and reported as unexercised / not passed rather than claimed as covered.
    - Auxiliary documents (`resource_ordered_records`, `tree_root`, `tree_waypoint`, `tree_node`, `tree_node_from_root`) are partitioned strictly and verified via type-strict JSON and negative mutation checks.
    - Outage fallback equivalence is verified with sidecar stopped and endpoint unreachable.
    - Positive STAFF and positive PUI queries are derived independently at runtime from actual records present in the respective snapshots, asserting real expected IDs, nonzero complete hits, and exact candidate/fallback parity.
- `archivalist_profiler.rb` + `archivalist_profile_wrappers.rb`: opt-in aggregate profiler
  (monotonic clock; calls, bytes, inclusive and exclusive time; nested time is subtracted so
  exclusive times plus the region remainder sum to the region wall time). Only the stage
  harness uses it, and only when `ARCHIVALIST_PROFILE=1`; wrappers forward args, kwargs,
  blocks, return values, exceptions and method visibility, and become pure pass-throughs after
  `uninstall_all!`. Nothing but aggregates (no record content, URIs, queries or credentials)
  is stored. Unwrapped work is reported as `unwrapped_remainder_in_region`, never guessed.
  `sidecar_http_incl_rust_compute` is loopback transport plus server-side Rust compute and is
  NOT separable without server-side timing. `test_archivalist_profiler.rb` holds the
  deterministic offline tests (fake clock, synthetic classes).
- `run_profiled_repository_stage.sh`: runs the repository stage harness in the existing app
  container with profiling enabled (`REPO_ID` required) and saves stdout, stderr and the true
  exit status under ignored `benchmarks/`. The harness times warmup + `REPEATS`
  inactive-wrapper passes first (wrappers are installed but are pass-throughs, so these are NOT
  fully uninstrumented; JSON keys keep the legacy `uninstrumented_*` names), then
  `PROFILE_REPEATS` profiled passes per mode. `PROFILE_MODE=off` runs a true no-wrapper
  baseline (no wrapper installed). Set `SOLR_BASE_URL` (e.g. `http://solr-bench:8983/solr`,
  allowlisted) to target an isolated Solr. Input snapshot cloning before each pass and Solr
  commit/fetch after it lie outside the timed pass and are not measured. Profiled/inactive
  median shows profiling overhead; `compare_all_staff_docs.py` checks ALL stored STAFF docs
  (the repository comparator alone excludes profile docs). JVM `-Xss` is raised for the whole
  run because wrapped recursive validators otherwise overflow JRuby's 512K stack cap.
  The sidecar is the **release** build, rebuilt on the host with
  `cargo build --release --locked` before every run (`SIDECAR_BUILD=0` skips the rebuild). Its
  SHA-256 is passed to the harness, which refuses a mismatching file and also hashes the
  running `/proc/<pid>/exe`. `SIDECAR_BINARY=<container path>` is an explicit override (no build
  or expected hash; its path-inferred build mode is recorded). Binary path, SHA-256, build mode,
  Git commit, dirty status and dirty paths go into `<prefix>_<ts>.build.json`, the `step4_sidecar`
  checkpoint and the final report. Runs before this change used the debug sidecar and are not
  a performance verdict on optimized Rust.
  `WARMUP_PASSES` (default 1) sets untimed warmups per mode; their wall times are recorded.
  `PUI_ABLATION=1` adds a no-profiler PUI ablation into a dedicated `stage_pui_<run>_ablation`
  core: an adjacent native baseline, builder only, document preparation, preparation plus
  inheritance candidate, common metadata alone and with preparation, auxiliary documents, and
  all capabilities. Deltas are against the adjacent native median and are not additive; a
  configuration whose projections fall back natively is reported `valid_measurement: false`.
  `PAYLOAD_ACCOUNTING=1` (`archivalist_payload_accounting.rb`) adds dedicated STAFF and PUI
  passes that count request/response bytes per call kind, per field and per projection, and
  repeated URIs and identical ancestor entries. It stores aggregates only, and its pass wall
  times are recorded as not comparable.
  `SIDECAR_INDEX_TIMING=1` (`archivalist_sidecar_timing.rb`) starts the stage sidecar with its
  `Server-Timing` header and, for every candidate and ablation pass, records `rust_server_ms`
  (the sum of server `total`, split into receive, parse, build and serialize), `round_trip_ms`
  (Ruby-side wall time of the sidecar HTTP calls), `transport_and_http_ms` (the difference,
  including the server's response write) and `outside_sidecar_ms` (pass wall time minus the
  round trips). It records per-pass values and medians; the cost per call is one header parse
  and two clock reads. A run with timing requested but no header returned fails.
  `RUBY_SIDE_TIMING=1` (`archivalist_ruby_side_timing.rb`, use with `PROFILE_MODE=off`) adds
  coarse exclusive timers to every timed pass, native included: request JSON encoding, reply
  JSON decoding, builder checks (`validate_*`, `compute_expected_*`, `reconstruct_expected_*`),
  `apply_rust_pui_*`, `RecordInheritance.merge`/`build_plan`, input preparation, the sidecar round
  trip and other JSON work. The remainder is `unattributed_ms`. Recursive helpers are not wrapped
  (their time lands in their caller), keeping it to roughly ten thousand timer calls per PUI pass.
  The overhead is estimated from a calibration loop and reported, never subtracted. The stdout
  prints a native-versus-candidate table per mode.
- `profiling_stack.py`: provisions and proves a DISPOSABLE profiling stack for full-round
  profiling. `setup --app-container <name>` starts NEW containers only, without restarting or
  modifying existing ones: a MySQL on a new labelled volume, a Solr with the repository configset
  plus an identity core, and a backend using the app container's image and mounts, migrated
  against the new database. They are labelled `archivalist.profiling-stack=<id>`, join the app's
  network under `archivalist-profiling-{db,solr,backend}` aliases, publish ports on 127.0.0.1
  only, and the state file goes under `benchmarks/` with mode 0600. `verify` proves database
  identity by challenge and response: it writes a fresh nonce directly into the stack database
  and requires the target backend to return it from an anonymous `GET /repositories/<id>`,
  which writes nothing to the backend being checked. It also requires the stack's Solr identity
  core. `teardown` removes only containers carrying the stack's label and keeps the database
  volume unless `--delete-db-volume` is given.
- `seed_large_repository.py`: creates a large, deterministic test repository through the API.
  It requires `--stack-state` and runs the stack identity proof before any login or write, so a
  main or shared test backend is rejected untouched. It sends each collection tree (resource,
  series, files, items with notes, dates, extents, subject/agent links, top-container instances
  and some digital objects) as one `/batch_imports` request. Presets are `small` (~1.2k
  archival objects), `medium` (~24k) and `large` (~70k), and each count can be overridden.
- `native_indexing_profile.rb` + `run_native_indexing_profile.sh`: complete native
  `run_index_round`s for STAFF and PUI from an empty index state into fresh run-scoped stage
  cores in the stack's Solr, with all Rust flags forced off, so backend fetching and Solr writes
  are measured. The wrapper requires `STACK_STATE`, runs `profiling_stack.py verify`, and passes
  the nonce and the stack's in-network URLs to the profiler. The profiler
  (`archivalist_profiling_stack.rb`) re-checks the same nonce through its own backend URL and the
  identity core through its own Solr URL before any login, core creation or indexing.
  `archivalist_ruby_side_timing.rb` attributes the time to backend HTTP, record fetching,
  document preparation, JSON, Solr update/commit/delete HTTP, PUI tree work and the coordinating
  thread's waiting. Timers are per-thread, so with several indexer threads the categories sum
  thread time and can exceed wall time. `TIMERS=0` gives wall time only (compare medians for
  overhead); `REPEATS`, `MODES`, `STAFF_THREADS`, `PUI_THREADS` and `APP_CONTAINER` are forwarded.
- Helpers that query records require the caller to provide test-specific values
  through environment variables such as `RESOURCE_URI` and `QUERY`.

Generated benchmark output belongs in the ignored `benchmarks/` directory and
must not be committed.
