//! Signed gossip whose signer's key has not arrived yet.
//!
//! Peers exchange keys when they discover each other, and a peer can gossip a
//! lesson in the moment between being discovered and its key being registered.
//! Applying that lesson unverified puts text nobody vouched for into every
//! later prompt. Dropping it loses the lesson for good: the originator sends
//! an announcement once, and the transport discards the re-announcements that
//! anti-entropy digests produce.
//!
//! So the announcement is held, outside every cache, and verified again on a
//! short schedule. It is applied the moment its signature checks out and
//! discarded if the key never arrives.

use std::sync::Arc;
use std::time::Duration;

use arkavo_gossip::{
    AdvisorAdjustmentAnnouncement, GossipError, GossipMessage, GossipProtocol, LessonAnnouncement,
};
use arkavo_router::Router;
use arkavo_router::learning::{Lesson, LessonPattern};
use tokio::sync::{RwLock, Semaphore};

use crate::server::policy_cache::PolicyCache;

/// Waits between attempts to verify a held announcement.
///
/// A key exchange is one round trip started at discovery, so it lands within
/// the first waits. The later ones cover a peer that had to be discovered from
/// the other side first. Past the last, the signer is unknown and stays so.
const KEY_WAITS: [Duration; 6] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// Announcements held at once, across the process. Anyone can send gossip
/// signed by a name no peer has, so what is held for them has to be bounded.
const MAX_HELD: usize = 64;

static HELD: Semaphore = Semaphore::const_new(MAX_HELD);

/// Gossip that carries a signature and changes what the model is told.
#[derive(Clone)]
pub(super) enum Signed {
    Lesson(LessonAnnouncement),
    Adjustment(AdvisorAdjustmentAnnouncement),
}

impl Signed {
    pub(super) fn of(message: &GossipMessage) -> Option<Self> {
        match message {
            GossipMessage::LessonAnnounce(ann) => Some(Self::Lesson(ann.clone())),
            GossipMessage::AdvisorAdjustmentAnnounce(ann) => Some(Self::Adjustment(ann.clone())),
            _ => None,
        }
    }

    fn message(&self) -> GossipMessage {
        match self {
            Self::Lesson(ann) => GossipMessage::LessonAnnounce(ann.clone()),
            Self::Adjustment(ann) => GossipMessage::AdvisorAdjustmentAnnounce(ann.clone()),
        }
    }

    fn signer(&self) -> &str {
        match self {
            Self::Lesson(ann) => &ann.originator,
            Self::Adjustment(ann) => &ann.originator,
        }
    }
}

/// The state verified gossip is applied to, owned so that a held announcement
/// can outlive the call that received it.
#[derive(Clone)]
pub(super) struct Targets {
    pub gossip: Arc<RwLock<GossipProtocol>>,
    pub policy_cache: Arc<RwLock<PolicyCache>>,
    pub router: Arc<RwLock<Option<Arc<Router>>>>,
    pub swarm_id: String,
}

impl Targets {
    /// Apply an announcement the protocol has verified.
    pub(super) async fn apply(&self, signed: &Signed) {
        match signed {
            Signed::Lesson(ann) => self.apply_lesson(ann).await,
            Signed::Adjustment(ann) => self.apply_adjustment(ann).await,
        }
    }

    async fn apply_lesson(&self, ann: &LessonAnnouncement) {
        let pattern = LessonPattern::new(
            ann.condition
                .clone()
                .unwrap_or_else(|| ann.category.clone()),
            ann.action
                .clone()
                .unwrap_or_else(|| "adjust approach".to_string()),
            ann.expected_outcome
                .clone()
                .unwrap_or_else(|| "improved quality".to_string()),
        );
        let lesson = Lesson::new(
            ann.originator.clone(),
            self.swarm_id.clone(),
            ann.category.clone(),
            pattern,
            ann.confidence,
            1,
        );

        self.policy_cache.write().await.add_lesson(lesson);

        tracing::info!(
            lesson_id = %ann.lesson_id,
            category = %ann.category,
            originator = %ann.originator,
            "Gossip lesson applied to policy cache for guidance injection"
        );
    }

    /// Apply a remote advisor adjustment using keep-best merge
    async fn apply_adjustment(&self, ann: &AdvisorAdjustmentAnnouncement) {
        use arkavo_router::prompt_advisor::{AdvisorIssue, DynamicSnapshot};

        let issue = match ann.issue.parse::<AdvisorIssue>() {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!("{}", e);
                return;
            }
        };

        let snapshot = DynamicSnapshot {
            label: ann.label.clone(),
            model_family: ann.model_family.clone(),
            issue,
            text: ann.text.clone(),
            success_rate: ann.stats.success_rate,
            applications: ann.stats.applications,
            feedback_count: ann.stats.feedback_count,
        };

        let router_guard = self.router.read().await;
        if let Some(router) = router_guard.as_ref() {
            router.advisor().import_dynamic_merge_best(vec![snapshot]);
            tracing::info!(
                "Applied remote advisor adjustment from {}: {} ({}, {})",
                ann.originator,
                ann.label,
                ann.model_family,
                ann.issue
            );
        }
    }

    /// Hold `signed` until its signer's key arrives, then apply it.
    ///
    /// Returns false when too much is already held and the announcement was
    /// dropped instead.
    pub(super) fn hold(self, signed: Signed) -> bool {
        let Ok(permit) = HELD.try_acquire() else {
            return false;
        };
        tokio::spawn(async move {
            let _held = permit;
            for wait in KEY_WAITS {
                tokio::time::sleep(wait).await;
                let verdict = self
                    .gossip
                    .read()
                    .await
                    .handle_message(signed.message())
                    .await;
                match verdict {
                    Ok(_) => {
                        tracing::info!(
                            signer = signed.signer(),
                            "Held gossip verified once its signer's key arrived"
                        );
                        self.apply(&signed).await;
                        return;
                    }
                    Err(GossipError::UnknownOriginator(_)) => {}
                    Err(e) => {
                        tracing::warn!(
                            signer = signed.signer(),
                            "Held gossip rejected, not applied: {e}"
                        );
                        return;
                    }
                }
            }
            tracing::warn!(
                signer = signed.signer(),
                "Held gossip dropped: no key arrived for its signer"
            );
        });
        true
    }
}
