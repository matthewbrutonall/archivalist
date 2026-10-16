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

## Rollback

Set all four sidecar enable flags to `false` and restart Archivalist. The sidecar
can then be stopped independently. Do not point the service at the primary Solr
collection and do not enable these paths by default until the canary gates pass.
