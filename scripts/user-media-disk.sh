#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Build private data media outside the checkout; stdout is the temporary image.
set -euo pipefail
exec python3 "$(dirname "$0")/user-media-disk.py" "$@"
