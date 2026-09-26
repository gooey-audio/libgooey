#!/bin/sh
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_dir"

cargo test --release --no-default-features --features ios --test live_control
cargo test --release --no-default-features --features ios --lib --test live_control_alloc
cargo build --release --no-default-features --features ios

test_tmp=$(mktemp -d "${TMPDIR:-/tmp}/gooey-live-control-c.XXXXXX")
started_simulator=0
simulator_id=

cleanup() {
  if [ "$started_simulator" -eq 1 ] && [ -n "$simulator_id" ]; then
    xcrun simctl shutdown "$simulator_id" >/dev/null 2>&1 || true
  fi
  rm -rf -- "$test_tmp"
}
trap cleanup EXIT HUP INT TERM

cc -std=c11 -Wall -Wextra -Werror \
  -Iinclude \
  tests/c/live_control_consumer.c \
  -Ltarget/release -lgooey \
  -o "$test_tmp/live-control-consumer"

DYLD_LIBRARY_PATH="$repo_dir/target/release${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
LD_LIBRARY_PATH="$repo_dir/target/release${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
  "$test_tmp/live-control-consumer"

scripts/build-ios.sh
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libgooey.a -headers include \
  -library target/aarch64-apple-ios-sim/release/libgooey.a -headers include \
  -output "$test_tmp/Gooey.xcframework" >/dev/null

simulator_slice="$test_tmp/Gooey.xcframework/ios-arm64-simulator"
xcrun --sdk iphonesimulator clang \
  -target arm64-apple-ios17.0-simulator \
  -std=c11 -Wall -Wextra -Werror \
  -I"$simulator_slice/Headers" \
  tests/c/live_control_consumer.c \
  "$simulator_slice/libgooey.a" \
  -o "$test_tmp/live-control-simulator"

simulator_id=$(xcrun simctl list devices available | awk -F '[()]' '/Booted/{print $2; exit}')
if [ -z "$simulator_id" ]; then
  simulator_id=$(xcrun simctl list devices available | awk -F '[()]' '/Shutdown/{print $2; exit}')
  if [ -z "$simulator_id" ]; then
    echo "No available iOS simulator found" >&2
    exit 1
  fi
  xcrun simctl boot "$simulator_id"
  xcrun simctl bootstatus "$simulator_id" -b
  started_simulator=1
fi
xcrun simctl spawn "$simulator_id" "$test_tmp/live-control-simulator"
