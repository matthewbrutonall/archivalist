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
- `POST /index/documents`: bounded preparation of common Solr document fields and, when enrichment is requested, staff-index mappings for resources, archival objects, accessions, and digital objects.

The example systemd unit in `deploy/` binds to loopback by default. Put authentication
and transport security in front of the sidecar before exposing it beyond the local
host.

Release archives can be built reproducibly with
`scripts/package_sidecar_release.sh`. See `deploy/CANARY.md` for the staged
non-production rollout, parity gate, outage fallback test, and rollback procedure.
The search, resource-tree, and index-document gates are automated by
`scripts/canary_shadow_search.sh`, `scripts/canary_resource_tree.sh`, and
`scripts/canary_index_document_builder.sh`. Staff-index enrichment parity is
checked separately by `scripts/canary_index_document_enrichment.sh`; PUI
full-record preparation intentionally remains native because inheritance is applied
later in its pipeline.

## Test and benchmark helpers

Scripts under `scripts/` are code-only harnesses. Helpers that query records require
the caller to provide test-specific values through environment variables such as
`RESOURCE_URI` and `QUERY`. Generated benchmark output belongs in the ignored
`benchmarks/` directory and must not be committed.
