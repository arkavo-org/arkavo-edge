//! Shared fixtures for the asset tests, and the reply path (ASSET-008).

use std::sync::Arc;

use arkavo_image_gen::testing::generated;
use arkavo_image_gen::{AssetJob, ImageGenError, ImageRequest, Progress};
use arkavo_protocol::types::{Message, MessagePart, TaskStatus};
use arkavo_tasks::task_executor::{TaskExecutor, TaskExecutorConfig};
use arkavo_tasks::task_store::{SqliteTaskStore, TaskStore};
use arkavo_tdf_iroh::{FetchPolicy, IrohNode, IrohNodeConfig, IrohTransport};
use arkavo_test_macros::spec;

use super::{AssetService, Assets, Delivery, deliveries_in, take};

pub(super) const QUICK: FetchPolicy = FetchPolicy {
    attempt_timeout: std::time::Duration::from_secs(5),
    backoff: std::time::Duration::from_millis(10),
};

pub(super) async fn transport() -> IrohTransport {
    let node = Arc::new(IrohNode::new(IrohNodeConfig::default()));
    node.start().await.expect("node starts");
    IrohTransport::new(node)
}

pub(super) async fn executor() -> (Arc<TaskExecutor>, Arc<dyn TaskStore>) {
    let store: Arc<dyn TaskStore> = Arc::new(
        SqliteTaskStore::new_in_memory()
            .await
            .expect("in-memory task store"),
    );
    let executor = Arc::new(TaskExecutor::new(
        store.clone(),
        TaskExecutorConfig::default(),
    ));
    (executor, store)
}

pub(super) async fn open_task(executor: &TaskExecutor) -> uuid::Uuid {
    executor
        .submit_task(Message {
            parts: vec![MessagePart::Text {
                content: "Make me a coin".to_string(),
            }],
            metadata: None,
        })
        .await
        .expect("submit")
}

/// Fetch every asset of `delivery` as the requester would, failing the test
/// on any asset that does not verify.
pub(super) async fn take_delivery(delivery: &Delivery) -> Vec<Vec<u8>> {
    let requester = transport().await;
    let dir = tempfile::tempdir().unwrap();
    let fetched = take(&requester, delivery, dir.path(), QUICK).await;
    assert!(fetched.failed.is_empty(), "{:?}", fetched.failed);
    let bytes = fetched
        .saved
        .iter()
        .map(|(_, path)| std::fs::read(path).unwrap())
        .collect();
    requester.shutdown().await.unwrap();
    bytes
}

fn request(prompt: &str) -> ImageRequest {
    ImageRequest {
        prompt: prompt.into(),
        ..ImageRequest::default()
    }
}

/// A job that reports one step of progress, then yields `names` as PNGs.
fn job(names: &'static [&'static str]) -> AssetJob {
    AssetJob::spawn(request("a gold coin"), move |tx| async move {
        tx.send_replace(Some(Progress::Step {
            image: 1,
            images: 1,
            step: 4,
            steps: 8,
        }));
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let dir = std::env::temp_dir().join(format!("arkavo-assets-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Ok(generated(&dir, names))
    })
}

/// ASSET-008: the reply waits for its generation, shows its progress on the
/// task, and lists references after the text; the requester fetches the
/// same bytes over the data plane.
#[spec("ASSET-008")]
#[tokio::test]
async fn replies_list_references_after_their_text() {
    let provider = transport().await;
    let service = Arc::new(AssetService::new(None, None, provider.clone()));
    let (executor, store) = executor().await;
    let task_id = open_task(&executor).await;

    let assets = Assets::of(&service, vec![job(&["coin_000.png", "coin_001.png"])]);
    let parts = assets.parts(&executor, &task_id).await;

    let task = store.get_task(&task_id).await.unwrap().unwrap();
    let progress = task
        .progress
        .and_then(|p| p.message)
        .expect("progress was reported");
    assert!(progress.contains("step 4 of 8"), "{progress}");
    let delivered = deliveries_in(&parts);
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].references.len(), 2);
    let serialized = serde_json::to_string(&parts).unwrap();
    assert!(serialized.len() < 64 * 1024);
    let files = take_delivery(&delivered[0]).await;
    assert!(files.iter().all(|f| f.starts_with(b"\x89PNG")));
    provider.shutdown().await.unwrap();
}

/// ASSET-008, SRV-016: a generation that fails is reported, not delivered,
/// and brought to the operator.
#[spec("ASSET-008", "SRV-016")]
#[tokio::test]
async fn a_failed_generation_is_reported_in_text() {
    let provider = transport().await;
    let service = Arc::new(AssetService::new(None, None, provider.clone()));
    let (executor, _) = executor().await;
    let task_id = open_task(&executor).await;
    let failing = AssetJob::spawn(request("a silver coin"), |_| async move {
        Err(ImageGenError::Busy { depth: 4 })
    });

    let parts = Assets::of(&service, vec![failing])
        .parts(&executor, &task_id)
        .await;

    assert!(deliveries_in(&parts).is_empty());
    let MessagePart::Text { content } = &parts[0] else {
        panic!("{parts:?}");
    };
    assert!(
        content.contains("a silver coin") && content.contains("busy"),
        "{content}"
    );
    // SRV-016: the operator hears of it too, pointed at this agent.
    let notices = crate::server::operator_notices::OperatorNotices::global().open();
    let failed = notices
        .iter()
        .find(|n| n.key == "image-generation-failed")
        .expect("a notice for the failed image");
    assert!(failed.text.contains("queue is full"), "{}", failed.text);
    assert_eq!(failed.details[0].value, "busy");
    assert_eq!(
        failed.focus.as_ref().map(|f| f.view.as_str()),
        Some("agents")
    );
    provider.shutdown().await.unwrap();
}

/// A reply without generations is the reply as it was.
#[tokio::test]
async fn no_generations_add_nothing() {
    let provider = transport().await;
    let service = Arc::new(AssetService::new(None, None, provider.clone()));
    let (executor, store) = executor().await;
    let task_id = open_task(&executor).await;

    let none = Assets::of(&service, Vec::new());
    assert!(none.is_empty());
    assert_eq!(none, Assets::default());
    assert!(none.parts(&executor, &task_id).await.is_empty());
    let task = store.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Submitted);
    assert!(service.descriptor().is_none());
    provider.shutdown().await.unwrap();
}

/// ASSET-010: an agent whose role withholds image generation builds no
/// generator, reports no capability and answers no asset request, whatever
/// its host; it keeps generate_image, which asks members.
#[spec("ASSET-010")]
#[tokio::test]
async fn a_withheld_role_only_asks_members() {
    let plane = transport().await;
    let service = AssetService::for_agent(
        Some(plane.node().clone()),
        Some(Arc::new(arkavo_mcp_mesh::MeshToolsState::new())),
        super::Identity {
            agent_id: "writer".into(),
            did: None,
        },
        0,
        super::Hosting::Withheld,
    )
    .await
    .expect("a data plane gives a service");

    assert!(service.descriptor().is_none());
    let mut registry = arkavo_mcp_tools::ToolRegistry::empty();
    service.register_tools(&mut registry, Arc::new(arkavo_image_gen::Outbox::default()));
    assert!(registry.get("generate_image").is_some());
    plane.shutdown().await.unwrap();
}
