#!/usr/bin/env python3
"""Run a child process and record its own peak resident memory (Linux).

The Rust candidate harnesses use this to measure the aspace-cli process's peak
RSS alongside wall time. The figure comes from the kernel's rusage for that
exact child PID (``os.wait4``), so it is a true per-process peak covering the
whole process lifetime. It does not include grandchildren, Docker containers,
Solr, MySQL or the ArchivesSpace backend.

Native JRuby indexing is measured differently (``/proc/self/status`` VmHWM
inside the container, also process-lifetime). The two are comparable only
when each native process runs a single pass in a fresh JVM, which is how the
``--fresh-native`` harness round runs (``REPEATS=1``). JVM RSS also depends on
heap settings (-Xmx), so reports must record those alongside the comparison.
"""

from __future__ import annotations

import os
import subprocess
import threading
from dataclasses import dataclass
from typing import Any, Dict, List, Mapping, Optional


@dataclass
class PeakRssResult:
    returncode: int
    stdout: str
    stderr: str
    peak_rss_kib: int


def _drain(stream, sink: List[str]) -> None:
    try:
        sink.append(stream.read())
    finally:
        stream.close()


def run_with_peak_rss(
    cmd: List[str], env: Optional[Mapping[str, str]] = None
) -> PeakRssResult:
    """Run ``cmd`` to completion, capturing text output and its peak RSS.

    Equivalent to ``subprocess.run(cmd, capture_output=True, text=True,
    env=env)`` plus ``peak_rss_kib`` (Linux ``ru_maxrss`` is in KiB).
    """
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=dict(env) if env is not None else None,
    )
    out: List[str] = []
    err: List[str] = []
    readers = [
        threading.Thread(target=_drain, args=(proc.stdout, out), daemon=True),
        threading.Thread(target=_drain, args=(proc.stderr, err), daemon=True),
    ]
    for reader in readers:
        reader.start()
    while True:
        try:
            _pid, status, rusage = os.wait4(proc.pid, 0)
            break
        except InterruptedError:
            continue
    for reader in readers:
        reader.join()
    returncode = os.waitstatus_to_exitcode(status)
    # Mark as reaped so Popen never waits on (or signals) a recycled PID.
    proc.returncode = returncode
    return PeakRssResult(
        returncode=returncode,
        stdout="".join(out),
        stderr="".join(err),
        peak_rss_kib=int(rusage.ru_maxrss),
    )


def compare_peak_memory(
    candidate_peak_kib: Optional[int],
    native_report: Optional[Dict[str, Any]],
    native_java_opts: Optional[str] = None,
    target_reduction_pct: float = 25.0,
) -> Dict[str, Any]:
    """Summarise candidate peak RSS against a fresh single-pass native report.

    Returns a JSON-serialisable dict. ``meets_target`` is None (not False)
    whenever either side is unmeasured, so a missing measurement is never
    reported as a pass or a fail. ``native_java_opts`` should be the JVM
    options the native round actually ran with, since they bound its RSS.
    """
    native_kib = None
    if native_report:
        results = native_report.get("results") or []
        if len(results) == 1:
            memory = results[0].get("process_memory") or {}
            value = memory.get("vm_hwm_after_kib")
            if isinstance(value, int) and value > 0:
                native_kib = value
    java_opts_note = native_java_opts
    summary: Dict[str, Any] = {
        "candidate_peak_rss_kib": candidate_peak_kib,
        "candidate_semantics": "aspace-cli process peak RSS from os.wait4 ru_maxrss (whole process lifetime)",
        "native_vm_hwm_kib": native_kib,
        "native_semantics": "fresh single-pass JRuby process VmHWM (process lifetime incl. JVM start-up)",
        "native_java_opts": java_opts_note,
        "target_reduction_pct": target_reduction_pct,
        "reduction_pct": None,
        "meets_target": None,
    }
    if isinstance(candidate_peak_kib, int) and candidate_peak_kib > 0 and native_kib:
        reduction = 100.0 * (native_kib - candidate_peak_kib) / native_kib
        summary["reduction_pct"] = round(reduction, 2)
        summary["meets_target"] = reduction >= target_reduction_pct
    return summary
