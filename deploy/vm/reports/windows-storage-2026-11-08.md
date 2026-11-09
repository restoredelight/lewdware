# Windows storage migration — 2026-11-08

Result: the Windows 11 VM directory now occupies **26.92 GiB**, including the
compressed baseline, fresh writable overlays, and retained virtio driver ISO.
It is stopped and ready for the next test run. The guest still has a 64 GiB
virtual disk; its partition was not shrunk.

The user authorized replacing live snapshots with a compact development
baseline and retiring the old images and installer ISO. This supersedes the
storage/backup handoff in [the earlier UI smoke report](windows-2026-11-08.md).

## Recovery and guest cleanup

- Booted the repaired original guest at 3 GB RAM. It reached the desktop.
- `Repair-Volume -DriveLetter C -Scan` returned `NoErrorsFound`.
- `fsutil dirty query C:` reported C: is not dirty; the volume reported Healthy.
- BitLocker reported Fully Decrypted / Protection Off.
- Removed Rust intermediates (`deps`, `incremental`, `build`, `.fingerprint`,
  `examples`) and vcpkg build/package/download caches. Retained source, Rust,
  MSVC, Node/pnpm, installed vcpkg libraries, runnable apps and PDB symbols.
- Guest free space increased from 5.72 to 17.36 GiB. Native source builds will
  recreate their caches after reset; the baseline prioritizes test runtime size.
- Retrim returned StorageWMI 40004. Ran the zero-fill fallback, leaving 1 GiB
  free while writing; the temporary file was removed afterwards.
- Launched the retained config app after cleanup and shut down normally.

## Baseline and reset verification

`vm-storage init` converted `disk.qcow2` and `OVMF_VARS.qcow2` into Zstandard
compressed backing images with no live snapshots. Each image passed a qcow2
check and a `qemu-img compare` against the original guest-visible contents.
The TPM state was preserved alongside them. Baseline files are read-only;
overlays use `lazy_refcounts=off`.

Quickemu initially mistook the tiny fresh disk overlay for an uninstalled disk.
Set `preallocation="metadata"` in the VM config to bypass that existing-file
size heuristic. This does not preallocate existing overlays. Also detached the
Windows installer ISO and persisted `vmctl-ram=3G` and the Windows SSH profile.

Verified the actual guest lifecycle:

1. Booted Windows through the new disk and firmware overlays. C: was healthy,
   and config opened visibly on the desktop.
2. Created `C:\lw\overlay-reset-marker.txt` in the test overlay.
3. Shut down normally, then ran `vm-storage reset --discard`.
4. Booted again. SSH reported `RESET_MARKER_ABSENT`; all four baseline binaries
   (`lewdware.exe`, engine, supervisor, and `lw`) were present. Config opened
   visibly again, and C: was not dirty.
5. Shut down normally and reset again, leaving fresh overlays. Both active and
   baseline disk/firmware images passed final qcow2 checks. The stopped guest's
   TPM file exactly matched the baseline copy.

Only after those checks, removed:

- `.vmctl-retired-58c138556b09/` — original disk/firmware/TPM files and the old
  `clean` / `dev` internal snapshots, including their saved RAM.
- `disk.pre-repair-2026-11-08.qcow2` — the now-obsolete repair backup.
- `windows-11.iso` — the detached Windows installer, about 7.9 GiB.

The old quickemu config was retained as
`~/Machines/windows-11.conf.before-compaction-2026-11-08` for reference; it is
not a rollback bundle now that the old image generations have been retired.

## Final storage

| Item | Physical allocation |
| --- | ---: |
| Compressed Windows backing disk | 26.15 GiB |
| Compressed firmware backing image | 268 KiB |
| Fresh disk overlay | 196 KiB |
| Fresh firmware overlay | 196 KiB |
| Entire Windows VM directory | **26.92 GiB** |

Before migration, the main disk alone occupied about 75 GiB, with another
9.15 GiB in saved RAM and 7.9 GiB in the Windows installer. The pre-repair copy
shared most blocks with the main image; it was not counted as another full
75 GiB of independent storage. `btrfs filesystem du` confirmed the final folder
has 26.92 GiB exclusive allocation and only 8 KiB shared. Host free space was
157 GiB after cleanup, though other host activity also affects that number.

## Repeat and extend

```sh
# Collect any evidence from the previous run before shutdown/reset.
deploy/vm/vm-storage --vm windows-11 reset --discard
deploy/vm/vmctl up windows-11
deploy/vm/vmctl ready
deploy/vm/vmctl wait /tmp/windows-desktop.png
```

Run reset only with the VM stopped. Boot/storage operations share a lock,
interrupted replacement journals block boot, missing managed files block boot,
and reset refuses to overwrite an unrelated replacement disk. Eight tests using
tiny real qcow2 images cover reset, firmware/TPM restoration, retained originals,
running-VM rejection, path validation, concurrent operations and interruptions.

Per-VM profiles and Unix source-sync support are implemented for later macOS
and Linux setup. Those guests have **not** been configured or UI-tested during
this migration. Linux build bundles still need compatible architecture and
runtime libraries; installer/update tests must use distributable packages.
The full rollout and coverage plan is in [the VM guide](../README.md).

Temporary screenshots: `/tmp/windows-after-cleanup.png`,
`/tmp/windows-compact-app.png`, `/tmp/windows-compact-reset-verified.png`.
The earlier engine/supervisor runtime findings remain open; this migration
changed test infrastructure and VM storage, not application behavior.
