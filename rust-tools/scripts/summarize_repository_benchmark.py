#!/usr/bin/env python3
import argparse
import glob
import json
import os
import re
import sys

def parse_stats(filepath):
    stats = {}
    if not os.path.exists(filepath):
        return stats
    with open(filepath, 'r') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            parts = re.split(r'\s{2,}', line)
            if len(parts) >= 4:
                name = parts[1]
                if name in ('archivesspaceai-app-1', 'archivesspaceai-db-1', 'archivesspaceai-solr-1'):
                    short_name = name.replace('archivesspaceai-', '').replace('-1', '')
                    cpu = parts[2]
                    mem = parts[3].split(' / ')[0]
                    stats[short_name] = f"{cpu}, {mem}"
            else:
                # fallback if parsing is weird
                if 'archivesspaceai-app-1' in line:
                    stats['app'] = line[:30] + "..."
                elif 'archivesspaceai-db-1' in line:
                    stats['db'] = line[:30] + "..."
                elif 'archivesspaceai-solr-1' in line:
                    stats['solr'] = line[:30] + "..."
    return stats

def parse_memory_to_mib(mem_str):
    # e.g., "756.8MiB / 125GiB" or "756.8MiB/125GiB"
    mem_str = mem_str.split('/')[0].strip().replace(' ', '')
    match = re.match(r'^([\d\.]+)([a-zA-Z]+)$', mem_str)
    if not match:
        raise ValueError(f"Unknown memory format: {mem_str}")
    val, unit = match.groups()
    val = float(val)
    unit = unit.lower()

    if unit in ('b', 'bytes'):
        return val / (1024**2)
    elif unit in ('kb', 'kib'):
        return val / 1024
    elif unit in ('mb', 'mib'):
        return val
    elif unit in ('gb', 'gib'):
        return val * 1024
    elif unit in ('tb', 'tib'):
        return val * (1024**2)
    raise ValueError(f"Unknown unit: {unit}")

def parse_during_stats(filepath):
    stats = {
        'app': {'cpu': None, 'mem': None},
        'db': {'cpu': None, 'mem': None},
        'solr': {'cpu': None, 'mem': None}
    }
    if not os.path.exists(filepath):
        return stats

    with open(filepath, 'r') as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith('---') or 'CONTAINER ID' in line:
                continue

            parts = re.split(r'\s{2,}', line)
            if len(parts) >= 4:
                name = parts[1]
                short_name = None
                if name == 'archivesspaceai-app-1':
                    short_name = 'app'
                elif name == 'archivesspaceai-db-1':
                    short_name = 'db'
                elif name == 'archivesspaceai-solr-1':
                    short_name = 'solr'

                if short_name:
                    cpu_str = parts[2].replace('%', '')
                    try:
                        cpu = float(cpu_str)
                        if stats[short_name]['cpu'] is None or cpu > stats[short_name]['cpu']:
                            stats[short_name]['cpu'] = cpu
                    except ValueError:
                        pass

                    try:
                        mem_mib = parse_memory_to_mib(parts[3])
                        if stats[short_name]['mem'] is None or mem_mib > stats[short_name]['mem']:
                            stats[short_name]['mem'] = mem_mib
                    except ValueError:
                        pass
    return stats

def format_peak(peak_stats):
    if peak_stats['cpu'] is None or peak_stats['mem'] is None:
        return "N/A"

    mem_mib = peak_stats['mem']
    if mem_mib >= 1024:
        mem_str = f"{mem_mib / 1024:.2f}GiB"
    else:
        mem_str = f"{mem_mib:.2f}MiB"

    return f"{peak_stats['cpu']:.2f}%, {mem_str}"

def main():
    parser = argparse.ArgumentParser(description="Generate a Markdown summary from repository benchmark outputs.")
    parser.add_argument("prefix", help="Benchmark prefix, e.g., rust-tools/benchmarks/bench_20260914_150342")
    parser.add_argument("--output", help="Optional Markdown file to append the report to", default=None)
    args = parser.parse_args()

    # Find matching json files
    pattern = f"{args.prefix}_c*.json"
    json_files = glob.glob(pattern)
    if not json_files:
        print(f"No files found matching {pattern}")
        sys.exit(1)

    # Sort by concurrency
    def get_c(f):
        m = re.search(r'_c(\d+)\.json$', f)
        return int(m.group(1)) if m else 0

    json_files.sort(key=get_c)

    table_header = (
        "| Concurrency | Cmd Time | Mean Total Sec | Tput (r/s) | AO Sec | AO Tput | "
        "App (Before \u2192 After) | DB (Before \u2192 After) | Solr (Before \u2192 After) | "
        "App Peak | DB Peak | Solr Peak |\n"
        "|---|---|---|---|---|---|---|---|---|---|---|---|"
    )

    lines = []
    lines.append(f"## Benchmark Report: `{os.path.basename(args.prefix)}`")
    lines.append("")
    lines.append(table_header)

    for jf in json_files:
        c_num = get_c(jf)
        with open(jf, 'r') as f:
            data = json.load(f)

        agg = data.get('aggregate', {})
        total_sec = agg.get('mean_total_elapsed_sec', 0.0)
        tput = agg.get('mean_overall_records_per_sec', 0.0)

        ao_sec = 0.0
        ao_tput = 0.0
        for t in agg.get('targets', []):
            if t.get('record_type') == 'archival_object':
                ao_sec = t.get('mean_total_elapsed_sec', 0.0)
                ao_tput = t.get('mean_records_per_sec', 0.0)
                break

        # Parse time
        time_file = jf.replace('.json', '_time.txt')
        cmd_time = "N/A"
        if os.path.exists(time_file):
            with open(time_file, 'r') as tf:
                for line in tf:
                    if line.startswith('real'):
                        cmd_time = line.strip().split('\t')[1]
                        break

        # Parse stats
        before_stats_file = jf.replace('.json', '_before.stats')
        after_stats_file = jf.replace('.json', '_after.stats')
        during_stats_file = jf.replace('.json', '_during.stats')

        b_stats = parse_stats(before_stats_file)
        a_stats = parse_stats(after_stats_file)
        d_stats = parse_during_stats(during_stats_file)

        app_str = f"{b_stats.get('app', 'N/A')} &rarr; {a_stats.get('app', 'N/A')}"
        db_str = f"{b_stats.get('db', 'N/A')} &rarr; {a_stats.get('db', 'N/A')}"
        solr_str = f"{b_stats.get('solr', 'N/A')} &rarr; {a_stats.get('solr', 'N/A')}"

        app_peak = format_peak(d_stats['app'])
        db_peak = format_peak(d_stats['db'])
        solr_peak = format_peak(d_stats['solr'])

        row = (
            f"| {c_num} | {cmd_time} | {total_sec:.2f} | {tput:.2f} | {ao_sec:.2f} | {ao_tput:.2f} | "
            f"{app_str} | {db_str} | {solr_str} | {app_peak} | {db_peak} | {solr_peak} |"
        )
        lines.append(row)

    report = "\n".join(lines)
    print(report)

    if args.output:
        with open(args.output, 'a') as f:
            f.write("\n" + report + "\n")
        print(f"\nAppended report to {args.output}")

if __name__ == "__main__":
    main()
