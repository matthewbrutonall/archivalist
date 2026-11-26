#!/usr/bin/env python3
"""Run a child process and record its own peak resident memory (Linux).

The Rust candidate harnesses use this to measure the aspace-cli process's peak
RSS alongside wall time. The figure comes from the kernel's rusage for that
exact target child PID (``os.wait4``), so it is a true per-process peak covering
the whole process lifetime. It does not include grandchildren, Docker containers,
Solr, MySQL or the ArchivesSpace backend.

On Linux, ``fork()`` duplicates the parent's page tables into the child's
initial ``mm_struct`` and ``execve()`` folds ``get_mm_hiwater_rss(old_mm)`` into
``tsk->signal->maxrss``. A direct ``fork()+execve()`` from a large Python runner
(such as fixed-reference mode holding ~9 GiB of native Solr documents) therefore
contaminates the child's ``ru_maxrss`` with the parent's RSS at fork time.
Because ``fork()`` resets ``signal->maxrss`` to zero in each new child, we first
``fork()+execve()`` a tiny clean Python helper (``python3 -S -E``) whose fresh
post-exec ``mm_struct`` is only ~11 MiB, and that helper forks/execs the target
command and reaps it with ``os.wait4``.

Native JRuby indexing is measured differently (``/proc/self/status`` VmHWM
inside the container, also process-lifetime). The two are comparable only
when each native process runs a single pass in a fresh JVM, which is how the
``--fresh-native`` harness round runs (``REPEATS=1``). JVM RSS also depends on
heap settings (-Xmx), so reports must record those alongside the comparison.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import threading
from dataclasses import dataclass
from typing import Any, Dict, List, Mapping, Optional


_CLEAN_EXEC_HELPER = """
import errno, json, os, signal, subprocess, sys
meta_fd = int(sys.argv[1])
cmd = sys.argv[2:]
try:
    proc = subprocess.Popen(cmd, close_fds=True)
except OSError as exc:
    err_payload = json.dumps({
        "os_error": True,
        "errno": exc.errno,
        "strerror": exc.strerror or str(exc),
        "filename": exc.filename,
    }).encode("utf-8")
    os.write(meta_fd, err_payload)
    os.close(meta_fd)
    sys.exit(127)

def _forward(signum, _frame):
    try:
        os.kill(proc.pid, signum)
    except ProcessLookupError:
        pass

for _sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
    signal.signal(_sig, _forward)

while True:
    try:
        _pid, status, rusage = os.wait4(proc.pid, 0)
        break
    except InterruptedError:
        continue
    except OSError as exc:
        if exc.errno == errno.EINTR:
            continue
        raise

rc = os.waitstatus_to_exitcode(status)
proc.returncode = rc
payload = json.dumps({
    "returncode": rc,
    "peak_rss_kib": int(rusage.ru_maxrss),
}).encode("utf-8")
os.write(meta_fd, payload)
os.close(meta_fd)
"""


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
    env=env)`` plus ``peak_rss_kib`` (Linux ``ru_maxrss`` is in KiB), isolated
    from parent-process RSS via a clean exec helper.
    """
    if not cmd:
        raise ValueError("cmd must not be empty")
    meta_r, meta_w = os.pipe()
    helper_cmd = [sys.executable, "-S", "-E", "-c", _CLEAN_EXEC_HELPER, str(meta_w), *cmd]
    try:
        proc = subprocess.Popen(
            helper_cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=dict(env) if env is not None else None,
            pass_fds=(meta_w,),
        )
    except Exception:
        os.close(meta_r)
        os.close(meta_w)
        raise
    os.close(meta_w)

    out: List[str] = []
    err: List[str] = []
    readers = [
        threading.Thread(target=_drain, args=(proc.stdout, out), daemon=True),
        threading.Thread(target=_drain, args=(proc.stderr, err), daemon=True),
    ]
    for reader in readers:
        reader.start()
    try:
        helper_rc = proc.wait()
        for reader in readers:
            reader.join()
        chunks: List[bytes] = []
        while True:
            chunk = os.read(meta_r, 4096)
            if not chunk:
                break
            chunks.append(chunk)
    finally:
        os.close(meta_r)

    raw_meta = b"".join(chunks).decode("utf-8").strip()
    if not raw_meta:
        if helper_rc != 0:
            return PeakRssResult(
                returncode=helper_rc,
                stdout="".join(out),
                stderr="".join(err),
                peak_rss_kib=0,
            )
        raise RuntimeError("Clean exec helper exited without reporting target rusage")

    meta = json.loads(raw_meta)
    if meta.get("os_error"):
        err_no = meta.get("errno") or 2
        raise OSError(err_no, meta.get("strerror") or "exec error", meta.get("filename"))

    return PeakRssResult(
        returncode=int(meta["returncode"]),
        stdout="".join(out),
        stderr="".join(err),
        peak_rss_kib=int(meta["peak_rss_kib"]),
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
