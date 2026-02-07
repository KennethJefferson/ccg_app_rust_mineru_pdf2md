use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tracing::{debug, info};
use walkdir::WalkDir;

use crate::error::ScanError;
use crate::types::{FileEntry, FileStatus, QueueItem};

#[derive(Debug)]
pub struct ScanResult {
    pub queue: Vec<QueueItem>,
    pub files: Vec<FileEntry>,
    pub total_found: usize,
    pub skipped: usize,
}

pub fn scan_directories(
    input_dirs: &[PathBuf],
    recursive: bool,
    output_dir: Option<&Path>,
) -> Result<ScanResult, ScanError> {
    for dir in input_dirs {
        if !dir.exists() {
            return Err(ScanError::DirNotFound(dir.clone()));
        }
        if !dir.is_dir() {
            return Err(ScanError::NotADirectory(dir.clone()));
        }
    }

    let mut pdf_files: Vec<PathBuf> = Vec::new();

    for dir in input_dirs {
        let walker = if recursive {
            WalkDir::new(dir)
        } else {
            WalkDir::new(dir).max_depth(1)
        };

        for entry in walker {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext.eq_ignore_ascii_case("pdf") {
                        pdf_files.push(path.to_path_buf());
                    }
                }
            }
        }
    }

    pdf_files.sort();
    let total_found = pdf_files.len();
    info!(count = total_found, "Found PDF files");

    let mut queue = Vec::new();
    let mut files = Vec::new();
    let mut skipped = 0;

    if let Some(out_dir) = output_dir {
        let mut name_counts: HashMap<String, usize> = HashMap::new();

        for pdf_path in &pdf_files {
            let stem = pdf_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let md_filename = resolve_collision(&stem, &mut name_counts);
            let output_path = out_dir.join(&md_filename);

            if output_path.exists() {
                debug!(path = %output_path.display(), "Skipping existing file");
                skipped += 1;
                files.push(FileEntry {
                    filename: md_filename,
                    status: FileStatus::Skipped,
                });
                continue;
            }

            let filename = md_filename.clone();
            queue.push(QueueItem {
                source_path: pdf_path.clone(),
                output_path,
                filename: filename.clone(),
            });
            files.push(FileEntry {
                filename,
                status: FileStatus::Pending,
            });
        }
    } else {
        for pdf_path in &pdf_files {
            let output_path = pdf_path.with_extension("md");
            let filename = output_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            if output_path.exists() {
                debug!(path = %output_path.display(), "Skipping existing file");
                skipped += 1;
                files.push(FileEntry {
                    filename,
                    status: FileStatus::Skipped,
                });
                continue;
            }

            queue.push(QueueItem {
                source_path: pdf_path.clone(),
                output_path,
                filename: filename.clone(),
            });
            files.push(FileEntry {
                filename,
                status: FileStatus::Pending,
            });
        }
    }

    if skipped > 0 {
        info!(skipped, "Skipped existing markdown files");
    }

    Ok(ScanResult {
        queue,
        files,
        total_found,
        skipped,
    })
}

fn resolve_collision(stem: &str, name_counts: &mut HashMap<String, usize>) -> String {
    let count = name_counts.entry(stem.to_string()).or_insert(0);
    let filename = if *count == 0 {
        format!("{stem}.md")
    } else {
        format!("{stem}_{count}.md")
    };
    *count += 1;
    filename
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_resolve_collision() {
        let mut counts = HashMap::new();
        assert_eq!(resolve_collision("report", &mut counts), "report.md");
        assert_eq!(resolve_collision("report", &mut counts), "report_1.md");
        assert_eq!(resolve_collision("report", &mut counts), "report_2.md");
        assert_eq!(resolve_collision("other", &mut counts), "other.md");
    }

    #[test]
    fn test_scan_nonexistent_dir() {
        let result = scan_directories(&[PathBuf::from("nonexistent_dir_xyz")], false, None);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ScanError::DirNotFound(_)));
    }

    #[test]
    fn test_scan_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = scan_directories(&[tmp.path().to_path_buf()], false, None).unwrap();
        assert_eq!(result.total_found, 0);
        assert_eq!(result.queue.len(), 0);
    }

    #[test]
    fn test_scan_finds_pdfs() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.pdf"), b"fake pdf").unwrap();
        fs::write(tmp.path().join("b.PDF"), b"fake pdf").unwrap();
        fs::write(tmp.path().join("c.txt"), b"not a pdf").unwrap();

        let result = scan_directories(&[tmp.path().to_path_buf()], false, None).unwrap();
        assert_eq!(result.total_found, 2);
        assert_eq!(result.queue.len(), 2);
    }

    #[test]
    fn test_scan_skips_existing_md() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("doc.pdf"), b"fake pdf").unwrap();
        fs::write(tmp.path().join("doc.md"), b"existing").unwrap();

        let result = scan_directories(&[tmp.path().to_path_buf()], false, None).unwrap();
        assert_eq!(result.total_found, 1);
        assert_eq!(result.skipped, 1);
        assert_eq!(result.queue.len(), 0);
    }

    #[test]
    fn test_scan_recursive() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(tmp.path().join("a.pdf"), b"fake").unwrap();
        fs::write(sub.join("b.pdf"), b"fake").unwrap();

        let non_recursive = scan_directories(&[tmp.path().to_path_buf()], false, None).unwrap();
        assert_eq!(non_recursive.total_found, 1);

        let recursive = scan_directories(&[tmp.path().to_path_buf()], true, None).unwrap();
        assert_eq!(recursive.total_found, 2);
    }

    #[test]
    fn test_scan_with_output_dir_collision() {
        let tmp_in = tempfile::tempdir().unwrap();
        let tmp_in2 = tempfile::tempdir().unwrap();
        let tmp_out = tempfile::tempdir().unwrap();

        fs::write(tmp_in.path().join("report.pdf"), b"fake").unwrap();
        fs::write(tmp_in2.path().join("report.pdf"), b"fake").unwrap();

        let result = scan_directories(
            &[tmp_in.path().to_path_buf(), tmp_in2.path().to_path_buf()],
            false,
            Some(tmp_out.path()),
        )
        .unwrap();

        assert_eq!(result.total_found, 2);
        assert_eq!(result.queue.len(), 2);
        let names: Vec<&str> = result.queue.iter().map(|q| q.filename.as_str()).collect();
        assert!(names.contains(&"report.md"));
        assert!(names.contains(&"report_1.md"));
    }
}
