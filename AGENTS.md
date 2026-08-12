# AGENTS.md

Guidance for coding agents working in this repository.

## What this project is

Rimage is a Rust image optimization crate with two deliverables:

- a library that extends [`zune_image`](https://github.com/etemesi254/zune-image) with extra codecs (`avif`, `mozjpeg`, `oxipng`, `tiff`, `webp`) and operations (resize, quantization, ICC)
- a CLI binary (`rimage`, built with `clap`) that drives the library

The library lives in `src/lib.rs`, `src/codecs/` and `src/operations/`; the binary lives in `src/main.rs` and `src/cli/`. The binary only compiles with the `build-binary` feature, so every command that touches it needs `--all-features` or `--features build-binary`.

The CLI exposes one subcommand per output codec: `avif`, `farbfeld`, `jpeg`, `jpeg_xl`, `mozjpeg`, `oxipng`, `png`, `ppm`, `qoi`, `webp`.
`tiff` has no subcommand: it is decode-only and reached through the decoder fallback in `pipeline::decode` for `.tiff`/`.tif` inputs.
`webp`, `avif`, and `jpeg_xl` are supported finitely, only static pictures are supported.
`jpeg_xl` is lossless-only.

Preprocessing covers `--resize` (with `--filter` and direction flags such as `--reduce-only`/`--enlarge-only`), `--quantization`/`--dithering`, and `--premultiply`.

Rust edition is 2024; the crate is developed on stable 1.97 and does not pin an MSRV. `let ... && ...` and `if let ... && ...` chains are used in `src/main.rs` and `src/cli/utils/paths/`.

## Repository layout

| Path                     | Responsibility                                                                                                                                                                  |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/lib.rs`             | Library entry point with `#![warn(missing_docs)]`; re-exports `operations` and `codecs`.                                                                                        |
| `src/main.rs`            | CLI entry point: path normalization, worker pool, EXIF/ICC handling, metadata output, atomic publishing.                                                                        |
| `src/cli.rs`             | clap `Command` root; `after_help` contains the codec table.                                                                                                                     |
| `src/cli/common.rs`      | `CommonArgs` trait: positional files, `-d/--directory`, `-r/--recursive`, `-s/--suffix`, `-b/--backup`, `-t/--threads`, `-x/--strip`, `--no-progress`, `--quiet`, `--metadata`. |
| `src/cli/codecs/`        | One module per CLI codec subcommand, wired onto the clap `Command` by the `Codecs` trait.                                                                                       |
| `src/cli/preprocessors/` | `Preprocessors` trait and `--resize` parsing (`ResizeValue`, `ResizeFilter`), quantization and premultiply flags.                                                               |
| `src/cli/pipeline.rs`    | `decode`, `operations`, `encoder`, `AvailableEncoders`; glue between clap and the library, with inline pipeline tests.                                                          |
| `src/cli/utils/`         | `paths` (glob and `file.list` expansion, output mapping, collision detection) and `threads` (concurrency limit).                                                                |
| `src/codecs/`            | Library codecs (`avif`, `mozjpeg`, `oxipng`, `tiff`, `webp`), each gated behind a feature.                                                                                      |
| `src/operations/`        | Library operations (`resize`, `quantize`, `icc`), each gated behind a feature with a sibling `tests.rs`.                                                                        |
| `src/test_utils.rs`      | `create_test_image_u8/u16/f32/animated` helpers, `#[cfg(test)]` only.                                                                                                           |
| `tests/deadlock.rs`      | End-to-end binary tests; only compiles with `build-binary`.                                                                                                                     |
| `tests/files/`           | Real encoded fixtures per format.                                                                                                                                               |
| `build.rs`               | Windows-only: embeds version resources via `winresource`; no-op elsewhere.                                                                                                      |

## Build, test, and lint

Native dependencies for the C codecs: `cmake`, `ninja`, `meson`, `nasm` , `yasm` (recommended, must on macOS). On Windows use the MSVC toolchain (Visual Studio Build Tools / Developer PowerShell); MozJpeg's SIMD code is incompatible with MinGW/GCC in release mode, so MSVC is required. If Strawberry Perl is installed, make sure its bundled `cmake` does not shadow the real one (see README).

```sh
# build the library and the CLI binary
cargo build --all-features

# run the binary during development
cargo run --features build-binary -- mozjpeg ./image.jpg

# unit and integration tests (install cargo-nextest first)
cargo nextest run --release --all-features

# doc tests; nextest does not run them
cargo test --doc --release --all-features

# plain cargo test also works if nextest is not installed
cargo test --all-features
```

Lint gates, both enforced in CI and both must be clean:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

Builds are slow because the C codecs compile from source: prefer `cargo check --all-features` while iterating and scope test runs with a filter (e.g. `cargo nextest run --all-features -E 'test(resize)'`).

CI (`.github/workflows/rimage.yml`) runs on eight targets: Linux gnu and musl (x86_64, aarch64), Windows MSVC (x86_64, i686) and macOS (x86_64, aarch64). `aarch64-unknown-linux-musl` runs under `cross` with QEMU and `--test-threads=1`; `Cross.toml` pins the cross image by digest and `.cargo/config.toml` provides the aarch64 runners/linkers. The `wasm32-unknown-emscripten` job is intentionally disabled (`if: false`) and kept as a reference. Markdown-only changes are skipped by the `paths-ignore` filter.

## Cargo features

- Operations: `resize` (`fast_image_resize`), `quantization` (`imagequant`), `icc` (`lcms2`); `metadata` enables `--metadata` via `serde`/`serde_json`.
- Codecs: `mozjpeg`, `oxipng`, `avif` (`ravif`/`libavif`), `webp`, `tiff`; `threads` enables parallel encoding paths.
- `build-binary` pulls in the CLI-only dependencies (`clap`, `anyhow`, `rayon`, `indicatif`, ...) and is required by the `[[bin]]` target.

Every codec and operation module is `#[cfg(feature = "...")]`-gated. `AvailableEncoders::encode` matches every codec variant unconditionally, so the binary only compiles with the codec features enabled (the default feature set or `--all-features`); the library compiles without them.

## Architecture and data flow

1. `main.rs` collects inputs through `cli::utils::paths` (globs, `file.list`, recursive expansion), deduplicates them, and rejects output collisions up front.
2. A bounded thread pool (`ConcurrencyLimiter`, `-t/--threads`) processes files. The permit is acquired inside the worker closure, which is what avoids the `-t 1` deadlock pinned by `tests/deadlock.rs`.
3. Each image is decoded (`pipeline::decode`, with a custom decoder fallback for avif/webp/tiff), then `pipeline::operations` builds the operation queue from CLI arguments and it runs in argument order.
4. `operations()` returns a `BTreeMap<usize, Box<dyn OperationsTrait>>` keyed by argument index. Chained `--resize` values compose: each maps the size the previous resize produced, and a skipped step leaves the current size unchanged.
5. Output is written to a unique temporary file and published by rename. `--backup` hard-links the input (copy fallback), never overwrites an existing destination, and the input is removed only after a successful publish.

## Concurrency and lifecycle constraints

- Do not change the `BTreeMap<usize, ...>` keys in `operations()`; they encode CLI argument order.
- Keep `ConcurrencyLimiter` permit acquisition inside the worker closure; acquiring it on the main thread deadlocks with `-t 1`.
- Output must be published atomically (temp file + rename); never write directly to the final output path.
- Keep `tests/deadlock.rs` compatible with `--test-threads=1`; CI runs it that way on the cross/QEMU target.
- Path handling in `src/main.rs` and `src/cli/utils/paths/` is deliberate and load bearing: output paths are compared case-insensitively on every platform and `--backup` destinations are never overwritten. Read the comments there before changing any of it.

## Code conventions

- The lint gates above are the only style rules; rustfmt and clippy handle the rest (`#![warn(missing_docs)]` warnings become errors under `-D warnings`).
- Keep codec and operation modules feature-gated with `#[cfg(feature = "...")]`.
- To add a codec: gate the module in `src/codecs/mod.rs`, add the feature to `Cargo.toml`, and add the matching subcommand in `src/cli/codecs/` wired into the `Codecs` trait.
- Library code uses actual error structures for robustness. Binary code uses `anyhow` for simplicity.

## Testing conventions

- Unit tests live in an inline `#[cfg(test)] mod tests` (e.g. `src/main.rs`, `src/cli.rs`) or in a sibling `tests.rs` (`src/operations/*`, `src/codecs/*`, `src/cli/utils/paths/`). For large test suits, it's recommended to move them to a separate file.
- `tests/deadlock.rs` covers end-to-end binary behavior and only compiles with `build-binary`.
- Pipeline tests live in `src/cli/pipeline.rs` and drive real CLI arguments through `operations()`, asserting both queued operations and final dimensions.
- Build fixture images with the helpers in `src/test_utils.rs` rather than reading files from disk; reserve `tests/files/` for cases that need a real encoded file.
