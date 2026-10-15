#!/usr/bin/env bash

set -e

# Default variables
REPO_ID="${REPO_ID:-2}"
URL="${URL:-http://localhost:4567}"
BATCH_SIZE="${BATCH_SIZE:-200}"
CONCURRENCIES="${CONCURRENCIES:-1 2 4 8}"
REPEAT="${REPEAT:-3}"
WARMUP="${WARMUP:-true}"
STATS_INTERVAL="${STATS_INTERVAL:-1}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_TOOLS_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT_DIR="$(dirname "$RUST_TOOLS_DIR")"
BENCHMARK_DIR="$RUST_TOOLS_DIR/benchmarks"

if [ "$(basename "$PROJECT_DIR")" != "Archivesspace AI" ]; then
    echo "Wrong project directory: $PROJECT_DIR" >&2
    exit 1
fi

mkdir -p "$BENCHMARK_DIR"

TIMESTAMP=$(date +"%Y%m%d_%H%M%S")
OUT_PREFIX="$BENCHMARK_DIR/bench_${TIMESTAMP}"

echo "Starting benchmark for repository-scan"
echo "URL: $URL"
echo "REPO_ID: $REPO_ID"
echo "BATCH_SIZE: $BATCH_SIZE"
echo "CONCURRENCIES: $CONCURRENCIES"
echo "REPEAT: $REPEAT"
echo "WARMUP: $WARMUP"
echo "Output prefix: $OUT_PREFIX"
echo "----------------------------------------"

SAMPLER_PID=""
cleanup() {
    if [ -n "$SAMPLER_PID" ] && kill -0 "$SAMPLER_PID" 2>/dev/null; then
        kill "$SAMPLER_PID" 2>/dev/null || true
        wait "$SAMPLER_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

for CONCURRENCY in $CONCURRENCIES; do
    echo "Running concurrency: $CONCURRENCY"

    BEFORE_STATS_FILE="${OUT_PREFIX}_c${CONCURRENCY}_before.stats"
    DURING_STATS_FILE="${OUT_PREFIX}_c${CONCURRENCY}_during.stats"
    AFTER_STATS_FILE="${OUT_PREFIX}_c${CONCURRENCY}_after.stats"
    JSON_OUT_FILE="${OUT_PREFIX}_c${CONCURRENCY}.json"

    echo "Capturing docker stats before..."
    docker stats --no-stream > "$BEFORE_STATS_FILE" || echo "Warning: docker stats before failed"

    echo "Starting background docker stats sampler..."
    > "$DURING_STATS_FILE"
    (
        while true; do
            echo "--- $(date --iso-8601=seconds) ---" >> "$DURING_STATS_FILE"
            docker stats --no-stream >> "$DURING_STATS_FILE" || true
            sleep "$STATS_INTERVAL"
        done
    ) &
    SAMPLER_PID=$!

    echo "Running cargo perf repository-scan..."

    # Construct args
    ARGS=( "-u" "$URL" "perf" "repository-scan" "--repo" "$REPO_ID" "--batch-size" "$BATCH_SIZE" "--concurrency" "$CONCURRENCY" "--repeat" "$REPEAT" "--json" )
    if [ "$WARMUP" = "true" ] || [ "$WARMUP" = "1" ]; then
        ARGS+=( "--warmup" )
    fi

    cd "$RUST_TOOLS_DIR"
    TIME_OUT_FILE="${OUT_PREFIX}_c${CONCURRENCY}_time.txt"
    # Capture raw time output to a separate file, while json goes to JSON_OUT_FILE
    set +e
    { time cargo run --offline -q -p aspace-cli -- "${ARGS[@]}" > "$JSON_OUT_FILE"; } 2> "$TIME_OUT_FILE"
    SCAN_STATUS=$?
    set -e

    # Stop the sampler
    cleanup
    SAMPLER_PID=""

    echo "Capturing docker stats after..."
    docker stats --no-stream > "$AFTER_STATS_FILE" || echo "Warning: docker stats after failed"
    if [ "$SCAN_STATUS" -ne 0 ]; then
        echo "Scan failed for concurrency $CONCURRENCY with exit code $SCAN_STATUS" >&2
        exit "$SCAN_STATUS"
    fi

    echo "Finished concurrency $CONCURRENCY"
    echo "----------------------------------------"
done

echo "All benchmarks completed. Files saved to $BENCHMARK_DIR/"
