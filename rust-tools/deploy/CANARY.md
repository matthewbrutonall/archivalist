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
subject/place terms. Accession date localization remains native. All markers are
removed before Solr and plugin hooks. It accounts for Solr's intentional copy-field
merge of `notes_published` into `notes`. Only after this passes should
`rust_index_document_enrichment_enabled` be set to `true`. The enrichment gate is
staff-index only; PUI full-record extraction remains native after inheritance and
privacy filtering. Before rollout, also require a complete native-versus-Rust
reindex comparison so accessions, digital objects, subjects, and agents are covered.

## Rollback

Set all sidecar enable flags, including `rust_index_document_builder_enabled`, to
`false` and restart Archivalist. The sidecar
can then be stopped independently. Do not point the service at the primary Solr
collection and do not enable these paths by default until the canary gates pass.
