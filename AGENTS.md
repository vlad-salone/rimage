# AGENTS.md

Guidance for coding agents working in this repository. It holds what is specific to `rimage` and cannot be read off the code. General engineering practice, Rust idiom and how to write a test are left to the agent. People can read it too: the commands below are the ones CI runs.

## What this is

`rimage` is an image optimization library and CLI in one crate. The library extends [`zune_image`](https://github.com/etemesi254/zune-image) with codecs it lacks and with operations; the binary drives that library with `clap`. Everything is behind Cargo features, and the binary only exists with `build-binary`.

Library codecs: `mozjpeg`, `oxipng`, `webp` and `avif` encode and decode, `tiff` and `svg` decode only. AVIF decoding links a system `dav1d` (1.3.0 or newer) through `pkg-config`. Operations: `resize`, `quantization`, `icc`. Beside them, `limits` derives the image size ceiling at run time, `error` defines `RimageError` and `exit` the exit codes.

The CLI has one subcommand per output format: `avif`, `farbfeld`, `jpeg`, `jpeg_xl`, `mozjpeg`, `oxipng`, `png`, `ppm`, `qoi`, `webp`. Animated inputs decode to their first frame. `jpeg_xl` output is lossless only.

Edition 2024. `rust-version` in `Cargo.toml` is the MSRV; CI runs on stable. Windows builds need the MSVC toolchain, and `build.rs` refuses the `-gnu` targets.

## Layout

```text
src/lib.rs               library root, `#![warn(missing_docs)]`
src/codecs/              one module per codec, each `#[cfg(feature = "...")]`
src/operations/          resize, quantize, icc; feature-gated, tests in a sibling `tests.rs`
src/limits.rs            size ceiling from the format's limits, available memory and free disk
src/error.rs             `RimageError`: which side failed, which format, a stable `kind()`, a hint
src/exit.rs              exit codes: 0 ok, 2 usage, 3 input, 4 output, 5 partial; 1 is left to the runtime
src/test_utils.rs        `create_test_image_*` helpers, `#[cfg(test)]` only
src/main.rs              the binary: input expansion, worker pool, metadata, atomic publish, run verdict
src/cli.rs               clap root command; the codec list in `after_help` follows the enabled features
src/cli/codecs/          one clap subcommand per output format, wired by the `Codecs` trait
src/cli/preprocessors/   `--resize`, `--quantization`, `--premultiply` and their parsing
src/cli/pipeline.rs      `decode`, `operations`, `encoder`, `AvailableEncoders`: glue between clap and the library
src/cli/utils/           `paths` (globs, `file.list`, output mapping, collisions), `threads`, `jpeg` (APP segments)
tests/*.rs               end-to-end tests of the built binary, compiled only with `build-binary`
tests/files/             real encoded fixtures per format; keep them small
ci/, Cross.toml, .cargo/ what the cross-compiled CI targets need; a host build ignores them
build.rs                 Windows only: the version resource through `winresource`
```

## Build, test and lint

The C codecs build from source, so the first build is slow. A host build needs a C compiler, `nasm` (the `mozjpeg` SIMD code on x86), `pkg-config` and a `dav1d` development package; `cmake` is no longer needed by anything in the graph, and `meson` and `ninja` only where `dav1d` itself is built from source, which CI does for the musl and cross-compiled targets. The exact packages per platform are the install steps in `.github/workflows/rimage.yml`; the Windows walkthrough is in the README under Build (Windows). They are not copied here because they move.

```sh
# the library and the binary; plain `cargo build` builds only the library
cargo build --all-features

# run the binary
cargo run --all-features -- mozjpeg ./image.jpg

# what CI runs, with RUSTFLAGS="-D warnings" set for all of it
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --release --all-features --no-fail-fast
cargo test --release --all-features --doc      # nextest does not run doctests

# plain cargo test works when nextest is not installed
cargo test --all-features

# while iterating: type-check, then run the tests you touched
cargo check --all-features
cargo nextest run --all-features -E 'test(resize)'
```

CI runs on eight targets: Linux gnu and musl on x86_64 and aarch64, Windows MSVC x86_64 and i686, macOS x86_64 and aarch64. `aarch64-unknown-linux-musl` runs under `cross` with QEMU and `--test-threads=1`. Markdown-only changes skip CI. The `wasm32-unknown-emscripten` job is kept but disabled.

`rustfmt` and `clippy` under `-D warnings` are the whole style guide. There is no other.

## Rules that are not visible from one file

- Every codec and operation is gated in three places: `Cargo.toml`, `src/codecs/mod.rs` or `src/operations/mod.rs`, and the CLI. `limits` and `console` are not default features. CI builds `--all-features` only, so when you touch gated code, build with that feature off yourself; a trimmed build stays warning-free.
- The release profile is `panic = "abort"`. In the binary, no `unwrap`, `expect`, `assert!` or unchecked arithmetic on a path an input file or an argument can reach. Return an error and let the run record it.
- Every early exit from the per-file worker in `src/main.rs` goes through `fail_pipeline!` or `refuse_file!`, so the exit code and the summary keep describing what happened.
- Output is written to a temporary file next to the destination and published by rename. Never write to the final path. `--backup` never overwrites an existing destination, and the input is removed only after the publish succeeded.
- The `ConcurrencyLimiter` permit is acquired inside the worker closure. Acquiring it on the main thread deadlocks with `-t 1`; `tests/deadlock.rs` pins this.
- `operations()` in `src/cli/pipeline.rs` returns a `BTreeMap<usize, Box<dyn OperationsTrait>>` keyed by argument index. The keys are the CLI order; do not change how they are computed.
- Output paths are compared case-insensitively on every platform, and names Windows would rewrite (trailing dots and spaces, device names) are refused. The comments in `src/cli/utils/paths/mod.rs` say why; read them before changing anything there.
- Size limits bound every decode and encode. A decoder checks the dimensions the header declares against the budget before it allocates, and a new decoder does the same.
- `zune-core` is pinned to `=0.5.1` on purpose; the comment in `Cargo.toml` links the upstream issue. Do not bump it as part of an unrelated change.
- Library errors are `RimageError`, which says which side failed and which format. The binary reports through `log`.

Anything narrower than this belongs in a comment next to the code, not here.

## Tests

- Unit tests are an inline `#[cfg(test)] mod tests`, or a sibling `tests.rs` when the module is large (`src/operations/*`, `src/codecs/*`, `src/cli/utils/paths/`).
- Build test images with the helpers in `src/test_utils.rs`. `tests/files/` is for cases that need a real encoded file; reach it through `CARGO_MANIFEST_DIR`, never a hard-coded path.
- End-to-end tests in `tests/` run the built binary and compile only with `build-binary`. They must pass in release mode under `--test-threads=1`, since that is how the QEMU target runs them.
- A bug fix comes with a test that fails without it. A new codec or operation comes with tests on the fixtures and, when it has a flag, a pipeline test in `src/cli/pipeline.rs`.

## Commits, pull requests and the changelog

- Commit subjects follow conventional commits, `type(scope): subject`, lower case, imperative, no emoji and no gitmoji: `fix(cli): reject malformed width and height resize values`. Scopes in use: `cli`, `codecs`, `limits`, `error`, `threads`, `ci`, `deps`, `docs`.
- Branch from `main`, open the PR against `main` and fill in `.github/pull_request_template.md`.
- A behaviour change adds a line to `CHANGELOG.md`. Between releases the top section is `# Unreleased` on line 5, with `### Breaking Changes`, `### Features`, `### Bug Fixes` and `### Improvements` as needed and one `-` bullet per change, starting with a verb. At release time that heading becomes `# [x.y.z](compare link) (date)`. Keep line 5 the release heading and the sections at `###`: `.github/workflows/deploy.yml` takes the release notes from line 6 to the next `#` or `##` heading.
- User-facing text (README, rustdoc, CLI help, error messages, changelog): plain English, backticks on every identifier, flag and path, no emoji. Public items have rustdoc; `missing_docs` is on.

## Do not

- Do not edit `Cargo.lock` by hand or run a blanket `cargo update` in a feature PR. Dependabot handles bumps.
- Do not add `#[allow(...)]` or loosen a lint to get CI green without a comment saying why.
- Do not commit agent configuration of your own: `CLAUDE.md`, `.claude/` and their equivalents are gitignored. This file is the shared one.
