use winresource::{VersionInfo, WindowsResource};

fn main() {
    // Without any rerun-if instruction Cargo re-runs this script when any
    // package file changes. The script only depends on itself and on the
    // version/target environment variables it reads, so scope the rebuilds
    // to those. (The CARGO_PKG_* names are how Cargo is told to watch the
    // manifest's version fields.)
    println!("cargo:rerun-if-changed=build.rs");
    for var in [
        "CARGO_PKG_VERSION_MAJOR",
        "CARGO_PKG_VERSION_MINOR",
        "CARGO_PKG_VERSION_PATCH",
        "CARGO_PKG_VERSION_PRE",
        "CARGO_CFG_TARGET_OS",
        "CARGO_CFG_TARGET_ENV",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }

    // only run if target os is windows
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "windows" {
        return;
    }

    // The winresource-based version-info build script only supports the MSVC
    // toolchain. Reject Windows GNU builds instead of failing later with a
    // confusing linker/resource error.
    if std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default() == "gnu" {
        eprintln!(
            "rimage on Windows only supports the MSVC toolchain; \
             x86_64-pc-windows-gnu / i686-pc-windows-gnu are not supported"
        );
        std::process::exit(1);
    }

    let packed = (env_u64("CARGO_PKG_VERSION_MAJOR") << 48)
        | (env_u64("CARGO_PKG_VERSION_MINOR") << 32)
        | (env_u64("CARGO_PKG_VERSION_PATCH") << 16)
        | (env_u64("CARGO_PKG_VERSION_PRE"));

    let mut res = WindowsResource::new();

    res.set_version_info(VersionInfo::FILEVERSION, packed)
        .set_version_info(VersionInfo::PRODUCTVERSION, packed);

    if let Err(e) = res.compile() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn env_u64(name: &str) -> u64 {
    std::env::var(name).unwrap_or_default().parse().unwrap_or(0)
}
