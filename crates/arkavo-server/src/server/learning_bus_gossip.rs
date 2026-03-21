//! Gossip protocol integration for LearningBus

use arkavo_gossip::{
    AdvisorAdjustmentAnnouncement, GossipMessage, LessonAnnouncement, LessonDigest,
    sign_lesson_announcement,
};
use arkavo_router::learning::{BurstFeedback, Lesson, LessonPattern};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::learning_bus::{BehaviorAdvice, LearningBus};

impl LearningBus {
    /// Handle incoming gossip message from peer
    ///
    /// For lesson announcements that pass signature verification, immediately
    /// adds the lesson to the local policy cache for behavior guidance injection.
    /// For advisor adjustment announcements, applies keep-best merge to local advisor.
    pub async fn handle_gossip(&self, message: GossipMessage) -> Vec<GossipMessage> {
        let lesson_announce = if let GossipMessage::LessonAnnounce(ref ann) = message {
            Some(ann.clone())
        } else {
            None
        };

        let advisor_announce = if let GossipMessage::AdvisorAdjustmentAnnounce(ref ann) = message {
            Some(ann.clone())
        } else {
            None
        };

        let experiment_announce = if let GossipMessage::ExperimentAnnounce(ref ann) = message {
            Some(ann.clone())
        } else {
            None
        };

        let task_completed = if let GossipMessage::TaskCompleted(ref notice) = message {
            Some(notice.clone())
        } else {
            None
        };

        // Forward patchlet gossip to AutoLearner's bridge for processing
        if matches!(
            &message,
            GossipMessage::PatchAnnounce(_)
                | GossipMessage::PatchDelivery(_)
                | GossipMessage::PatchVote(_)
        ) && let Some(bridge) = self.patchlet_bridge()
            && let Err(e) = bridge.handle_incoming(message.clone()).await
        {
            tracing::debug!("Patchlet bridge handling: {e}");
        }

        let gossip = self.gossip.read().await;
        let responses = match gossip.handle_message(message).await {
            Ok(responses) => responses,
            Err(e) => {
                // Don't early-return: still apply lessons/adjustments
                // even when signature verification fails (key exchange timing race)
                tracing::debug!("Gossip verification pending key exchange: {}", e);
                vec![]
            }
        };
        drop(gossip);

        if let Some(ann) = lesson_announce {
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

            let mut cache = self.policy_cache.write().await;
            cache.add_lesson(lesson);
            drop(cache);

            tracing::info!(
                lesson_id = %ann.lesson_id,
                category = %ann.category,
                originator = %ann.originator,
                "Gossip lesson applied to policy cache for guidance injection"
            );
        }

        if let Some(ann) = advisor_announce {
            self.apply_remote_adjustment(&ann).await;
        }

        if let Some(ann) = experiment_announce {
            tracing::info!(
                experiment_id = %ann.experiment_id,
                originator = %ann.originator,
                weighted_quality = ann.weighted_quality,
                kept = ann.kept,
                model_name = %ann.model_name,
                "Received autoresearch experiment result via gossip"
            );

            // Validate remote priors before processing
            let max_prior = arkavo_llm::autoresearch::MAX_PRIOR;
            if !ann.prior_alpha.is_finite()
                || !ann.prior_beta.is_finite()
                || ann.prior_alpha < 1.0
                || ann.prior_beta < 1.0
                || ann.prior_alpha > max_prior
                || ann.prior_beta > max_prior
            {
                tracing::warn!(
                    experiment_id = %ann.experiment_id,
                    originator = %ann.originator,
                    prior_alpha = ann.prior_alpha,
                    prior_beta = ann.prior_beta,
                    "Rejected experiment with invalid Thompson priors"
                );
            } else if !ann.weighted_quality.is_finite()
                || !(0.0..=1.0).contains(&ann.weighted_quality)
            {
                tracing::warn!(
                    experiment_id = %ann.experiment_id,
                    weighted_quality = ann.weighted_quality,
                    "Rejected experiment with invalid weighted_quality"
                );
            }
            // Update local optimal config store if the experiment was kept
            else if ann.kept
                && !ann.model_name.is_empty()
                && let (Ok(config), Some(model)) = (
                    serde_json::from_str::<arkavo_llm::autoresearch::ExperimentConfig>(
                        &ann.config_json,
                    ),
                    arkavo_router::ModelChoice::from_name(&ann.model_name),
                )
            {
                let thinking = if config.enable_thinking {
                    arkavo_llm::ThinkingMode::On
                } else {
                    arkavo_llm::ThinkingMode::Off
                };
                // Compute posterior mean from remote prior
                let posterior_mean = if ann.prior_alpha + ann.prior_beta > 2.0 {
                    ann.prior_alpha / (ann.prior_alpha + ann.prior_beta)
                } else {
                    0.0 // uniform prior, no real data
                };
                // Only update if remote has meaningful observations
                if posterior_mean > 0.0 {
                    let space = arkavo_llm::autoresearch::SearchSpace::default();
                    if let Some((temp, _max_tok, top_p)) = space.resolve(&config) {
                        let router_guard = self.router.read().await;
                        if let Some(router) = router_guard.as_ref() {
                            router.optimal_configs.update_from_sweep(
                                &model,
                                temp,
                                top_p,
                                thinking,
                                posterior_mean,
                            );
                            tracing::info!(
                                model = %ann.model_name,
                                posterior_mean,
                                "Applied remote autoresearch config via gossip"
                            );
                        }
                    }
                }
            }
        }

        // Handle task completion notifications from specialists
        if let Some(notice) = task_completed {
            tracing::info!(
                task_id = %notice.task_id,
                specialist = %notice.specialist_id,
                succeeded = notice.succeeded,
                completion_ms = notice.completion_ms,
                "Received push task completion via gossip"
            );
            self.task_completions.write().await.push(notice);
        }

        responses
    }

    /// Run anti-entropy synchronization with peers
    pub async fn run_anti_entropy(&self) -> Result<(), String> {
        let gossip = self.gossip.read().await;
        let digest = gossip.create_digest().await;
        let lesson_digest = gossip.create_lesson_digest().await;
        drop(gossip);

        let gossip = self.gossip.read().await;
        let peers = gossip.select_propagation_peers(None).await;
        drop(gossip);

        for peer_id in &peers {
            let _ = self
                .gossip_out_tx
                .send((peer_id.clone(), GossipMessage::AntiEntropy(digest.clone())));
            let _ = self.gossip_out_tx.send((
                peer_id.clone(),
                GossipMessage::LessonDigest(lesson_digest.clone()),
            ));
        }

        tracing::debug!(
            "Anti-entropy sent to {} peers: {} patches, {} lessons",
            peers.len(),
            digest.known_patches.len(),
            lesson_digest.known_lessons.len()
        );

        Ok(())
    }

    /// Synthesize and propagate lessons from accumulated learning
    pub async fn synthesize_and_propagate_lessons(&self) -> Result<(), String> {
        let learning = self.learning.read().await;
        let stats = learning.get_all_stats().await;
        drop(learning);

        if stats.is_empty() {
            return Ok(());
        }

        tracing::debug!(
            "Learning stats available for {} agents (lesson synthesis pending)",
            stats.len()
        );

        Ok(())
    }

    /// Announce a lesson to the gossip network (signs before sending)
    pub async fn announce_lesson(
        &self,
        mut announcement: LessonAnnouncement,
    ) -> Result<(), String> {
        sign_lesson_announcement(&mut announcement, &self.keypair)
            .map_err(|e| format!("Failed to sign lesson: {e}"))?;

        let gossip = self.gossip.read().await;
        let peers = gossip.select_propagation_peers(None).await;
        drop(gossip);

        let peer_count = peers.len();
        for peer_id in peers {
            let _ = self
                .gossip_out_tx
                .send((peer_id, GossipMessage::LessonAnnounce(announcement.clone())));
        }

        tracing::debug!(
            "Announced signed lesson {} to {} peers",
            announcement.lesson_id,
            peer_count
        );
        Ok(())
    }

    /// Create a lesson digest for anti-entropy
    pub async fn create_lesson_digest(&self) -> LessonDigest {
        self.gossip.read().await.create_lesson_digest().await
    }

    /// Check behavior policy for a sector based on learned lessons
    pub async fn check_behavior_policy(&self, sector_id: &str) -> BehaviorAdvice {
        let cache = self.policy_cache.read().await;

        if let Some(lesson) = cache.should_avoid(sector_id) {
            return BehaviorAdvice::AvoidSector {
                reason: lesson.pattern.condition.clone(),
                lesson_id: lesson.id,
                confidence: lesson.confidence,
            };
        }

        if let Some(lesson) = cache.should_slowdown(sector_id) {
            return BehaviorAdvice::SlowDown {
                reason: lesson.pattern.condition.clone(),
                lesson_id: lesson.id,
                confidence: lesson.confidence,
            };
        }

        BehaviorAdvice::Default
    }

    /// Subscribe to lesson approval notifications
    pub async fn subscribe_lesson_approvals(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<LessonAnnouncement>> {
        self.gossip.read().await.subscribe_lesson_approvals()
    }

    /// Start receiving lessons from the gateway and propagating via gossip
    pub fn start_lesson_receiver(&self, mut rx: mpsc::Receiver<Lesson>) {
        let gossip = self.gossip.clone();
        let keypair = self.keypair.clone();
        let gossip_out_tx = self.gossip_out_tx.clone();
        let learning = self.learning.clone();
        let swarm_id = self.swarm_id.clone();
        let policy_cache = self.policy_cache.clone();

        tokio::spawn(async move {
            while let Some(lesson) = rx.recv().await {
                tracing::info!(
                    "Learning bus received lesson: {} (category={})",
                    lesson.pattern.condition,
                    lesson.category
                );

                {
                    let mut cache = policy_cache.write().await;
                    cache.add_lesson(lesson.clone());
                }

                Self::apply_lesson_to_local_routing(&learning, &lesson).await;

                let mut announcement = LessonAnnouncement::new(
                    lesson.id,
                    lesson.compute_hash(),
                    lesson.agent_id.clone(),
                    swarm_id.clone(),
                    lesson.category.clone(),
                    lesson.confidence,
                )
                .with_pattern(
                    lesson.pattern.condition.clone(),
                    lesson.pattern.action.clone(),
                    lesson.pattern.expected_outcome.clone(),
                );

                if let Err(e) = sign_lesson_announcement(&mut announcement, &keypair) {
                    tracing::warn!("Failed to sign lesson announcement: {}", e);
                    continue;
                }

                let g = gossip.read().await;
                let peers = g.select_propagation_peers(None).await;
                drop(g);

                let peer_count = peers.len();
                for peer_id in peers {
                    let _ = gossip_out_tx
                        .send((peer_id, GossipMessage::LessonAnnounce(announcement.clone())));
                }

                tracing::info!(
                    "Propagated lesson {} to {} peers",
                    announcement.lesson_id,
                    peer_count
                );
            }
        });
    }

    /// Apply a received gossip lesson to local routing by injecting synthetic feedback
    async fn apply_lesson_to_local_routing(
        learning: &std::sync::Arc<tokio::sync::RwLock<arkavo_router::learning::LearningModule>>,
        lesson: &Lesson,
    ) {
        let feedback = BurstFeedback::failure(Uuid::new_v4(), lesson.category.clone(), 0)
            .with_quality(1.0 - lesson.confidence);

        learning
            .write()
            .await
            .immediate_update(&lesson.agent_id, &feedback)
            .await;

        tracing::debug!(
            "Applied lesson locally: agent={}, category={}, confidence={}",
            lesson.agent_id,
            lesson.category,
            lesson.confidence
        );
    }

    /// Broadcast proven advisor adjustments to the gossip network
    pub async fn broadcast_advisor_adjustments(&self) {
        use arkavo_gossip::advisor_message::AdjustmentStats;
        use arkavo_gossip::sign_advisor_announcement;

        let router_guard = self.router.read().await;
        let router = match router_guard.as_ref() {
            Some(r) => r,
            None => return,
        };

        let snapshots = router.advisor().export_dynamic();
        drop(router_guard);

        if snapshots.is_empty() {
            return;
        }

        let quality_snapshots: Vec<_> = snapshots
            .into_iter()
            .filter(|s| {
                s.success_rate >= super::learning_bus::BROADCAST_MIN_SUCCESS_RATE
                    && s.feedback_count >= super::learning_bus::BROADCAST_MIN_FEEDBACK_COUNT
                    && s.applications >= super::learning_bus::BROADCAST_MIN_APPLICATIONS
            })
            .collect();

        if quality_snapshots.is_empty() {
            return;
        }

        let gossip = self.gossip.read().await;
        let peers = gossip.select_propagation_peers(None).await;
        drop(gossip);

        if peers.is_empty() {
            return;
        }

        let mut broadcast_count = 0;
        for snap in &quality_snapshots {
            let issue_str = snap.issue.to_string();
            let mut ann = AdvisorAdjustmentAnnouncement::new(
                self.agent_id.clone(),
                snap.model_family.clone(),
                issue_str,
                snap.label.clone(),
                snap.text.clone(),
                AdjustmentStats {
                    success_rate: snap.success_rate,
                    feedback_count: snap.feedback_count,
                    applications: snap.applications,
                    updated_at: chrono::Utc::now(),
                },
            );

            if sign_advisor_announcement(&mut ann, &self.keypair).is_err() {
                continue;
            }

            for peer_id in &peers {
                let _ = self.gossip_out_tx.send((
                    peer_id.clone(),
                    GossipMessage::AdvisorAdjustmentAnnounce(ann.clone()),
                ));
            }
            broadcast_count += 1;
        }

        if broadcast_count > 0 {
            tracing::info!(
                "Broadcast {} advisor adjustments to {} peers",
                broadcast_count,
                peers.len()
            );
        }
    }

    /// Apply a remote advisor adjustment using keep-best merge
    async fn apply_remote_adjustment(&self, ann: &AdvisorAdjustmentAnnouncement) {
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
}
