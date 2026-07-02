use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::input::{self, Source};

pub fn resolve(paths: &[PathBuf], recursive: bool) -> Result<Option<Vec<Source>>, Vec<String>> {
    if paths.is_empty() {
        return Ok(None);
    }

    let mut result = Vec::new();
    let mut errors = Vec::new();

    for path in paths {
        if path.is_dir() {
            if !recursive {
                errors.push(format!("{}: Is a directory", path.display()));
                continue;
            }
            for entry in WalkDir::new(path).into_iter() {
                match entry {
                    Ok(entry)
                        if entry.file_type().is_file()
                            && input::kind_for_path(entry.path()).is_some() =>
                    {
                        result.push(Source::File(entry.path().to_path_buf()));
                    }
                    Ok(_) => {}
                    Err(e) => errors.push(e.to_string()),
                }
            }
        } else {
            result.push(Source::File(path.to_path_buf()));
        }
    }

    result.sort_by(|a, b| source_path(a).cmp(source_path(b)));

    if errors.is_empty() {
        Ok(Some(result))
    } else {
        Err(errors)
    }
}

fn source_path(source: &Source) -> &Path {
    match source {
        Source::File(path) => path,
        Source::Stdin => Path::new("stdin"),
    }
}
