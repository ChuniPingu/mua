# mua

Rust command-line image conversion tool:

- `mua_img` validates raster images, creates DDS textures, and edits AFB containers.

The Rust source is licensed under either MIT or Apache-2.0. Dependencies retain their own licenses.

## Requirements

- Rust 1.97.0 (installed automatically by rustup from `rust-toolchain.toml`)

## Build and quality checks

```powershell
./scripts/build.ps1
./scripts/check.ps1
./scripts/format.ps1
./scripts/clippy.ps1
./scripts/test.ps1
```

`build.ps1` publishes `target/release/mua/` with `mua_img` and license notices.

## Commands

```text
mua_img check -s INPUT
mua_img jacket -s INPUT -d OUTPUT
mua_img stage -b BACKGROUND [-s TEMPLATE] -d OUTPUT [-n NOTES_FIELD] [--fx1 PATH ... --fx4 PATH]
mua_img extract-dds -s INPUT -d DIRECTORY
```

Embedded templates from `assets/` are used when `-s` is omitted.

Exit codes are 0 for success, 1 for an operational error, and 64 for CLI usage errors.
