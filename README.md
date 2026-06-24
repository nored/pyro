# Pyro

**Flash OS images to USB drives & SD cards — safely and easily.**

Pyro is a small, fast image flasher built from scratch in **Rust (Tauri) + React/TypeScript**.
It writes and verifies OS images, with native fingerprint/Touch ID elevation and a
couple of extras the usual flashers don't have.

> Not a fork of any existing flasher — its own code and UI.

## Features

- **Sources:** local file, **flash from URL (streamed — writes while it downloads)**, or **clone** another drive.
- **Erase / format:** wipe a USB stick or SD card and lay down a fresh **exFAT / FAT32 / ext4** filesystem (like Raspberry Pi Imager). exFAT is the painless cross-platform default.
- **URL auth & history:** HTTP Basic Auth (username/password), a recent-URLs list, image name from `Content-Disposition`, and test-on-focus-off with a ✓/✗ indicator.
- **Formats:** `.img` `.iso` `.dmg` raw, plus on-the-fly decompression of `.gz` `.xz` `.zst` `.bz2` `.zip`.
- **Verify:** reads the drive back and checksums it after writing.
- **bmap:** auto-detects a sibling `.bmap` and skips blank blocks for faster writes.
- **Multiple targets:** write to several drives at once.
- **Safety:** removable-drive detection, system disks hidden behind an explicit "unsafe" toggle, and a too-small-drive guard.
- **Native elevation:** privileged write goes through polkit (Linux — fingerprint via `fprintd`) or the macOS auth dialog (**Touch ID**).
- **Boot-partition config drop:** copy one or more files (e.g. `config.txt`, `ssh`, `user-data`) onto the boot partition after flashing.
- **Partition picker + boot-file editor:** after writing, pick which partition to mount, then edit/rename/add files on it *before* ejecting.
- Drag-and-drop, desktop notification, cancel, live speed/ETA, persisted settings, English + German UI.

## Platforms

Linux and macOS. Each is built on its own OS — there is no cross-compile, so a macOS `.dmg`/`.app` must be built on a Mac.

## Develop

```sh
npm install
npm run tauri:dev
```

## Build

```sh
npm install
npm run tauri:build           # all default bundles for the host OS
npm run tauri:build -- --bundles appimage   # Linux: AppImage only
```

`tauri:build` automatically builds and stages the privileged `pyro-helper`
(`scripts/stage-helper.mjs` → `src-tauri/binaries/pyro-helper-<target-triple>`),
so it gets bundled next to the main binary in the AppImage / `.app`.

### Requirements

- Rust toolchain + Node.js.
- **Linux:** webkit2gtk-4.1, a polkit auth agent, `lsblk`, `mount`/`umount`, `partprobe`, `parted`, `wipefs`.
  For fingerprint auth: install `fprintd`, enrol a finger, and make sure polkit's PAM stack uses `pam_fprintd`.
  For the Erase feature: `exfatprogs` (exFAT), `dosfstools` (FAT32), `e2fsprogs` (ext4).
- **macOS:** Xcode Command Line Tools (`xcode-select --install`). Everything else — disk formatting (`diskutil`),
  Touch ID elevation (system auth dialog) — is built in. A Mac `.dmg`/`.app` must be built on a Mac.

> The macOS `.app` bundles `pyro-helper` in `Contents/MacOS/`; Pyro runs it via
> the system administrator dialog (Touch ID) when you flash or erase.

## License

MIT — see [LICENSE](LICENSE).
