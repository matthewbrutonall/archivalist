#!/usr/bin/env bash
# Runs native_indexing_profile.rb (complete native STAFF/PUI index rounds into fresh, run-scoped
# stage cores, Rust flags forced off) inside the existing app container and saves stdout, stderr
# and the TRUE exit status under ignored benchmarks/. A full round indexes EVERY repository the
# backend can see, so point it at an isolated backend seeded with seed_large_repository.py.
# Set SOLR_BASE_URL (e.g. http://solr-bench:8983/solr) to target an isolated Solr; it is forwarded
# and still subject to the allowlist. Options are forwarded as-is: REPEATS (default 1), MODES
# (staff,pui), TIMERS (1 = category timers, 0 = wall time only), STAFF_THREADS, PUI_THREADS.
# Usage: SOLR_BASE_URL=http://solr-bench:8983/solr [REPEATS=3] rust-tools/scripts/run_native_indexing_profile.sh
set -uo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TS="$(date +%Y%m%d_%H%M%S)"
BASE="$HERE/benchmarks/native_profile_run_$TS"
mkdir -p "$HERE/benchmarks"

GIT_COMMIT="$(git -C "$HERE" rev-parse HEAD 2>/dev/null || echo unrecorded)"
GIT_DIRTY_PATHS="$(git -C "$HERE/.." status --porcelain 2>/dev/null)"
printf '{\n  "git_commit": "%s",\n  "git_dirty": "%s"\n}\n' "$GIT_COMMIT" "$([ -n "$GIT_DIRTY_PATHS" ] && echo true || echo false)" > "$BASE.build.json"

CONTAINER_JAVA_OPTS="$(docker exec archivesspaceai-app-1 printenv JAVA_OPTS)"
OPT_ENV=()
for var in SOLR_BASE_URL BACKEND_URL REPEATS MODES TIMERS STAFF_THREADS PUI_THREADS; do
  [ -n "${!var:-}" ] && OPT_ENV+=(-e "$var=${!var}")
done
docker exec "${OPT_ENV[@]}" -e JAVA_OPTS="$CONTAINER_JAVA_OPTS -Xss${PROFILE_XSS:-16m}" -w /archivesspace archivesspaceai-app-1 \
  /archivesspace/scripts/jruby /archivesspace/rust-tools/scripts/native_indexing_profile.rb \
  > "$BASE.stdout" 2> "$BASE.stderr"
rc=$?
echo "$rc" > "$BASE.exit"
echo "stdout=$BASE.stdout build=$BASE.build.json exit=$rc"
exit "$rc"
