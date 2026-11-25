#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Regenerate the project-owned user-mode loops with NASM.
set -euo pipefail
cd "$(dirname "$0")"
nasm -f bin -DFLAT cpu-ceiling-user.asm -o cpu-ceiling-flat32-cpl3.bin
nasm -f bin -DFLAT -DPAGED cpu-ceiling-user.asm -o cpu-ceiling-flat-paged32-cpl3.bin
nasm -f bin -DFLAT -DPAGED -DFARCALL cpu-ceiling-user.asm -o cpu-ceiling-flat-paged32-cpl3-farcall.bin
nasm -f bin -DLDT -DPAGED cpu-ceiling-user.asm -o cpu-ceiling-paged-protected16-cpl3.bin
nasm -f bin -DV86 -DPAGED cpu-ceiling-user.asm -o cpu-ceiling-paged-v86.bin
nasm -f bin -DV86 -DPAGED -DTRAPPED cpu-ceiling-user.asm -o cpu-ceiling-paged-v86-iopl0.bin
