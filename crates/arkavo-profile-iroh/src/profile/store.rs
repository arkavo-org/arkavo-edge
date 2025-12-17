//! Profile storage using Iroh blob transport.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument};

use arkavo_tdf_iroh::IrohTransport;

use super::model::{CreatorProfile, ProfileEntry};
use crate::config::Config;

/// Profile store using Iroh for blob storage.
pub struct ProfileStore {
    /// Iroh transport for blob operations
    transport: Arc<IrohTransport>,
    /// In-memory index mapping public_id -> profile entry
    index: Arc<RwLock<HashMap<[u8; 32], ProfileEntry>>>,
}

impl ProfileStore {
    /// Create a new profile store.
    pub async fn new(config: &Config) -> Result<Self> {
        info!("Initializing profile store with data path: {:?}", config.iroh_data_path);

        // Create data directory if it doesn't exist
        if !config.iroh_data_path.exists() {
            std::fs::create_dir_all(&config.iroh_data_path)?;
        }

        let transport = Arc::new(IrohTransport::new().await?);
        info!("Iroh transport initialized");

        Ok(Self {
            transport,
            index: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Publish a profile and return its Iroh ticket.
    #[instrument(skip(self, profile), fields(public_id = %profile.public_id))]
    pub async fn publish(&self, profile: &CreatorProfile) -> Result<String> {
        // Serialize profile to JSON
        let data = serde_json::to_vec(profile)?;
        debug!("Serialized profile: {} bytes", data.len());

        // Stage in Iroh blob store
        let ticket = self.transport.stage(&data).await?;
        info!("Profile staged with ticket: {}", ticket);

        // Update index
        let public_id = profile.public_id_bytes()?;
        let entry = ProfileEntry {
            public_id,
            ticket: ticket.clone(),
            version: profile.version,
            updated_at: profile.updated_at,
        };

        {
            let mut index = self.index.write().await;
            index.insert(public_id, entry);
        }

        Ok(ticket)
    }

    /// Fetch a profile by public ID.
    #[instrument(skip(self))]
    pub async fn fetch(&self, public_id: &[u8; 32]) -> Result<Option<CreatorProfile>> {
        let ticket = {
            let index = self.index.read().await;
            index.get(public_id).map(|e| e.ticket.clone())
        };

        if let Some(ticket) = ticket {
            let profile = self.fetch_by_ticket(&ticket).await?;
            Ok(Some(profile))
        } else {
            Ok(None)
        }
    }

    /// Fetch a profile by Iroh ticket.
    #[instrument(skip(self))]
    pub async fn fetch_by_ticket(&self, ticket: &str) -> Result<CreatorProfile> {
        let data = self.transport.fetch(ticket).await?;
        let profile: CreatorProfile = serde_json::from_slice(&data)?;
        Ok(profile)
    }

    /// Update an existing profile.
    #[instrument(skip(self, profile), fields(public_id = %profile.public_id))]
    pub async fn update(&self, profile: &CreatorProfile) -> Result<String> {
        let public_id = profile.public_id_bytes()?;

        // Check if profile exists
        {
            let index = self.index.read().await;
            if !index.contains_key(&public_id) {
                anyhow::bail!("Profile not found");
            }
        }

        // Publish updated profile
        self.publish(profile).await
    }

    /// Search profiles by query.
    #[instrument(skip(self))]
    pub async fn search(
        &self,
        query: Option<&str>,
        categories: Option<&[super::model::ContentCategory]>,
        limit: usize,
    ) -> Result<Vec<CreatorProfile>> {
        let index = self.index.read().await;
        let mut results = Vec::new();

        for entry in index.values() {
            if results.len() >= limit {
                break;
            }

            if let Ok(profile) = self.fetch_by_ticket(&entry.ticket).await {
                // Filter by query
                if let Some(q) = query {
                    let q_lower = q.to_lowercase();
                    let matches = profile.display_name.to_lowercase().contains(&q_lower)
                        || profile.bio.to_lowercase().contains(&q_lower)
                        || profile.handle.as_ref().map_or(false, |h| h.to_lowercase().contains(&q_lower));

                    if !matches {
                        continue;
                    }
                }

                // Filter by categories
                if let Some(cats) = categories {
                    let has_category = profile
                        .content_categories
                        .iter()
                        .any(|c| cats.contains(c));

                    if !has_category {
                        continue;
                    }
                }

                results.push(profile);
            }
        }

        Ok(results)
    }

    /// Get featured profiles.
    #[instrument(skip(self))]
    pub async fn get_featured(&self, limit: usize) -> Result<Vec<CreatorProfile>> {
        let index = self.index.read().await;
        let mut profiles = Vec::new();

        // Get most recently updated profiles as "featured"
        let mut entries: Vec<_> = index.values().collect();
        entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        for entry in entries.iter().take(limit) {
            if let Ok(profile) = self.fetch_by_ticket(&entry.ticket).await {
                profiles.push(profile);
            }
        }

        Ok(profiles)
    }

    /// Check if the store has a profile.
    pub async fn has_profile(&self, public_id: &[u8; 32]) -> bool {
        let index = self.index.read().await;
        index.contains_key(public_id)
    }

    /// Get profile count.
    pub async fn count(&self) -> usize {
        let index = self.index.read().await;
        index.len()
    }

    /// Get the entry for a profile.
    pub async fn get_entry(&self, public_id: &[u8; 32]) -> Option<ProfileEntry> {
        let index = self.index.read().await;
        index.get(public_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_store_roundtrip() {
        let config = Config::default();
        let store = ProfileStore::new(&config).await.unwrap();

        let public_id = [1u8; 32];
        let mut profile = CreatorProfile::new(&public_id);
        profile.display_name = "Test Creator".to_string();
        profile.bio = "A test creator profile".to_string();

        // Publish
        let ticket = store.publish(&profile).await.unwrap();
        assert!(!ticket.is_empty());

        // Fetch by public ID
        let fetched = store.fetch(&public_id).await.unwrap();
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.display_name, "Test Creator");

        // Fetch by ticket
        let fetched_by_ticket = store.fetch_by_ticket(&ticket).await.unwrap();
        assert_eq!(fetched_by_ticket.display_name, "Test Creator");
    }
}
