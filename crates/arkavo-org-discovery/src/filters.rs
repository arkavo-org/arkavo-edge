use crate::discovery::RepoInfo;
use crate::error::Result;
use regex::Regex;

pub struct RepoFilter {
    include_pattern: Option<Regex>,
    exclude_pattern: Option<Regex>,
    include_archived: bool,
    include_private: bool,
}

impl RepoFilter {
    pub fn new() -> Self {
        Self {
            include_pattern: None,
            exclude_pattern: None,
            include_archived: false,
            include_private: true,
        }
    }

    pub fn with_include_pattern(mut self, pattern: &str) -> Result<Self> {
        self.include_pattern = Some(Regex::new(pattern)?);
        Ok(self)
    }

    pub fn with_exclude_pattern(mut self, pattern: &str) -> Result<Self> {
        self.exclude_pattern = Some(Regex::new(pattern)?);
        Ok(self)
    }

    pub fn with_archived(mut self, include: bool) -> Self {
        self.include_archived = include;
        self
    }

    pub fn with_private(mut self, include: bool) -> Self {
        self.include_private = include;
        self
    }

    pub fn filter(&self, repos: Vec<RepoInfo>) -> Vec<RepoInfo> {
        repos
            .into_iter()
            .filter(|repo| {
                if !self.include_archived && repo.is_archived {
                    return false;
                }

                if !self.include_private && repo.is_private {
                    return false;
                }

                if let Some(ref include) = self.include_pattern {
                    if !include.is_match(&repo.full_name) {
                        return false;
                    }
                }

                if let Some(ref exclude) = self.exclude_pattern {
                    if exclude.is_match(&repo.full_name) {
                        return false;
                    }
                }

                true
            })
            .collect()
    }
}

impl Default for RepoFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_repo(name: &str, is_archived: bool, is_private: bool) -> RepoInfo {
        RepoInfo {
            full_name: name.to_string(),
            owner: "test-org".to_string(),
            name: name.split('/').last().unwrap_or(name).to_string(),
            is_archived,
            is_private,
            default_branch: "main".to_string(),
            language: None,
            size_kb: 100,
        }
    }

    #[test]
    fn test_filter_archived() {
        let repos = vec![
            create_test_repo("test-org/active-repo", false, false),
            create_test_repo("test-org/archived-repo", true, false),
        ];

        let filter = RepoFilter::new().with_archived(false);
        let filtered = filter.filter(repos);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].full_name, "test-org/active-repo");
    }

    #[test]
    fn test_filter_private() {
        let repos = vec![
            create_test_repo("test-org/public-repo", false, false),
            create_test_repo("test-org/private-repo", false, true),
        ];

        let filter = RepoFilter::new().with_private(false);
        let filtered = filter.filter(repos);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].full_name, "test-org/public-repo");
    }

    #[test]
    fn test_include_pattern() {
        let repos = vec![
            create_test_repo("test-org/backend-api", false, false),
            create_test_repo("test-org/backend-auth", false, false),
            create_test_repo("test-org/frontend-web", false, false),
        ];

        let filter = RepoFilter::new()
            .with_include_pattern(".*backend.*")
            .unwrap();
        let filtered = filter.filter(repos);

        assert_eq!(filtered.len(), 2);
        assert!(filtered[0].full_name.contains("backend"));
        assert!(filtered[1].full_name.contains("backend"));
    }

    #[test]
    fn test_exclude_pattern() {
        let repos = vec![
            create_test_repo("test-org/app-main", false, false),
            create_test_repo("test-org/app-archive", false, false),
            create_test_repo("test-org/tool-archive", false, false),
        ];

        let filter = RepoFilter::new()
            .with_exclude_pattern(".*-archive")
            .unwrap();
        let filtered = filter.filter(repos);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].full_name, "test-org/app-main");
    }

    #[test]
    fn test_combined_filters() {
        let repos = vec![
            create_test_repo("test-org/backend-api", false, true),
            create_test_repo("test-org/backend-archive", true, false),
            create_test_repo("test-org/frontend-web", false, false),
        ];

        let filter = RepoFilter::new()
            .with_include_pattern(".*backend.*")
            .unwrap()
            .with_exclude_pattern(".*-archive")
            .unwrap()
            .with_archived(false)
            .with_private(true);

        let filtered = filter.filter(repos);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].full_name, "test-org/backend-api");
    }
}
