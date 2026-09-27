use crate::error::{A2aError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::{File, create_dir_all};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::{RwLock, Semaphore};
use tracing::{info, warn};

const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024; // 100MB
const CHUNK_SIZE: usize = 8192; // 8KB chunks
const MAX_CONCURRENT_TRANSFERS: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mime_type: Option<String>,
    pub checksum: Option<String>,
    pub created_at: i64,
    pub chunks_total: usize,
    pub chunks_received: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChunk {
    pub file_id: String,
    pub chunk_index: usize,
    pub data: Vec<u8>,
    pub is_last: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileUploadRequest {
    pub metadata: FileMetadata,
    pub chunk: FileChunk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileUploadResponse {
    pub file_id: String,
    pub chunks_received: usize,
    pub chunks_total: usize,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDownloadRequest {
    pub file_id: String,
    pub chunk_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDownloadResponse {
    pub metadata: FileMetadata,
    pub chunk: FileChunk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransferProgress {
    pub file_id: String,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub percentage: f32,
    pub chunks_transferred: usize,
    pub total_chunks: usize,
}

struct TransferSession {
    metadata: FileMetadata,
    chunks: HashMap<usize, Vec<u8>>,
    #[allow(dead_code)]
    temp_path: PathBuf,
    created_at: chrono::DateTime<chrono::Utc>,
}

pub struct FileTransferManager {
    upload_dir: PathBuf,
    temp_dir: PathBuf,
    max_file_size: u64,
    concurrent_limit: Arc<Semaphore>,
    active_uploads: Arc<RwLock<HashMap<String, TransferSession>>>,
    completed_files: Arc<RwLock<HashMap<String, FileMetadata>>>,
}

impl FileTransferManager {
    pub fn new(base_dir: impl AsRef<Path>) -> Result<Self> {
        let base_path = base_dir.as_ref();
        let upload_dir = base_path.join("uploads");
        let temp_dir = base_path.join("temp");

        Ok(Self {
            upload_dir,
            temp_dir,
            max_file_size: MAX_FILE_SIZE,
            concurrent_limit: Arc::new(Semaphore::new(MAX_CONCURRENT_TRANSFERS)),
            active_uploads: Arc::new(RwLock::new(HashMap::new())),
            completed_files: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn initialize(&self) -> Result<()> {
        create_dir_all(&self.upload_dir)
            .await
            .map_err(|e| A2aError::FileTransfer(format!("Failed to create upload dir: {e}")))?;

        create_dir_all(&self.temp_dir)
            .await
            .map_err(|e| A2aError::FileTransfer(format!("Failed to create temp dir: {e}")))?;

        info!("File transfer manager initialized");
        Ok(())
    }

    pub async fn handle_upload(&self, request: FileUploadRequest) -> Result<FileUploadResponse> {
        let _permit = self
            .concurrent_limit
            .acquire()
            .await
            .map_err(|e| A2aError::FileTransfer(format!("Semaphore error: {e}")))?;

        let file_id = request.metadata.id.clone();

        if request.metadata.size > self.max_file_size {
            return Err(A2aError::FileTransfer(format!(
                "File size {} exceeds maximum allowed size {}",
                request.metadata.size, self.max_file_size
            )));
        }

        let mut uploads = self.active_uploads.write().await;

        let session = uploads.entry(file_id.clone()).or_insert_with(|| {
            let temp_path = self.temp_dir.join(format!("{file_id}.tmp"));
            TransferSession {
                metadata: request.metadata.clone(),
                chunks: HashMap::new(),
                temp_path,
                created_at: chrono::Utc::now(),
            }
        });

        session
            .chunks
            .insert(request.chunk.chunk_index, request.chunk.data);

        let chunks_received = session.chunks.len();
        let chunks_total = session.metadata.chunks_total;
        let completed = chunks_received == chunks_total;

        if completed {
            self.finalize_upload(&file_id, session).await?;
            uploads.remove(&file_id);
        }

        Ok(FileUploadResponse {
            file_id,
            chunks_received,
            chunks_total,
            completed,
        })
    }

    async fn finalize_upload(&self, file_id: &str, session: &TransferSession) -> Result<()> {
        let final_path = self.upload_dir.join(&session.metadata.name);

        let file = File::create(&final_path)
            .await
            .map_err(|e| A2aError::FileTransfer(format!("Failed to create file: {e}")))?;

        let mut writer = BufWriter::new(file);

        for i in 0..session.metadata.chunks_total {
            let chunk = session
                .chunks
                .get(&i)
                .ok_or_else(|| A2aError::FileTransfer(format!("Missing chunk {i}")))?;

            writer
                .write_all(chunk)
                .await
                .map_err(|e| A2aError::FileTransfer(format!("Failed to write chunk: {e}")))?;
        }

        writer
            .flush()
            .await
            .map_err(|e| A2aError::FileTransfer(format!("Failed to flush file: {e}")))?;

        let mut completed = self.completed_files.write().await;
        completed.insert(file_id.to_string(), session.metadata.clone());

        info!(
            "File upload completed: {} -> {}",
            file_id,
            final_path.display()
        );
        Ok(())
    }

    pub async fn handle_download(
        &self,
        request: FileDownloadRequest,
    ) -> Result<FileDownloadResponse> {
        let _permit = self
            .concurrent_limit
            .acquire()
            .await
            .map_err(|e| A2aError::FileTransfer(format!("Semaphore error: {e}")))?;

        let completed = self.completed_files.read().await;
        let metadata = completed
            .get(&request.file_id)
            .ok_or_else(|| A2aError::FileTransfer(format!("File not found: {}", request.file_id)))?
            .clone();

        let file_path = self.upload_dir.join(&metadata.name);

        if !file_path.exists() {
            return Err(A2aError::FileTransfer(format!(
                "File not found on disk: {}",
                file_path.display()
            )));
        }

        let chunk_index = request.chunk_index.unwrap_or(0);
        let chunk = self.read_chunk(&file_path, chunk_index, CHUNK_SIZE).await?;

        Ok(FileDownloadResponse { metadata, chunk })
    }

    async fn read_chunk(&self, path: &Path, index: usize, chunk_size: usize) -> Result<FileChunk> {
        let file = File::open(path)
            .await
            .map_err(|e| A2aError::FileTransfer(format!("Failed to open file: {e}")))?;

        let mut reader = BufReader::new(file);
        let offset = (index * chunk_size) as u64;

        reader
            .seek(tokio::io::SeekFrom::Start(offset))
            .await
            .map_err(|e| A2aError::FileTransfer(format!("Failed to seek: {e}")))?;

        let mut buffer = vec![0u8; chunk_size];
        let bytes_read = reader
            .read(&mut buffer)
            .await
            .map_err(|e| A2aError::FileTransfer(format!("Failed to read: {e}")))?;

        buffer.truncate(bytes_read);

        let is_last = bytes_read < chunk_size;

        Ok(FileChunk {
            file_id: path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string(),
            chunk_index: index,
            data: buffer,
            is_last,
        })
    }

    pub async fn get_transfer_progress(&self, file_id: &str) -> Result<FileTransferProgress> {
        let uploads = self.active_uploads.read().await;

        if let Some(session) = uploads.get(file_id) {
            let chunks_transferred = session.chunks.len();
            let total_chunks = session.metadata.chunks_total;
            // Calculate actual bytes transferred (sum of chunk sizes)
            let bytes_transferred: u64 = session
                .chunks
                .values()
                .map(|chunk| chunk.len() as u64)
                .sum();
            let total_bytes = session.metadata.size;
            let percentage = if total_bytes > 0 {
                ((bytes_transferred as f32 / total_bytes as f32) * 100.0).min(99.99)
            } else {
                0.0
            };

            return Ok(FileTransferProgress {
                file_id: file_id.to_string(),
                bytes_transferred,
                total_bytes,
                percentage,
                chunks_transferred,
                total_chunks,
            });
        }

        let completed = self.completed_files.read().await;
        if let Some(metadata) = completed.get(file_id) {
            return Ok(FileTransferProgress {
                file_id: file_id.to_string(),
                bytes_transferred: metadata.size,
                total_bytes: metadata.size,
                percentage: 100.0,
                chunks_transferred: metadata.chunks_total,
                total_chunks: metadata.chunks_total,
            });
        }

        Err(A2aError::FileTransfer(format!(
            "Transfer not found: {file_id}"
        )))
    }

    pub async fn cleanup_stale_uploads(&self, max_age_seconds: i64) -> Result<usize> {
        let now = chrono::Utc::now();
        let mut uploads = self.active_uploads.write().await;
        let mut removed = 0;

        uploads.retain(|id, session| {
            let age = (now - session.created_at).num_seconds();
            if age >= max_age_seconds {
                warn!("Removing stale upload: {id}");
                removed += 1;
                false
            } else {
                true
            }
        });

        if removed > 0 {
            info!("Cleaned up {removed} stale uploads");
        }

        Ok(removed)
    }

    pub async fn list_files(&self) -> Result<Vec<FileMetadata>> {
        let completed = self.completed_files.read().await;
        Ok(completed.values().cloned().collect())
    }

    pub async fn delete_file(&self, file_id: &str) -> Result<()> {
        let mut completed = self.completed_files.write().await;

        if let Some(metadata) = completed.remove(file_id) {
            let file_path = self.upload_dir.join(&metadata.name);

            if file_path.exists() {
                tokio::fs::remove_file(&file_path)
                    .await
                    .map_err(|e| A2aError::FileTransfer(format!("Failed to delete file: {e}")))?;
            }

            info!("Deleted file: {file_id}");
            Ok(())
        } else {
            Err(A2aError::FileTransfer(format!("File not found: {file_id}")))
        }
    }
}

pub fn calculate_chunks(file_size: u64, chunk_size: usize) -> usize {
    (file_size as usize).div_ceil(chunk_size)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use uuid::Uuid;

    async fn create_test_manager() -> (FileTransferManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let manager = FileTransferManager::new(temp_dir.path()).unwrap();
        manager.initialize().await.unwrap();
        (manager, temp_dir)
    }

    #[tokio::test]
    async fn test_single_chunk_upload() {
        let (manager, _temp) = create_test_manager().await;

        let file_id = Uuid::new_v4().to_string();
        let data = b"Hello, World!";

        let metadata = FileMetadata {
            id: file_id.clone(),
            name: "test.txt".to_string(),
            size: data.len() as u64,
            mime_type: Some("text/plain".to_string()),
            checksum: None,
            created_at: chrono::Utc::now().timestamp(),
            chunks_total: 1,
            chunks_received: 0,
        };

        let chunk = FileChunk {
            file_id: file_id.clone(),
            chunk_index: 0,
            data: data.to_vec(),
            is_last: true,
        };

        let request = FileUploadRequest { metadata, chunk };
        let response = manager.handle_upload(request).await.unwrap();

        assert_eq!(response.chunks_received, 1);
        assert!(response.completed);
    }

    #[tokio::test]
    async fn test_multi_chunk_upload() {
        let (manager, _temp) = create_test_manager().await;

        let file_id = Uuid::new_v4().to_string();
        let chunk1_data = b"Hello, ";
        let chunk2_data = b"World!";
        let total_size = chunk1_data.len() + chunk2_data.len();

        let metadata = FileMetadata {
            id: file_id.clone(),
            name: "test.txt".to_string(),
            size: total_size as u64,
            mime_type: Some("text/plain".to_string()),
            checksum: None,
            created_at: chrono::Utc::now().timestamp(),
            chunks_total: 2,
            chunks_received: 0,
        };

        let request1 = FileUploadRequest {
            metadata: metadata.clone(),
            chunk: FileChunk {
                file_id: file_id.clone(),
                chunk_index: 0,
                data: chunk1_data.to_vec(),
                is_last: false,
            },
        };

        let response1 = manager.handle_upload(request1).await.unwrap();
        assert_eq!(response1.chunks_received, 1);
        assert!(!response1.completed);

        let request2 = FileUploadRequest {
            metadata,
            chunk: FileChunk {
                file_id: file_id.clone(),
                chunk_index: 1,
                data: chunk2_data.to_vec(),
                is_last: true,
            },
        };

        let response2 = manager.handle_upload(request2).await.unwrap();
        assert_eq!(response2.chunks_received, 2);
        assert!(response2.completed);
    }

    #[tokio::test]
    async fn test_file_size_limit() {
        let (mut manager, _temp) = create_test_manager().await;
        manager.max_file_size = 10;

        let file_id = Uuid::new_v4().to_string();
        let metadata = FileMetadata {
            id: file_id.clone(),
            name: "test.txt".to_string(),
            size: 20,
            mime_type: None,
            checksum: None,
            created_at: chrono::Utc::now().timestamp(),
            chunks_total: 1,
            chunks_received: 0,
        };

        let request = FileUploadRequest {
            metadata,
            chunk: FileChunk {
                file_id,
                chunk_index: 0,
                data: vec![0; 20],
                is_last: true,
            },
        };

        let result = manager.handle_upload(request).await;
        assert!(result.is_err());
    }
}
