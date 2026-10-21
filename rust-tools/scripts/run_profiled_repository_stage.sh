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
# Usage: REPO_ID=4 [REPEATS=3] [PROFILE_REPEATS=3] rust-tools/scripts/run_profiled_repository_stage.sh
set -uo pipefail
: "${REPO_ID:?REPO_ID must be set explicitly}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TS="$(date +%Y%m%d_%H%M%S)"
PROFILE_FLAG=1; PREFIX=profile_stage_run
if [ "${PROFILE_MODE:-on}" = "off" ]; then PROFILE_FLAG=0; PREFIX=noprofile_stage_run; fi
BASE="$HERE/benchmarks/${PREFIX}_$TS"
mkdir -p "$HERE/benchmarks"
CONTAINER_JAVA_OPTS="$(docker exec archivesspaceai-app-1 printenv JAVA_OPTS)"
SOLR_ENV=(); [ -n "${SOLR_BASE_URL:-}" ] && SOLR_ENV=(-e "SOLR_BASE_URL=$SOLR_BASE_URL")
docker exec "${SOLR_ENV[@]}" -e REPO_ID="$REPO_ID" -e REPEATS="${REPEATS:-3}" -e PROFILE_REPEATS="${PROFILE_REPEATS:-3}" \
  -e ARCHIVALIST_PROFILE="$PROFILE_FLAG" -e JAVA_OPTS="$CONTAINER_JAVA_OPTS -Xss${PROFILE_XSS:-16m}" -w /archivesspace archivesspaceai-app-1 \
  /archivesspace/scripts/jruby /archivesspace/rust-tools/scripts/repository_integration_stage_harness.rb \
  > "$BASE.stdout" 2> "$BASE.stderr"
rc=$?
echo "$rc" > "$BASE.exit"
echo "stdout=$BASE.stdout exit=$rc"
exit "$rc"
