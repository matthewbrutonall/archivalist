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
ownership of Solr writes and stale-child deletions.

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
- Helpers that query records require the caller to provide test-specific values
  through environment variables such as `RESOURCE_URI` and `QUERY`.

Generated benchmark output belongs in the ignored `benchmarks/` directory and
must not be committed.
