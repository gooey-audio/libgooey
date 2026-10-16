#!/usr/bin/env bash
# Upload PR assets (screenshots, screen recordings, audio renders) to Gooey
# Audio's shared public Vercel Blob store and print Markdown lines for a PR
# body. The repos are private and gh can't attach files, so assets are hosted
# as public, unguessable blob URLs. Identical in every Gooey project except
# PROJECT_SLUG; works with macOS bash 3.2 and Linux. Usage notes live in the
# repo's AGENTS.md and, in the iOS apps, .agents/skills/pr-screenshots/SKILL.md.
set -euo pipefail

PROJECT_SLUG="libgooey"
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd -P)"
BLOB_API_URL="${VERCEL_BLOB_API_URL:-https://vercel.com/api/blob}"
BLOB_API_VERSION="12"

log() { printf 'pr-screenshot.sh: %s\n' "$*" >&2; }
die() { log "error: $*"; exit 1; }

usage() {
  cat >&2 <<'USAGE'
usage: scripts/pr-screenshot.sh <file> [<file>...]

Uploads each file to Gooey Audio's shared public Vercel Blob store and prints
one Markdown line per file on stdout, ready to paste into a PR body:
  PNG / JPEG / GIF   inline image   ![name](url)
  MP4 / MOV          link           [name (video)](url)
  WAV / MP3 / M4A    link           [name (audio)](url)
Requires GOOEY_DEV_PUBLIC_READ_WRITE_TOKEN, the read-write token of the shared
public "gooey audio dev" Blob store (not a private store's BLOB_READ_WRITE_TOKEN).
USAGE
}

[ $# -gt 0 ] || { usage; exit 2; }
case "$1" in -h|--help) usage; exit 0 ;; esac
[ -n "${GOOEY_DEV_PUBLIC_READ_WRITE_TOKEN:-}" ] \
  || die "GOOEY_DEV_PUBLIC_READ_WRITE_TOKEN is not set (token for Gooey Audio's shared public Blob store)"
command -v jq >/dev/null || die "jq is required (brew install jq / dnf install jq)"
command -v curl >/dev/null || die "curl is required"

branch="$(git -C "$ROOT_DIR" symbolic-ref --quiet --short HEAD 2>/dev/null)" || branch="detached"
prefix="$PROJECT_SLUG/pr-screenshots/$(printf '%s' "$branch" | tr -c 'A-Za-z0-9._-' '-')"

for file in "$@"; do
  [ -f "$file" ] || die "no such file: $file"
  name="${file##*/}"
  stem="${name%.*}"
  ext="$(printf '%s' "${name##*.}" | tr '[:upper:]' '[:lower:]')"
  case "$ext" in
    png) content_type="image/png"; kind="image" ;;
    jpg|jpeg) content_type="image/jpeg"; kind="image" ;;
    gif) content_type="image/gif"; kind="image" ;;
    mp4) content_type="video/mp4"; kind="video" ;;
    mov) content_type="video/quicktime"; kind="video" ;;
    wav) content_type="audio/wav"; kind="audio" ;;
    mp3) content_type="audio/mpeg"; kind="audio" ;;
    m4a) content_type="audio/mp4"; kind="audio" ;;
    *) die "unsupported file type: $file (png, jpg, gif, mp4, mov, wav, mp3, m4a)" ;;
  esac

  pathname="$(jq -rn --arg p "$prefix/$name" '$p | @uri')"
  response="$(mktemp)"
  http_status="$(curl -sS -o "$response" -w '%{http_code}' -X PUT \
    "$BLOB_API_URL/?pathname=$pathname" \
    -H "authorization: Bearer $GOOEY_DEV_PUBLIC_READ_WRITE_TOKEN" \
    -H "x-api-version: $BLOB_API_VERSION" \
    -H "x-vercel-blob-access: public" \
    -H "x-content-type: $content_type" \
    -H "x-add-random-suffix: 1" \
    --data-binary "@$file")" || { rm -f "$response"; die "upload failed: $file"; }

  case "$http_status" in
    2*) ;;
    *)
      body="$(cat "$response")"; rm -f "$response"
      case "$body" in
        *"private store"*) log "GOOEY_DEV_PUBLIC_READ_WRITE_TOKEN must belong to a store created with public access" ;;
      esac
      die "upload of $file returned HTTP $http_status: $body"
      ;;
  esac
  url="$(jq -r '.url // empty' "$response")"; rm -f "$response"
  [ -n "$url" ] || die "upload of $file returned no URL"

  log "uploaded $file"
  # GitHub inlines images from external hosts; video and audio only as links.
  case "$kind" in
    image) printf '![%s](%s)\n' "$stem" "$url" ;;
    *) printf '[%s (%s)](%s)\n' "$stem" "$kind" "$url" ;;
  esac
done
