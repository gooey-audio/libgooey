#!/usr/bin/env bash
#
# Print the latest release tag and the next version to cut.
#
# Tag convention: vMAJOR.MINOR.PATCH (stable only). Pre-release suffixes
# (v1.2.0-beta.1) and non-semver tags (vbeta, v1.0.0-snapshot) are ignored.
#
# Usage:
#   ./scripts/next-version.sh            # next minor: v1.1.18 -> v1.2.0
#   ./scripts/next-version.sh patch      # next patch: v1.1.18 -> v1.1.19
#   ./scripts/next-version.sh major      # next major: v1.1.18 -> v2.0.0
#   ./scripts/next-version.sh --latest   # print the latest tag as-is
#
# The next version goes to stdout so it can be captured:
#   git tag "$(./scripts/next-version.sh)"

set -euo pipefail

usage() {
  sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
}

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

BUMP="minor"
case "${1:-}" in
  "") BUMP="minor" ;;
  major|minor|patch) BUMP="$1" ;;
  --latest|-l) BUMP="latest" ;;
  -h|--help) usage; exit 0 ;;
  *)
    echo "error: unknown argument '$1' (expected major, minor, patch, or --latest)" >&2
    exit 2
    ;;
esac

# Highest stable vX.Y.Z tag by version order. git's version sort handles
# multi-digit components (v1.10.0 > v1.9.0) correctly.
LATEST="$(git -C "$REPO_ROOT" tag --sort=-v:refname --list 'v[0-9]*' \
  | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
  | head -n 1)"

if [[ -z "$LATEST" ]]; then
  echo "error: no stable vX.Y.Z tags found in $REPO_ROOT" >&2
  exit 1
fi

if [[ "$BUMP" == "latest" ]]; then
  echo "$LATEST"
  exit 0
fi

version="${LATEST#v}"
IFS=. read -r major minor patch <<<"$version"

case "$BUMP" in
  major) NEXT="v$((major + 1)).0.0" ;;
  minor) NEXT="v${major}.$((minor + 1)).0" ;;
  patch) NEXT="v${major}.${minor}.$((patch + 1))" ;;
esac

echo "latest tag: $LATEST" >&2
echo "$NEXT"
