#!/usr/bin/env bash
#
# benchmark_recommended_profile.sh
#
# This script runs the CSV matrix benchmark using our current conservative
# local operating envelope.
#
# Note: This profile is specifically tuned for local ArchivesSpace instances
# (e.g., typical developer hardware) and is not intended as universal production
# guidance. Production environments may handle vastly different concurrency and
# batch size values.
#
# Defaults can be overridden via environment variables.

set -euo pipefail

# --- Default Profile Configuration ---
export REPO_ID="${REPO_ID:-2}"
export URL="${URL:-http://localhost:4567}"
export CONCURRENCIES="${CONCURRENCIES:-12}"
export BATCH_SIZES="${BATCH_SIZES:-100}"
export REPEAT="${REPEAT:-3}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_TOOLS_DIR="$(dirname "$SCRIPT_DIR")"
BENCHMARK_DIR="$RUST_TOOLS_DIR/benchmarks"

echo "============================================================"
echo " Running Benchmark: Local Recommended Profile"
echo "============================================================"
echo " Note: This is not universal production guidance."
echo ""
echo " Configuration:"
echo "   REPO_ID       : ${REPO_ID}"
echo "   URL           : ${URL}"
echo "   CONCURRENCIES : ${CONCURRENCIES}"
echo "   BATCH_SIZES   : ${BATCH_SIZES}"
echo "   REPEAT        : ${REPEAT}"
echo "============================================================"

# Call the matrix script which will use the exported variables
"${SCRIPT_DIR}/benchmark_csv_matrix.sh"

echo "============================================================"
echo " Benchmark complete. Finding latest CSV and summarizing..."
echo "============================================================"

if [ -d "$BENCHMARK_DIR" ]; then
    # Find the most recently created CSV file
    NEWEST_CSV=$(ls -t "$BENCHMARK_DIR"/benchmark_matrix_*.csv 2>/dev/null | head -n 1 || true)

    if [ -n "$NEWEST_CSV" ] && [ -f "$NEWEST_CSV" ]; then
        echo "Found: $NEWEST_CSV"
        echo ""
        "${SCRIPT_DIR}/summarize_benchmark_csv.sh" "$NEWEST_CSV"
    else
        echo "Error: Could not find any benchmark_matrix_*.csv files in $BENCHMARK_DIR."
        exit 1
    fi
else
    echo "Error: Benchmark directory $BENCHMARK_DIR does not exist."
    exit 1
fi
