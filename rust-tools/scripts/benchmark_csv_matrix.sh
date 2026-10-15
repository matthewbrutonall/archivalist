#!/usr/bin/env bash
# Runs a matrix of repository-scan CSV benchmarks

set -euo pipefail

# Default variables
REPO_ID="${REPO_ID:-2}"
URL="${URL:-http://localhost:4567}"
CONCURRENCIES="${CONCURRENCIES:-1 2 4 8}"
BATCH_SIZES="${BATCH_SIZES:-25 50 100}"
REPEAT="${REPEAT:-3}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_TOOLS_DIR="$(dirname "$SCRIPT_DIR")"
BENCHMARK_DIR="$RUST_TOOLS_DIR/benchmarks"

# Create benchmarks directory if it doesn't exist
mkdir -p "$BENCHMARK_DIR"

TIMESTAMP=$(date +"%Y%m%d_%H%M%S")
OUT_CSV="$BENCHMARK_DIR/benchmark_matrix_${TIMESTAMP}.csv"
COMMANDS_LOG="$BENCHMARK_DIR/benchmark_matrix_${TIMESTAMP}_commands.log"
ERRORS_LOG="$BENCHMARK_DIR/benchmark_matrix_${TIMESTAMP}_errors.log"

echo "Starting repository-scan CSV benchmark matrix..."
echo "Output CSV: $OUT_CSV"
echo "Commands log: $COMMANDS_LOG"
echo "Errors log: $ERRORS_LOG"
echo "URL: $URL"
echo "REPO_ID: $REPO_ID"
echo "CONCURRENCIES: $CONCURRENCIES"
echo "BATCH_SIZES: $BATCH_SIZES"
echo "REPEAT: $REPEAT"

cd "$RUST_TOOLS_DIR"

TMP_OUT=$(mktemp)
trap 'rm -f "$TMP_OUT"' EXIT

HEADER_WRITTEN=0

for c in $CONCURRENCIES; do
    for b in $BATCH_SIZES; do
        echo "Running: concurrency=$c, batch_size=$b"

        CMD=(cargo run --offline -q -p aspace-cli -- -u "$URL" perf repository-scan --repo "$REPO_ID" --csv --repeat "$REPEAT" --batch-size "$b" --concurrency "$c" --warmup)

        # Log the command
        echo "[c=$c, b=$b] ${CMD[*]}" >> "$COMMANDS_LOG"

        # Allow failure to not break the whole matrix
        if "${CMD[@]}" > "$TMP_OUT" 2>>"$ERRORS_LOG"; then
            if [ "$HEADER_WRITTEN" -eq 0 ]; then
                # Include header
                cat "$TMP_OUT" > "$OUT_CSV"
                HEADER_WRITTEN=1
            else
                # Append skipping header
                tail -n +2 "$TMP_OUT" >> "$OUT_CSV"
            fi
        else
            echo "Warning: Scan failed for concurrency=$c, batch_size=$b" >&2
        fi
    done
done

if [ "$HEADER_WRITTEN" -eq 0 ]; then
    echo "Error: No benchmark cells succeeded. No CSV data written." >&2
    if [ -s "$ERRORS_LOG" ]; then
        echo "Check the errors log for details: $ERRORS_LOG" >&2
    fi
    exit 1
fi

echo "Done."
echo "Results saved to:"
echo "  - $OUT_CSV"
echo "  - $COMMANDS_LOG"
if [ -s "$ERRORS_LOG" ]; then
    echo "  - $ERRORS_LOG"
else
    rm -f "$ERRORS_LOG"
fi
