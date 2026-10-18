use arkavo_test_macros::spec;

use super::*;

fn failed_image(text: &str) -> Anomaly {
    Anomaly {
        key: "image-generation-failed".into(),
        severity: NoticeSeverity::Warning,
        title: "Image generation failed".into(),
        text: text.into(),
        focus: Some(NoticeFocus {
            view: "agents".into(),
            agent: None,
            task: None,
        }),
        details: vec![detail("Code", "generation_failed")],
    }
}

/// SRV-016: a raised anomaly is open with what the operator can do about
/// it: look where it points, retry its task, acknowledge it.
#[spec("SRV-016")]
#[test]
fn a_raised_anomaly_is_open_with_its_actions() {
    let hub = OperatorNotices::default();
    hub.identify("artist");

    hub.raise(failed_image("The GPU ran out of memory."));

    let open = hub.open();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].agent_id, "artist");
    let actions: Vec<_> = open[0].actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(actions, ["inspect", "acknowledge"]);
    assert_eq!(open[0].actions[0].label, "Inspect agent");
    assert_eq!(
        open[0].focus.as_ref().and_then(|f| f.agent.as_deref()),
        Some("artist"),
        "a notice about this agent points at it"
    );
}

/// SRV-016: the same anomaly again updates its notice, counted, keeping the
/// higher severity, so a flapping failure is one notice that changes.
#[spec("SRV-016")]
#[test]
fn a_repeated_anomaly_updates_its_notice() {
    let hub = OperatorNotices::default();
    hub.raise(failed_image("first"));
    let mut again = failed_image("second");
    again.severity = NoticeSeverity::Critical;
    hub.raise(again);
    hub.raise(failed_image("third"));

    let open = hub.open();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].count, 3);
    assert_eq!(open[0].revision, 3);
    assert_eq!(open[0].severity, NoticeSeverity::Critical);
    assert_eq!(open[0].text, "third");
}

/// SRV-016: acknowledging closes a notice; an action it does not offer, or
/// a notice that is not open, is refused.
#[spec("SRV-016")]
#[test]
fn acknowledging_closes_a_notice() {
    let hub = OperatorNotices::default();
    hub.raise(failed_image("x"));
    let id = hub.open()[0].id.clone();

    let refused = hub.respond(&NoticeResponse {
        notice_id: id.clone(),
        action_id: "pause".into(),
    });
    assert!(refused.unwrap_err().contains("no action pause"));
    hub.respond(&NoticeResponse {
        notice_id: id.clone(),
        action_id: "inspect".into(),
    })
    .unwrap();
    assert_eq!(hub.open().len(), 1, "inspecting leaves the notice open");

    hub.respond(&NoticeResponse {
        notice_id: id.clone(),
        action_id: "acknowledge".into(),
    })
    .unwrap();
    assert_eq!(hub.open().len(), 0);
    assert!(
        hub.respond(&NoticeResponse {
            notice_id: id,
            action_id: "acknowledge".into()
        })
        .is_err()
    );
}

/// SRV-016: at most twenty notices stay open; the oldest closes first.
#[spec("SRV-016")]
#[test]
fn the_oldest_notice_closes_past_the_limit() {
    const KEYS: [&str; 21] = [
        "k0", "k1", "k2", "k3", "k4", "k5", "k6", "k7", "k8", "k9", "k10", "k11", "k12", "k13",
        "k14", "k15", "k16", "k17", "k18", "k19", "k20",
    ];
    let hub = OperatorNotices::default();
    for key in KEYS {
        hub.raise(Anomaly {
            key: key.into(),
            ..failed_image(key)
        });
    }
    let open = hub.open();
    assert_eq!(open.len(), MAX_OPEN);
    assert_eq!(open[0].key, "k1");
}
