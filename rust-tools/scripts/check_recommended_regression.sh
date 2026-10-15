#!/usr/bin/env bash
#
# check_recommended_regression.sh
#
# Runs the recommended profile benchmark and compares it against a baseline CSV.
# Fails if any regression is detected by compare_benchmark_csv.sh.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_TOOLS_DIR="$(dirname "$SCRIPT_DIR")"
BENCHMARK_DIR="$RUST_TOOLS_DIR/benchmarks"

# Accept baseline from env var or first argument
BASELINE_ARG="${BASELINE_CSV:-${1:-}}"
BASELINE_SOURCE=""

if [[ -n "$BASELINE_ARG" ]]; then
    BASELINE="$BASELINE_ARG"
    BASELINE_SOURCE="argument/env"
else
    POINTER_FILE="$BENCHMARK_DIR/recommended_baseline.txt"
    if [[ ! -f "$POINTER_FILE" ]]; then
        echo "Error: BASELINE_CSV was not provided, and default baseline pointer file '$POINTER_FILE' is missing."
        exit 1
    fi

    BASELINE_VAL=$(cat "$POINTER_FILE" | tr -d '[:space:]')

    # Try resolving relative path
    REPO_ROOT="$(dirname "$RUST_TOOLS_DIR")"
    if [[ "$BASELINE_VAL" = /* ]] && [[ -f "$BASELINE_VAL" ]]; then
        BASELINE="$BASELINE_VAL"
    elif [[ -f "$REPO_ROOT/$BASELINE_VAL" ]]; then
        BASELINE="$REPO_ROOT/$BASELINE_VAL"
    elif [[ -f "$RUST_TOOLS_DIR/$BASELINE_VAL" ]]; then
        BASELINE="$RUST_TOOLS_DIR/$BASELINE_VAL"
    elif [[ -f "$BASELINE_VAL" ]]; then
        BASELINE="$BASELINE_VAL"
    else
        echo "Error: Baseline file '$BASELINE_VAL' specified in '$POINTER_FILE' not found."
        exit 1
    fi
    BASELINE_SOURCE="default file ($POINTER_FILE)"
fi

if [[ ! -f "$BASELINE" ]]; then
    echo "Error: Baseline file '$BASELINE' not found."
    exit 1
fi

echo "============================================================"
echo " Checking for Regression using Recommended Profile"
echo " Baseline: $BASELINE"
echo " Baseline source: $BASELINE_SOURCE"
echo "============================================================"

# Run the recommended profile benchmark
"${SCRIPT_DIR}/benchmark_recommended_profile.sh"

echo "============================================================"
echo " Identifying Candidate CSV..."
echo "============================================================"

if [[ -d "$BENCHMARK_DIR" ]]; then
    # Find the most recently created CSV file
    NEWEST_CSV=$(ls -t "$BENCHMARK_DIR"/benchmark_matrix_*.csv 2>/dev/null | head -n 1 || true)

    if [[ -n "$NEWEST_CSV" ]] && [[ -f "$NEWEST_CSV" ]]; then
        CANDIDATE="$NEWEST_CSV"
        echo "Candidate: $CANDIDATE"
    else
        echo "Error: Could not find any benchmark_matrix_*.csv files in $BENCHMARK_DIR."
        exit 1
    fi
else
    echo "Error: Benchmark directory $BENCHMARK_DIR does not exist."
    exit 1
fi

echo "============================================================"
echo " Comparing Candidate vs Baseline"
echo " Baseline : $BASELINE"
echo " Candidate: $CANDIDATE"
echo "============================================================"

set +e
"${SCRIPT_DIR}/compare_benchmark_csv.sh" "$BASELINE" "$CANDIDATE"
COMPARE_EXIT=$?
set -e

if [[ $COMPARE_EXIT -ne 0 ]]; then
    echo ""
    echo "Error: Regression detected in one or more cells."
    exit $COMPARE_EXIT
fi

echo ""
echo "Success: No regressions detected."
exit 0
