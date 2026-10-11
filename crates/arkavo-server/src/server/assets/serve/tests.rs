use std::sync::Arc;
use std::time::Duration;

use arkavo_image_gen::testing::fake_generator;
use arkavo_image_gen::{ASSET_REQUEST_SCHEMA, AssetRequest, ImageRequest};
use arkavo_protocol::types::{Message, MessagePart, TaskStatus};
use arkavo_tasks::task_store::TaskStore;
use arkavo_test_macros::spec;

use super::super::deliveries_in;
use super::super::tests::{executor, take_delivery, transport};
use super::{AssetService, accept_request, request_in};

fn coin(count: u32) -> ImageRequest {
    ImageRequest {
        prompt: "a gold coin".into(),
        width: Some(256),
        height: Some(256),
        count: Some(count),
        seed: Some(7),
        name: Some("coin".into()),
        transparent: Some(true),
        ..ImageRequest::default()
    }
}

fn asking(items: Vec<ImageRequest>) -> (Message, AssetRequest) {
    let request = AssetRequest {
        items,
        deadline_secs: 120,
        requester: Some("did:key:z6Mkrequester".into()),
    };
    let message = Message {
        parts: vec![MessagePart::Data {
            schema: ASSET_REQUEST_SCHEMA.to_string(),
            content: serde_json::to_value(&request).unwrap(),
        }],
        metadata: None,
    };
    (message, request)
}

/// The task once it leaves Submitted and Working.
async fn finished(store: &Arc<dyn TaskStore>, task_id: &str) -> arkavo_tasks::types::Task {
    let id = uuid::Uuid::parse_str(task_id).unwrap();
    for _ in 0..600 {
        let task = store.get_task(&id).await.unwrap().unwrap();
        if !matches!(task.status, TaskStatus::Submitted | TaskStatus::Working) {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("task {task_id} did not finish");
}

/// ASSET-003: a host that does not generate fails the task with
/// IMAGE_GENERATION_UNAVAILABLE and gives it to no model.
#[spec("ASSET-003")]
#[tokio::test]
async fn a_host_that_cannot_generate_fails_the_task() {
    let provider = transport().await;
    let service = Arc::new(AssetService::new(None, None, provider.clone()));
    let (executor, store) = executor().await;
    let (message, request) = asking(vec![coin(1)]);
    assert!(request_in(&message).unwrap().is_ok());

    let accepted = accept_request(Some(&service), None, &executor, message, request)
        .await
        .unwrap();

    let task = finished(&store, &accepted.task_id).await;
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(task.error.unwrap().code, "IMAGE_GENERATION_UNAVAILABLE");
    provider.shutdown().await.unwrap();
}

/// ASSET-002 edge: more than 16 images is refused before a task is opened.
#[spec("ASSET-002")]
#[tokio::test]
async fn an_oversized_request_is_refused_before_any_task() {
    let (executor, store) = executor().await;
    let (message, request) = asking((0..5).map(|_| coin(4)).collect());

    let refused = accept_request(None, None, &executor, message, request)
        .await
        .unwrap_err();

    assert!(
        refused.message().starts_with("invalid_params"),
        "{refused:?}"
    );
    assert!(store.list_tasks(None).await.unwrap().is_empty());
}

/// A data part that is not an asset request is reported as such.
#[test]
fn malformed_requests_are_named() {
    let message = Message {
        parts: vec![MessagePart::Data {
            schema: ASSET_REQUEST_SCHEMA.to_string(),
            content: serde_json::json!({"items": "a coin"}),
        }],
        metadata: None,
    };
    assert!(request_in(&message).unwrap().is_err());
    let plain = Message {
        parts: vec![MessagePart::Text {
            content: "hi".into(),
        }],
        metadata: None,
    };
    assert!(request_in(&plain).is_none());
}

/// ASSET-003, ASSET-005: a capable host generates without its model,
/// reports progress, and completes the task with a summary and references
/// the requester fetches over the data plane.
#[spec("ASSET-003", "ASSET-005")]
#[tokio::test]
async fn a_capable_host_answers_with_references() {
    let fake = fake_generator(1, Duration::ZERO).await;
    let provider = transport().await;
    let service = Arc::new(AssetService::new(
        Some(fake.generator.clone()),
        None,
        provider.clone(),
    ));
    let (executor, store) = executor().await;
    let (message, request) = asking(vec![coin(2)]);

    let accepted = accept_request(Some(&service), None, &executor, message, request)
        .await
        .unwrap();
    let task = finished(&store, &accepted.task_id).await;

    assert_eq!(task.status, TaskStatus::Completed, "{:?}", task.error);
    let result: Message = serde_json::from_value(task.result.unwrap()).unwrap();
    let MessagePart::Text { content } = &result.parts[0] else {
        panic!("{result:?}");
    };
    assert!(
        content.contains("coin_000.png (256x256, alpha)"),
        "{content}"
    );
    assert!(content.contains("Qwen Research License"), "{content}");
    let delivered = deliveries_in(&result.parts);
    assert_eq!(delivered[0].manifest.assets.len(), 2);
    assert_eq!(
        delivered[0].manifest.producer.as_deref(),
        Some("did:key:z6Mtest")
    );
    assert_eq!(take_delivery(&delivered[0]).await.len(), 2);
    assert!(service.descriptor().unwrap().models[0].ready);
    provider.shutdown().await.unwrap();
}

/// ASSET-002: a request this host cannot take is refused before a task is
/// opened, so the requester can ask another member.
#[spec("ASSET-002")]
#[tokio::test]
async fn an_entry_this_host_does_not_offer_is_refused_up_front() {
    let fake = fake_generator(0, Duration::ZERO).await;
    let provider = transport().await;
    let service = Arc::new(AssetService::new(
        Some(fake.generator.clone()),
        None,
        provider.clone(),
    ));
    let (executor, store) = executor().await;
    let quality = ImageRequest {
        model: Some("qwen-image-quality".into()),
        ..coin(1)
    };
    let (message, request) = asking(vec![quality]);

    let refused = accept_request(Some(&service), None, &executor, message, request)
        .await
        .unwrap_err();

    assert!(
        refused.message().starts_with("no_capable_member"),
        "{refused:?}"
    );
    assert!(store.list_tasks(None).await.unwrap().is_empty());
    provider.shutdown().await.unwrap();
}
