#!/usr/bin/env bash
# The workspace tests on the release's target, x86_64-unknown-linux-musl, in
# the image the release binaries are built in (clux/muslrust:stable, as in
# Dockerfile). Run before a release (docs/adr/0015-test-a-rule-once-in-its-crate.md):
# musl's standard library differs from glibc's in what it can read (a file's
# birth time, for one), and the bundled C code (SQLite, liblzma) builds for
# musl too.
#
#   dev/musl-test.sh [cargo test arguments...]
#
# - The tests run as the calling user, not root: some expect a write to be
#   refused, and root's writes are not.
# - The build goes to target/musl in this checkout. The crates.io registry and
#   the git checkouts are this machine's ($CARGO_HOME, else ~/.cargo), so
#   nothing is downloaded again; the machine's own cargo config is not read,
#   the project's .cargo/config.toml is.
# - `-j 4`, as on the development machine: linking the test binaries with
#   more jobs fills the memory.
# - Tests marked #[ignore] are skipped, as in the usual run.
set -euo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_DIR="${CARGO_HOME:-$HOME/.cargo}"
IMAGE=clux/muslrust:stable

mkdir -p "$CARGO_DIR/registry" "$CARGO_DIR/git" "$REPO/target/musl"
tty=()
if [ -t 1 ]; then
  tty=(-t)
fi

exec docker run --rm --init "${tty[@]}" \
  --user "$(id -u):$(id -g)" \
  -e HOME=/tmp \
  -e CARGO_TARGET_DIR=/src/target/musl \
  -v "$REPO:/src" \
  -v "$CARGO_DIR/registry:/opt/cargo/registry" \
  -v "$CARGO_DIR/git:/opt/cargo/git" \
  -w /src \
  "$IMAGE" \
  cargo test --locked --workspace -j 4 "$@"
