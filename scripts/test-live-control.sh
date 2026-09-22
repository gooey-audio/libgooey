#!/bin/sh
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_dir"

cargo test --all-features --lib --test live_control --test live_control_alloc
cargo build --no-default-features --features ios

test_tmp=$(mktemp -d "${TMPDIR:-/tmp}/gooey-live-control-c.XXXXXX")
trap 'rm -rf "$test_tmp"' EXIT HUP INT TERM

cc -std=c11 -Wall -Wextra -Werror \
  -Iinclude \
  tests/c/live_control_consumer.c \
  -Ltarget/debug -lgooey \
  -o "$test_tmp/live-control-consumer"

DYLD_LIBRARY_PATH="$repo_dir/target/debug${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
LD_LIBRARY_PATH="$repo_dir/target/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
  "$test_tmp/live-control-consumer"
