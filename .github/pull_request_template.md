<!-- English preferred. Title as a commit subject: `feat(cli): ...`, `fix(codecs): ...`, `docs: ...`. No emoji. -->

## What and why

<!-- What changes, and the issue or discussion it comes from. -->

## New dependencies

<!-- Crate, version and licence, or "none". -->

## Checklist

- [ ] `CHANGELOG.md` has a line under `# Unreleased`, if behaviour changes
- [ ] a test covers the change; a bug fix has one that fails without it
- [ ] feature-gated code still builds with the feature off
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo nextest run --release --all-features` pass locally
