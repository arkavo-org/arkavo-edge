// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use super::*;
use crate::job::Outbox;
use crate::output::{AssetEntry, Manifest};
use std::sync::Mutex;

struct FakeDelegate {
    calls: Mutex<Vec<ImageRequest>>,
    fail: bool,
}

#[async_trait]
impl Delegate for FakeDelegate {
    async fn generate(
        &self,
        request: &ImageRequest,
        _deadline: Instant,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<Generated> {
        self.calls.lock().unwrap().push(request.clone());
        progress(Progress::Queued);
        if self.fail {
            return Err(ImageGenError::Delegation("member unreachable".into()));
        }
        let manifest: Manifest = serde_json::from_value(json!({
            "model": {"id": "qwen-image-2.1-turbo", "repo": "Qwen/Qwen-Image-2.1-Turbo", "revision": "d",
                      "diffusion": "Q8_0", "text_encoder": "Q8_0"},
            "license": "Qwen Research License Agreement", "license_url": "u",
            "runtime": "stable-diffusion.cpp master-956-1b0ba10", "prompt": request.prompt, "assets": []
        }))
        .unwrap();
        let mut manifest = manifest;
        manifest.assets.push(AssetEntry {
            name: "coin_000.png".into(),
            bytes: 10,
            sha256: "0".repeat(64),
            width: 1024,
            height: 1024,
            has_alpha: true,
            tileable: false,
            seed: 3,
        });
        Ok(Generated::owning(
            std::env::temp_dir().join(format!("arkavo-tool-{:016x}", rand::random::<u64>())),
            manifest,
        ))
    }
}

fn delegate(fail: bool) -> Arc<FakeDelegate> {
    Arc::new(FakeDelegate {
        calls: Mutex::new(Vec::new()),
        fail,
    })
}

/// IMG-014: a host that generates nothing hands the work to a member, and
/// the model sees names and sizes, never bytes.
#[arkavo_test_macros::spec("IMG-014", "ASSET-002")]
#[tokio::test]
async fn incapable_hosts_delegate() {
    let outbox = Arc::new(Outbox::default());
    let member = delegate(false);
    let tool = GenerateImageTool::new(None, Some(member.clone()), outbox.clone());
    let result = tool
        .execute(json!({"prompt": "gold coin", "transparent": true}))
        .await
        .unwrap();
    assert_eq!(result["success"], true);
    assert_eq!(result["status"], "done");
    assert_eq!(result["assets"][0]["name"], "coin_000.png");
    assert!(
        result["license"]
            .as_str()
            .unwrap()
            .contains("Qwen Research License")
    );
    assert!(!result.to_string().contains("base64"));
    assert_eq!(member.calls.lock().unwrap().len(), 1);
    assert_eq!(outbox.take().len(), 1);
}

/// IMG-015: with neither a local generator nor a member, nothing is
/// generated anywhere.
#[arkavo_test_macros::spec("IMG-015")]
#[tokio::test]
async fn no_capable_member_is_a_result_the_model_sees() {
    let outbox = Arc::new(Outbox::default());
    let tool = GenerateImageTool::new(None, None, outbox);
    let result = tool.execute(json!({"prompt": "coin"})).await.unwrap();
    assert_eq!(result["success"], false);
    assert_eq!(result["code"], "no_capable_member");
}

#[arkavo_test_macros::spec("IMG-007")]
#[tokio::test]
async fn invalid_requests_are_refused_before_starting() {
    let outbox = Arc::new(Outbox::default());
    let member = delegate(false);
    let tool = GenerateImageTool::new(None, Some(member.clone()), outbox.clone());
    let result = tool
        .execute(json!({"prompt": "coin", "negative_prompt": "blur"}))
        .await
        .unwrap();
    assert_eq!(result["code"], "invalid_params");
    assert!(member.calls.lock().unwrap().is_empty());
    assert!(outbox.take().is_empty());
    let err = tool.execute(json!({"prompt": 3})).await.unwrap_err();
    assert!(matches!(err, ToolError::InvalidParams(_)));
}

#[arkavo_test_macros::spec("IMG-014")]
#[tokio::test]
async fn delegate_failures_reach_the_model() {
    let outbox = Arc::new(Outbox::default());
    let tool = GenerateImageTool::new(None, Some(delegate(true)), outbox);
    let result = tool.execute(json!({"prompt": "coin"})).await.unwrap();
    assert_eq!(result["success"], false);
    assert_eq!(result["code"], "delegation_failed");
}

#[arkavo_test_macros::spec("IMG-014")]
#[test]
fn the_schema_lists_every_model() {
    let schema = schema();
    assert_eq!(schema.name, "generate_image");
    let names = schema.parameters["properties"]["model"]["enum"]
        .as_array()
        .unwrap();
    assert_eq!(names.len(), 4);
    assert!(
        schema
            .description
            .contains("qwen-image-2.1: Qwen-Image-2.1: 40 steps")
    );
    assert!(schema.description.contains("No cloud service"));
    assert_eq!(schema.parameters["required"], json!(["prompt"]));
}

#[arkavo_test_macros::spec("IMG-014")]
#[test]
fn registration_adds_generate_image() {
    let mut registry = ToolRegistry::empty();
    register_tools(&mut registry, None, None, Arc::new(Outbox::default()));
    assert!(registry.get("generate_image").is_some());
}
