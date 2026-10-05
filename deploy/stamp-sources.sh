#!/usr/bin/env bash
# Runs in the Rust build stages of Dockerfile and Dockerfile.browser, before
# `cargo build`, in the workspace whose target/ is a BuildKit cache mount:
#
#   stamp-sources.sh STORE PATH...
#
# Sets the modification time of every file under PATH to the time this cache
# first saw the file's current content, and records path, SHA-256 and time in
# STORE, a file in the cache.
#
# Cargo rebuilds a workspace crate when one of its files is newer than the
# crate's last build. COPY keeps the times the files have in the build context,
# and the cache is shared by every checkout that builds on the machine (the
# worktrees of the repository): a file that differs from the last build but is
# older than it would count as unchanged, and the image would get another
# checkout's binary. With these times a crate is rebuilt when the content of
# one of its files changed since that crate's last build into the cache,
# whatever times the checkout has. Touching a file without changing it no
# longer rebuilds anything.
set -euo pipefail

store=$1
shift

# sha256sum escapes such a path, and the reading below does not undo that.
if find "$@" -type f -path $'*[\n\\\\]*' | grep -q .; then
  echo "stamp-sources.sh: a path has a newline or a backslash" >&2
  exit 1
fi

now=$(date +%s.%N)
sums=$(mktemp)
next=$(mktemp)
find "$@" -type f -print0 | xargs -0 -r sha256sum >"$sums"
touch "$store"
# A file keeps its recorded time while its content stays the same; a new or
# changed one gets the time of now, later than every build before this one.
awk -v now="$now" 'BEGIN { FS = OFS = "\t" }
  FILENAME == ARGV[1] { sum[$1] = $2; time[$1] = $3; next }
  {
    s = substr($0, 1, 64); p = substr($0, 67)
    print p, s, ((p in sum) && sum[p] == s) ? time[p] : now
  }' "$store" "$sums" >"$next"
cut -f3 "$next" | sort -u | while read -r t; do
  awk -F '\t' -v t="$t" '$3 == t "" { print $1 }' "$next" |
    xargs -d '\n' -r touch -c -m -d "@$t" --
done
mv "$next" "$store"
rm -f "$sums"
