# Prepends the MSVC toolchain and Windows SDK to PATH/LIB for this shell only.
#
# Why this exists: Git Bash ships GNU coreutils `link.exe`, which shadows MSVC's
# linker and makes every `cargo build` fail with "link: extra operand". The MSVC
# environment (INCLUDE/LIB/PATH) is normally set up by vcvarsall.bat, which is a
# cmd.exe script and is therefore awkward to use from bash. This file sets the
# same variables directly.
#
# It only affects the current shell. Nothing is written to the persistent
# environment, the registry, or any profile.
#
# The MSVC and SDK versions are auto-detected from the filesystem. If the
# detection fails, override them by setting RIMAGE_MSVC_ROOT and
# RIMAGE_SDK_VER before sourcing this file.
#
# Usage: source ci/msvc-env.sh

_sdk_root='/c/Program Files (x86)/Windows Kits/10'

# --- Target architecture ---------------------------------------------------
# x64 is the default because Git Bash itself is x64-only (on ARM64 Windows it
# runs emulated, and the x64 toolchain is what it can execute). Override for
# an ARM64-native environment (e.g. MSYS2 clangarm64): RIMAGE_ARCH=arm64, or
# for a 32-bit target: RIMAGE_ARCH=x86.
_arch="${RIMAGE_ARCH:-x64}"
case "$_arch" in
    x64)   _host_dir="Hostx64" ;;
    x86)   _host_dir="Hostx64" ;;
    arm64) _host_dir="Hostarm64" ;;
    *)
        echo "msvc-env: unsupported RIMAGE_ARCH '$_arch' (expected x64, x86 or arm64)" >&2
        return 1 2>/dev/null || exit 1
        ;;
esac

# --- Auto-detect the MSVC tools root --------------------------------------
# Picks the highest-numbered MSVC version across every installed VS edition.
# Override: set RIMAGE_MSVC_ROOT to the full MSVC tools path before sourcing.
if [ -z "${RIMAGE_MSVC_ROOT:-}" ]; then
    _vs_base='/c/Program Files (x86)/Microsoft Visual Studio'
    _msvc_root=""
    if [ -d "$_vs_base" ]; then
        # Gather every MSVC tools tree from any edition (BuildTools,
        # Community, Professional, Enterprise), then pick the highest
        # version (lexicographic sort = numeric for dotted versions like
        # 14.51.36231).
        _msvc_root=$(
            for _edition in "$_vs_base"/*/BuildTools \
                            "$_vs_base"/*/Community \
                            "$_vs_base"/*/Professional \
                            "$_vs_base"/*/Enterprise; do
                _vc_tools="$_edition/VC/Tools/MSVC"
                if [ -d "$_vc_tools" ]; then
                    for _ver in "$_vc_tools"/*; do
                        [ -d "$_ver" ] && printf '%s\n' "$_ver"
                    done
                fi
            done | sort -V | tail -1
        )
    fi
    if [ -z "$_msvc_root" ]; then
        echo "msvc-env: could not auto-detect MSVC tools root." >&2
        echo "msvc-env: set RIMAGE_MSVC_ROOT to the full path, e.g.:" >&2
        echo "msvc-env:   export RIMAGE_MSVC_ROOT='/c/Program Files (x86)/Microsoft Visual Studio/18/BuildTools/VC/Tools/MSVC/14.51.36231'" >&2
        return 1 2>/dev/null || exit 1
    fi
else
    _msvc_root="$RIMAGE_MSVC_ROOT"
fi

# An ARM64 host without the native tools installed can still use the
# x64-hosted cross-compiler, which Windows on ARM runs emulated.
if [ ! -d "$_msvc_root/bin/$_host_dir/$_arch" ] && [ -d "$_msvc_root/bin/Hostx64/$_arch" ]; then
    echo "msvc-env: $_host_dir/$_arch not found, falling back to Hostx64/$_arch" >&2
    _host_dir="Hostx64"
fi

# --- Auto-detect the Windows SDK version ---------------------------------
# Override: set RIMAGE_SDK_VER to the desired SDK version string.
if [ -z "${RIMAGE_SDK_VER:-}" ]; then
    _sdk_ver=""
    if [ -d "$_sdk_root/Lib" ]; then
        # Pick the highest version that has both Lib and bin directories.
        _sdk_ver=$(ls -1 "$_sdk_root/Lib" 2>/dev/null | sort -V | tail -1)
    fi
    if [ -z "$_sdk_ver" ]; then
        echo "msvc-env: could not auto-detect Windows SDK version under $_sdk_root." >&2
        echo "msvc-env: set RIMAGE_SDK_VER to the desired version, e.g.:" >&2
        echo "msvc-env:   export RIMAGE_SDK_VER='10.0.26100.0'" >&2
        return 1 2>/dev/null || exit 1
    fi
else
    _sdk_ver="$RIMAGE_SDK_VER"
fi

echo "msvc-env: detected MSVC root: $_msvc_root"
echo "msvc-env: detected SDK ver:    $_sdk_ver"

for _dir in \
    "$_msvc_root/bin/$_host_dir/$_arch" \
    "$_msvc_root/lib/$_arch" \
    "$_sdk_root/Lib/$_sdk_ver/ucrt/$_arch" \
    "$_sdk_root/Lib/$_sdk_ver/um/$_arch" \
    "$_sdk_root/bin/$_sdk_ver/$_arch"
do
    if [ ! -d "$_dir" ]; then
        echo "msvc-env: missing directory: $_dir" >&2
        return 1 2>/dev/null || exit 1
    fi
done

export PATH="$_sdk_root/bin/$_sdk_ver/$_arch:$_msvc_root/bin/$_host_dir/$_arch:$PATH"

# link.exe cannot read Git Bash's /c/... paths, so LIB and INCLUDE must use
# native Windows paths with backslashes. `cygpath -w` does the conversion.
_win() { cygpath -w "$1"; }

# LIB tells link.exe where to resolve kernel32.lib & friends.
export LIB="$(_win "$_msvc_root/lib/$_arch");$(_win "$_sdk_root/Lib/$_sdk_ver/ucrt/$_arch");$(_win "$_sdk_root/Lib/$_sdk_ver/um/$_arch")${LIB:+;$LIB}"

# INCLUDE is needed when a -sys crate compiles C sources.
export INCLUDE="$(_win "$_msvc_root/include");$(_win "$_sdk_root/Include/$_sdk_ver/ucrt");$(_win "$_sdk_root/Include/$_sdk_ver/um");$(_win "$_sdk_root/Include/$_sdk_ver/shared")${INCLUDE:+;$INCLUDE}"

# winresource (build.rs of this crate) resolves rc.exe from its own toolkit_path
# instead of PATH, and derives that path from registry lookups that do not work
# reliably here. RC_PATH short-circuits that lookup with an explicit path.
export RC_PATH="$(_win "$_sdk_root/bin/$_sdk_ver/$_arch/rc.exe")"

_RC_PATH="$RC_PATH"
unset _vs_base _edition _vc_tools _ver _msvc_root _sdk_root _sdk_ver _dir _arch _host_dir
unset RIMAGE_MSVC_ROOT RIMAGE_SDK_VER RIMAGE_ARCH 2>/dev/null || true

echo "msvc-env: link -> $(command -v link)"
echo "msvc-env: rc   -> $_RC_PATH"
unset _RC_PATH
