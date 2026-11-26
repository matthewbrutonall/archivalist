#!/usr/bin/env bash
# Runs the repository integration stage harness with opt-in aggregate profiling inside the
# existing app container and saves stdout, stderr and the TRUE exit status next to each other.
# Read-only against the source backend; writes only unique stage-owned Solr cores (retained).
# The wrappers add stack frames in deep recursive validators, so the JVM thread stack is raised
# (PROFILE_XSS, default 16m) for ALL passes of the run (uninstrumented and profiled alike).
# Set SOLR_BASE_URL (e.g. http://solr-bench:8983/solr) to target an isolated Solr; it is forwarded
# and still subject to the harness allowlist. Unset keeps the harness default.
# PROFILE_MODE=off runs the identical harness with ARCHIVALIST_PROFILE=0 so NO wrapper is
# installed at all (true no-wrapper baseline, same JVM flags); files are named noprofile_*.
#
# Sidecar binary: by default the release sidecar is rebuilt from the current tree with
# `cargo build --release --locked` immediately before the run, hashed, and the harness refuses
# to launch anything whose SHA-256 differs (it also hashes the running /proc/<pid>/exe).
# SIDECAR_BUILD=0 skips the build but still hashes and verifies the existing release binary.
# SIDECAR_BINARY=/archivesspace/... (container path) overrides the binary explicitly; no build
# or expected hash is applied and the harness records the hash and path-inferred build mode.
# Git commit, dirty status and dirty paths are recorded in <prefix>_<ts>.build.json and the report.
# WARMUP_PASSES (default 1) sets untimed warmups per mode; their wall times are recorded.
# PUI_ABLATION=1 adds a no-profiler PUI ablation (each Rust PUI capability alone, adjacent native).
# PAYLOAD_ACCOUNTING=1 adds dedicated byte-accounting passes (bytes only; timings not comparable).
# SIDECAR_INDEX_TIMING=1 starts the sidecar with its Server-Timing header and aggregates it per
# pass: Rust server time, Ruby-side HTTP round trip, and time outside sidecar calls.
# RUBY_SIDE_TIMING=1 adds coarse exclusive Ruby-side timers (JSON encode/decode, validation,
# inheritance, copies, input preparation) to every timed pass. Use with PROFILE_MODE=off.
# Usage: REPO_ID=4 [REPEATS=3] [PROFILE_REPEATS=3] rust-tools/scripts/run_profiled_repository_stage.sh
set -uo pipefail
: "${REPO_ID:?REPO_ID must be set explicitly}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TS="$(date +%Y%m%d_%H%M%S)"
PROFILE_FLAG=1; PREFIX=profile_stage_run
if [ "${PROFILE_MODE:-on}" = "off" ]; then PROFILE_FLAG=0; PREFIX=noprofile_stage_run; fi
BASE="$HERE/benchmarks/${PREFIX}_$TS"
mkdir -p "$HERE/benchmarks"

sha256_of() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

GIT_COMMIT="$(git -C "$HERE" rev-parse HEAD 2>/dev/null || echo unrecorded)"
GIT_DIRTY_PATHS="$(git -C "$HERE/.." status --porcelain 2>/dev/null)"
if [ "$GIT_COMMIT" = unrecorded ]; then GIT_DIRTY=unrecorded; elif [ -n "$GIT_DIRTY_PATHS" ]; then GIT_DIRTY=true; else GIT_DIRTY=false; fi

BUILD_CMD=""; EXPECTED_SHA=""; BUILD_MODE=""
if [ -n "${SIDECAR_BINARY:-}" ]; then
  CONTAINER_BIN="$SIDECAR_BINARY"
else
  CONTAINER_BIN=/archivesspace/rust-tools/target/release/aspace-shadow-sidecar
  HOST_BIN="$HERE/target/release/aspace-shadow-sidecar"
  BUILD_MODE=release
  if [ "${SIDECAR_BUILD:-1}" != 0 ]; then
    BUILD_CMD="cargo build --release --locked -p aspace-cli --bin aspace-shadow-sidecar"
    (cd "$HERE" && $BUILD_CMD) || { echo "Release sidecar build failed; not running benchmark" >&2; exit 1; }
  fi
  [ -x "$HOST_BIN" ] || { echo "Release sidecar missing at $HOST_BIN" >&2; exit 1; }
  EXPECTED_SHA="$(sha256_of "$HOST_BIN")"
fi

{
  printf '{\n  "git_commit": "%s",\n  "git_dirty": "%s",\n' "$GIT_COMMIT" "$GIT_DIRTY"
  printf '  "build_command": "%s",\n  "container_binary": "%s",\n' "${BUILD_CMD:-none}" "$CONTAINER_BIN"
  printf '  "host_sha256": "%s",\n' "${EXPECTED_SHA:-not computed (explicit override)}"
  printf '  "rustc": "%s",\n  "cargo": "%s"\n}\n' "$(rustc -V 2>/dev/null || echo unavailable)" "$(cargo -V 2>/dev/null || echo unavailable)"
} > "$BASE.build.json"

CONTAINER_JAVA_OPTS="$(docker exec archivesspaceai-app-1 printenv JAVA_OPTS)"
SOLR_ENV=(); [ -n "${SOLR_BASE_URL:-}" ] && SOLR_ENV=(-e "SOLR_BASE_URL=$SOLR_BASE_URL")
docker exec "${SOLR_ENV[@]}" -e REPO_ID="$REPO_ID" -e REPEATS="${REPEATS:-3}" -e PROFILE_REPEATS="${PROFILE_REPEATS:-3}" \
  -e ARCHIVALIST_PROFILE="$PROFILE_FLAG" -e JAVA_OPTS="$CONTAINER_JAVA_OPTS -Xss${PROFILE_XSS:-16m}" \
  -e WARMUP_PASSES="${WARMUP_PASSES:-1}" -e ARCHIVALIST_PUI_ABLATION="${PUI_ABLATION:-0}" \
  -e ARCHIVALIST_PAYLOAD_ACCOUNTING="${PAYLOAD_ACCOUNTING:-0}" \
  -e ARCHIVALIST_SIDECAR_INDEX_TIMING="${SIDECAR_INDEX_TIMING:-0}" \
  -e ARCHIVALIST_RUBY_SIDE_TIMING="${RUBY_SIDE_TIMING:-0}" \
  -e ARCHIVALIST_SIDECAR_BINARY="${SIDECAR_BINARY:-}" -e ARCHIVALIST_SIDECAR_SHA256="$EXPECTED_SHA" \
  -e ARCHIVALIST_SIDECAR_BUILD_MODE="$BUILD_MODE" -e ARCHIVALIST_SIDECAR_BUILD_COMMAND="$BUILD_CMD" \
  -e ARCHIVALIST_GIT_COMMIT="$GIT_COMMIT" -e ARCHIVALIST_GIT_DIRTY="$GIT_DIRTY" -e ARCHIVALIST_GIT_DIRTY_PATHS="$GIT_DIRTY_PATHS" \
  -w /archivesspace archivesspaceai-app-1 \
  /archivesspace/scripts/jruby /archivesspace/rust-tools/scripts/repository_integration_stage_harness.rb \
  > "$BASE.stdout" 2> "$BASE.stderr"
rc=$?
echo "$rc" > "$BASE.exit"
echo "stdout=$BASE.stdout build=$BASE.build.json exit=$rc"
exit "$rc"
