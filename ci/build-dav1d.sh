#!/usr/bin/env bash
# Builds a static dav1d and installs it into a prefix, for targets that have
# no system dav1d package (musl and cross-compiled GNU targets).
#
# Usage: build-dav1d.sh <install-prefix> [meson-cross-file]
#
# The following environment variables select the toolchain when cross file is
# omitted and the target differs from the host: CC, AR.
set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "Usage: build-dav1d.sh <install-prefix> [meson-cross-file]" >&2
    exit 2
fi

prefix=$1
version=1.5.4
# The commit 1.5.4 points at, pinned by its own sha: the tag is annotated, so
# `git rev-parse 1.5.4` would hand back the tag object instead, and comparing
# that against HEAD can never match. A tag can be re-pointed upstream;
# verifying the checkout against the commit keeps the build reproducible.
commit=54706fc6bc0cdecab7e9593974a4039cc038fca7

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

git clone --depth 1 --branch "$version" \
    https://code.videolan.org/videolan/dav1d.git "$work/dav1d"
if [[ $(git -C "$work/dav1d" rev-parse HEAD) != "$commit" ]]; then
    echo "build-dav1d: tag $version no longer points at the pinned commit $commit" >&2
    exit 1
fi

# dav1d 1.5.x spells these options enable_tools/enable_tests; the shorter
# tools/build_tests names are rejected as unknown project options
args=(-Ddefault_library=static -Denable_tools=false -Denable_tests=false -Dlibdir=lib --prefix="$prefix")
if [[ $# -gt 1 ]]; then
    # realpath(1) is GNU-only; cd+pwd resolves the cross file on macOS too.
    args+=(--cross-file "$(cd "$(dirname "$2")" && pwd)/$(basename "$2")")
fi

# options must not sit between meson setup's two positional arguments:
# argparse then fails to bind the trailing sourcedir ("unrecognized
# arguments"), so run from inside the source tree and pass only the builddir
(cd "$work/dav1d" && meson setup "$work/build" "${args[@]}")
ninja -C "$work/build" install
