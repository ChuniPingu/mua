# mua

Rust command-line media conversion tools:

- `mua_wav` validates audio and normalizes it to a target PCM WAV format.
- `mua_img` validates raster images, creates DDS textures, and edits AFB containers.

The Rust source is licensed under either MIT or Apache-2.0. FFmpeg and other dependencies retain
their own licenses. Windows release builds statically link a custom LGPL FFmpeg build; distributors
must follow the notices in `legal/`.

## Requirements

- Rust 1.97.0 (installed automatically by rustup from `rust-toolchain.toml`)
- Visual Studio 2022 C++ build tools
- LLVM/libclang for `ffmpeg-sys-next` binding generation
- A Microsoft vcpkg checkout selected by `VCPKG_ROOT`

## Build and quality checks

```powershell
./scripts/build.ps1
./scripts/check.ps1
./scripts/format.ps1
./scripts/clippy.ps1
./scripts/test.ps1
```

`build.ps1` installs `ffmpeg[custom]:x64-windows-static` from the overlay under `vcpkg/` and
publishes `target/release/mua/` with `mua_wav`, `mua_img`, and legal notices.

## Commands

```text
mua_wav check -s INPUT
mua_wav normalize -s INPUT -d OUTPUT [-o SECONDS]

mua_img check -s INPUT
mua_img jacket -s INPUT -d OUTPUT
mua_img stage -b BACKGROUND [-s TEMPLATE] -d OUTPUT [-n NOTES_FIELD] [--fx1 PATH ... --fx4 PATH]
mua_img extract-dds -s INPUT -d DIRECTORY
```

Embedded templates from `assets/` are used when `-s` is omitted.

Exit codes are 0 for success, 1 for an operational error, 2 for a `mua_wav normalize` no-op, and
64 for CLI usage errors.
