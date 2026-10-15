#!/usr/bin/env bash
# summarize_benchmark_csv.sh
# Summarizes a repository-scan benchmark matrix CSV

set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <benchmark_matrix.csv>"
    exit 1
fi

CSV_FILE="$1"

if [[ ! -f "$CSV_FILE" ]]; then
    echo "Error: File '$CSV_FILE' not found."
    exit 1
fi

# AWK script to aggregate and output stats
RAW_STATS=$(awk -F, '
NR>1 {
    run_idx = $2
    tot_rec = $4
    batch_sz = $6
    conc = $7
    elapsed = $9
    success = $11

    # We aggregate all rows, assuming matrix script output contains complete runs
    key = run_idx SUBSEP conc SUBSEP batch_sz
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
        conc = parts[2]
        batch_sz = parts[3]

        if (run_elapsed[key] > 0) {
            run_rec_sec = run_records[key] / run_elapsed[key]
        } else {
            run_rec_sec = 0
        }

        cb_key = conc SUBSEP batch_sz

        cb_runs[cb_key]++
        cb_elapsed_sum[cb_key] += run_elapsed[key]
        cb_rec_sec_sum[cb_key] += run_rec_sec

        if (!(cb_key in cb_seen)) {
            cb_seen[cb_key] = 1
            cb_list[++num_cbs] = cb_key
        }
    }

    for (i = 1; i <= num_cbs; i++) {
        cb_key = cb_list[i]
        split(cb_key, parts, SUBSEP)
        conc = parts[1]
        batch_sz = parts[2]

        runs = cb_runs[cb_key]
        mean_elapsed = cb_elapsed_sum[cb_key] / runs
        mean_rec_sec = cb_rec_sec_sum[cb_key] / runs

        printf "%d %d %d %.3f %.3f\n", conc, batch_sz, runs, mean_elapsed, mean_rec_sec
    }
}' "$CSV_FILE" | sort -n -k1,1 -k2,2)

if [[ -z "$RAW_STATS" ]]; then
    echo "No data found in CSV."
    exit 0
fi

# Output markdown table
echo "| Concurrency | Batch Size | Runs | Mean Elapsed (s) | Mean Records/sec |"
echo "|-------------|------------|------|------------------|------------------|"
while read -r conc batch_size runs mean_elapsed mean_rps; do
    printf "| %11s | %10s | %4s | %16s | %16s |\n" "$conc" "$batch_size" "$runs" "$mean_elapsed" "$mean_rps"
done <<< "$RAW_STATS"

# Find best throughput cell
BEST_CELL=$(echo "$RAW_STATS" | sort -n -k5,5 -r | head -n 1)
read -r best_conc best_batch best_runs best_elapsed best_rps <<< "$BEST_CELL"

echo ""
echo "**Best Throughput:** Concurrency $best_conc, Batch Size $best_batch -> $best_rps records/sec"

# Determine trend for the best batch size
TREND_STATS=$(echo "$RAW_STATS" | awk -v bb="$best_batch" '$2 == bb {print $1, $5}' | sort -n -k1,1)

echo ""
echo "**Concurrency Trend (Batch Size $best_batch):**"
PREV_RPS=0
PREV_CONC=""
while read -r conc rps; do
    if [[ -z "$PREV_CONC" ]]; then
        echo "- Concurrency $conc: $rps rec/sec (Baseline)"
    else
        DIFF=$(awk -v r1="$rps" -v r2="$PREV_RPS" 'BEGIN { printf "%.3f", r1 - r2 }')
        IMPROVEMENT=$(awk -v diff="$DIFF" 'BEGIN { if (diff > 0) print "Improves"; else if (diff < 0) print "Degrades"; else print "Flat" }')
        printf -- "- Concurrency %s: %s rec/sec (%s by %+.3f)\n" "$conc" "$rps" "$IMPROVEMENT" "$DIFF"
    fi
    PREV_RPS="$rps"
    PREV_CONC="$conc"
done <<< "$TREND_STATS"
