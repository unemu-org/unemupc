#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Regenerate the project-owned Bochs VBE framebuffer programs with NASM.
set -euo pipefail
cd "$(dirname "$0")"
nasm -f bin bochs-vbe-lfb.asm -o bochs-vbe-lfb32.bin
nasm -f bin -DBPP16 bochs-vbe-lfb.asm -o bochs-vbe-lfb16.bin
nasm -f bin -DSPEED=0x200000 bochs-vbe-lfb.asm -o bochs-vbe-speed-ram.bin
nasm -f bin -DSPEED=0xe0000000 bochs-vbe-lfb.asm -o bochs-vbe-speed-lfb.bin
nasm -f bin -DSPEED=0xa0000 -DBANKED bochs-vbe-lfb.asm -o bochs-vbe-speed-bank.bin
nasm -f bin bochs-vbe-window.asm -o bochs-vbe-window.bin
