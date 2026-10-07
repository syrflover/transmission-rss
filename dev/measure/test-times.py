#!/usr/bin/env python3
"""A table of `cargo test` logs' test binary times, one column per log.

    test-times.py <label>=<log> ...

A binary is a `Running <target> (<path>)` line, and its time is the
`finished in` of the `test result:` line after it; doc tests are left out.
cargo names an integration test binary after its file (`it`), not its
package, so a binary is labelled with the package whose lib ran last: cargo
runs a package's targets together, its lib first. A `src/main.rs` starts a
package only when its name does not extend the last lib's (trss-probe has no
lib; trss-browser's `trss_browserd` is its own).
"""
import re
import sys

RUNNING = re.compile(r"^\s*Running (.+?) \(([^)]*)\)")
RESULT = re.compile(
    r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored.*finished in ([\d.]+)s"
)
COLOUR = re.compile(r"\x1b\[[0-9;]*m")


def binaries(log):
    """{label: (seconds, passed, failed, ignored)} of one log."""
    rows = {}
    package = "?"
    current = None
    for line in open(log, errors="replace"):
        line = COLOUR.sub("", line)
        m = RUNNING.match(line)
        if m:
            target, path = m.groups()
            name = re.sub(r"-[0-9a-f]{16}$", "", path.rsplit("/", 1)[-1])
            if target == "unittests src/lib.rs" or (
                target == "unittests src/main.rs" and not name.startswith(package)
            ):
                package = name
            current = f"{package} {target}"
            continue
        m = RESULT.search(line)
        if m and current:
            passed, failed, ignored, secs = m.groups()
            rows[current] = (float(secs), int(passed), int(failed), int(ignored))
            current = None
    return rows


def main(args):
    cols = [(label, binaries(log)) for label, log in (a.split("=", 1) for a in args)]
    labels = sorted(
        {k for _, rows in cols for k in rows},
        key=lambda k: -max(rows.get(k, (0,))[0] for _, rows in cols),
    )
    print("| binary | " + " | ".join(label for label, _ in cols) + " |")
    print("| --- |" + " ---: |" * len(cols))
    for k in labels:
        cells = [f"{rows[k][0]:.2f}" if k in rows else "–" for _, rows in cols]
        print(f"| {k} | " + " | ".join(cells) + " |")
    sums = [sum(r[0] for r in rows.values()) for _, rows in cols]
    print("| sum | " + " | ".join(f"{s:.1f}" for s in sums) + " |")
    print()
    for label, rows in cols:
        passed, failed, ignored = (sum(r[i] for r in rows.values()) for i in (1, 2, 3))
        print(f"{label}: {len(rows)} binaries, {passed} passed, {failed} failed, {ignored} ignored")


if __name__ == "__main__":
    main(sys.argv[1:])
