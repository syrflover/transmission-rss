#!/usr/bin/env python3
"""Transmission's torrents, one per line, to compare before and after an update.

    python3 torrent-list.py [RPC URL] > torrents.tsv

Prints the hash, download folder, name and labels of every torrent, separated
by tabs and sorted by hash, so that `diff` of two lists shows the torrents that
went, came, moved or were renamed. Progress and state are left out: they change
on their own. The URL defaults to http://127.0.0.1:9091/transmission/rpc, the
port docker-compose.yml publishes; give one with `user:password@` when
Transmission asks for a login. Needs only Python's standard library.
"""
import base64
import json
import sys
import urllib.error
import urllib.parse
import urllib.request

FIELDS = ["hashString", "downloadDir", "name", "labels"]


def rpc(url, body):
    parts = urllib.parse.urlsplit(url)
    headers = {"Content-Type": "application/json"}
    if parts.username is not None:
        login = f"{urllib.parse.unquote(parts.username)}:{urllib.parse.unquote(parts.password or '')}"
        headers["Authorization"] = "Basic " + base64.b64encode(login.encode()).decode()
        url = urllib.parse.urlunsplit(parts._replace(netloc=parts.hostname + (f":{parts.port}" if parts.port else "")))
    data = json.dumps(body).encode()
    # Transmission answers the first request with 409 and the session id to
    # send with the next.
    for _ in range(2):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, data, headers), timeout=30) as response:
                return json.load(response)
        except urllib.error.HTTPError as e:
            if e.code != 409:
                raise
            headers["X-Transmission-Session-Id"] = e.headers["X-Transmission-Session-Id"]
    raise RuntimeError("Transmission kept asking for a new session id")


def clean(text):
    return " ".join(str(text).split("\t")).replace("\n", " ")


def main(args):
    url = args[0] if args else "http://127.0.0.1:9091/transmission/rpc"
    answer = rpc(url, {"method": "torrent-get", "arguments": {"fields": FIELDS}})
    if answer.get("result") != "success":
        print(f"torrent-list: Transmission answered {answer.get('result')!r}", file=sys.stderr)
        return 1
    torrents = sorted(answer["arguments"]["torrents"], key=lambda t: t["hashString"])
    for t in torrents:
        print("\t".join([t["hashString"], clean(t["downloadDir"]), clean(t["name"]), ",".join(t.get("labels", []))]))
    print(f"torrent-list: {len(torrents)} torrent(s)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
