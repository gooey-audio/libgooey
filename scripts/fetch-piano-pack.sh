#!/usr/bin/env bash
#
# Fetch a multi-sampled acoustic piano pack for `cargo run --example piano`.
#
# libgooey never vendors sample data: packs are large, and a host that does not
# use the piano should not pay for them. This script downloads one into the
# gitignored `assets/` directory.
#
# Default pack: Salamander Grand Piano V3 by Alexander Holm (Yamaha C5),
# licensed CC-BY 3.0. **Attribution is required** if you ship it — credit
# "Salamander Grand Piano V3 by Alexander Holm (CC-BY 3.0)".
#
# The 44.1 kHz / 16-bit build is used because it ships plain WAV, which the
# `bounce` feature's `hound` decoder reads directly. It is ~490 MB compressed.
#
# Usage:
#   ./scripts/fetch-piano-pack.sh              # download + extract
#   ./scripts/fetch-piano-pack.sh --force      # re-download even if present
#
# Re-running is safe: an existing, complete extraction is left alone.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ASSET_DIR="$REPO_ROOT/assets/piano"
ARCHIVE_NAME="SalamanderGrandPianoV3_44.1khz16bit.tar.bz2"
ARCHIVE_URL="https://archive.org/download/SalamanderGrandPianoV3/$ARCHIVE_NAME"
ARCHIVE_PATH="$ASSET_DIR/$ARCHIVE_NAME"

FORCE=0
if [[ "${1:-}" == "--force" ]]; then
  FORCE=1
fi

mkdir -p "$ASSET_DIR"

find_sfz() {
  find "$ASSET_DIR" -name '*.sfz' -type f 2>/dev/null | head -n 1
}

EXISTING="$(find_sfz)"
if [[ -n "$EXISTING" && $FORCE -eq 0 ]]; then
  echo "Pack already present."
  echo
  echo "  cargo run --example piano --features native,crossterm,bounce -- '$EXISTING'"
  exit 0
fi

if [[ ! -f "$ARCHIVE_PATH" || $FORCE -eq 1 ]]; then
  echo "Downloading $ARCHIVE_NAME (~490 MB) from archive.org..."
  # -L follows redirects, -C - resumes a partial download.
  curl -L -C - --fail --progress-bar -o "$ARCHIVE_PATH" "$ARCHIVE_URL"
fi

echo "Extracting..."
tar -xjf "$ARCHIVE_PATH" -C "$ASSET_DIR"

SFZ="$(find_sfz)"
if [[ -z "$SFZ" ]]; then
  echo "error: no .sfz mapping found under $ASSET_DIR" >&2
  echo "The archive layout may have changed. Point the example at the .sfz" >&2
  echo "file by hand, or see docs/multisample-instruments.md." >&2
  exit 1
fi

echo
echo "Done. Sample data is gitignored under assets/."
echo "License: CC-BY 3.0 — credit Alexander Holm if you redistribute it."
echo
echo "  cargo run --example piano --features native,crossterm,bounce -- '$SFZ'"
