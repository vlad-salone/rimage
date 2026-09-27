use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::ffi::OsStr;

use super::*;

fn unique_test_dir(name: &str) -> PathBuf {
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("rimage-{name}-{}-{id}", std::process::id()))
}

#[test]
fn find_common_path() {
    let paths = vec![
        PathBuf::from("/path/to/image1.jpg"),
        PathBuf::from("/path/to/image2.jpg"),
        PathBuf::from("/path/to/image3.jpg"),
    ];
    assert_eq!(get_common_path(&paths), Some(PathBuf::from("/path/to")));

    let paths = vec![
        PathBuf::from("/path/to/test/image1.jpg"),
        PathBuf::from("/path/image2.jpg"),
        PathBuf::from("/path/to/image3.jpg"),
    ];
    assert_eq!(get_common_path(&paths), Some(PathBuf::from("/path")));
}

#[test]
fn common_path_is_none_for_empty_input() {
    assert_eq!(get_common_path(&[]), None);
}

#[cfg(windows)]
#[test]
fn common_path_is_none_for_unrelated_windows_drives() {
    let paths = vec![
        PathBuf::from(r"C:\projects\a\image1.jpg"),
        PathBuf::from(r"D:\projects\a\image2.jpg"),
    ];
    assert_eq!(get_common_path(&paths), None);
}

#[test]
fn common_path_and_strip_prefix_ignore_ascii_case_on_all_platforms() {
    let paths = vec![PathBuf::from("/Photos/a"), PathBuf::from("/photos/b")];
    let common = get_common_path(&paths).unwrap();
    assert_eq!(common, PathBuf::from("/Photos"));
    assert_eq!(
        strip_prefix_components(Path::new("/photos/b"), &common),
        Some(PathBuf::from("b"))
    );
}

#[test]
fn paths_equivalent_ignores_ascii_case_on_all_platforms() {
    // Regression: a same-format conversion with `--backup` and a suffix such
    // as `@Backup` maps the output to `img@Backup.png` while the backup is
    // `img@backup.png`. Path comparisons treat names as case-insensitive on
    // every platform (fail-safe), so the collision must be detected before
    // publishing over the backup.
    assert!(paths_equivalent(
        Path::new("/img@Backup.png"),
        Path::new("/img@backup.png")
    ));
    assert!(paths_equivalent(
        Path::new("/photos/a/B.JPG"),
        Path::new("/PHOTOS/A/b.jpg")
    ));
    assert!(!paths_equivalent(Path::new("/a.png"), Path::new("/b.png")));
}

#[cfg(unix)]
#[test]
fn paths_equivalent_resolves_symlink_aliases() {
    use std::os::unix::fs::symlink;

    let root = unique_test_dir("path-equiv-symlink");
    fs::create_dir_all(&root).unwrap();
    let real = root.join("real");
    fs::create_dir_all(&real).unwrap();
    let file = real.join("a.png");
    fs::write(&file, b"x").unwrap();
    let link = root.join("link");
    symlink(&real, &link).unwrap();

    assert!(paths_equivalent(&link.join("a.png"), &file));
    assert!(!paths_equivalent(&link.join("b.png"), &file));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn recursive_single_file_stays_under_output_directory() {
    let root = unique_test_dir("recursive-single");
    let input_dir = root.join("input/photos");
    let output_root = root.join("output");
    fs::create_dir_all(&input_dir).unwrap();
    let input = input_dir.join("image.jpg");
    fs::write(&input, b"test").unwrap();

    let paths = get_paths(vec![input.clone()], Some(output_root.clone()), None, true).unwrap();

    assert_eq!(paths, vec![(input, output_root.join("image"))]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn recursive_files_preserve_relative_parent_structure() {
    let root = unique_test_dir("recursive-multiple");
    let input_root = root.join("input");
    let output_root = root.join("output");
    fs::create_dir_all(input_root.join("a")).unwrap();
    fs::create_dir_all(input_root.join("b")).unwrap();
    let first = input_root.join("a/image.jpg");
    let second = input_root.join("b/image.jpg");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();

    let paths = get_paths(vec![first, second], Some(output_root.clone()), None, true).unwrap();

    assert_eq!(paths[0].1, output_root.join("a/image"));
    assert_eq!(paths[1].1, output_root.join("b/image"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn duplicate_outputs_are_rejected() {
    let root = unique_test_dir("duplicate-output");
    fs::create_dir_all(root.join("a")).unwrap();
    fs::create_dir_all(root.join("b")).unwrap();
    let first = root.join("a/image.jpg");
    let second = root.join("b/image.png");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();

    let error = get_paths(vec![first, second], Some(root.join("out")), None, false).unwrap_err();
    assert!(error.contains("same output path"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_suffixes_are_rejected() {
    for bad in [
        "",
        "..\\evil",
        "a/b",
        "..",
        ".",
        "a\\b",
        "out:2x",
        "na*me",
        "bad\u{1f}",
        "tail.",
        "tail ",
    ] {
        assert!(!valid_suffix(bad), "suffix {bad:?} must be rejected");
    }
    for ok in ["2x", "updated", "v2.1", "a..b"] {
        assert!(valid_suffix(ok), "suffix {ok:?} must be accepted");
    }
}

#[test]
fn normalize_lexically_removes_dot_components() {
    assert_eq!(
        normalize_lexically(Path::new("/a/./b/../c")),
        PathBuf::from("/a/c")
    );
    assert_eq!(normalize_lexically(Path::new("a/../b")), PathBuf::from("b"));
}

#[test]
fn collect_files_expands_globs_but_prefers_existing_literals() {
    let root = unique_test_dir("glob");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.jpg"), b"x").unwrap();
    fs::write(root.join("b.jpg"), b"x").unwrap();
    fs::write(root.join("photo[1].jpg"), b"literal").unwrap();
    fs::write(root.join("photo1.jpg"), b"pattern match").unwrap();

    let pattern = root.join("*.jpg");
    let mut files = collect_files(std::slice::from_ref(&pattern));
    files.sort();
    assert_eq!(files.len(), 4);

    let literal = root.join("photo[1].jpg");
    assert_eq!(collect_files(std::slice::from_ref(&literal)), vec![literal]);
    assert!(collect_files(&[root.join("nomatch_*.jpg")]).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_list_name_matches_case_insensitively() {
    assert!(is_file_list_path(Path::new("file.list")));
    assert!(is_file_list_path(Path::new("./FILE.LIST")));
    assert!(is_file_list_path(&Path::new("dir").join("File.List")));
    assert!(!is_file_list_path(Path::new("other.list")));
    assert!(!is_file_list_path(Path::new("file.txt")));
    assert!(!is_file_list_path(Path::new("myfile.list")));
}

#[test]
fn file_list_reads_one_path_per_line() {
    let root = unique_test_dir("file-list");
    fs::create_dir_all(&root).unwrap();
    let list = root.join("file.list");
    fs::write(
        &list,
        b"photos/a.jpg\r\n  photos/b.jpg\t\n\n   \n#not-a-comment.jpg\n",
    )
    .unwrap();

    let entries = read_file_list_entries(&list).unwrap();
    assert_eq!(
        entries,
        vec![
            PathBuf::from("photos/a.jpg"),
            PathBuf::from("photos/b.jpg"),
            PathBuf::from("#not-a-comment.jpg"),
        ]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_list_must_be_readable_utf8() {
    let root = unique_test_dir("file-list-utf8");
    fs::create_dir_all(&root).unwrap();
    let list = root.join("file.list");
    fs::write(&list, b"\xff\xfe invalid utf-8").unwrap();

    let error = read_file_list_entries(&list).unwrap_err();
    assert!(error.contains("file.list"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_list_strips_only_matched_quote_pairs() {
    let root = unique_test_dir("file-list-quotes");
    fs::create_dir_all(&root).unwrap();
    let list = root.join("file.list");
    // A quoted path loses the pair; a name with a lone quote keeps it.
    fs::write(&list, "\"photos/a.jpg\"\n'photos/b.jpg'\n\"odd.jpg\n").unwrap();

    let entries = read_file_list_entries(&list).unwrap();
    assert_eq!(
        entries,
        vec![
            PathBuf::from("photos/a.jpg"),
            PathBuf::from("photos/b.jpg"),
            PathBuf::from("\"odd.jpg"),
        ]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_list_keeps_trailing_backslashes() {
    let root = unique_test_dir("file-list-backslash");
    fs::create_dir_all(&root).unwrap();
    let list = root.join("file.list");
    // `dir\` is a legitimate Unix file name; only '/' marks a directory.
    fs::write(&list, "dir\\\ndir/\n").unwrap();

    let entries = read_file_list_entries(&list).unwrap();
    assert_eq!(entries, vec![PathBuf::from("dir\\"), PathBuf::from("dir")]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_list_beyond_the_size_cap_is_rejected() {
    let root = unique_test_dir("file-list-huge");
    fs::create_dir_all(&root).unwrap();
    let list = root.join("file.list");
    fs::write(&list, vec![b'x'; (MAX_FILE_LIST_BYTES + 2) as usize]).unwrap();

    let error = read_file_list_entries(&list).unwrap_err();
    assert!(error.contains("size limit"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn expand_file_lists_uses_only_lists_and_ignores_other_inputs() {
    let root = unique_test_dir("expand-file-list");
    fs::create_dir_all(root.join("photos")).unwrap();
    fs::write(root.join("photos/a.jpg"), b"a").unwrap();
    fs::write(root.join("photos/a.png"), b"a").unwrap();
    fs::write(root.join("photos/b.png"), b"b").unwrap();
    fs::write(root.join("direct.jpg"), b"d").unwrap();
    fs::write(root.join("file.list"), "photos/a.jpg\nphotos/*.png\n\n").unwrap();

    let mut files = expand_file_lists(&[root.join("file.list"), root.join("direct.jpg")], |path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
    })
    .unwrap();
    files.sort();

    let mut expected = vec![
        root.join("photos/a.jpg"),
        root.join("photos/a.png"),
        root.join("photos/b.png"),
    ];
    expected.sort();
    assert_eq!(files, expected);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn expand_file_lists_without_lists_keeps_all_inputs() {
    let root = unique_test_dir("expand-without-list");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("direct.jpg"), b"d").unwrap();
    fs::write(root.join("other.png"), b"o").unwrap();

    let direct = root.join("direct.jpg");
    let other = root.join("other.png");
    let mut files = expand_file_lists(&[direct.clone(), other.clone()], |path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
    })
    .unwrap();
    files.sort();

    let mut expected = vec![direct, other];
    expected.sort();
    assert_eq!(files, expected);
    fs::remove_dir_all(root).unwrap();
}

/// Win32 strips trailing dots and spaces from file names before writing, so
/// accepting them would silently redirect the output to a different file —
/// and "CON .png" would normalise onto the reserved CON device.
#[cfg(windows)]
#[test]
fn windows_output_names_cannot_end_in_a_dot_or_space() {
    for bad in ["out.", "out ", "out. ", "CON .png", "con.txt", "NUL."] {
        assert!(
            validate_output_file_name(OsStr::new(bad)).is_err(),
            "{bad:?} must be rejected"
        );
    }
    for ok in [
        "out.jpg",
        "out .jpg",
        "out..jpg",
        "concepts.png",
        "logo.png",
    ] {
        assert!(
            validate_output_file_name(OsStr::new(ok)).is_ok(),
            "{ok:?} must be accepted"
        );
    }
}

#[cfg(unix)]
#[test]
fn file_symlinks_are_accepted() {
    use std::os::unix::fs::symlink;

    let root = unique_test_dir("symlink");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("target.jpg");
    let link = root.join("link.jpg");
    fs::write(&target, b"x").unwrap();
    symlink(&target, &link).unwrap();

    assert!(check_input_file(&link).is_ok());
    fs::remove_dir_all(root).unwrap();
}
