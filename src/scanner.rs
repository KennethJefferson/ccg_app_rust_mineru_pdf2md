use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::{debug, info, warn};
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

pub fn read_page_count(path: &Path) -> Option<u32> {
    match lopdf::Document::load(path) {
        Ok(doc) => Some(doc.get_pages().len() as u32),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Failed to read PDF page count");
            None
        }
    }
}

pub fn calculate_timeout(page_count: Option<u32>, max_timeout_secs: u64) -> Duration {
    let max = Duration::from_secs(max_timeout_secs);
    let floor = Duration::from_secs(60);
    match page_count {
        Some(pages) => {
            let computed = Duration::from_secs_f64(pages as f64 * 3.0);
            computed.max(floor).min(max)
        }
        None => max,
    }
}

pub fn scan_directories(
    input_dirs: &[PathBuf],
    recursive: bool,
    max_timeout_secs: u64,
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
                page_count: None,
            });
            continue;
        }

        let page_count = read_page_count(pdf_path);
        let timeout = calculate_timeout(page_count, max_timeout_secs);

        info!(
            file = %filename,
            pages = ?page_count,
            timeout_secs = timeout.as_secs(),
            "Queued"
        );

        queue.push(QueueItem {
            source_path: pdf_path.clone(),
            output_path,
            filename: filename.clone(),
            page_count,
            timeout,
        });
        files.push(FileEntry {
            filename,
            status: FileStatus::Pending,
            page_count,
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
        let result = scan_directories(&[PathBuf::from("nonexistent_dir_xyz")], false, 600);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ScanError::DirNotFound(_)));
    }

    #[test]
    fn test_scan_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = scan_directories(&[tmp.path().to_path_buf()], false, 600).unwrap();
        assert_eq!(result.total_found, 0);
        assert_eq!(result.queue.len(), 0);
    }

    #[test]
    fn test_scan_finds_pdfs() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.pdf"), b"fake pdf").unwrap();
        fs::write(tmp.path().join("b.PDF"), b"fake pdf").unwrap();
        fs::write(tmp.path().join("c.txt"), b"not a pdf").unwrap();

        let result = scan_directories(&[tmp.path().to_path_buf()], false, 600).unwrap();
        assert_eq!(result.total_found, 2);
        assert_eq!(result.queue.len(), 2);
        // Fake PDFs can't be parsed by lopdf, so page_count should be None
        for item in &result.queue {
            assert!(item.page_count.is_none());
            // None page_count falls back to max timeout
            assert_eq!(item.timeout, Duration::from_secs(600));
        }
    }

    #[test]
    fn test_scan_skips_existing_md() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("doc.pdf"), b"fake pdf").unwrap();
        fs::write(tmp.path().join("doc.md"), b"existing").unwrap();

        let result = scan_directories(&[tmp.path().to_path_buf()], false, 600).unwrap();
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

        let non_recursive = scan_directories(&[tmp.path().to_path_buf()], false, 600).unwrap();
        assert_eq!(non_recursive.total_found, 1);

        let recursive = scan_directories(&[tmp.path().to_path_buf()], true, 600).unwrap();
        assert_eq!(recursive.total_found, 2);
    }

    #[test]
    fn test_calculate_timeout_with_pages() {
        // 100 pages * 3.0 = 300s
        assert_eq!(calculate_timeout(Some(100), 600), Duration::from_secs(300));
        // 10 pages * 3.0 = 30s, but floor is 60s
        assert_eq!(calculate_timeout(Some(10), 600), Duration::from_secs(60));
        // 1000 pages * 3.0 = 3000s, but capped at 600
        assert_eq!(calculate_timeout(Some(1000), 600), Duration::from_secs(600));
        // 1000 pages * 3.0 = 3000s, capped at 3600
        assert_eq!(calculate_timeout(Some(1000), 3600), Duration::from_secs(3000));
    }

    #[test]
    fn test_calculate_timeout_none_pages() {
        // No page count -> use max timeout
        assert_eq!(calculate_timeout(None, 600), Duration::from_secs(600));
        assert_eq!(calculate_timeout(None, 3600), Duration::from_secs(3600));
    }
}
