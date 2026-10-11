//! The generate_image tool every agent offers (IMG-014).

use std::sync::Arc;
use std::time::Duration;

use arkavo_mcp::ToolSchema;
use arkavo_mcp_tools::{Tool, ToolError, ToolRegistry};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::catalog::{self, ALIASES, entries};
use crate::error::{ImageGenError, Result};
use crate::generator::Generator;
use crate::job::{AssetJob, AssetSink};
use crate::output::Generated;
use crate::progress::Progress;
use crate::request::ImageRequest;

/// How long one requested generation may take, weight fetch included.
pub const JOB_DEADLINE: Duration = Duration::from_hours(1);

/// How long the tool call waits before answering that the job continues.
pub const TOOL_WAIT: Duration = Duration::from_secs(90);

/// Generation on another member, returning its assets as local files.
#[async_trait]
pub trait Delegate: Send + Sync {
    async fn generate(
        &self,
        request: &ImageRequest,
        deadline: Instant,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<Generated>;
}

pub struct GenerateImageTool {
    local: Option<Arc<Generator>>,
    delegate: Option<Arc<dyn Delegate>>,
    sink: Arc<dyn AssetSink>,
    schema: ToolSchema,
}

impl GenerateImageTool {
    pub fn new(
        local: Option<Arc<Generator>>,
        delegate: Option<Arc<dyn Delegate>>,
        sink: Arc<dyn AssetSink>,
    ) -> Self {
        Self {
            local,
            delegate,
            sink,
            schema: schema(),
        }
    }
}

/// Register generate_image with the generator this host runs, if any, and
/// the members it can delegate to.
pub fn register_tools(
    registry: &mut ToolRegistry,
    local: Option<Arc<Generator>>,
    delegate: Option<Arc<dyn Delegate>>,
    sink: Arc<dyn AssetSink>,
) {
    registry.register(
        "generate_image",
        Box::new(GenerateImageTool::new(local, delegate, sink)),
    );
}

fn schema() -> ToolSchema {
    let models: Vec<String> = entries()
        .iter()
        .map(|e| format!("{}: {}", e.id, e.summary))
        .chain(
            ALIASES
                .iter()
                .map(|(alias, id)| format!("{alias}: alias of {id}")),
        )
        .collect();
    ToolSchema {
        name: "generate_image".to_string(),
        aliases: None,
        description: format!(
            "Generate images (game assets, sprites, textures, concept art) with a local Qwen-Image model on this \
             swarm. No cloud service is used. The files go to the requester with your reply; you get their names, \
             sizes and seeds. Call once per distinct asset. Models: {}.",
            models.join("; ")
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "What to draw, 1 to 2000 characters"},
                "model": {"type": "string", "enum": catalog::names(),
                          "description": format!("Default {}", catalog::DEFAULT_MODEL)},
                "width": {"type": "integer", "description": "Multiple of 32 from 256 to 2752; default per model"},
                "height": {"type": "integer", "description": "Multiple of 32 from 256 to 2752; default per model"},
                "count": {"type": "integer", "description": "Images to generate, 1 to 4"},
                "seed": {"type": "integer", "description": "Seed for reproducible output"},
                "name": {"type": "string", "description": "File stem: letters, digits, _ and -"},
                "transparent": {"type": "boolean", "description": "Transparent background, for sprites and icons"},
                "tileable": {"type": "boolean", "description": "Seamlessly tiling texture"},
                "guidance": {"type": "number", "description": "qwen-image-2.1 only: CFG scale above 1, with negative_prompt"},
                "negative_prompt": {"type": "string", "description": "qwen-image-2.1 only, with guidance above 1"}
            },
            "required": ["prompt"]
        }),
    }
}

impl GenerateImageTool {
    /// Generate here when this host offers the entry, else on a member; a
    /// local refusal that another member could serve goes to the delegate.
    fn start(&self, request: ImageRequest) -> AssetJob {
        let local = self.local.clone();
        let delegate = self.delegate.clone();
        AssetJob::spawn(request.clone(), move |tx| async move {
            let deadline = Instant::now() + JOB_DEADLINE;
            let report = |p: Progress| {
                tx.send_replace(Some(p));
            };
            let job = request.validate()?;
            let here = local.filter(|g| g.offer(job.entry.id).is_some());
            if let Some(generator) = here {
                match generator.generate(&request, deadline, &report).await {
                    Err(e) if e.try_elsewhere() && delegate.is_some() => {}
                    outcome => return outcome,
                }
            }
            match delegate {
                Some(delegate) => delegate.generate(&request, deadline, &report).await,
                None => Err(ImageGenError::NoCapableMember {
                    what: job.entry.id.to_string(),
                }),
            }
        })
    }
}

#[async_trait]
impl Tool for GenerateImageTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, params: Value) -> arkavo_mcp_tools::Result<Value> {
        let request: ImageRequest =
            serde_json::from_value(params).map_err(|e| ToolError::InvalidParams(e.to_string()))?;
        if let Err(e) = request.validate() {
            return Ok(failure(&e));
        }
        let job = self.start(request);
        self.sink.accept(job.clone());
        Ok(match job.wait_until(Instant::now() + TOOL_WAIT).await {
            Some(Ok(generated)) => done(&generated),
            Some(Err(e)) => failure(&e),
            None => json!({
                "success": true,
                "status": "generating",
                "progress": job.progress().map(|p| p.to_string()),
                "note": "Still generating; the files will be delivered to the requester with your reply."
            }),
        })
    }
}

fn done(generated: &Generated) -> Value {
    let m = &generated.manifest;
    json!({
        "success": true,
        "status": "done",
        "model": m.model.id,
        "license": m.license_line(),
        "transparency_missing": m.transparency_missing,
        "assets": m.assets.iter().map(|a| json!({
            "name": a.name,
            "width": a.width,
            "height": a.height,
            "has_alpha": a.has_alpha,
            "tileable": a.tileable,
            "seed": a.seed,
            "bytes": a.bytes,
        })).collect::<Vec<_>>(),
        "note": "The files are delivered to the requester with your reply."
    })
}

fn failure(error: &ImageGenError) -> Value {
    json!({
        "success": false,
        "code": error.code(),
        "error": error.to_string(),
    })
}

#[cfg(test)]
mod tests;
