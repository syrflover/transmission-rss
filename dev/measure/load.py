#!/usr/bin/env python3
"""The loads of run.sh on the development environment's trss-web
(127.0.0.1:8080).

    load.py web <rounds>   a browser's reads, six requests at a time: the library
                           list, every work's detail and cover, and the other
                           screens' lists, <rounds> times; prints one JSON line
    load.py rescan         rescans every watch folder in turn through the
                           worker's command; prints one JSON line
"""
import json
import statistics
import sys
import time
import urllib.request
import uuid
from concurrent.futures import ThreadPoolExecutor

BASE = "http://127.0.0.1:8080"
HEADERS = {"Host": "127.0.0.1:8080", "Origin": BASE}


def get(path):
    req = urllib.request.Request(BASE + path, headers=HEADERS)
    start = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            body = r.read()
            status = r.status
    except urllib.error.HTTPError as e:
        body = e.read()
        status = e.code
    return status, body, time.perf_counter() - start


def post(path, payload):
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(payload).encode(),
        headers={**HEADERS, "Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.status, json.loads(r.read())


def paths():
    _, body, _ = get("/api/library/works")
    works = json.loads(body)["items"]
    out = [
        "/api/library/works",
        "/api/library/watch-folders",
        "/api/library/storage",
        "/api/history",
        "/api/todo",
        "/api/todo/count",
        "/api/rules",
        "/api/subscriptions",
        "/api/schedule/week",
        "/api/collect/status",
        "/api/subtitle-jobs",
        "/api/subtitle-jobs/done",
        "/api/channels",
    ]
    for w in works:
        out.append(f"/api/library/works/{w['id']}")
        if w.get("cover_url"):
            out.append(w["cover_url"])
    return out


def web(rounds):
    todo = paths() * rounds
    start = time.perf_counter()
    with ThreadPoolExecutor(6) as pool:
        results = list(pool.map(get, todo))
    wall = time.perf_counter() - start
    times = sorted(t for _, _, t in results)
    bad = [s for s, _, _ in results if s >= 400]
    print(json.dumps({
        "requests": len(results),
        "bytes": sum(len(b) for _, b, _ in results),
        "errors": len(bad),
        "error_codes": sorted(set(bad)),
        "wall_s": round(wall, 3),
        "p50_ms": round(statistics.median(times) * 1000, 1),
        "p95_ms": round(times[int(len(times) * 0.95)] * 1000, 1),
        "max_ms": round(times[-1] * 1000, 1),
    }))


def rescan():
    _, body, _ = get("/api/library/watch-folders")
    folders = json.loads(body)["folders"]
    start = time.perf_counter()
    states = []
    for f in folders:
        cid = str(uuid.uuid4())
        post("/api/commands", {"id": cid, "kind": "watch_rescan", "payload": {"folder_id": f["id"]}})
        while True:
            _, body, _ = get(f"/api/commands/{cid}")
            view = json.loads(body)
            if view.get("state") not in ("pending", "running"):
                states.append(view.get("state"))
                break
            time.sleep(0.05)
    print(json.dumps({
        "folders": len(folders),
        "works": sum(f["works"] for f in folders),
        "states": states,
        "wall_s": round(time.perf_counter() - start, 3),
    }))


if __name__ == "__main__":
    if sys.argv[1] == "web":
        web(int(sys.argv[2]))
    else:
        rescan()
