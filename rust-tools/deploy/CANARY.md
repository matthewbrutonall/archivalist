# Rust sidecar canary

This runbook keeps every Rust fast path disabled until its native equivalent has
been measured and compared on a non-production Archivalist instance.

## 1. Package and install

Run `scripts/package_sidecar_release.sh`, verify the generated SHA-256 checksum,
then copy the archive to the canary host. Install the binary as
`/usr/local/bin/aspace-shadow-sidecar`, the unit as
`/etc/systemd/system/aspace-shadow-sidecar.service`, and the environment file as
`/etc/archivalist/aspace-shadow-sidecar.env`. Adapt the service user and paths.

Keep `ARCHIVALIST_SIDECAR_HOST=127.0.0.1` and keep the collection set to a separate
shadow collection. The binary refuses the primary `archivesspace` collection.

## 2. Start and observe

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now aspace-shadow-sidecar
curl --fail http://127.0.0.1:4570/health
curl --fail http://127.0.0.1:4570/ready
curl --fail http://127.0.0.1:4570/metrics
```

The service exposes Prometheus-format process counters at `/metrics`. Journal logs
remain available with `journalctl -u aspace-shadow-sidecar`.

## 3. Enable only search

Add the following to the canary instance's `config/config.rb`, then restart it:

```ruby
AppConfig[:shadow_search_sidecar_enabled] = true
AppConfig[:shadow_search_sidecar_url] = "http://127.0.0.1:4570"
AppConfig[:shadow_search_sidecar_timeout_seconds] = 1
AppConfig[:pui_shadow_search_sidecar_enabled] = false
AppConfig[:shadow_resource_tree_sidecar_enabled] = false
```

Run the parity and latency gate with a representative large resource:

```bash
REPO_ID=2 RESOURCE_ID=18 QUERY=correspondence ADMIN_PASSWORD='canary-password' \
  scripts/canary_shadow_search.sh
```

Acceptance requires exact parity for `total_hits` and the ordered URI/title/type
projection on every iteration, no fallback response, and no sidecar errors.

## 4. Prove outage fallback

While the search flag remains enabled, stop only the sidecar and repeat a search
with `shadow_sidecar=true`. The backend must return a normal response through its
native Solr path and log a fallback warning. Restart the sidecar and require `/ready`
to pass before continuing.

```bash
sudo systemctl stop aspace-shadow-sidecar
# Repeat the canary search request and inspect the backend log.
sudo systemctl start aspace-shadow-sidecar
curl --fail http://127.0.0.1:4570/ready
```

## 5. Tree gates

After the search gate passes, enable
`AppConfig[:shadow_resource_tree_sidecar_enabled]` and test root waypoints, node
waypoints, and 64–128-node root paths separately. Retain the shipped thresholds of
200 root children, 100 node children, and 64 path nodes. Exercise the same outage
fallback before accepting each path.

Use one backend with all sidecar flags disabled as the baseline and a second,
otherwise identical backend with only the resource-tree flag enabled:

```ruby
AppConfig[:shadow_search_sidecar_enabled] = false
AppConfig[:pui_shadow_search_sidecar_enabled] = false
AppConfig[:shadow_resource_tree_sidecar_enabled] = true
AppConfig[:shadow_resource_tree_waypoint_sidecar_enabled] = true
AppConfig[:shadow_resource_tree_path_sidecar_enabled] = true
AppConfig[:shadow_resource_tree_sidecar_url] = "http://127.0.0.1:4570"
AppConfig[:shadow_resource_tree_sidecar_timeout_seconds] = 1
```

Select a nested node with at least 100 immediate children and 64–128 unique node
IDs belonging to the same resource. Run all three gates with:

```bash
BASELINE_BACKEND_URL=http://127.0.0.1:4567 \
CANARY_BACKEND_URL=http://127.0.0.1:4569 \
REPO_ID=2 RESOURCE_ID=18 \
NODE_URI=/repositories/2/archival_objects/1712 \
NODE_IDS=1,2,3,...,64 ADMIN_PASSWORD='canary-password' \
  scripts/canary_resource_tree.sh
```

The script compares the complete canonical JSON response, records native and Rust
latencies, and verifies from the sidecar request counter that every canary request
used Rust. It performs three warm-up rounds by default and alternates native/Rust
request order to reduce cache and ordering bias. For the outage gate, stop the sidecar and rerun with
`EXPECT_FAST_PATH=false`; every response must still match the baseline, and the
backend log must contain a fallback warning for each tree path.

Treat the two specific flags as independent acceptance gates. Leave either flag
`false` if its canary is slower or fails parity. The master
`shadow_resource_tree_sidecar_enabled` flag remains the single emergency kill
switch for both accelerators. Use `GATES=root_paths` or a comma-separated subset
to repeat only the paths being accepted or tested for outage fallback.

When only batched root paths pass the gate, use:

```ruby
AppConfig[:shadow_resource_tree_sidecar_enabled] = true
AppConfig[:shadow_resource_tree_waypoint_sidecar_enabled] = false
AppConfig[:shadow_resource_tree_path_sidecar_enabled] = true
```

This keeps root and nested waypoint responses on LargeTree while accelerating only
eligible 64–128-node `node_from_root` batches.

## 6. Index-document preparation gate

This gate changes only preparation of the common document envelope. Ruby still
runs every document hook, schedules batches, and writes to Solr. First run the
contract repeatedly against a representative non-production resource:

```bash
REPO_ID=2 RESOURCE_ID=18 ADMIN_PASSWORD='canary-password' \
  scripts/canary_index_document_builder.sh
```

Then enable only this gate and restart the indexer:

```ruby
AppConfig[:rust_index_document_builder_enabled] = true
AppConfig[:rust_index_document_builder_url] = "http://127.0.0.1:4570"
AppConfig[:rust_index_document_builder_timeout_seconds] = 2
AppConfig[:rust_index_document_builder_max_batch] = 1000
AppConfig[:rust_index_document_enrichment_enabled] = false
```

Reindex a disposable repository and compare its Solr document count and search
results with the native baseline. Stop the sidecar during one batch and confirm the
index completes through the native builder with a fallback warning. This flag is
independent of the search and resource-tree flags.

After the base builder passes, run the staff-index enrichment gate against a
resource that is already present in the native Solr collection:

```bash
REPO_ID=2 RESOURCE_ID=18 ADMIN_PASSWORD='canary-password' \
  scripts/canary_index_document_enrichment.sh
```

This compares subjects, agents, publication-aware notes, dates, years, level,
summary, extents, audit fields, and the resource-specific identifiers, finding-aid
metadata, classifications, instances, containers, ancestors, linked rights agents,
and language codes now prepared by Rust. The private handoff marker includes
`resource_archival_object_metadata` for resources and archival objects and is
joined by `accession_digital_object_metadata` for accessions and digital objects.
Subject and agent authority mappings use `agent_subject_metadata`, including
authority identifiers, rules, repository usage, related agents, and nested
subject/place terms. Digital-object components and classifications use
`component_classification_metadata`. Repositories, events, and locations use
`repository_event_location_metadata`, including repository sort fields, event
linked-record titles and URIs, and location profile and resolved owner-repository
fields. Container profiles, location profiles, and assessments use
`profile_assessment_metadata`, including profile dimensions, units, notes, and
typeahead sort keys, plus assessment linked records, collections, surveyors,
reviewers, survey year ranges, booleans, formats, ratings, and conservation issues.
Top containers use `top_container_metadata`, including series and collection
URIs, titles, levels, and identifier permutations, published-series URIs and
titles, resolved container-profile display strings, current container-location
fields and deduplicated location-profile display strings, exported/empty booleans,
indicator and barcode sort keys, ASCII typeahead text, ILS holding IDs,
subcontainer barcodes, and created-for-collection URIs.
Jobs use `job_metadata`, covering deterministic staff-index fields: `job_type`,
`report_type`, `job_report_type` (matching Ruby truthiness for empty string vs
false/nil), `status`, `owner`, `queue_position`, and types ordering
`['job', job_type, report_type]`.
Synthetic `collection_management` child documents are generated in Rust for valid
parent records (`resource`, `accession`, `digital_object`) with a collection-management
subrecord. The child document includes native fields: `id`, `uri`, `parent_id`,
`parent_title`, `parent_type`, `title`, `title_sort`, `types`, `primary_type`, `json`,
`processing_priority`, `processing_status`, `processing_hours_total`,
`processing_funding_source`, `processors`, `suppressed`, `repository`, and audit
fields (`created_by`, `last_modified_by`, `system_mtime`, `user_mtime`, `create_time`).
Inputs where exact parity cannot be guaranteed (such as non-standard alphabetic
characters outside the ASCII and Latin extended ranges) are withheld by Rust so
native Ruby child generation runs.
Accession date localization, unresolved location owner-repository HTTP lookups,
and job dynamic title translation via I18n, localized timestamps via
`Time.parse(...).getlocal`, and output-file discovery via `JSONModel::HTTP`
remain native. All markers (`_archivalist_rust_fields` and
`_archivalist_collection_management_documents`) are removed before Solr and plugin
hooks. A valid Rust child handoff suppresses only the built-in native child generator;
all third-party `add_extra_documents_hook` registrations execute unchanged exactly
once. Stale-child deletion via `records_with_children` is preserved. It accounts
for Solr's intentional copy-field merge of `notes_published` into `notes`. Only
after this passes should `rust_index_document_enrichment_enabled` be set to `true`.
The enrichment gate is staff-index only; PUI full-record extraction, top-container
publication decisions (`is_linked_to_published_record` / `pui_container`), and PUI
removal of `internal_note` and container-profile notes remain native after
inheritance and privacy filtering. Job and collection-management child enrichment is
completely bypassed in PUI.
Before rollout, also require a complete native-versus-Rust reindex comparison so
accessions, digital objects and components, subjects, agents, classifications,
repositories, events, locations, container profiles, location profiles,
assessments, top containers, jobs, and collection-management child documents are covered.

## 7. Full STAFF index parity gate

This gate validates complete, stored Solr document parity between a native indexer
and a Rust-enriched indexer across all record types for an entire disposable repository.

> **CRITICAL SAFETY RULE**: Never point this workflow, any reindex command, or either
> comparator URL at the primary production Solr collection (`archivesspace`). All
> runs must use dedicated non-production collections or separate non-production Solr
> instances. The comparator is strictly read-only: it will never trigger a reindex,
> alter configuration, write to Solr, or touch production data.

### Step A: Produce native baseline index in disposable collection 1

Point a non-production indexer at a dedicated disposable collection, e.g.
`archivesspace_native_test`, with all Rust index acceleration flags disabled:

```ruby
AppConfig[:solr_url] = "http://127.0.0.1:8985/solr/archivesspace_native_test"
AppConfig[:rust_index_document_builder_enabled] = false
AppConfig[:rust_index_document_enrichment_enabled] = false
```

Run a clean reindex of the designated disposable repository (e.g. `REPO_ID=2`). Wait
for indexing and Solr commits to complete.

### Step B: Produce Rust-enriched candidate index in disposable collection 2

Verify the sidecar is healthy (`curl --fail http://127.0.0.1:4570/ready`). Point the
non-production indexer at a second separate disposable collection, e.g.
`archivesspace_rust_test`, with builder and enrichment enabled:

```ruby
AppConfig[:solr_url] = "http://127.0.0.1:8985/solr/archivesspace_rust_test"
AppConfig[:rust_index_document_builder_enabled] = true
AppConfig[:rust_index_document_builder_url] = "http://127.0.0.1:4570"
AppConfig[:rust_index_document_builder_timeout_seconds] = 2
AppConfig[:rust_index_document_builder_max_batch] = 1000
AppConfig[:rust_index_document_enrichment_enabled] = true
```

Run a clean reindex of the same disposable repository (`REPO_ID=2`). Wait for
indexing and Solr commits to complete.

### Step C: Run the parity comparator

Execute the read-only parity comparator comparing the two completed collections:

```bash
python3 scripts/compare_staff_index_parity.py \
  --baseline-url http://127.0.0.1:8985/solr/archivesspace_native_test \
  --candidate-url http://127.0.0.1:8985/solr/archivesspace_rust_test \
  --repository 2
```

For automated pipeline checks, supply `--json` to produce machine-readable output:

```bash
python3 scripts/compare_staff_index_parity.py \
  --baseline-url http://127.0.0.1:8985/solr/archivesspace_native_test \
  --candidate-url http://127.0.0.1:8985/solr/archivesspace_rust_test \
  --repository 2 \
  --json
```

Acceptance requires:
1. Exit code `0` (`OVERALL RESULT: PASS`).
2. Exact document count match and zero missing IDs on either side.
3. Zero missing fields and zero scalar or type differences.
4. Zero array ordering discrepancies (ordering is strictly verified, not normalized away).
5. Zero occurrences of the internal handoff field `_archivalist_rust_fields` anywhere.

### Step D: Prove sidecar-outage fallback separately

To prove that a sidecar failure during indexing falls back to native Ruby preparation
without corruption:

1. Stop only the non-production sidecar:
   ```bash
   sudo systemctl stop aspace-shadow-sidecar
   ```
2. Keep `rust_index_document_builder_enabled` and `rust_index_document_enrichment_enabled`
   set to `true`, but point `AppConfig[:solr_url]` to a third non-production collection,
   `archivesspace_fallback_test`:
   ```ruby
   AppConfig[:solr_url] = "http://127.0.0.1:8985/solr/archivesspace_fallback_test"
   AppConfig[:rust_index_document_builder_enabled] = true
   AppConfig[:rust_index_document_enrichment_enabled] = true
   ```
3. Reindex the disposable repository (`REPO_ID=2`). Verify the indexer logs fallback
   warnings for unreachable sidecar endpoints and successfully completes using native
   Ruby preparation.
4. Compare the fallback collection to the native baseline:
   ```bash
   python3 scripts/compare_staff_index_parity.py \
     --baseline-url http://127.0.0.1:8985/solr/archivesspace_native_test \
     --candidate-url http://127.0.0.1:8985/solr/archivesspace_fallback_test \
     --repository 2
   ```
   Acceptance requires exit code `0`, confirming complete fidelity with the baseline.
5. Restart the sidecar and confirm readiness:
   ```bash
   sudo systemctl start aspace-shadow-sidecar
   curl --fail http://127.0.0.1:4570/ready
   ```

## 8. Rollback

Set all sidecar enable flags, including `rust_index_document_builder_enabled` and
`rust_index_document_enrichment_enabled`, to `false` and restart Archivalist. The sidecar
can then be stopped independently. Do not point the service at the primary Solr
collection and do not enable these paths by default until all canary gates pass.
