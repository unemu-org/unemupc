# Local user-media qualification

DOOM is commercial software. Supply your own legal copy; this repository ships
no game files. No game file or disk-image digest, frame digest, screenshot,
console transcript or private path belongs in the repository. The temporary
base is identified in memory by SHA-256 for the existing sector-overlay
interface; that value is never logged or stored here.

## Prepare the directory

Copy an installed registered DOOM 1.2 directory, or extract the installer's LHA
archive using `lhasa` from nixpkgs (`nix shell nixpkgs#lhasa`, then `lha x` on
the archive). Extraction of multi-disk installers may require joining their
archive segments first. Keep the result outside the checkout. The directory
must directly contain `DOOM.EXE`, `DOOM.WAD` and any other installed files.
Names are case insensitive; use ordinary DOS 8.3 filenames and no subdirectories
or symbolic links. Set `UNEMUPC_USER_DOOM_DIR` to that directory.

Inside the project devShell:

```sh
cargo test --offline -p machine-file user_media_doom_386 -- --ignored --nocapture
# Optional: both 486 CPU profiles use the same helper.
cargo test --offline -p machine-file user_media_doom_486 -- --ignored --nocapture
```

The tests are ignored by default. With the variable absent they explicitly
skip, including when the golden lane includes ignored tests. `user_media_doom_`
is the opt-in name filter; the existing `user_rom_` firmware filter does not
select commercial game media. CI must not supply this variable.

The existing open SeaBIOS, SeaVGABIOS and FreeDOS fixtures are required. The
helper uses the shipped generic IDE profile, boots FreeDOS from A:, types `C:`
and `DOOM` through the emulated keyboard, and gives C: a volatile sector overlay.
No configuration writes reach the immutable base or the supplied directory.

## Sound variant

`user_media_doom_sound_386` uses `machines/generic-386-games.toml` with SB16
at 220h, IRQ5 and DMA1/DMA5. It is separately ignored and requires the same
`UNEMUPC_USER_DOOM_DIR` opt-in:

```sh
cargo test --offline -p machine-file --test generic386 user_media_doom_sound_386 -- --ignored --nocapture
```

Before hashing or mounting the generated data disk, the test replaces its
`DEFAULT.CFG` with these test-owned DOOM 1.2 settings:

```text
snd_sfxdevice 3
snd_musicdevice 3
snd_sbport 544
snd_sbirq 5
snd_sbdma 1
snd_mport -1
snd_channels 3
```

These select Sound Blaster effects and OPL music; 544 is decimal 220h.
The source directory is only read. The disk base receives its full SHA-256
after the generated configuration is copied; subsequent guest writes stay
in the volatile sector overlay. The probe requires the same non-trivial
mode 13h picture plus DSP command writes, OPL register-data writes and
channel-1 address/count advancement during a poll with no DMA-controller
register writes. It observes the existing bounded port traces and restores
device snapshots into a separate 8237 for side-effect-free inspection.
Trace overflow is a failure. No audio or game-derived hash is recorded.

**MEASURED**, DOOM 1.2, x86jit `6a38ae49`: all selected startup identifiers
and the DOS/4G banner appear. The qualifying mode 13h picture has 243 palette
indices at 160,240,957 retired instructions (43.7 s in this probe). At that
point the SB16 has seen 15 DSP command writes and 528 OPL register-data writes,
and the DMA advancement check has passed. This qualifies sound initialization
and DMA activity at startup; it does not claim attract-mode endurance or pin
the game's audio content. The sound trace rings hold 65,536 events per device
and are drained every 100,000 scheduling units; other diagnostics keep their
small rings. A synthetic-media test checks generated configuration replacement,
deterministic disk bytes and preservation of the supplied directory's defaults.

## Temporary disk

`scripts/user-media-disk.sh` reads only `UNEMUPC_USER_DOOM_DIR` and prints the
path to a newly built image under the system temporary directory. It requires
Python 3 and mtools from the devShell. The caller owns that temporary directory;
the Rust probe removes it on success and unwind.

The disk starts at 34 MiB and grows with the input, with four heads, 32 sectors
per track and 512-byte sectors. An inactive FAT16 partition begins at LBA 32;
the builder verifies its data-cluster count. The minimum size allows 8 KiB
clusters, limiting DOS FAT-chain traversal during WAD seeks. Filenames are sorted and uppercased,
FAT timestamps use the FAT epoch in UTC, and the volume serial is fixed. The
builder never computes a digest. Synthetic tests check byte-for-byte
reproducibility despite changed host mtimes, MBR/BPB layout and copied contents.

The bounded probe reports retired instructions and host elapsed time for the
FreeDOS prompt, command injection, short startup identifiers, any DOS/4G banner,
graphics transition and a 320×200 picture with at least 32 distinct palette
indices. It checks BIOS mode 13h and requires all selected startup identifiers.
It does not save pictures or hash screens. The instruction allowance is fixed;
wall time is diagnostic only. A stop reports CPU state, deliveries and the last
observed port I/O. A budget failure reports throughput rather than starting an
unbounded run.

## Findings

Qualification results are recorded after running the local probe. A CPU stop
must be handed to the x86jit maintainers with CS:IP, linear address, the faulting
instruction's own bytes only, mode, CPL, retired count, delivery state and last
port I/O. Board/device failures require isolation tests before a fix. Neither
case permits publishing game bytes, media hashes or frame hashes.

MEASURED on the pinned CPU revision `fc07c81f`, registered DOOM 1.2 on
`generic-386-isa` boots from the open FreeDOS floppy and reaches the game picture.
No board fix or CPU boundary was needed. The 34 MiB temporary FAT16 disk uses
8 KiB clusters; an earlier 2 KiB-cluster probe spent its 250-million-instruction
launch allowance in DOS/BIOS disk service before graphics. Increasing cluster
size avoids unnecessary FAT-chain traversal without changing the supplied files.

| 386 milestone | Retired instructions | Host elapsed |
| --- | ---: | ---: |
| FreeDOS prompt | 10,716,384 | 3.44 s |
| DOOM command injected | 10,815,206 | 3.46 s |
| DOS/4G banner | 14,080,125 | 4.51 s |
| DOOM / V_Init | 23,045,406 | 7.53 s |
| M_LoadDefaults | 23,145,189 | 7.55 s |
| Z_Init / W_Init | 23,244,159 | 7.58 s |
| R_Init | 24,806,794 | 7.94 s |
| 320×200 graphics capture | 153,366,005 | 36.07 s |
| Mode 13h, 243 distinct indices | 159,571,631 | 37.67 s |

The first 320×200 capture precedes the BIOS data-area mode update; qualification
waits for the non-trivial picture and checks mode 13h there. Sampling is every
100,000 machine scheduling units, so these counts are observed checkpoints,
not instruction-exact transition locations. Host elapsed times are diagnostic
and vary by host load. The limit is 250 million retired instructions after
command injection, plus a bounded boot allowance. No attract-mode endurance
claim is made by this startup probe.

Both 486 profiles also pass the same probe (run together):

| CPU profile | DOS/4G banner | R_Init | 320×200 graphics | Qualifying picture | Host elapsed at picture |
| --- | ---: | ---: | ---: | ---: | ---: |
| 80486DX | 19,180,424 | 35,107,182 | 219,462,780 | 226,874,445 | 98.78 s |
| 80486DX with CPUID | 19,180,424 | 35,107,210 | 219,462,808 | 226,874,473 | 98.67 s |

Counts are retired instructions. Both pictures have 243 distinct indices and
BIOS mode 13h. Different virtual clock rates change timer/disk-wait work; host
contention makes these concurrent wall times unsuitable for comparing CPU speed.
