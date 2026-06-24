# Pyro

**Flash OS images to USB drives & SD cards — safely and easily.**

Pyro is a small, fast image flasher built from scratch in **Rust (Tauri) + React/TypeScript**.
It writes and verifies OS images, with native fingerprint/Touch ID elevation and a
couple of extras the usual flashers don't have.

> Not a fork of any existing flasher — its own code and UI.

## Features

- **Sources:** local file, **flash from URL (streamed — writes while it downloads)**, or **clone** another drive.
- **Formats:** `.img` `.iso` `.dmg` raw, plus on-the-fly decompression of `.gz` `.xz` `.zst` `.bz2` `.zip`.
- **Verify:** reads the drive back and checksums it after writing.
- **bmap:** auto-detects a sibling `.bmap` and skips blank blocks for faster writes.
- **Multiple targets:** write to several drives at once.
- **Safety:** removable-drive detection, system disks hidden behind an explicit "unsafe" toggle, and a too-small-drive guard.
- **Native elevation:** privileged write goes through polkit (Linux — fingerprint via `fprintd`) or the macOS auth dialog (**Touch ID**).
- **Boot-partition config drop:** copy one or more files (e.g. `config.txt`, `ssh`, `user-data`) onto the boot partition after flashing.
- **Boot-file editor:** optionally edit/rename/add files on the boot partition *before* ejecting.
- Drag-and-drop, desktop notification, cancel, live speed/ETA, persisted settings, English + German UI.

## Platforms

Linux and macOS. (Each is built on its own OS — there is no cross-compile for macOS.)

## Develop

```sh
npm install
npm run tauri:dev
```

## Build

```sh
npm run tauri:build           # all default bundles for the host OS
npm run tauri:build -- --bundles appimage   # Linux AppImage only
```

### Requirements

- Rust toolchain + Node.js.
- **Linux:** webkit2gtk-4.1, a polkit auth agent, `lsblk`, `mount`/`umount`, `partprobe`.
  For fingerprint auth: install `fprintd`, enrol a finger, and make sure polkit's PAM stack uses `pam_fprintd`.
- **macOS:** nothing extra; Touch ID works through the system auth dialog if configured.

## License

MIT — see [LICENSE](LICENSE).
