#!/usr/bin/env bash
# What a database connection's noting of changes costs per written row (ticket
# lighter-structure/0164): 10,000 inserts and 10,000 updates of a `WITHOUT ROWID` table with and
# without the TEMP triggers, in one transaction and one row per `Db::run`, and
# 10,000 read-only calls, in the dev and the release profile. The measurement
# is the ignored test `changes::tests::from_the_connection::
# measure_what_noting_changes_costs_per_row` in trss-core.
#
#   dev/measure/db-announce-cost.sh <out dir>
#
# Each profile's output goes to <out dir>/<profile>.log; every run prints the
# non-announcing and the announcing handle twice, in turn.
set -euo pipefail
out="${1:?usage: dev/measure/db-announce-cost.sh <out dir>}"
mkdir -p "$out"
for profile in dev release; do
  flags=()
  [[ $profile == release ]] && flags=(--release)
  cargo test -p trss-core --lib -j 4 "${flags[@]}" -- --ignored --nocapture \
    measure_what_noting_changes_costs_per_row 2>&1 | tee "$out/$profile.log"
done
