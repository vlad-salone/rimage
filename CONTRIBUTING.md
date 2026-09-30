# Contributing to Rimage

Thank you for considering contributing to Rimage! This document will guide you through the process of contributing to the project. Please read it carefully before you start.

## Code of Conduct

Rimage is committed to fostering a welcoming community. Everyone participating in the project is expected to adhere to the [Code of Conduct](CODE_OF_CONDUCT.md).

## Getting Started

To get started with contributing, you'll need to:

1. Fork the repository on GitHub.
2. Clone your fork locally.
3. Install Rust and Cargo (if you haven't already).
4. Install the native build tools: a C compiler, `nasm`, `pkg-config` and a `dav1d` development package. The install steps in [`.github/workflows/rimage.yml`](.github/workflows/rimage.yml) list the exact packages per platform.
5. Run `cargo build --all-features` to ensure everything builds properly, binary included.
   > Note: On Windows, use a Visual Studio build environment like Developer PowerShell for VS 2019/2022.

[`AGENTS.md`](AGENTS.md) holds the commands CI runs and the rules the code follows. It is written for coding agents, but it is also the shortest accurate description of the build for people.

## Making Changes

Once you have a working environment set up, you can start making changes. Before you start, make sure to:

1. Create a new branch for your changes.
2. Write tests for any new functionality (optional, but recommended).
3. Ensure that all tests pass before submitting a pull request.
4. Ensure that your code adheres to the project's style guidelines (run `cargo fmt` to automatically format your code).
5. Ensure that your code passes Clippy's linter (run `cargo clippy`).
6. Commit changes according to the [conventional commits](https://www.conventionalcommits.org/en/v1.0.0/).

## Submitting Changes

Once you've made your changes and ensured that all tests pass, you can submit your changes by:

1. Pushing your changes to your fork.
2. Creating a pull request against the `main` branch of the main repository.
3. Wait for feedback from the maintainers.

## Pull Request Guidelines

When submitting a pull request, please:

1. Include a clear description of the changes you've made.
2. Include a reference to any relevant issues or pull requests.
3. Ensure that your code is well-documented and easy to understand.

## Code Licensing

All contributions to Rimage are dual licensed under either the [Apache License 2.0](LICENSE-APACHE) or the [MIT license](LICENSE-MIT). Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you shall be dual licensed as above, without any additional terms or conditions.

## Conclusion

We appreciate all contributions to Rimage, no matter how small. If you have any questions or concerns, please reach out to us on the project's issue tracker.
