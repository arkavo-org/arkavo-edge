//! Creator profile data model.
//!
//! This model matches the Swift `CreatorProfile` struct in ArkavoKit
//! for cross-platform compatibility.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Full creator profile with extended fields.
/// Matches the Swift `CreatorProfile` struct for cross-platform sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorProfile {
    /// Unique identifier
    pub id: Uuid,
    /// 32-byte SHA256 hash as hex string
    pub public_id: String,
    /// Decentralized identifier (optional)
    pub did: Option<String>,
    /// User handle (optional)
    pub handle: Option<String>,

    // Display information
    pub display_name: String,
    pub bio: String,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,

    // Social links
    pub social_links: Vec<CreatorSocialLink>,

    // Content
    pub content_categories: Vec<ContentCategory>,
    pub streaming_schedule: Option<StreamingSchedule>,
    pub patron_tiers: Vec<PatronTier>,
    pub featured_content_ids: Vec<String>,

    // Metadata
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: i32,
}

impl CreatorProfile {
    /// Create a new empty profile with the given public ID.
    pub fn new(public_id: &[u8; 32]) -> Self {
        Self {
            id: Uuid::new_v4(),
            public_id: hex::encode(public_id),
            did: None,
            handle: None,
            display_name: String::new(),
            bio: String::new(),
            avatar_url: None,
            banner_url: None,
            social_links: Vec::new(),
            content_categories: Vec::new(),
            streaming_schedule: None,
            patron_tiers: Vec::new(),
            featured_content_ids: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            version: 1,
        }
    }

    /// Get the public ID as bytes.
    pub fn public_id_bytes(&self) -> Result<[u8; 32], hex::FromHexError> {
        let bytes = hex::decode(&self.public_id)?;
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }
}

/// Social link for a creator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorSocialLink {
    pub id: Uuid,
    pub platform: CreatorSocialPlatform,
    pub username: String,
    pub url: String,
    pub is_verified: bool,
}

/// Social platform enum.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CreatorSocialPlatform {
    Twitter,
    Instagram,
    YouTube,
    TikTok,
    Twitch,
    Bluesky,
    Patreon,
    Discord,
    Reddit,
    Custom,
}

/// Content category enum.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ContentCategory {
    Gaming,
    Music,
    Art,
    Technology,
    Education,
    Lifestyle,
    Entertainment,
    Sports,
    News,
    Cooking,
    Fitness,
    Travel,
    Science,
    Comedy,
    Other,
}

/// Streaming schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingSchedule {
    pub slots: Vec<ScheduleSlot>,
    pub timezone: String,
}

/// Schedule slot for streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSlot {
    pub id: Uuid,
    /// Day of week: 1 = Sunday, 7 = Saturday
    pub day_of_week: i32,
    /// Start hour (0-23)
    pub start_hour: i32,
    /// Start minute (0-59)
    pub start_minute: i32,
    /// Duration in minutes
    pub duration_minutes: i32,
    pub title: Option<String>,
}

/// Patron tier for monetization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatronTier {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// Price in cents
    pub price_cents: i32,
    pub benefits: Vec<String>,
    pub is_active: bool,
}

/// Entry in the profile index.
#[derive(Debug, Clone)]
pub struct ProfileEntry {
    /// 32-byte public ID
    pub public_id: [u8; 32],
    /// Iroh blob ticket
    pub ticket: String,
    /// Profile version
    pub version: i32,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_serialization() {
        let public_id = [0u8; 32];
        let profile = CreatorProfile::new(&public_id);

        let json = serde_json::to_string(&profile).unwrap();
        let parsed: CreatorProfile = serde_json::from_str(&json).unwrap();

        assert_eq!(profile.id, parsed.id);
        assert_eq!(profile.public_id, parsed.public_id);
    }

    #[test]
    fn test_public_id_bytes() {
        let public_id = [1u8; 32];
        let profile = CreatorProfile::new(&public_id);

        let bytes = profile.public_id_bytes().unwrap();
        assert_eq!(bytes, public_id);
    }
}
