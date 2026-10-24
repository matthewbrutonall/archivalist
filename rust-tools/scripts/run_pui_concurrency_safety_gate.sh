#!/usr/bin/env bash
# Runs the bounded live PUI concurrency safety gate (pui_concurrency_safety_gate.rb) on
# Repository 3 only against the verified disposable profiling stack.
#
# Safety & Container Lifecycle:
#   - Verifies exact profiling stack container names, labels, and initial runtime states from
#     STACK_STATE before starting anything.
#   - Refuses to run if either isolated profiling Solr or isolated profiling backend is already
#     running/active so the cleanup trap never stops a service this invocation did not start.
#   - Never starts, stops, or reconfigures main Solr (archivesspaceai-solr-1), bench Solr
#     (archivesspaceai-solr-bench), main app, or main/test DBs.
#   - Temporarily starts ONLY the isolated profiling Solr and isolated profiling backend for this
#     bounded gate, verifies DB challenge-response nonce and Solr identity core via
#     profiling_stack.py verify, runs the gate once (PUI_THREADS=1 vs PUI_THREADS=4 on repo 3),
#     runs read-only full cursor-paginated comparison via compare_native_profile_cores.compare(),
#     and ALWAYS stops the isolated backend and isolated Solr in an EXIT/INT/TERM trap while
#     leaving the isolated profiling DB running and preserving all containers, volumes, and cores.
set -euo pipefail

: "${STACK_STATE:?STACK_STATE must point to the profiling_stack.py state file}"
if [ -n "${BACKEND_URL:-}${SOLR_BASE_URL:-}" ]; then
  echo "ERROR: BACKEND_URL/SOLR_BASE_URL are taken from the verified profiling stack; unset them" >&2
  exit 2
fi

REPO_ID="${REPO_ID:-3}"
if [ "$REPO_ID" != "3" ]; then
  echo "ERROR: Bounded concurrency gate only authorizes REPO_ID=3 (got REPO_ID=$REPO_ID)" >&2
  exit 2
fi

HERE="$(cd "$(dirname "$0")/.." && pwd)"
APP_CONTAINER="${APP_CONTAINER:-archivesspaceai-app-1}"
TS="$(date +%Y%m%d_%H%M%S)"
BASE="$HERE/benchmarks/pui_concurrency_gate_run_$TS"
mkdir -p "$HERE/benchmarks"

# Parse and validate profiling stack container names, labels, and initial states before starting anything
STACK_META_RAW="$(python3 -I - "$STACK_STATE" <<'PY'
import json, subprocess, sys
with open(sys.argv[1]) as fh:
    st = json.load(fh)
stack_id = st["stack_id"]
containers = st["containers"]
for role in ("db", "solr", "backend"):
    name = containers[role]
    res = subprocess.run(["docker", "container", "inspect", name], capture_output=True, text=True)
    if res.returncode != 0:
        raise SystemExit(f"Container {name} does not exist")
    info = json.loads(res.stdout)[0]
    labels = info["Config"].get("Labels") or {}
    if labels.get("archivalist.profiling-stack") != stack_id or labels.get("archivalist.profiling-role") != role:
        raise SystemExit(f"Container {name} label mismatch: {labels}")
    state = info.get("State") or {}
    running = bool(state.get("Running"))
    status = str(state.get("Status") or "").lower()
    is_active = running or status in ("running", "restarting", "paused")
    if role == "db":
        if not running or (status and status != "running"):
            raise SystemExit(f"Isolated DB container {name} must already be running (Running={running}, Status={state.get('Status')!r})")
    else:
        if is_active or (status and status not in ("exited", "created", "stopped")):
            raise SystemExit(
                f"Refusing to run while isolated {role} container {name} is already running "
                f"(Running={running}, Status={state.get('Status')!r})"
            )
for forbidden in ("archivesspaceai-solr-1", "archivesspaceai-solr-bench"):
    res = subprocess.run(["docker", "container", "inspect", forbidden], capture_output=True, text=True)
    if res.returncode == 0:
        f_state = json.loads(res.stdout)[0].get("State") or {}
        f_status = str(f_state.get("Status") or "").lower()
        if bool(f_state.get("Running")) or f_status in ("running", "restarting", "paused"):
            raise SystemExit(f"Refusing to run while {forbidden} is running")
print(stack_id)
print(containers["db"])
print(containers["solr"])
print(containers["backend"])
print(st["solr_host_url"])
print(st["backend_host_url"])
PY
)"
readarray -t STACK_META <<< "$STACK_META_RAW"

STACK_ID="${STACK_META[0]}"
DB_CONTAINER="${STACK_META[1]}"
SOLR_CONTAINER="${STACK_META[2]}"
BACKEND_CONTAINER="${STACK_META[3]}"
SOLR_HOST_URL="${STACK_META[4]}"
BACKEND_HOST_URL="${STACK_META[5]}"

STARTED_SOLR=0
STARTED_BACKEND=0

cleanup() {
  local exit_status=$?
  echo "--- Cleanup trap: stopping isolated profiling backend and Solr (leaving DB running) ---" >&2
  if [ "$STARTED_BACKEND" -eq 1 ]; then
    docker stop "$BACKEND_CONTAINER" >/dev/null 2>&1 || true
  fi
  if [ "$STARTED_SOLR" -eq 1 ]; then
    docker stop "$SOLR_CONTAINER" >/dev/null 2>&1 || true
  fi
  docker ps -a --filter "label=archivalist.profiling-stack=$STACK_ID" --format '  {{.Names}}\t{{.Status}}' >&2 || true
  exit "$exit_status"
}
trap cleanup EXIT INT TERM

echo "Starting isolated profiling Solr ($SOLR_CONTAINER) and backend ($BACKEND_CONTAINER)..."
docker start "$SOLR_CONTAINER" >/dev/null
STARTED_SOLR=1
docker start "$BACKEND_CONTAINER" >/dev/null
STARTED_BACKEND=1

# Wait for isolated Solr and backend readiness
python3 -I - "$SOLR_HOST_URL" "$BACKEND_HOST_URL" <<'PY'
import json, sys, time, urllib.request
solr_url, backend_url = sys.argv[1], sys.argv[2]

def wait_json(url, pred, label, timeout_s):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=5) as r:
                data = json.loads(r.read())
                if pred(data):
                    print(f"Ready: {label}", flush=True)
                    return
        except Exception:
            pass
        time.sleep(2)
    raise SystemExit(f"Timed out waiting for {label} ({url})")

wait_json(f"{solr_url.rstrip('/')}/admin/cores?action=STATUS&wt=json",
          lambda d: d.get("responseHeader", {}).get("status") == 0,
          "isolated profiling Solr", 120)
wait_json(f"{backend_url.rstrip('/')}/",
          lambda d: "databaseProductName" in d,
          "isolated profiling backend", 300)
PY

# Verify challenge-response DB identity nonce and Solr identity core
STACK_ENV="$(python3 -I "$HERE/scripts/profiling_stack.py" verify --state "$STACK_STATE" --emit-env)"
STACK_ARGS=()
while IFS='=' read -r key value; do
  case "$key" in
    ARCHIVALIST_PROFILING_STACK_BACKEND_URL) STACK_ARGS+=(-e "BACKEND_URL=$value") ;;
    ARCHIVALIST_PROFILING_STACK_SOLR_URL) STACK_ARGS+=(-e "SOLR_BASE_URL=$value") ;;
    ARCHIVALIST_PROFILING_STACK_*) STACK_ARGS+=(-e "$key=$value") ;;
  esac
done <<< "$STACK_ENV"

sha256_of() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

GIT_COMMIT="$(git -C "$HERE" rev-parse HEAD 2>/dev/null || echo unrecorded)"
GIT_DIRTY_PATHS="$(git -C "$HERE/.." status --porcelain 2>/dev/null || true)"
if [ "$GIT_COMMIT" = unrecorded ]; then GIT_DIRTY=unrecorded; elif [ -n "$GIT_DIRTY_PATHS" ]; then GIT_DIRTY=true; else GIT_DIRTY=false; fi

CONTAINER_BIN=/archivesspace/rust-tools/target/release/aspace-shadow-sidecar
HOST_BIN="$HERE/target/release/aspace-shadow-sidecar"
BUILD_MODE=release
BUILD_CMD="cargo build --release --locked -p aspace-cli --bin aspace-shadow-sidecar"
if [ "${SIDECAR_BUILD:-1}" != 0 ]; then
  (cd "$HERE" && $BUILD_CMD)
fi
[ -x "$HOST_BIN" ] || { echo "ERROR: Release sidecar missing at $HOST_BIN" >&2; exit 1; }
EXPECTED_SHA="$(sha256_of "$HOST_BIN")"

{
  printf '{\n  "git_commit": "%s",\n  "git_dirty": "%s",\n' "$GIT_COMMIT" "$GIT_DIRTY"
  printf '  "build_command": "%s",\n  "container_binary": "%s",\n' "$BUILD_CMD" "$CONTAINER_BIN"
  printf '  "host_sha256": "%s",\n  "stack_id": "%s"\n}\n' "$EXPECTED_SHA" "$STACK_ID"
} > "$BASE.build.json"

CONTAINER_JAVA_OPTS="$(docker exec "$APP_CONTAINER" printenv JAVA_OPTS)"
set +e
docker exec "${STACK_ARGS[@]}" \
  -e REPO_ID="$REPO_ID" \
  -e JAVA_OPTS="$CONTAINER_JAVA_OPTS -Xss${PROFILE_XSS:-16m}" \
  -e ARCHIVALIST_SIDECAR_BINARY="" \
  -e ARCHIVALIST_SIDECAR_SHA256="$EXPECTED_SHA" \
  -e ARCHIVALIST_SIDECAR_BUILD_MODE="$BUILD_MODE" \
  -e ARCHIVALIST_SIDECAR_BUILD_COMMAND="$BUILD_CMD" \
  -e ARCHIVALIST_GIT_COMMIT="$GIT_COMMIT" \
  -e ARCHIVALIST_GIT_DIRTY="$GIT_DIRTY" \
  -e ARCHIVALIST_GIT_DIRTY_PATHS="$GIT_DIRTY_PATHS" \
  -w /archivesspace "$APP_CONTAINER" \
  /archivesspace/scripts/jruby /archivesspace/rust-tools/scripts/pui_concurrency_safety_gate.rb \
  > "$BASE.stdout" 2> "$BASE.stderr"
rc=$?
set -e
echo "$rc" > "$BASE.exit"

if [ "$rc" -ne 0 ]; then
  echo "Gate failed with exit=$rc (stdout=$BASE.stdout stderr=$BASE.stderr)" >&2
  exit "$rc"
fi

CONTAINER_REPORT_PATH="$(sed -n 's/^Gate Report: //p' "$BASE.stdout" | tail -n 1)"
REPORT_PATH="$HERE/benchmarks/$(basename "$CONTAINER_REPORT_PATH")"
if [ -z "$CONTAINER_REPORT_PATH" ] || [ ! -f "$REPORT_PATH" ]; then
  echo "ERROR: Gate report path missing after exit 0 ($REPORT_PATH)" >&2
  exit 1
fi

# Cross-check with compare_native_profile_cores.compare() read-only function while isolated Solr is up
python3 -I - "$HERE/scripts" "$SOLR_HOST_URL" "$REPORT_PATH" > "$BASE.comparator.txt" <<'PY'
import json, re, sys
sys.path.insert(0, sys.argv[1])
import compare_native_profile_cores as comparator

solr_base = sys.argv[2].rstrip("/")
with open(sys.argv[3]) as fh:
    report = json.load(fh)

core_t1 = report["passes"]["pui_threads_1"]["core"]
core_t4 = report["passes"]["pui_threads_4"]["core"]
expected = int(report["repository"]["total_records"])

pattern = re.compile(r"\Astage_pui_[0-9]{8}_[0-9]{6}_[0-9]+_[0-9a-f]{8}[a-z0-9]{2}_candidate\Z")
if not pattern.fullmatch(core_t1) or not pattern.fullmatch(core_t4) or core_t1 == core_t4:
    raise SystemExit(f"Invalid candidate core names: {core_t1!r}, {core_t4!r}")

rc = comparator.compare(solr_base, core_t1, core_t4, expected)
if rc != 0:
    raise SystemExit(f"compare_native_profile_cores.compare returned {rc}")
PY

echo "stdout=$BASE.stdout stderr=$BASE.stderr build=$BASE.build.json report=$REPORT_PATH comparator=$BASE.comparator.txt exit=$rc"
