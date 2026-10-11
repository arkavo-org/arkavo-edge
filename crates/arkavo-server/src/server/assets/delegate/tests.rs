//! The delegate against members on loopback: JSON-RPC servers that answer
//! with this crate's own asset handlers, or refuse, or go quiet.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use arkavo_image_gen::Progress;
use arkavo_image_gen::testing::fake_generator;
use arkavo_image_gen::tool::Delegate;
use arkavo_mcp_mesh::MeshToolsState;
use arkavo_protocol::metrics::MetricsCollector;
use arkavo_protocol::rate_limit::{RateLimitConfig, RateLimiter};
use arkavo_protocol::types::{
    AgentCapabilitiesGetResponse, MessageSendRequest, MessageSendResponse, TaskGetRequest,
    TaskGetResponse, TaskStatus,
};
use arkavo_tdf_iroh::{IrohTicket, IrohTransport};
use arkavo_test_macros::spec;
use jsonrpsee::RpcModule;
use jsonrpsee::server::{Server, ServerHandle};
use jsonrpsee::types::ErrorObjectOwned;
use serde_json::json;
use tokio::time::Instant;

use super::super::serve::{accept_request, request_in};
use super::super::tests::{QUICK, deliveries, executor, open_task, take_delivery, transport};
use super::super::{AssetService, Assets, Identity};
use super::MeshDelegate;

const QUIET_TASK: &str = "0d6f1a8e-3b7c-4e0f-9a51-6c2d8b7e4f10";

/// How a scripted member answers message/send.
enum Answer {
    /// With this crate's asset handlers and generator.
    Serve(Arc<AssetService>),
    /// Refuses as a full queue does.
    Busy,
    /// Accepts, then never moves.
    Quiet,
}

struct Member {
    handle: ServerHandle,
}

impl Member {
    async fn start(
        mesh: &MeshToolsState,
        agent_id: &str,
        descriptor: Option<serde_json::Value>,
        answer: Answer,
    ) -> Self {
        let (executor, store) = executor().await;
        let context = Arc::new((answer, executor, store, descriptor, agent_id.to_string()));
        let mut module = RpcModule::new(context);
        module
            .register_method("agent.capabilities.get", |_, c, _| {
                let mut report: AgentCapabilitiesGetResponse = serde_json::from_value(json!({
                    "agent_id": c.4, "name": c.4, "purpose": "", "model": "gemma-4-12b",
                    "capabilities": [], "mcp_tools": [], "load": 0.0,
                    "accepting_tasks": true, "public_key": "", "version": "1.0"
                }))
                .expect("a report");
                report.image_generation.clone_from(&c.3);
                Ok::<_, ErrorObjectOwned>(report)
            })
            .unwrap();
        module
            .register_async_method("message/send", |params, c, _| async move {
                let request: MessageSendRequest = params.one()?;
                match &c.0 {
                    Answer::Serve(service) => {
                        let asked = request_in(&request.message)
                            .expect("an asset request")
                            .expect("well formed");
                        accept_request(Some(service), None, &c.1, request.message, asked).await
                    }
                    Answer::Busy => Err(ErrorObjectOwned::owned(
                        -32010,
                        "busy: image generation queue is full (4 waiting)",
                        None::<()>,
                    )),
                    Answer::Quiet => Ok(MessageSendResponse {
                        task_id: QUIET_TASK.to_string(),
                        status: TaskStatus::Submitted,
                        response: None,
                    }),
                }
            })
            .unwrap();
        module
            .register_async_method("tasks/get", |params, c, _| async move {
                let request: TaskGetRequest = params.one()?;
                if matches!(c.0, Answer::Quiet) {
                    return Ok(TaskGetResponse {
                        task_id: request.task_id,
                        status: TaskStatus::Working,
                        result: None,
                        error: None,
                        progress: None,
                    });
                }
                crate::server::handlers::tasks::handle_tasks_get(
                    &Arc::new(MetricsCollector::new(false)),
                    &RateLimiter::new(RateLimitConfig::default()),
                    &c.2,
                    request,
                )
                .await
            })
            .unwrap();
        let server = Server::builder().build("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", server.local_addr().unwrap());
        mesh.agent_addresses
            .write()
            .await
            .insert(agent_id.to_string(), address);
        Self {
            handle: server.start(module),
        }
    }

    async fn stop(self) {
        self.handle.stop().unwrap();
        self.handle.stopped().await;
    }
}

fn coin() -> arkavo_image_gen::ImageRequest {
    arkavo_image_gen::ImageRequest {
        prompt: "a gold coin".into(),
        width: Some(256),
        height: Some(256),
        seed: Some(7),
        name: Some("coin".into()),
        transparent: Some(true),
        ..arkavo_image_gen::ImageRequest::default()
    }
}

fn commander(mesh: Arc<MeshToolsState>, transport: IrohTransport, stall: Duration) -> MeshDelegate {
    let identity = Identity {
        agent_id: "commander".into(),
        did: Some("did:key:z6Mkcommander".into()),
    };
    MeshDelegate::new(mesh, identity, transport).with_patience(QUICK, stall)
}

/// ASSET-002, ASSET-006, ASSET-009: a member that refuses is passed over;
/// the one that accepts generates, its progress is relayed, and its assets
/// come back verified. Re-staged for the requester, they carry this agent's
/// tickets, not the member's, and the member as producer.
#[spec("ASSET-002", "ASSET-006", "ASSET-009")]
#[tokio::test]
async fn work_goes_to_a_capable_member_and_comes_back_restaged() {
    // Slow weights keep the member fetching across several polls, so its
    // progress is seen and relayed.
    let fake = fake_generator(1, Duration::from_millis(1500)).await;
    let descriptor = serde_json::to_value(fake.generator.descriptor()).unwrap();
    let mut busy_descriptor = descriptor.clone();
    // Ready and otherwise equal, so it ranks first and is asked first.
    busy_descriptor["models"][0]["ready"] = json!(true);
    let member_plane = transport().await;
    let mesh = Arc::new(MeshToolsState::new());
    let artist = Member::start(
        &mesh,
        "artist",
        Some(descriptor),
        Answer::Serve(Arc::new(AssetService::new(
            Some(fake.generator.clone()),
            None,
            member_plane.clone(),
        ))),
    )
    .await;
    let busy = Member::start(&mesh, "busy", Some(busy_descriptor), Answer::Busy).await;
    let plain = Member::start(&mesh, "writer", None, Answer::Busy).await;

    let own_plane = transport().await;
    let delegate = Arc::new(commander(
        mesh.clone(),
        own_plane.clone(),
        Duration::from_mins(5),
    ));
    let relayed = Arc::new(Mutex::new(Vec::new()));
    let seen = relayed.clone();
    let delegated = arkavo_image_gen::AssetJob::spawn(coin(), move |tx| async move {
        let report = |p: Progress| {
            seen.lock().unwrap().push(p.clone());
            tx.send_replace(Some(p));
        };
        delegate
            .generate(&coin(), Instant::now() + Duration::from_secs(120), &report)
            .await
    });

    let service = Arc::new(AssetService::new(None, None, own_plane.clone()));
    let (executor, _) = executor().await;
    let task_id = open_task(&executor).await;
    let parts = Assets::of(&service, vec![delegated])
        .parts(&executor, &task_id)
        .await;

    let delivered = deliveries(&parts);
    assert_eq!(delivered.len(), 1, "{parts:?}");
    let delivery = &delivered[0];
    assert_eq!(
        delivery.manifest.producer.as_deref(),
        Some("did:key:z6Mtest")
    );
    let own_id = own_plane.node().endpoint_id().await.unwrap();
    for reference in &delivery.references {
        let ticket = IrohTicket::from_string(&reference.ticket).unwrap();
        assert_eq!(ticket.inner().addr().id, own_id, "re-staged here");
    }
    let files = take_delivery(delivery).await;
    assert!(files[0].starts_with(b"\x89PNG"));
    assert!(
        relayed
            .lock()
            .unwrap()
            .iter()
            .any(|p| matches!(p, Progress::Relayed { member, .. } if member == "artist")),
        "the member's progress was relayed"
    );

    for member in [artist, busy, plain] {
        member.stop().await;
    }
    own_plane.shutdown().await.unwrap();
    member_plane.shutdown().await.unwrap();
}

/// ASSET-002: with no member reporting the capability, the work fails with
/// no_capable_member; a member's name or purpose is never enough.
#[spec("ASSET-002", "IMG-015")]
#[tokio::test]
async fn without_a_capable_member_the_work_fails() {
    let mesh = Arc::new(MeshToolsState::new());
    let writer = Member::start(&mesh, "image-artist", None, Answer::Busy).await;
    let own_plane = transport().await;
    let delegate = commander(mesh, own_plane.clone(), Duration::from_mins(5));

    let error = delegate
        .generate(&coin(), Instant::now() + Duration::from_secs(30), &|_| {})
        .await
        .unwrap_err();

    assert_eq!(error.code(), "no_capable_member", "{error}");
    writer.stop().await;
    own_plane.shutdown().await.unwrap();
}

/// ASSET-009: a member whose task stops moving is reported as stalled.
#[spec("ASSET-009")]
#[tokio::test]
async fn a_member_that_stops_moving_is_stalled() {
    let fake = fake_generator(0, Duration::ZERO).await;
    let descriptor = serde_json::to_value(fake.generator.descriptor()).unwrap();
    let mesh = Arc::new(MeshToolsState::new());
    let quiet = Member::start(&mesh, "quiet", Some(descriptor), Answer::Quiet).await;
    let own_plane = transport().await;
    let delegate = commander(mesh, own_plane.clone(), Duration::from_secs(2));

    let error = delegate
        .generate(&coin(), Instant::now() + Duration::from_secs(60), &|_| {})
        .await
        .unwrap_err();

    assert_eq!(error.code(), "step_stalled", "{error}");
    assert!(error.to_string().contains("quiet"), "{error}");
    quiet.stop().await;
    own_plane.shutdown().await.unwrap();
}
