# DOS/32 Advanced fixture

This product uses DOS/32 Advanced DOS Extender technology.

The FreeDOS 1.4 package repository's **DOS/32A 9.1.2a** package is an open
DOS extender and loader. The archive's own `DOC/DOS32A/README` identifies
version 9.1.2 (2006-04-20); the repository package adds the `a` suffix.

Run `bash vendor/dos32a/fetch.sh`. A matching cached `dos32a.zip` needs no
network. The script downloads only when absent, then checks both size and
SHA-256; a mismatched cache fails without overwriting it.

- Source: <https://www.ibiblio.org/pub/micro/pc-stuff/freedos/files/repositories/1.4/util/dos32a.zip>
- Size: **367,156 bytes**.
- SHA-256: `1cb89287703b3dad7ae279c18ae53ce4699fd33d685ddb7e7f46879d2fd953c1`.
- Loader: `BIN/DOS32A.EXE`; full source: `SOURCE/DOS32A/SOURCES.ZIP`.
- Licence: **DOS/32 Advanced DOS Extender Software License**, copyright
  1996–2006 Narech K.; the complete verbatim notice is in [LICENSE](LICENSE)
  and `DOC/DOS32A/LICENSE` inside the archive. Source and binary redistribution
  are permitted subject to retaining the notice/disclaimer, including the
  acknowledgment above in end-user documentation, and avoiding the upstream
  product names for derived products. This is a BSD-like licence with
  additional acknowledgment and naming conditions.

The fixtures CI lane fetches/verifies the archive and assembles the
[project-owned client](../dos-extender-test/README.md). Tests extract the
loader offline, derive a temporary floppy from the SHA-256-checked FreeDOS
base, add the client and AUTOEXEC, and mount the result read-only. The VCPI
variant also extracts JEMMEX from the pinned FreeDOS 1.4 source archive
(`vendor/freedos/fetch-jemm.sh`). The base and cached archives are unchanged.
Missing inputs produce a named skip in optional runs and fail with
`UNEMUPC_REQUIRE_FIXTURES=1`. No test downloads files.

See [qualification and CPU findings](../../backlog/docs/design/protected-mode-gaps.md#dos-extender-qualification)
for the measured scope; having the fixture does not imply successful execution.
