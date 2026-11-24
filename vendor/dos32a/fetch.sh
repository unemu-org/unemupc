#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Preserve the complete source/licence archive for the DOS/32A boot goldens.
set -euo pipefail

if command -v sha256sum >/dev/null 2>&1; then
    SHA256=(sha256sum)
elif command -v shasum >/dev/null 2>&1; then
    SHA256=(shasum -a 256)
else
    echo "missing SHA-256 tool: install sha256sum or shasum (or use nix develop)" >&2
    exit 1
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARCHIVE="$HERE/dos32a.zip"
URL="https://www.ibiblio.org/pub/micro/pc-stuff/freedos/files/repositories/1.4/util/dos32a.zip"
SIZE=367156
HASH=1cb89287703b3dad7ae279c18ae53ce4699fd33d685ddb7e7f46879d2fd953c1

if [ ! -f "$ARCHIVE" ]; then
    mkdir -p "$(dirname "$ARCHIVE")"
    curl -fsSL -o "$ARCHIVE" "$URL"
fi
actual_size="$(wc -c < "$ARCHIVE" | tr -d '[:space:]')"
if [ "$actual_size" != "$SIZE" ]; then
    echo "size mismatch for $ARCHIVE: expected $SIZE, got $actual_size" >&2
    exit 1
fi
actual_hash="$("${SHA256[@]}" "$ARCHIVE" | awk '{print $1}')"
if [ "$actual_hash" != "$HASH" ]; then
    echo "SHA-256 mismatch for $ARCHIVE: expected $HASH, got $actual_hash" >&2
    echo "Remove the cached archive and fetch the pinned release again; do not change its expected hash." >&2
    exit 1
fi
echo "DOS/32A 9.1.2a archive verified"
