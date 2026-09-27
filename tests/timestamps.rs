//! End-to-end tests for `-p/--preserve-timestamps`.
//!
//! These only compile when the binary is built (`--features build-binary`),
//! because `CARGO_BIN_EXE_rimage` is only defined then.

#![cfg(feature = "build-binary")]

use std::{fs, path::Path, process::Command};

use filetime::{FileTime, set_file_mtime};

/// The modification time to stamp the staged input with.
///
/// Deliberately carries a nanosecond part: whatever the filesystem actually
/// kept of it is what the output has to match, so a whole-second rounding in the
/// tool shows up as a difference. Filesystems differ in the precision they
/// store (NTFS counts in 100ns ticks, FAT in whole seconds), so no expectation
/// here is written against this literal — [`stage`] reads the stored value back.
fn stamp() -> FileTime {
    FileTime::from_unix_time(946_684_800, 123_456_789)
}

/// A copy of the fixture stamped with [`stamp`], an empty output directory
/// beside it, and the modification time the filesystem actually recorded for it.
fn stage(dir: &str) -> (String, String, FileTime) {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let inputs = format!("{dir}/in");
    let outputs = format!("{dir}/out");
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(&inputs).unwrap();
    fs::create_dir_all(&outputs).unwrap();

    let input = format!("{inputs}/img.png");
    fs::copy(format!("{manifest}/tests/files/png/f1t.png"), &input).unwrap();
    set_file_mtime(&input, stamp()).unwrap();
    let stored = modification_time(Path::new(&input));

    (input, outputs, stored)
}

fn modification_time(path: &Path) -> FileTime {
    FileTime::from_last_modification_time(&fs::metadata(path).unwrap())
}

/// Encodes the staged fixture into `outputs`, requiring the run to succeed.
fn encode(input: &str, outputs: &str, flags: &[&str]) {
    let mut args = vec!["png", input, "-d", outputs, "--quiet"];
    args.extend_from_slice(flags);

    let status = Command::new(env!("CARGO_BIN_EXE_rimage"))
        .args(&args)
        .status()
        .unwrap();
    assert!(status.success(), "rimage exited with {status}");
}

#[test]
fn preserve_timestamps_stamps_the_output_with_the_input_time() {
    let dir = format!(
        "{}/target/tmp-preserve-timestamps",
        env!("CARGO_MANIFEST_DIR")
    );
    let (input, outputs, stored) = stage(&dir);

    encode(&input, &outputs, &["-p"]);

    let output = Path::new(&outputs).join("img.png");
    assert_eq!(modification_time(&output), stored);
    // The input is only read, so stamping the output must not move it.
    assert_eq!(modification_time(Path::new(&input)), stored);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_output_keeps_the_time_it_was_written_without_the_flag() {
    let dir = format!(
        "{}/target/tmp-preserve-timestamps-off",
        env!("CARGO_MANIFEST_DIR")
    );
    let (input, outputs, stored) = stage(&dir);

    encode(&input, &outputs, &[]);

    let output = Path::new(&outputs).join("img.png");
    let written = modification_time(&output);
    assert!(
        written > stored && written > stamp(),
        "--preserve-timestamps must be what carries the time over, got {written:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}
