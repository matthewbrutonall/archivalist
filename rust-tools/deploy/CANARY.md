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

The script derives dynamic-enum base names from native Solr document keys ending
`_enum_s` and passes them in the sidecar payload. `ARKS_ENABLED` defaults safely
to `false` (requiring `ark_name` to be omitted by both Rust and native documents).
To compare `ark_name` presence and value against an instance with ARKs enabled in
`AppConfig` and indexed in Solr, pass `ARKS_ENABLED=true`:

```bash
REPO_ID=2 RESOURCE_ID=18 ARKS_ENABLED=true ADMIN_PASSWORD='canary-password' \
  scripts/canary_index_document_enrichment.sh
```

This compares subjects, agents, publication-aware notes, dates, years, level,
summary, extents, audit fields, exact dynamic-enum sets and values (`*_enum_s`),
`ark_name` presence and values according to `ARKS_ENABLED`, and the resource-specific
identifiers, finding-aid metadata, classifications, instances, containers, ancestors,
linked rights agents, and language codes now prepared by Rust. The private handoff
marker includes `resource_archival_object_metadata` for resources and archival objects
and is joined by `accession_digital_object_metadata` for accessions and digital objects,
and `accession_date_metadata` for accessions.
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
remain native. Dynamic enum projections (`<field>_enum_s` and `type_enum_s`) and ARK
fields (`ark_name`) are accelerated by Rust when staff enrichment is enabled, using the
handoff capability marker `dynamic_enum_and_ark_metadata`. Ruby derives dynamic-enum
schema field names and checks `AppConfig[:arks_enabled]`, while Rust executes deterministic
traversal, `_resolved` exclusion, flattening, deduplication, and trim semantics. Ruby strictly
validates the returned projection before skipping native prepare hooks. If the response is
unsupported, missing, malformed, or mismatched, complete native preparation runs for this
slice. All markers (`_archivalist_rust_fields` and `_archivalist_collection_management_documents`)
are removed before Solr and plugin hooks. A valid Rust child handoff suppresses only the
built-in native child generator; all third-party `add_extra_documents_hook` registrations execute
unchanged exactly once. Stale-child deletion via `records_with_children` is preserved. It accounts
for Solr's intentional copy-field merge of `notes_published` into `notes`. Only
after this passes should `rust_index_document_enrichment_enabled` be set to `true`.
The enrichment gate `rust_index_document_enrichment_enabled` is staff-index only;
PUI document preparation is gated separately under `rust_pui_document_preparation_enabled`
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
logic on missing, malformed, or mismatched sidecar responses. Collection-management child
documents, dynamic enums, and ARKs remain staff-only and are completely bypassed in PUI.
Before rollout, also require a complete native-versus-Rust reindex comparison so
accessions, digital objects and components, subjects, agents, classifications,
repositories, events, locations, container profiles, location profiles,
assessments, top containers, jobs, collection-management child documents, dynamic enums,
and ARKs are covered.

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

## 8. PUI document preparation parity gate and canary procedure

This gate validates complete, stored public Solr document parity between a native baseline
PUI index and a candidate PUI index prepared with Rust acceleration enabled for an entire
disposable repository.

> **CRITICAL SAFETY RULE**: Never point this workflow, any canary script, or comparator
> at the primary production Solr collection (`archivesspace`). All canary procedures
> must target isolated non-production collections or dedicated test cores.

### Step A: Low-level isolated record canary verification

Run the dedicated non-production canary script on sample archival objects and resources:

```bash
REPO_ID=2 ARCHIVAL_OBJECT_ID=123 CANARY_SOLR_URL="http://127.0.0.1:8985/solr/test_canary_pui" \
  scripts/canary_pui_document_preparation.sh
```

**Scope Notice**: This canary exercises the low-level Rust sidecar document preparation contract directly (it does not perform Ruby RecordInheritance merge). The full two-index comparator in Step B is the end-to-end gate.

The canary verifies:
1. Sidecar `/health` returns HTTP 200 with `served_by == "aspace-shadow-sidecar"`.
2. `_archivalist_rust_fields` marker contains `["pui_final_document_metadata", "pui_routing_metadata", "pui_notes_metadata"]`.
3. `_archivalist_pui_final` contains clean `json` and `fullrecord_published`.
4. `_archivalist_pui_routing` contains exact `id`, `pui_parent_id`, and `types` with expected values.
5. `_archivalist_pui_notes` contains `notes_published` extracted strictly from direct record notes.
6. Root document and parsed JSON have no `ancestors`, no top-container `internal_note`, no container-profile `notes`, and no private `_archivalist_*` keys.
7. Unpublished `fullrecord` field is suppressed.
8. Exact exit code `0`.

### Step B: Full repository privacy-normalized PUI parity comparison

Run the offline type-strict privacy-normalized PUI parity comparator between a native PUI reindex
(`test_pui_baseline`) and a candidate PUI reindex (`test_pui_candidate`):

```bash
python3 scripts/compare_pui_index_parity.py \
  --baseline-url http://127.0.0.1:8985/solr/test_pui_baseline \
  --candidate-url http://127.0.0.1:8985/solr/test_pui_candidate \
  --repository 2
```

**Honest Parity Specification**:
Native ArchivesSpace retains root Solr `ancestors`, keeps `"ancestors": []` in JSON, and serializes JSON before stripping top-container `internal_note` and container-profile `notes`. The Rust candidate intentionally hardens these boundaries by removing root `ancestors`, stripping `ancestors` from JSON, and stripping nested container notes from JSON and `fullrecord_published`.

The comparator enforces this via a strict two-step verification rule:
1. Candidate is scanned strictly for any privacy leaks (root ancestors, JSON ancestors, internal notes, profile notes, unpublished fullrecord, or `_archivalist_*` keys). Candidate is NEVER normalized to hide leaks; any leak immediately fails.
2. Only the 4 documented native-private fields are normalized from the baseline (root Solr ancestors, JSON ancestors, top-container internal note, container-profile notes).
3. Candidate parsed JSON must structurally match the normalized baseline JSON.
4. All remaining Solr fields, types, values, and array orderings must match with exact type-strict equality. Non-private fields are exact; four documented private baseline fields are normalized.

Acceptance requires exit code `0` (`RESULT: SUCCESS - Privacy-normalized PUI parity verified`):
- Equal public document counts.
- Zero missing or extra documents.
- Zero scalar, array, or type differences on all non-private fields.
- Four documented private baseline fields normalized.
- Zero privacy leaks.

### Step C: Sidecar outage and untrusted response fallback

Verify that a sidecar outage, timeout, or malformed/malicious response causes the
indexer to safely discard the untrusted projection and execute unchanged native
`PUIIndexer#configure_doc_rules`, `PUIIndexer#add_notes`, and `PUIIndexer#final_doc_rules`
for the batch without leaking private data or halting indexing.

## 9. Rollback

Set all sidecar enable flags, including `rust_index_document_builder_enabled`,
`rust_index_document_enrichment_enabled`, and `rust_pui_document_preparation_enabled`,
to `false` and restart Archivalist:

```ruby
AppConfig[:rust_index_document_builder_enabled] = false
AppConfig[:rust_index_document_enrichment_enabled] = false
AppConfig[:rust_pui_document_preparation_enabled] = false
```

The sidecar can then be stopped independently. Do not point the service at the primary
Solr collection and do not enable these paths by default until all canary gates pass.
