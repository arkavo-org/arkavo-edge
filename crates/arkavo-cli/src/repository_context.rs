use arkavo_git::GitManager;
use arkavo_memory::storage::MemoryStorage;
use chrono::{DateTime, Utc};
use ignore::gitignore::GitignoreBuilder;
use indicatif::{ProgressBar, ProgressStyle};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use tiktoken_rs::{CoreBPE, cl100k_base};
use tokio::sync::Mutex;

const MAX_CONTEXT_TOKENS: usize = 2000;  // Conservative default for Ollama
const CACHE_SIZE: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub size: u64,
    pub modified: DateTime<Utc>,
    pub is_dir: bool,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryContext {
    pub working_directory: String,
    pub is_git_repo: bool,
    pub current_branch: Option<String>,
    pub git_status: Option<GitStatus>,
    pub recent_commits: Vec<CommitInfo>,
    pub project_files: Vec<FileInfo>,
    pub project_type: Option<String>,
    pub dependencies: HashMap<String, Vec<String>>,
    pub token_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatus {
    pub modified: Vec<String>,
    pub added: Vec<String>,
    pub deleted: Vec<String>,
    pub untracked: Vec<String>,
    pub renamed: Vec<(String, String)>,
    pub conflicted: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub id: String,
    pub author: String,
    pub message: String,
    pub timestamp: i64,
}

pub struct RepositoryContextManager {
    git_manager: GitManager,
    memory_storage: Arc<MemoryStorage>,
    file_cache: Arc<Mutex<LruCache<String, String>>>,
    token_encoder: CoreBPE,
}

impl RepositoryContextManager {
    pub async fn new(memory_storage: Arc<MemoryStorage>) -> anyhow::Result<Self> {
        let cache_size = NonZeroUsize::new(CACHE_SIZE).unwrap();
        Ok(Self {
            git_manager: GitManager::new(),
            memory_storage,
            file_cache: Arc::new(Mutex::new(LruCache::new(cache_size))),
            token_encoder: cl100k_base()?,
        })
    }

    pub async fn build_context(&self) -> anyhow::Result<RepositoryContext> {
        let progress = ProgressBar::new_spinner();
        progress.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        progress.set_message("Building repository context...");

        let current_dir = env::current_dir()?;
        let mut context = RepositoryContext {
            working_directory: current_dir.display().to_string(),
            is_git_repo: false,
            current_branch: None,
            git_status: None,
            recent_commits: Vec::new(),
            project_files: Vec::new(),
            project_type: None,
            dependencies: HashMap::new(),
            token_count: 0,
        };

        // Check if it's a git repository
        if let Ok(repo) = self.git_manager.open_repo(&current_dir) {
            context.is_git_repo = true;
            progress.set_message("Gathering git information...");

            // Get current branch
            if let Ok(branch) = self.git_manager.get_current_branch(&repo) {
                context.current_branch = Some(branch);
            }

            // Get repository status
            if let Ok(status) = self.git_manager.status(&repo) {
                context.git_status = Some(GitStatus {
                    modified: status.modified,
                    added: status.added,
                    deleted: status.deleted,
                    untracked: status.untracked,
                    renamed: status.renamed,
                    conflicted: status.conflicted,
                });
            }

            // Get recent commits
            progress.set_message("Loading recent commits...");
            context.recent_commits = self.get_recent_commits(&repo, 10)?;
        }

        // Build file list with gitignore support
        progress.set_message("Scanning project files...");
        context.project_files = self.scan_project_files(&current_dir).await?;

        // Detect project type and dependencies
        progress.set_message("Analyzing project structure...");
        self.detect_project_type(&mut context, &current_dir).await?;

        // Calculate token count
        let context_string = self.context_to_string(&context);
        context.token_count = self.count_tokens(&context_string);

        // Truncate if necessary
        if context.token_count > MAX_CONTEXT_TOKENS {
            progress.set_message("Optimizing context size...");
            context = self.truncate_context(context, MAX_CONTEXT_TOKENS)?;
        }

        progress.finish_with_message("Repository context ready");
        Ok(context)
    }

    fn get_recent_commits(
        &self,
        repo: &arkavo_git::Repository,
        limit: usize,
    ) -> anyhow::Result<Vec<CommitInfo>> {
        let mut commits = Vec::new();
        let mut revwalk = repo.revwalk()?;
        revwalk.push_head()?;

        for (i, oid) in revwalk.enumerate() {
            if i >= limit {
                break;
            }

            let oid = oid?;
            let commit = repo.find_commit(oid)?;

            commits.push(CommitInfo {
                id: oid.to_string(),
                author: commit.author().name().unwrap_or("Unknown").to_string(),
                message: commit
                    .message()
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string(),
                timestamp: commit.time().seconds(),
            });
        }

        Ok(commits)
    }

    async fn scan_project_files(&self, root: &Path) -> anyhow::Result<Vec<FileInfo>> {
        let mut files = Vec::new();
        let gitignore = self.build_gitignore(root)?;

        self.scan_directory(root, root, &mut files, &gitignore, 0)
            .await?;

        // Sort by path for consistent output
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    fn build_gitignore(&self, root: &Path) -> anyhow::Result<ignore::gitignore::Gitignore> {
        let mut builder = GitignoreBuilder::new(root);

        // Add .gitignore files
        let gitignore_path = root.join(".gitignore");
        if gitignore_path.exists() {
            builder.add(&gitignore_path);
        }

        // Add global gitignore patterns
        builder.add_line(None, "target/")?;
        builder.add_line(None, "node_modules/")?;
        builder.add_line(None, ".git/")?;
        builder.add_line(None, "*.pyc")?;
        builder.add_line(None, "__pycache__/")?;

        Ok(builder.build()?)
    }

    async fn scan_directory(
        &self,
        root: &Path,
        dir: &Path,
        files: &mut Vec<FileInfo>,
        gitignore: &ignore::gitignore::Gitignore,
        depth: usize,
    ) -> anyhow::Result<()> {
        if depth > 5 {
            return Ok(()); // Limit recursion depth
        }

        let entries = fs::read_dir(dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let relative_path = path.strip_prefix(root).unwrap_or(&path);

            // Check gitignore
            let is_ignored = gitignore
                .matched_path_or_any_parents(relative_path, path.is_dir())
                .is_ignore();

            if is_ignored {
                continue;
            }

            let metadata = entry.metadata()?;
            let modified = metadata.modified()?.into();

            files.push(FileInfo {
                path: relative_path.display().to_string(),
                size: metadata.len(),
                modified,
                is_dir: metadata.is_dir(),
                content_hash: None,
            });

            if metadata.is_dir() && !path.ends_with("node_modules") && !path.ends_with("target") {
                Box::pin(self.scan_directory(root, &path, files, gitignore, depth + 1)).await?;
            }
        }

        Ok(())
    }

    async fn detect_project_type(
        &self,
        context: &mut RepositoryContext,
        root: &Path,
    ) -> anyhow::Result<()> {
        // Check for Rust project
        if root.join("Cargo.toml").exists() {
            context.project_type = Some("Rust".to_string());
            if let Ok(content) = self.read_file_cached(&root.join("Cargo.toml")).await {
                context.dependencies.insert(
                    "cargo".to_string(),
                    self.extract_cargo_dependencies(&content),
                );
            }
        }

        // Check for Node.js project
        if root.join("package.json").exists() {
            context.project_type = Some("Node.js".to_string());
            if let Ok(content) = self.read_file_cached(&root.join("package.json")).await {
                context
                    .dependencies
                    .insert("npm".to_string(), self.extract_npm_dependencies(&content));
            }
        }

        // Check for Python project
        if root.join("requirements.txt").exists() {
            context.project_type = Some("Python".to_string());
            if let Ok(content) = self.read_file_cached(&root.join("requirements.txt")).await {
                context.dependencies.insert(
                    "pip".to_string(),
                    content.lines().map(|s| s.to_string()).collect(),
                );
            }
        }

        Ok(())
    }

    async fn read_file_cached(&self, path: &Path) -> anyhow::Result<String> {
        let path_str = path.display().to_string();

        // Generate cache key with file hash
        let metadata = fs::metadata(path)?;
        let modified = metadata.modified()?;
        let cache_key = format!(
            "cache:file:{}@{}",
            path_str,
            self.hash_string(&format!("{:?}", modified))
        );

        // Check in-memory LRU cache
        {
            let mut cache = self.file_cache.lock().await;
            if let Some(content) = cache.get(&cache_key) {
                return Ok(content.clone());
            }
        }

        // Check arkavo-memory
        let memory_results = self
            .memory_storage
            .search(&cache_key, 1, Some("file_cache"))
            .await?;

        if let Some(result) = memory_results.first() {
            let content = result.memory.content.clone();

            // Update LRU cache
            let mut cache = self.file_cache.lock().await;
            cache.put(cache_key.clone(), content.clone());

            return Ok(content);
        }

        // Read from disk
        let content = fs::read_to_string(path)?;

        // Store in arkavo-memory
        let memory = arkavo_memory::models::Memory {
            id: uuid::Uuid::new_v4(),
            content: content.clone(),
            metadata: Some(json!({
                "type": "file_cache",
                "path": path_str,
                "modified": format!("{:?}", modified),
                "size": metadata.len(),
            })),
            category: Some("file_cache".to_string()),
            embedding: vec![0.0; 384], // Placeholder embedding
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        if let Err(e) = self.memory_storage.store(memory).await {
            eprintln!("Warning: Failed to cache file in memory: {}", e);
        }

        // Update LRU cache
        {
            let mut cache = self.file_cache.lock().await;
            cache.put(cache_key, content.clone());
        }

        Ok(content)
    }

    fn hash_string(&self, input: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn extract_cargo_dependencies(&self, content: &str) -> Vec<String> {
        let mut deps = Vec::new();
        let mut in_deps = false;

        for line in content.lines() {
            if line.trim() == "[dependencies]" {
                in_deps = true;
                continue;
            }
            if line.trim().starts_with('[') {
                in_deps = false;
            }
            if in_deps && line.contains('=') {
                if let Some(dep_name) = line.split('=').next() {
                    deps.push(dep_name.trim().to_string());
                }
            }
        }

        deps
    }

    fn extract_npm_dependencies(&self, content: &str) -> Vec<String> {
        let mut deps = Vec::new();

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(content) {
            if let Some(dependencies) = json.get("dependencies").and_then(|d| d.as_object()) {
                deps.extend(dependencies.keys().cloned());
            }
            if let Some(dev_deps) = json.get("devDependencies").and_then(|d| d.as_object()) {
                deps.extend(dev_deps.keys().cloned());
            }
        }

        deps
    }

    fn count_tokens(&self, text: &str) -> usize {
        self.token_encoder.encode_with_special_tokens(text).len()
    }

    fn context_to_string(&self, context: &RepositoryContext) -> String {
        let mut result = String::new();

        result.push_str(&format!(
            "Working directory: {}\n",
            context.working_directory
        ));
        result.push_str(&format!("Git repository: {}\n", context.is_git_repo));

        if let Some(branch) = &context.current_branch {
            result.push_str(&format!("Current branch: {}\n", branch));
        }

        if let Some(status) = &context.git_status {
            result.push_str("\nGit status:\n");
            if !status.modified.is_empty() {
                result.push_str(&format!("  Modified: {} files\n", status.modified.len()));
            }
            if !status.added.is_empty() {
                result.push_str(&format!("  Added: {} files\n", status.added.len()));
            }
            if !status.deleted.is_empty() {
                result.push_str(&format!("  Deleted: {} files\n", status.deleted.len()));
            }
            if !status.untracked.is_empty() {
                result.push_str(&format!("  Untracked: {} files\n", status.untracked.len()));
            }
        }

        if !context.recent_commits.is_empty() {
            result.push_str("\nRecent commits:\n");
            for commit in &context.recent_commits {
                result.push_str(&format!(
                    "  {} - {} ({})\n",
                    &commit.id[..8],
                    commit.message,
                    commit.author
                ));
            }
        }

        if let Some(project_type) = &context.project_type {
            result.push_str(&format!("\nProject type: {}\n", project_type));
        }

        if !context.dependencies.is_empty() {
            result.push_str("\nDependencies:\n");
            for (manager, deps) in &context.dependencies {
                result.push_str(&format!("  {} ({} packages)\n", manager, deps.len()));
            }
        }

        result.push_str("\nProject structure:\n");
        for file in context.project_files.iter().take(30) {
            result.push_str(&format!("  - {}\n", file.path));
        }

        if context.project_files.len() > 30 {
            result.push_str(&format!(
                "  ... and {} more files\n",
                context.project_files.len() - 30
            ));
        }

        result
    }

    fn truncate_context(
        &self,
        mut context: RepositoryContext,
        max_tokens: usize,
    ) -> anyhow::Result<RepositoryContext> {
        // Priority-based truncation
        // 1. Keep git status and branch (highest priority)
        // 2. Keep recent commits (high priority)
        // 3. Truncate project files (medium priority)
        // 4. Truncate dependencies (low priority)

        let mut current_tokens = self.count_tokens(&self.context_to_string(&context));

        // First, limit project files
        while current_tokens > max_tokens && context.project_files.len() > 20 {
            context
                .project_files
                .truncate(context.project_files.len() - 10);
            current_tokens = self.count_tokens(&self.context_to_string(&context));
        }

        // Then, limit commits
        while current_tokens > max_tokens && context.recent_commits.len() > 5 {
            context.recent_commits.pop();
            current_tokens = self.count_tokens(&self.context_to_string(&context));
        }

        // Finally, remove dependencies if needed
        if current_tokens > max_tokens {
            context.dependencies.clear();
        }

        context.token_count = current_tokens;
        Ok(context)
    }
}
