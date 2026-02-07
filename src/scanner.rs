use std::path::PathBuf;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_nonexistent_dir() {
        let result = scan_directories(&[PathBuf::from("nonexistent_dir_xyz")], false);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ScanError::DirNotFound(_)));
    }

    #[test]
    fn test_scan_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = scan_directories(&[tmp.path().to_path_buf()], false).unwrap();
        assert_eq!(result.total_found, 0);
        assert_eq!(result.queue.len(), 0);
    }

    #[test]
    fn test_scan_finds_pdfs() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.pdf"), b"fake pdf").unwrap();
        fs::write(tmp.path().join("b.PDF"), b"fake pdf").unwrap();
        fs::write(tmp.path().join("c.txt"), b"not a pdf").unwrap();

        let result = scan_directories(&[tmp.path().to_path_buf()], false).unwrap();
        assert_eq!(result.total_found, 2);
        assert_eq!(result.queue.len(), 2);
    }

    #[test]
    fn test_scan_skips_existing_md() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("doc.pdf"), b"fake pdf").unwrap();
        fs::write(tmp.path().join("doc.md"), b"existing").unwrap();

        let result = scan_directories(&[tmp.path().to_path_buf()], false).unwrap();
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

        let non_recursive = scan_directories(&[tmp.path().to_path_buf()], false).unwrap();
        assert_eq!(non_recursive.total_found, 1);

        let recursive = scan_directories(&[tmp.path().to_path_buf()], true).unwrap();
        assert_eq!(recursive.total_found, 2);
    }

}
