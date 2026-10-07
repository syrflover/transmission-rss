#!/usr/bin/env python3
"""A table of run.sh measurements, one column per label.

    summary.py <label>=<out dir>[,<out dir>...][:<symfs>] ...

Each cell is the mean over the label's measurements, with the least and the
most. <symfs> is the folder holding the image's /usr/local/bin (and, for a
dynamic libc, the library and its debug file) for perf's symbols; without it
the allocator rows are left out.
"""
import json
import re
import statistics
import subprocess
import sys
from pathlib import Path

# musl (mallocng), mimalloc and glibc's allocator functions.
ALLOC = re.compile(
    r"^(malloc|free|realloc|calloc|cfree|aligned_alloc|posix_memalign|memalign"
    r"|__libc_(malloc|malloc_impl|free|realloc|calloc|memalign)"
    r"|alloc_slot|alloc_group|get_meta|enframe|__malloc_\w+|__lock|__unlock|__wake|__futexwait"
    r"|nontrivial_free|get_stride|size_to_class|try_avail|activate_group|free_group|okay_to_free"
    r"|mi_\w+|_mi_\w+"
    r"|_int_\w+|malloc_consolidate|unlink_chunk|sysmalloc|tcache\w*|arena_get2|_mid_memalign|systrim|heap_trim|new_heap|grow_heap)$"
)
# SQLite's SQL parser.
PARSER = re.compile(r"^(sqlite3RunParser|yy_reduce|sqlite3GetToken|keywordCode)$")


def phases(out):
    rows = {}
    for line in (out / "stats.txt").read_text().splitlines():
        phase, container, *fields = line.split()
        rows[(phase, container)] = {k: float(v) for k, v in (f.split("=", 1) for f in fields)}
    return rows


def shares(data, symfs):
    report = subprocess.run(
        ["perf", "report", "-i", str(data), f"--symfs={symfs}", "--no-children", "--sort", "sym",
         "--stdio", "-g", "none"],
        capture_output=True, text=True, check=True,
    ).stdout
    alloc = parser = 0.0
    for line in report.splitlines():
        m = re.match(r"\s+([\d.]+)%\s+\[\.\]\s+(.*)$", line)
        if m:
            sym = re.sub(r"(\.(constprop|isra|part|cold)\.\d+)+$", "", m.group(2).strip().split("@")[0])
            alloc += float(m.group(1)) if ALLOC.match(sym) else 0
            parser += float(m.group(1)) if PARSER.match(sym) else 0
    return alloc, parser


def measurement(out, symfs):
    st = phases(out)
    web = json.loads((out / "web.json").read_text())
    rescans = [json.loads(l) for l in (out / "rescan.json").read_text().splitlines()]

    # Measurements made before run.sh recorded page faults and memory.events
    # lack those counters; their rows are left out.
    def delta(container, a, b, key):
        if key not in st[(a, container)]:
            return None
        return st[(b, container)][key] - st[(a, container)][key]

    def total(key):
        if key not in end["trss-web"]:
            return None
        return end["trss-web"][key] + end["trss-worker"][key]

    end = {c: st[("rescan", c)] for c in ("trss-web", "trss-worker")}
    row = {
        "web load CPU s": delta("trss-web", "warm", "web", "usage_usec") / 1e6,
        "web load user s": delta("trss-web", "warm", "web", "user_usec") / 1e6,
        "web load kernel s": delta("trss-web", "warm", "web", "system_usec") / 1e6,
        "web load page faults": delta("trss-web", "warm", "web", "pgfault"),
        "web load wall s": web["wall_s"],
        "web requests failed": web["errors"],
        "web p50 ms": web["p50_ms"],
        "web p95 ms": web["p95_ms"],
        "rescan CPU s": delta("trss-worker", "web", "rescan", "usage_usec") / 1e6,
        "rescan user s": delta("trss-worker", "web", "rescan", "user_usec") / 1e6,
        "rescan kernel s": delta("trss-worker", "web", "rescan", "system_usec") / 1e6,
        "rescan page faults": delta("trss-worker", "web", "rescan", "pgfault"),
        "rescans not done": sum(s != "done" for r in rescans for s in r["states"]),
        "web peak MiB": end["trss-web"]["mem_peak"] / 2**20,
        "worker peak MiB": end["trss-worker"]["mem_peak"] / 2**20,
        "memory.events max (web+worker)": total("ev_max"),
        "oom_kill (web+worker)": total("oom_kill"),
    }
    row = {k: v for k, v in row.items() if v is not None}
    if symfs:
        for p in ("web", "worker"):
            alloc, parser = shares(out / f"{p}.perf", symfs)
            row[f"{p} user samples in the allocator %"] = alloc
            row[f"{p} user samples in the SQL parser %"] = parser
    return row


def main(args):
    if not args:
        print(__doc__)
        return 2
    columns = {}
    for arg in args:
        label, rest = arg.split("=", 1)
        dirs, _, symfs = rest.partition(":")
        columns[label] = [measurement(Path(d), symfs) for d in dirs.split(",")]
    keys = list(dict.fromkeys(k for rows in columns.values() for r in rows for k in r))
    print("| | " + " | ".join(columns) + " |")
    print("| --- |" + " --- |" * len(columns))
    for k in keys:
        cells = []
        for rows in columns.values():
            xs = [r[k] for r in rows if k in r]
            cells.append(f"{statistics.mean(xs):.2f} ({min(xs):.2f}–{max(xs):.2f})" if xs else "–")
        print(f"| {k} | " + " | ".join(cells) + " |")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
