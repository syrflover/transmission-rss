#!/usr/bin/env bash
# How long the workspace's test build takes after an edit to trss-core, the
# yardstick of the lighter-structure change (tickets lighter-structure/0144 and lighter-structure/0161).
#
#   dev/measure/core-edit-build.sh <out dir>
#
# Builds first, then three times after an edit, each with
#
#   cargo test --locked --workspace --no-run -j 4 --timings
#
#   prime  the first edit; it readies the incremental caches and is not
#          compared, since what it reuses depends on what was built before
#   warm   a second edit, reusing the caches of the first: an edit loop
#   cold   a third edit with CARGO_INCREMENTAL=0: every crate that depends
#          on trss-core compiles in full. A build without the edit comes
#          first with the same setting, which cargo counts in its
#          fingerprint, so that the crates that do not depend on trss-core
#          are not built in the measured one.
#
# then puts lib.rs back, also when a build fails or is stopped, and builds
# once more so that target/ is as it was. lib.rs is put back with a new
# modification time: with its old one, cargo would keep the edited build. Each build's log, wall time and
# cargo timing report go to <out dir>/<name>/; read the report with
# dev/measure/build-units.py.
#
# The edit adds a public constant with a new value to
# crates/trss-core/src/lib.rs, which changes trss-core's metadata. A touch is
# not enough: rustc runs through kache (~/.cargo/config.toml), which answers
# an unchanged file from its cache (2.3 s on 2026-10-21).
set -euo pipefail
[[ $# -eq 1 ]] || { sed -n '5p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
out=$1
cd "$(git rev-parse --show-toplevel)"
mkdir -p "$out"
lib=crates/trss-core/src/lib.rs
touch "$lib"
cargo test --locked --workspace --no-run -j 4 > "$out/before.log" 2>&1

backup=$(mktemp)
cp "$lib" "$backup"
trap 'cp "$backup" "$lib"; rm -f "$backup"' EXIT

# Edits lib.rs anew and builds with --timings into $out/$1/.
measure() {
  local name=$1 start end report
  mkdir -p "$out/$name"
  cp "$backup" "$lib"
  printf '\n#[doc(hidden)]\npub const MEASURE_EDIT: u64 = %s;\n' "$(date +%s%N)" >> "$lib"
  start=$(date +%s%N)
  cargo test --locked --workspace --no-run -j 4 --timings > "$out/$name/build.log" 2>&1
  end=$(date +%s%N)
  report=$(ls -t target/cargo-timings/cargo-timing-*.html | head -1)
  cp "$report" "$out/$name/"
  printf '%-5s wall %d.%03d s, %s\n' "$name" $(((end - start) / 1000000000)) \
    $(((end - start) / 1000000 % 1000)) "$(grep -o 'Finished.*' "$out/$name/build.log")" |
    tee "$out/$name/wall.txt"
}

measure prime
measure warm
cp "$backup" "$lib"
CARGO_INCREMENTAL=0 cargo test --locked --workspace --no-run -j 4 > "$out/cold-before.log" 2>&1
CARGO_INCREMENTAL=0 measure cold

cp "$backup" "$lib"
cargo test --locked --workspace --no-run -j 4 > "$out/after.log" 2>&1
