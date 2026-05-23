use std::path::{Path, PathBuf};

pub fn get_paths(
    files: Vec<PathBuf>,
    out_dir: Option<PathBuf>,
    suffix: Option<String>,
    recursive: bool,
) -> impl Iterator<Item = (PathBuf, PathBuf)> {
    let common_path = if recursive {
        get_common_path(&files)
    } else {
        None
    };

    files
        .into_iter()
        .filter_map(move |path| -> Option<(PathBuf, PathBuf)> {
            if !path.is_file() {
                log::warn!("{path:?} is not a file");
                return None;
            }

            let file_name = path
                .file_stem()
                .and_then(|f| f.to_str())
                .unwrap_or("optimized_image");

            let mut out_path = match &out_dir {
                Some(dir) => {
                    if let Some(common) = &common_path {
                        let relative_path =
                            path.parent().unwrap().strip_prefix(common).unwrap_or(&path);
                        dir.join(relative_path)
                    } else {
                        dir.to_owned()
                    }
                }
                None => path.parent().map(|p| p.to_path_buf()).unwrap_or_default(),
            };

            if let Some(s) = &suffix {
                out_path.push(format!("{file_name}@{s}"));
            } else {
                out_path.push(file_name);
            }

            Some((path, out_path))
        })
}

fn get_common_path(paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }

    let mut common_path = paths[0].clone();

    for path in paths.iter().skip(1) {
        common_path = common_path
            .iter()
            .zip(path.iter())
            .take_while(|&(a, b)| a == b)
            .map(|(a, _)| a)
            .collect();
    }

    Some(common_path)
}

#[inline]
pub fn collect_files<P: AsRef<Path>>(input: &[P]) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        input.iter().flat_map(apply_glob_pattern).collect()
    }

    #[cfg(not(windows))]
    {
        input.iter().map(|p| PathBuf::from(p.as_ref())).collect()
    }
}

#[cfg(windows)]
fn apply_glob_pattern<P: AsRef<Path>>(path: P) -> Vec<PathBuf> {
    let matches = path
        .as_ref()
        .to_str()
        .and_then(|pattern| glob::glob(pattern).ok())
        .map(|paths| paths.flatten().collect::<Vec<_>>());

    match matches {
        Some(paths) if !paths.is_empty() => paths,
        _ => vec![PathBuf::from(path.as_ref())],
    }
}

#[cfg(test)]
mod tests;
