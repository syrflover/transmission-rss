#!/usr/bin/env python3
"""What a running trss tells the screens through `GET /api/events` (ticket 0164).

    dev/measure/record-events.py record <out.tsv> [url]
    dev/measure/record-events.py count <in.tsv> [from] [to]

`record` keeps the stream open and appends one line per event to <out.tsv>:
the arrival time (Unix seconds), the event name and its JSON data. The url
defaults to the development environment's http://127.0.0.1:8080/api/events.
It runs until it is stopped, and opens the stream again when it closes.

`count` prints, for the `change` events between the Unix times [from] and [to]
(default: all of them), how many arrived of each kind, how many of them had a
null id, and the most that arrived within one second.
"""
import collections
import json
import sys
import time
import urllib.request

DEFAULT_URL = "http://127.0.0.1:8080/api/events"


def record(out, url):
    while True:
        try:
            with urllib.request.urlopen(url) as stream, open(out, "a") as sink:
                event = "message"
                for raw in stream:
                    line = raw.decode().rstrip("\r\n")
                    if line.startswith("event:"):
                        event = line[len("event:"):].strip()
                    elif line.startswith("data:"):
                        data = line[len("data:"):].strip()
                        sink.write(f"{time.time():.3f}\t{event}\t{data}\n")
                        sink.flush()
                    elif not line:
                        event = "message"
        except OSError as error:
            print(f"stream closed: {error}", file=sys.stderr)
        time.sleep(1)


def count(path, start, end):
    kinds = collections.Counter()
    nulls = collections.Counter()
    per_second = collections.Counter()
    for line in open(path):
        at, event, data = line.rstrip("\n").split("\t", 2)
        at = float(at)
        if event != "change" or not start <= at <= end:
            continue
        change = json.loads(data)
        kinds[change["kind"]] += 1
        if change["id"] is None:
            nulls[change["kind"]] += 1
        per_second[int(at)] += 1
    print(f"{'kind':<20} {'events':>7} {'null id':>8}")
    for kind, n in kinds.most_common():
        print(f"{kind:<20} {n:>7} {nulls[kind]:>8}")
    print(f"{'total':<20} {sum(kinds.values()):>7} {sum(nulls.values()):>8}")
    if per_second:
        second, most = per_second.most_common(1)[0]
        print(f"most in one second: {most} (at {second}), "
              f"seconds with events: {len(per_second)}")


def main(args):
    if len(args) >= 2 and args[0] == "record":
        record(args[1], args[2] if len(args) > 2 else DEFAULT_URL)
    elif len(args) >= 2 and args[0] == "count":
        start = float(args[2]) if len(args) > 2 else float("-inf")
        end = float(args[3]) if len(args) > 3 else float("inf")
        count(args[1], start, end)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
