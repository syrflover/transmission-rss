#!/usr/bin/env python3
"""The units of one `cargo build --timings` report, summed per package.

    build-units.py <cargo-timing-*.html> [<top>]

Prints the report's total time, the number of units and the sum of their
durations, then each package's sum (its lib, bins and test targets
together), then the <top> longest units (default 15) with their target.
A unit's duration is cargo's, from its start to its end, so units that ran
side by side add up to more than the total time.
"""
import json
import re
import sys


def units(path):
    text = open(path, encoding="utf-8").read()
    data = re.search(r"const UNIT_DATA = (\[.*?\n\]);", text, re.S)
    total = re.search(r"<td>Total time:</td><td>([^<]+)</td>", text)
    return json.loads(data.group(1)), total.group(1) if total else "?"


def main():
    if len(sys.argv) < 2:
        print(__doc__.strip().splitlines()[2].strip())
        sys.exit(2)
    rows, total = units(sys.argv[1])
    top = int(sys.argv[2]) if len(sys.argv) > 2 else 15
    print(f"total time {total}, {len(rows)} units, sum {sum(u['duration'] for u in rows):.1f}s")
    per = {}
    for u in rows:
        per[u["name"]] = per.get(u["name"], 0) + u["duration"]
    print("\nper package")
    for name, d in sorted(per.items(), key=lambda kv: -kv[1]):
        print(f"  {name:20} {d:7.1f}s")
    print(f"\nlongest {top} units")
    for u in sorted(rows, key=lambda u: -u["duration"])[:top]:
        target = u["target"].strip() or "lib"
        print(f"  {u['name']:20} {target:40} {u['duration']:7.1f}s")


if __name__ == "__main__":
    main()
