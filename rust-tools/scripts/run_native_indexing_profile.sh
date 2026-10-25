#!/usr/bin/env bash
# Runs native_indexing_profile.rb (complete native STAFF/PUI index rounds into fresh, run-scoped
# stage cores, Rust flags forced off) inside the app container against a DISPOSABLE profiling
# stack created by profiling_stack.py, and saves stdout, stderr and the TRUE exit status under
# ignored benchmarks/.
#
# Safety: STACK_STATE (the state file from `profiling_stack.py setup`) is required. Before the
# profiler starts, `profiling_stack.py verify` writes a fresh nonce directly into the stack's
# database and proves the stack backend returns it; the profiler then re-checks that nonce
# through the in-network backend URL and the stack's Solr identity core before any login, core
# creation or indexing. BACKEND_URL / SOLR_BASE_URL are taken from the stack and cannot be
# overridden. Options forwarded: REPEATS (default 1), MODES (staff,pui), TIMERS (1 = category
# timers, 0 = wall time only), STAFF_THREADS, PUI_THREADS, PUI_FETCH_AUDIT (1 = PUI archival_object
# fetch audit, default off), PUI_TREE_BATCH_SIZE (tree-discovery batch size override, clamped 1..200,
# default unchanged), PUI_RECORDS_PER_THREAD (ordinary PUI records-per-thread override, 1..200,
# default unchanged). APP_CONTAINER defaults to archivesspaceai-app-1.
# Usage: STACK_STATE=rust-tools/benchmarks/profiling_stack_<id>.json [REPEATS=3] [PUI_FETCH_AUDIT=1] \
#          rust-tools/scripts/run_native_indexing_profile.sh
set -uo pipefail
: "${STACK_STATE:?STACK_STATE must point to the profiling_stack.py state file}"
if [ -n "${BACKEND_URL:-}${SOLR_BASE_URL:-}" ]; then
  echo "BACKEND_URL/SOLR_BASE_URL are taken from the profiling stack; unset them" >&2
  exit 2
fi
HERE="$(cd "$(dirname "$0")/.." && pwd)"
APP_CONTAINER="${APP_CONTAINER:-archivesspaceai-app-1}"
TS="$(date +%Y%m%d_%H%M%S)"
BASE="$HERE/benchmarks/native_profile_run_$TS"
mkdir -p "$HERE/benchmarks"

STACK_ENV="$(python3 -I "$HERE/scripts/profiling_stack.py" verify --state "$STACK_STATE" --emit-env)" || {
  echo "Profiling stack verification failed; not running" >&2
  exit 2
}
STACK_ARGS=()
while IFS='=' read -r key value; do
  case "$key" in
    ARCHIVALIST_PROFILING_STACK_BACKEND_URL) STACK_ARGS+=(-e "BACKEND_URL=$value") ;;
    ARCHIVALIST_PROFILING_STACK_SOLR_URL) STACK_ARGS+=(-e "SOLR_BASE_URL=$value") ;;
    ARCHIVALIST_PROFILING_STACK_*) STACK_ARGS+=(-e "$key=$value") ;;
  esac
done <<< "$STACK_ENV"

GIT_COMMIT="$(git -C "$HERE" rev-parse HEAD 2>/dev/null || echo unrecorded)"
GIT_DIRTY_PATHS="$(git -C "$HERE/.." status --porcelain 2>/dev/null)"
printf '{\n  "git_commit": "%s",\n  "git_dirty": "%s",\n  "stack_state": "%s"\n}\n' "$GIT_COMMIT" \
  "$([ -n "$GIT_DIRTY_PATHS" ] && echo true || echo false)" "$STACK_STATE" > "$BASE.build.json"

CONTAINER_JAVA_OPTS="$(docker exec "$APP_CONTAINER" printenv JAVA_OPTS)"
OPT_ENV=()
for var in REPEATS MODES TIMERS STAFF_THREADS PUI_THREADS PUI_FETCH_AUDIT PUI_TREE_BATCH_SIZE PUI_RECORDS_PER_THREAD; do
  [ -n "${!var:-}" ] && OPT_ENV+=(-e "$var=${!var}")
done
docker exec "${STACK_ARGS[@]}" "${OPT_ENV[@]}" -e JAVA_OPTS="$CONTAINER_JAVA_OPTS -Xss${PROFILE_XSS:-16m}" \
  -w /archivesspace "$APP_CONTAINER" \
  /archivesspace/scripts/jruby /archivesspace/rust-tools/scripts/native_indexing_profile.rb \
  > "$BASE.stdout" 2> "$BASE.stderr"
rc=$?
echo "$rc" > "$BASE.exit"
echo "stdout=$BASE.stdout build=$BASE.build.json exit=$rc"
exit "$rc"
