#!/usr/bin/env bash
# compare_benchmark_csv.sh
# Compares two benchmark CSV files and outputs a markdown table of records/sec.

set -euo pipefail

if [[ $# -lt 2 ]]; then
    echo "Usage: $0 <baseline.csv> <candidate.csv>"
    exit 1
fi

BASELINE="$1"
CANDIDATE="$2"

if [[ ! -f "$BASELINE" ]]; then
    echo "Error: Baseline file '$BASELINE' not found."
    exit 1
fi

if [[ ! -f "$CANDIDATE" ]]; then
    echo "Error: Candidate file '$CANDIDATE' not found."
    exit 1
fi

# AWK script to aggregate and compare
AWK_SCRIPT='
FNR>1 {
    run_idx = $2
    tot_rec = $4
    batch_sz = $6
    conc = $7
    elapsed = $9

    file_id = (NR == FNR) ? 1 : 2

    key = file_id SUBSEP run_idx SUBSEP conc SUBSEP batch_sz
    run_records[key] += tot_rec
    run_elapsed[key] += elapsed

    if (!(key in run_seen)) {
        run_seen[key] = 1
        runs_list[++num_runs] = key
    }
}
END {
    # Aggregate across runs for the same conc + batch_sz
    for (i = 1; i <= num_runs; i++) {
        key = runs_list[i]
        split(key, parts, SUBSEP)
        file_id = parts[1]
        conc = parts[3]
        batch_sz = parts[4]

        if (run_elapsed[key] > 0) {
            run_rec_sec = run_records[key] / run_elapsed[key]
        } else {
            run_rec_sec = 0
        }

        cb_key = conc SUBSEP batch_sz

        cb_runs[file_id, cb_key]++
        cb_elapsed_sum[file_id, cb_key] += run_elapsed[key]
        cb_rec_sec_sum[file_id, cb_key] += run_rec_sec

        if (!(cb_key in cb_seen)) {
            cb_seen[cb_key] = 1
            cb_list[++num_cbs] = cb_key
        }
    }

    has_regression = 0

    # Calculate means and output
    for (i = 1; i <= num_cbs; i++) {
        cb_key = cb_list[i]
        split(cb_key, parts, SUBSEP)
        conc = parts[1]
        batch_sz = parts[2]

        has_1 = ((1, cb_key) in cb_runs)
        has_2 = ((2, cb_key) in cb_runs)

        if (!has_1) {
            print "WARNING: Cell conc=" conc " batch_size=" batch_sz " exists in candidate but not in baseline." > "/dev/stderr"
            continue
        }
        if (!has_2) {
            print "WARNING: Cell conc=" conc " batch_size=" batch_sz " exists in baseline but not in candidate." > "/dev/stderr"
            continue
        }

        runs_1 = cb_runs[1, cb_key]
        mean_rps_1 = cb_rec_sec_sum[1, cb_key] / runs_1

        runs_2 = cb_runs[2, cb_key]
        mean_rps_2 = cb_rec_sec_sum[2, cb_key] / runs_2

        if (mean_rps_1 > 0) {
            pct_change = ((mean_rps_2 - mean_rps_1) / mean_rps_1) * 100.0
        } else {
            pct_change = (mean_rps_2 > 0) ? 9999.0 : 0.0
        }

        status = "stable"
        if (pct_change >= 5.0) {
            status = "improved"
        } else if (pct_change <= -5.0) {
            status = "regressed"
        }

        if (status == "regressed") {
            has_regression = 1
        }

        printf "%d %d %.3f %.3f %.3f %s\n", conc, batch_sz, mean_rps_1, mean_rps_2, pct_change, status
    }

    exit has_regression
}'

set +e
COMPARE_RAW=$(awk -F, "$AWK_SCRIPT" "$BASELINE" "$CANDIDATE" | sort -n -k1,1 -k2,2)
AWK_EXIT=$?
set -e

# Output markdown table
echo "| Concurrency | Batch Size | Baseline Rec/s | Candidate Rec/s | % Change | Status |"
echo "|-------------|------------|----------------|-----------------|----------|--------|"
if [[ -n "$COMPARE_RAW" ]]; then
    while read -r conc batch_size rps1 rps2 pct status; do
        printf "| %11s | %10s | %14.3f | %15.3f | %7.2f%% | %9s |\n" "$conc" "$batch_size" "$rps1" "$rps2" "$pct" "$status"
    done <<< "$COMPARE_RAW"
fi

if [[ $AWK_EXIT -eq 1 ]]; then
    echo ""
    echo "Error: One or more matching cells regressed."
    exit 1
fi

exit 0
