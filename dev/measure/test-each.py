#!/usr/bin/env python3
"""Each test's time in one test binary, run alone and run together (ticket lighter-structure/0160).

    test-each.py <out dir> <cargo test selection> ...

e.g. `test-each.py dev/local/measure/0160/x -p trss --test it`. The binary
is built with `cargo test --locked -j 4 --no-run <selection>` and then run
by `cargo test` twice, so that it gets the environment cargo gives it (cargo
sets SSL_CERT_FILE and SSL_CERT_DIR, and the [env] of .cargo/config.toml):

  alone     `--test-threads=1`: each test with the machine to itself
  together  libtest's default thread count, as `cargo test` runs it

each with `-Zunstable-options --format json --report-time`, which give every
test's own time; RUSTC_BOOTSTRAP=1 lets a stable libtest take them. It is set
for the binary only, through a runner (`env`), never for cargo, which would
rebuild. The wall time is libtest's own (`finished in`).

Writes the runs' JSON lines, each after the seconds since cargo started at
which it arrived (alone.jsonl, together.jsonl), cargo's output (*.cargo.log)
and a report (report.txt) into <out dir>, and prints the report: per run the
wall time, the sum of the tests' times, the count and the CPU time; the test
that ended last when they ran together, with when it began and ended; how
much longer the tests took together than alone; and the longest tests in
each.

The CPU time is the binary's, its child processes' and cargo's, user and
system. A test's time is its own function's, setup included. The gap
between the wall time and the sum, alone, is what the harness spends between
tests. A test much slower together than alone waits on something the tests
share: the machine (then its CPU time grows with it), or a lock or a server,
which more threads do not make faster.
"""
import json
import re
import resource
import subprocess
import sys
import time
from pathlib import Path

TOP = 15


def build(selection):
    """Builds the one test binary `selection` names; its path."""
    out = subprocess.run(
        ["cargo", "test", "--locked", "-j", "4", "--no-run", "--message-format=json", *selection],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    found = []
    for line in out.splitlines():
        msg = json.loads(line)
        if msg.get("reason") == "compiler-artifact" and msg.get("executable") and msg["profile"]["test"]:
            found.append(msg["executable"])
    if len(found) != 1:
        sys.exit(f"the selection names {len(found)} test binaries, not one: {found}")
    return found[0]


def host():
    out = subprocess.run(["rustc", "-vV"], check=True, capture_output=True, text=True).stdout
    return re.search(r"^host: (\S+)", out, re.M).group(1)


def run(selection, path, threads):
    """Runs the binary through cargo, with the environment cargo gives it."""
    runner = f'target.{host()}.runner=["env","RUSTC_BOOTSTRAP=1"]'
    args = ["cargo", "test", "--locked", "-j", "4", "--config", runner, *selection, "--"]
    args += ["-Zunstable-options", "--format", "json", "--report-time"]
    if threads:
        args.append(f"--test-threads={threads}")
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.monotonic()
    tests = {}
    wall = None
    began = 0.0
    with open(path, "w") as out, open(path.with_suffix(".cargo.log"), "w") as err:
        proc = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=err, text=True)
        for line in proc.stdout:
            at = time.monotonic() - start
            out.write(f"{at:.4f} {line}")
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if event.get("type") == "suite" and event.get("event") == "started":
                began = at
            if event.get("type") == "suite" and "exec_time" in event:
                wall = event["exec_time"]
            if event.get("type") == "test" and event.get("event") in ("ok", "failed"):
                tests[event["name"]] = (event.get("exec_time", 0.0), event["event"], at - began)
        status = proc.wait()
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    cpu = (after.ru_utime - before.ru_utime, after.ru_stime - before.ru_stime)
    if "Compiling" in path.with_suffix(".cargo.log").read_text():
        sys.exit("cargo compiled something while running the tests; run again")
    return wall, status, tests, cpu


def main(args):
    if len(args) < 2:
        print(__doc__.strip().splitlines()[2].strip())
        sys.exit(2)
    out = Path(args[0])
    out.mkdir(parents=True, exist_ok=True)
    selection = args[1:]
    exe = build(selection)
    runs = {
        "alone": run(selection, out / "alone.jsonl", 1),
        "together": run(selection, out / "together.jsonl", None),
    }
    lines = [f"binary {exe}", ""]
    for name, (wall, status, tests, (user, system)) in runs.items():
        total = sum(t for t, _, _ in tests.values())
        failed = sum(1 for _, e, _ in tests.values() if e == "failed")
        lines.append(
            f"{name:9} wall {wall:6.2f}s  sum of tests {total:7.2f}s  "
            f"{len(tests)} tests, {failed} failed, exit {status}, "
            f"cpu user {user:.2f}s system {system:.2f}s"
        )
    alone, together = runs["alone"][2], runs["together"][2]
    if together:
        name, (took, _, end) = max(together.items(), key=lambda kv: kv[1][2])
        lines.append(
            f"together, the last test to end: {name}, {took:.2f}s from {end - took:.2f}s to {end:.2f}s"
        )
    both = [n for n in alone if n in together]
    ratio = sum(together[n][0] for n in both) / max(sum(alone[n][0] for n in both), 1e-9)
    lines.append(f"\nsum of tests together / alone: {ratio:.2f}")
    for name, tests in (("alone", alone), ("together", together)):
        lines.append(f"\nlongest {TOP} tests, {name} (alone / together)")
        for n, (t, _, _) in sorted(tests.items(), key=lambda kv: -kv[1][0])[:TOP]:
            a = alone.get(n, (float("nan"),))[0]
            g = together.get(n, (float("nan"),))[0]
            lines.append(f"  {a:6.3f}s {g:6.3f}s  {n}")
    report = "\n".join(lines)
    (out / "report.txt").write_text(report + "\n")
    print(report)


if __name__ == "__main__":
    main(sys.argv[1:])
