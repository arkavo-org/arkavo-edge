//! Lesson consolidation and axiom promotion for the synaptic pipeline.
//!
//! Phase 2: Groups duplicate lessons by failure mode, merges clusters.
//! Phase 3: Promotes lessons surviving 3+ consolidation cycles to
//! Canonical tier (axioms). Axioms have falsifiability — confidence
//! decays on failure, demotion at 0.3, deletion at 0.0.

use arkavo_router::learning::{Lesson, LessonSource, MemoryTier};
use std::collections::HashMap;

/// Minimum cluster size before consolidation merges them.
const MIN_CLUSTER_SIZE: usize = 3;

/// Consolidation cycles before promotion to Canonical (axiom).
const PROMOTION_THRESHOLD: u64 = 3;

/// Confidence decay per falsification event.
const FALSIFICATION_DECAY: f64 = 0.1;

/// Confidence below which an axiom is demoted back to Transient.
const DEMOTION_THRESHOLD: f64 = 0.3;

/// Consolidate lessons by grouping on (category, failure_mode_key).
///
/// Clusters of `MIN_CLUSTER_SIZE` or more are replaced by a single
/// summary lesson with aggregated episode count and max confidence.
/// Non-Machine lessons pass through unchanged.
pub(super) fn consolidate_lessons(lessons: Vec<Lesson>) -> Vec<Lesson> {
    let mut clusters: HashMap<(String, String), Vec<Lesson>> = HashMap::new();
    let mut passthrough: Vec<Lesson> = Vec::new();

    for lesson in lessons {
        if lesson.source != LessonSource::Machine {
            passthrough.push(lesson);
            continue;
        }
        let key = (lesson.category.clone(), lesson.pattern.failure_mode_key());
        clusters.entry(key).or_default().push(lesson);
    }

    let mut result = passthrough;

    for (_, cluster) in clusters {
        if cluster.len() < MIN_CLUSTER_SIZE {
            result.extend(cluster);
        } else {
            result.push(merge_cluster(cluster));
        }
    }

    result
}

/// Merge a cluster of similar lessons into one summary.
///
/// Picks the highest-confidence lesson as the representative,
/// sums episode counts, and takes max confidence.
fn merge_cluster(mut cluster: Vec<Lesson>) -> Lesson {
    cluster.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let total_episodes: u32 = cluster.iter().map(|l| l.episode_count).sum();
    let max_confidence = cluster.iter().map(|l| l.confidence).fold(0.0_f64, f64::max);

    let best = cluster.into_iter().next().expect("non-empty cluster");

    Lesson::new(
        best.agent_id,
        best.swarm_id,
        best.category,
        best.pattern,
        max_confidence,
        total_episodes,
    )
}

/// Get the consolidation cycle count from lesson metadata.
fn consolidation_cycles(lesson: &Lesson) -> u64 {
    lesson
        .pattern
        .metadata
        .as_ref()
        .and_then(|m| m.get("consolidation_cycles"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

/// Increment consolidation cycle count in lesson metadata.
fn increment_cycles(lesson: &mut Lesson) {
    let cycles = consolidation_cycles(lesson) + 1;
    let meta = lesson
        .pattern
        .metadata
        .get_or_insert_with(|| serde_json::json!({}));
    if let Some(obj) = meta.as_object_mut() {
        obj.insert(
            "consolidation_cycles".to_string(),
            serde_json::json!(cycles),
        );
    }
}

/// Promote lessons that have survived enough consolidation cycles to Canonical tier.
/// Called after each consolidation pass. Returns the number of promotions.
pub(super) fn promote_axioms(lessons: &mut [Lesson]) -> usize {
    let mut promoted = 0;
    for lesson in lessons.iter_mut() {
        if lesson.tier == MemoryTier::Canonical || lesson.source != LessonSource::Machine {
            continue;
        }
        increment_cycles(lesson);
        let cycles = consolidation_cycles(lesson);
        if cycles >= PROMOTION_THRESHOLD {
            lesson.tier = MemoryTier::Canonical;
            lesson.confidence = 1.0; // Axioms start at full confidence
            lesson.updated_at = chrono::Utc::now();
            promoted += 1;
            tracing::info!(
                category = %lesson.category,
                cycles,
                condition = %lesson.pattern.condition,
                "Lesson promoted to axiom"
            );
        }
    }
    promoted
}

/// Record a falsification event against a lesson. Returns true if the lesson
/// should be deleted (confidence reached 0.0).
pub(super) fn falsify(lesson: &mut Lesson) -> bool {
    lesson.confidence = (lesson.confidence - FALSIFICATION_DECAY).max(0.0);
    lesson.updated_at = chrono::Utc::now();

    if lesson.confidence <= 0.0 {
        tracing::info!(
            category = %lesson.category,
            condition = %lesson.pattern.condition,
            "Axiom deleted — confidence reached 0.0"
        );
        return true; // Delete
    }

    if lesson.tier == MemoryTier::Canonical && lesson.confidence < DEMOTION_THRESHOLD {
        lesson.tier = MemoryTier::Transient;
        tracing::info!(
            category = %lesson.category,
            confidence = lesson.confidence,
            condition = %lesson.pattern.condition,
            "Axiom demoted — confidence below threshold"
        );
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_router::learning::LessonPattern;

    fn make_lesson(category: &str, condition: &str, action: &str, confidence: f64) -> Lesson {
        Lesson::new(
            "agent1".to_string(),
            "swarm1".to_string(),
            category.to_string(),
            LessonPattern::new(condition.to_string(), action.to_string(), String::new()),
            confidence,
            1,
        )
    }

    #[test]
    fn below_threshold_passes_through() {
        let lessons = vec![
            make_lesson("err", "calling step", "avoid: Human594 not found", 0.9),
            make_lesson("err", "calling step", "avoid: Human22609 not found", 0.9),
        ];
        let result = consolidate_lessons(lessons);
        assert_eq!(
            result.len(),
            2,
            "2 lessons below threshold should pass through"
        );
    }

    #[test]
    fn at_threshold_merges() {
        let lessons = vec![
            make_lesson("err", "calling step", "avoid: Human594 not found", 0.8),
            make_lesson("err", "calling step", "avoid: Human22609 not found", 0.9),
            make_lesson("err", "calling step", "avoid: Human100 not found", 0.85),
        ];
        let result = consolidate_lessons(lessons);
        assert_eq!(result.len(), 1, "3 similar lessons should merge to 1");
        assert_eq!(result[0].episode_count, 3);
        assert!((result[0].confidence - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn different_categories_stay_separate() {
        let lessons = vec![
            make_lesson("err", "calling step", "avoid: Human594 not found", 0.9),
            make_lesson("err", "calling step", "avoid: Human100 not found", 0.9),
            make_lesson("err", "calling step", "avoid: Human200 not found", 0.9),
            make_lesson("general", "calling step", "observe first", 0.8),
        ];
        let result = consolidate_lessons(lessons);
        // 3 err merge to 1, 1 general passes through
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn different_failure_modes_stay_separate() {
        let lessons = vec![
            make_lesson("err", "calling step", "avoid: EntityId not found", 0.9),
            make_lesson("err", "calling step", "avoid: EntityId not found", 0.9),
            make_lesson("err", "calling step", "avoid: EntityId not found", 0.9),
            make_lesson("err", "calling action", "avoid: TargetId not found", 0.9),
            make_lesson("err", "calling action", "avoid: TargetId not found", 0.9),
            make_lesson("err", "calling action", "avoid: TargetId not found", 0.9),
        ];
        let result = consolidate_lessons(lessons);
        assert_eq!(
            result.len(),
            2,
            "Two distinct failure modes should produce 2 lessons"
        );
    }

    #[test]
    fn episode_counts_sum() {
        let mut l1 = make_lesson("err", "calling step", "avoid: Human594 not found", 0.9);
        l1.episode_count = 5;
        let mut l2 = make_lesson("err", "calling step", "avoid: Human100 not found", 0.8);
        l2.episode_count = 3;
        let mut l3 = make_lesson("err", "calling step", "avoid: Human200 not found", 0.85);
        l3.episode_count = 2;

        let result = consolidate_lessons(vec![l1, l2, l3]);
        assert_eq!(result[0].episode_count, 10);
    }

    #[test]
    fn empty_input() {
        let result = consolidate_lessons(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn human_lessons_never_consolidated() {
        let mut lessons = vec![
            make_lesson("err", "calling step", "avoid: Human594 not found", 0.9),
            make_lesson("err", "calling step", "avoid: Human100 not found", 0.9),
            make_lesson("err", "calling step", "avoid: Human200 not found", 0.9),
        ];
        for l in &mut lessons {
            l.source = LessonSource::Human;
        }
        let result = consolidate_lessons(lessons);
        assert_eq!(
            result.len(),
            3,
            "Human lessons should never be consolidated"
        );
    }

    #[test]
    fn promote_after_threshold_cycles() {
        let mut lessons = vec![make_lesson("err", "calling step", "avoid: not found", 0.9)];
        // Simulate 3 consolidation cycles
        for _ in 0..PROMOTION_THRESHOLD {
            promote_axioms(&mut lessons);
        }
        assert_eq!(lessons[0].tier, MemoryTier::Canonical);
        assert!((lessons[0].confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn no_premature_promotion() {
        let mut lessons = vec![make_lesson("err", "calling step", "avoid: not found", 0.9)];
        for _ in 0..(PROMOTION_THRESHOLD - 1) {
            promote_axioms(&mut lessons);
        }
        assert_eq!(lessons[0].tier, MemoryTier::Transient);
    }

    #[test]
    fn falsification_decays_confidence() {
        let mut lesson = make_lesson("err", "calling step", "avoid: not found", 1.0);
        lesson.tier = MemoryTier::Canonical;
        let deleted = falsify(&mut lesson);
        assert!(!deleted);
        assert!((lesson.confidence - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn falsification_demotes_axiom() {
        let mut lesson = make_lesson("err", "calling step", "avoid: not found", 0.35);
        lesson.tier = MemoryTier::Canonical;
        let deleted = falsify(&mut lesson);
        assert!(!deleted);
        assert_eq!(lesson.tier, MemoryTier::Transient);
    }

    #[test]
    fn falsification_deletes_at_zero() {
        let mut lesson = make_lesson("err", "calling step", "avoid: not found", 0.05);
        let deleted = falsify(&mut lesson);
        assert!(deleted);
    }

    #[test]
    fn human_lessons_not_promoted() {
        let mut lessons = vec![make_lesson("err", "calling step", "avoid: not found", 0.9)];
        lessons[0].source = LessonSource::Human;
        for _ in 0..5 {
            promote_axioms(&mut lessons);
        }
        assert_eq!(lessons[0].tier, MemoryTier::Transient);
    }
}
