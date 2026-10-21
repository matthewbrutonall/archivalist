#!/usr/bin/env python3
"""Summarize an ARCHIVALIST_PROFILE stage-run profile JSON (aggregates only).

Usage: summarize_indexing_profile.py repo_stage_profile_<RUN_ID>.json

Reads `modes.<label>` entries written by repository_integration_stage_harness.rb and prints,
per mode: uninstrumented and profiled medians (so profiling overhead is visible) and the median
across profiled repeats of each category's exclusive milliseconds and share of the region.
Medians are per category over repeats (not guaranteed to sum to the region median).
"""
import json
import statistics
import sys


def median(values):
    return statistics.median(values) if values else None


def summarize_mode(mode):
    repeats = mode.get("profiled_repeats", [])
    cats = {}
    for rep in repeats:
        summary = rep.get("summary") or {}
        for cat, ms in summary.get("by_category_exclusive_ms", {}).items():
            cats.setdefault(cat, []).append(ms)
    region = [rep["summary"]["region_ms"] for rep in repeats if rep.get("summary")]
    region_median = median(region)
    rows = {}
    for cat, values in cats.items():
        # a category absent from a repeat contributed 0 ms there
        padded = values + [0.0] * (len(repeats) - len(values))
        med = median(padded)
        rows[cat] = {
            "median_exclusive_ms": round(med, 3),
            "share_of_region_median": round(med / region_median, 4) if region_median else None,
        }
    uninstr = mode.get("uninstrumented_median_ms")
    profiled = mode.get("profiled_median_ms")
    return {
        "uninstrumented_median_ms": uninstr,
        "profiled_median_ms": profiled,
        "profiling_overhead_ratio": round(profiled / uninstr, 3) if uninstr and profiled else None,
        "region_median_ms": region_median,
        "categories": dict(sorted(rows.items(), key=lambda kv: -kv[1]["median_exclusive_ms"])),
    }


def summarize(doc):
    return {label: summarize_mode(mode) for label, mode in doc.get("modes", {}).items()}


def main(argv):
    if len(argv) != 2:
        print(__doc__)
        return 2
    with open(argv[1]) as fh:
        doc = json.load(fh)
    out = summarize(doc)
    for label, s in out.items():
        print(f"== {label}: inactive-wrapper {s['uninstrumented_median_ms']} ms, profiled {s['profiled_median_ms']} ms "
              f"(x{s['profiling_overhead_ratio']})")
        for cat, row in s["categories"].items():
            print(f"   {cat:<60} {row['median_exclusive_ms']:>10} ms  {row['share_of_region_median']}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
