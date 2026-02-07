use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum ScanError {
    #[error("Input directory does not exist: {0}")]
    DirNotFound(PathBuf),

    #[error("Path is not a directory: {0}")]
    NotADirectory(PathBuf),

    #[error("Walk error: {0}")]
    Walk(#[from] walkdir::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum ApiError {
    #[error("Request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("Server returned error: {0}")]
    Server(String),

    #[error("Request timed out after {0}s")]
    Timeout(u64),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),
}
