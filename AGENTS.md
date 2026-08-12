# AGENTS.md

## Project overview

Rimage is a Rust image optimization tool. It ships as both a library (`rimage`) and a CLI binary (`rimage`, feature-gated behind `build-binary`).
The CLI exposes one subcommand per codec, a preprocessing pipeline (resize, quantization, alpha premultiply, ICC), bounded parallel processing, metadata handling, and atomic output publishing.

- Rust edition 2024, rustc 1.97.1+ (recommanded)
- Licensed MIT OR Apache-2.0

## Repository layout

| Path                     | Responsibility                                                                                                                                            |
| ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/lib.rs`             | Library entry point with `#![warn(missing_docs)]`; re-exports `operations` and `codecs`. `src/test_utils.rs` is compiled only under `cfg(test)`.          |
| `src/main.rs`            | CLI binary entry point: input collection, concurrency, progress reporting, EXIF/ICC handling, atomic output publishing.                                   |
| `src/cli/cli.rs`         | Builds the clap command tree: codec subcommands + common args + preprocessors.                                                                            |
| `src/cli/common.rs`      | `CommonArgs` trait: input files, `-d/--directory`, `-r/--recursive`, `-s/--suffix`, `-b/--backup`, `-t/--threads`, `-x/--strip`, progress flags.          |
| `src/cli/codecs/`        | One module per CLI codec subcommand; the `Codecs` trait wires them onto the clap `Command`.                                                               |
| `src/cli/pipeline.rs`    | `decode`, `operations`, `encoder`, `AvailableEncoders`. Turns parsed arguments into an ordered operation queue; contains extensive inline pipeline tests. |
| `src/cli/preprocessors/` | `--resize` value/filter parsing (`ResizeValue`, `ResizeFilter`) and preprocessing argument definitions.                                                   |
| `src/cli/utils/`         | `paths` (input collection, normalization, collision detection) and `threads` (concurrency limiter).                                                       |
| `src/operations/`        | Library-side image operations: `resize`, `quantize`, `icc`. Each module has a sibling `tests.rs`.                                                         |
| `src/codecs/`            | Library-side codecs: `avif`, `mozjpeg`, `oxipng`, `tiff`, `webp`, each split into `encoder/` and/or `decoder/` with tests.                                |
| `tests/deadlock.rs`      | End-to-end regression tests that run the built binary; only compiles with `build-binary`.                                                                 |
| `tests/files/`           | Test fixture images.                                                                                                                                      |
| `build.rs`               | Windows-only: embeds version resource info via `winresource`; no-op on other targets.                                                                     |

## Build, test, and lint commands

The commands below mirror `.github/workflows/rimage.yml`, which is the source of truth for CI.

```sh
# Build (all features)
cargo build --release --all-features

# Tests on native targets
# Requires `cargo-nextest` first, install with `cargo install cargo-nextest --locked`
cargo nextest run --release --all-features
cargo test --doc --release --all-features

# Tests on cross targets (aarch64-unknown-linux-musl in CI)
cross test --release --all-features --target <triple> -- --test-threads=1
cross test --release --all-features --target <triple> --doc

# Lint (CI runs with -D warnings)
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

CI runs the test matrix on Linux gnu/musl (x86_64, aarch64), Windows MSVC
(x86_64, i686) and macOS (x86_64, aarch64). `Cross.toml` pins the cross image for `aarch64-unknown-linux-musl`; `.cargo/config.toml` provides the QEMU runner and linkers for aarch64 Linux targets. The WASM job in the workflow is disabled on purpose (`if: false`, see: <https://github.com/vlad-salone/rimage/issues/144>) and is kept as a reference.

### Native toolchain requirements

- Rust stable toolchain with `rustfmt` and `clippy` components.
- C/C++ compiler: MSVC Build Tools on Windows, clang/gcc on Linux and macOS.
  - Note: Not support GNU on Windows.
- `cmake`, `ninja`, `meson`, `nasm`, `yasm` (recommanded, must on MacOS) for the C dependencies (`libaom`, `libavif`, `libwebp`, `mozjpeg`, `lcms2`, `libdeflate`).
  - Note: If strawberry-perl is installed on Windows, please check `cmake.exe` and `make.exe` is not use version provided by strawberry-perl, otherwise it will cause build failure.
  - Note: `libaom` requires older version of the `nasm` binary or `yasm` instead for a successful build.

## Cargo features

`Cargo.toml` gates codecs and operations with optional dependencies:

- `resize` -> `fast_image_resize`; `quantization` -> `imagequant`;
  `icc` -> `lcms2`; `metadata` -> `serde`/`serde_json`.
- `mozjpeg`, `oxipng`, `avif` (ravif/libavif), `webp`, `tiff` -> codec crates.
- `threads` enables parallel encoding paths.
- `build-binary` pulls in the CLI-only dependencies (`clap`, `anyhow`, `rayon`, `indicatif`, `regex`, ...) and is required by the `[[bin]]` target.

`AvailableEncoders::encode` in `src/cli/pipeline.rs` matches every codec variant unconditionally, so the binary only compiles when the codec features are enabled (the default feature set or `--all-features`). The library compiles without them.

## Architecture and data flow

1. `main.rs` collects inputs through `cli::utils::paths` (globs, `file.list`, recursive expansion), deduplicates them, and rejects collisions up front.
2. A bounded thread pool (`ConcurrencyLimiter`, `-t/--threads`) processes files.
   The permit is acquired inside the worker closure, which is what avoids the single-threaded deadlock (`tests/deadlock.rs` pins this).
3. For each image, `pipeline::decode` opens it, `pipeline::operations` builds the operation queue from CLI arguments, and the queue runs in argument order.
4. `pipeline::operations` returns a `BTreeMap<usize, Box<dyn OperationsTrait>>` keyed by argument index, so operations execute in the order they were passed. Multiple `--resize` values chain: each value maps from the size the previous resize produced, and a skipped step leaves the current size unchanged.
5. Encoded output is written to a unique temporary file and published by rename. `--backup` uses hard links with a no-clobber fallback, and the input is removed only after a successful publish.

## Concurrency and lifecycle constraints

- Never change the `BTreeMap<usize, ...>` keys in `operations()` — they encode CLI argument order.
- Keep `ConcurrencyLimiter` permit acquisition inside the worker closure; acquiring it on the main thread deadlocks with `-t 1`.
- Output must be published atomically (temp file + rename); do not write directly to the final output path.
- Keep `tests/deadlock.rs` compatible with `--test-threads=1`, since CI runs it that way on cross/QEMU targets.

## Code style

- Comments and doc comments in English; public library items need `///` docs (`#![warn(missing_docs)]`).
- Logging via the `log` crate (`trace`/`debug` for pipeline setup, `error` for failures); CLI progress via `indicatif` and `indicatif-log-bridge`.
- Library code uses actual error structures for robustness. Binary code uses anyhow for simplicity.
- Clap arguments use the `arg!` macro with `indoc!` `long_help`; use `visible_alias` when offering alternative names (e.g. `--reduce-only`).
- Feature-gate codec and operation imports with `#[cfg(feature = "...")]`.
- Keep `CHANGELOG.md` updated for user-visible changes, grouped under `Breaking Changes` / `Features` / `Bug Fixes` / `Improvements` / `Dependencies` without emoji.
- Commits follow Conventional Commits (see `CONTRIBUTING.md`).

## Testing conventions

- Unit tests live in `#[cfg(test)] mod tests` inside the module (e.g. `value.rs`) or in a sibling `tests.rs` (`src/operations/*`, `src/codecs/*`).
  - Note: For large test suits, it is recommended to move them to a separate file (`/tests/deadlock.rs`).
- Pipeline tests live in `src/cli/pipeline.rs` and drive real CLI arguments through `operations()`, asserting both queued operations and final dimensions.
- `tests/deadlock.rs` covers end-to-end binary behavior and only compiles with `build-binary`.
- Test images are fixtures under `tests/files/`; `src/test_utils.rs` provides image builders for library tests.

## Generated files and files that must not be edited manually

- `Cargo.lock` is committed (binary crate); update it through `cargo` commands only.
- `target/` and `build.rs` outputs are build artifacts.
- `tests/files/` fixtures are regenerated through project tooling (see the CHANGELOG note about regenerating the AVIF fixture), not hand-edited.
