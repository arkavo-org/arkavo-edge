use std::path::Path;

pub fn get_repo_info(path: &Path) -> Result<RepoInfo, Box<dyn std::error::Error>> {
    Ok(RepoInfo {
        path: path.to_path_buf(),
        file_count: 0,
    })
}

pub struct RepoInfo {
    pub path: std::path::PathBuf,
    pub file_count: usize,
}