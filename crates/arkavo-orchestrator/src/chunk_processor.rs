//! RLM Chunk Processor for distributed context processing across agent mesh.
//!
//! Implements parallel chunk processing by distributing context chunks
//! across available agents in the mesh, collecting results via async channels.

use arkavo_context::ContextManifest;
use arkavo_protocol::agent_registry::{AgentInfo, AgentRegistry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, instrument, warn};

/// Configuration for chunk processing.
#[derive(Debug, Clone)]
pub struct ChunkProcessorConfig {
    /// Maximum number of agents to use for processing.
    pub max_agents: usize,
    /// Maximum chunks per agent batch.
    pub max_chunks_per_batch: usize,
    /// Timeout per chunk in milliseconds.
    pub chunk_timeout_ms: u64,
    /// Required capability for chunk processing agents.
    pub required_capability: String,
}

impl Default for ChunkProcessorConfig {
    fn default() -> Self {
        Self {
            max_agents: 8,
            max_chunks_per_batch: 5,
            chunk_timeout_ms: 30_000,
            required_capability: "chunk_processing".to_string(),
        }
    }
}

/// A batch of chunks assigned to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkBatch {
    /// Agent ID processing this batch.
    pub agent_id: String,
    /// Chunk indices in this batch.
    pub chunk_indices: Vec<usize>,
    /// The query/prompt to apply to each chunk.
    pub query: String,
    /// Manifest ID for chunk retrieval.
    pub manifest_id: String,
}

/// Result from processing a single chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkResult {
    /// Index of the processed chunk.
    pub chunk_index: usize,
    /// Agent that processed this chunk.
    pub agent_id: String,
    /// Processing result/output.
    pub output: String,
    /// Tokens used for this chunk.
    pub tokens_used: usize,
    /// Processing time in milliseconds.
    pub processing_time_ms: u64,
    /// Whether processing succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

/// Aggregated result from all chunk processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingResult {
    /// Manifest ID that was processed.
    pub manifest_id: String,
    /// Total chunks processed.
    pub chunks_processed: usize,
    /// Successful chunk count.
    pub successful_chunks: usize,
    /// Failed chunk count.
    pub failed_chunks: usize,
    /// Total tokens used across all agents.
    pub total_tokens: usize,
    /// Total processing time in milliseconds.
    pub total_time_ms: u64,
    /// Per-agent cost breakdown.
    pub agent_costs: HashMap<String, AgentCost>,
    /// Individual chunk results.
    pub chunk_results: Vec<ChunkResult>,
    /// Aggregated output (combined from all chunks).
    pub aggregated_output: String,
}

/// Cost tracking per agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCost {
    /// Agent ID.
    pub agent_id: String,
    /// Chunks processed by this agent.
    pub chunks_processed: usize,
    /// Tokens used by this agent.
    pub tokens_used: usize,
    /// Processing time by this agent.
    pub time_ms: u64,
}

/// Manages parallel chunk processing across agent mesh.
pub struct ChunkProcessor {
    registry: Arc<AgentRegistry>,
    config: ChunkProcessorConfig,
    active_batches: RwLock<HashMap<String, Vec<ChunkBatch>>>,
}

impl ChunkProcessor {
    /// Create a new chunk processor.
    pub fn new(registry: Arc<AgentRegistry>, config: ChunkProcessorConfig) -> Self {
        Self {
            registry,
            config,
            active_batches: RwLock::new(HashMap::new()),
        }
    }

    /// Create with default configuration.
    pub fn with_registry(registry: Arc<AgentRegistry>) -> Self {
        Self::new(registry, ChunkProcessorConfig::default())
    }

    /// Find available agents for chunk processing.
    #[instrument(skip(self))]
    pub async fn find_available_agents(&self) -> Vec<AgentInfo> {
        let all_agents = self.registry.get_all_agents().await;

        let mut available: Vec<AgentInfo> = all_agents
            .into_iter()
            .filter(|agent| {
                agent.is_available
                    && agent
                        .capabilities
                        .iter()
                        .any(|c| c == &self.config.required_capability || c == "llm")
            })
            .collect();

        // Sort by load (ascending) for load balancing
        available.sort_by(|a, b| {
            a.load
                .partial_cmp(&b.load)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Limit to max agents
        available.truncate(self.config.max_agents);

        debug!(
            available_count = available.len(),
            max_agents = self.config.max_agents,
            "Found available agents for chunk processing"
        );

        available
    }

    /// Distribute chunks across available agents.
    fn distribute_chunks(
        &self,
        manifest: &ContextManifest,
        agents: &[AgentInfo],
        query: &str,
    ) -> Vec<ChunkBatch> {
        if agents.is_empty() {
            warn!("No agents available for chunk distribution");
            return Vec::new();
        }

        let chunk_count = manifest.chunk_count;
        let agent_count = agents.len();

        // Calculate chunks per agent (round-robin distribution)
        let base_chunks = chunk_count / agent_count;
        let extra_chunks = chunk_count % agent_count;

        let mut batches = Vec::new();
        let mut chunk_idx = 0;

        for (i, agent) in agents.iter().enumerate() {
            // Some agents get one extra chunk for even distribution
            let chunks_for_agent = base_chunks + usize::from(i < extra_chunks);

            if chunks_for_agent == 0 {
                continue;
            }

            // Split into sub-batches if too many chunks
            let mut agent_chunk_indices: Vec<usize> =
                (chunk_idx..chunk_idx + chunks_for_agent).collect();
            chunk_idx += chunks_for_agent;

            while !agent_chunk_indices.is_empty() {
                let batch_size = agent_chunk_indices
                    .len()
                    .min(self.config.max_chunks_per_batch);
                let batch_indices: Vec<usize> = agent_chunk_indices.drain(..batch_size).collect();

                batches.push(ChunkBatch {
                    agent_id: agent.agent_id.clone(),
                    chunk_indices: batch_indices,
                    query: query.to_string(),
                    manifest_id: manifest.id.clone(),
                });
            }
        }

        info!(
            manifest_id = %manifest.id,
            chunk_count,
            agent_count,
            batch_count = batches.len(),
            "Chunks distributed across agents"
        );

        batches
    }

    /// Spawn chunk processors across the agent mesh.
    ///
    /// This is the main entry point for RLM parallel processing.
    #[instrument(skip(self, manifest, process_fn), fields(manifest_id = %manifest.id))]
    pub async fn spawn_chunk_processors<F, Fut>(
        &self,
        manifest: &ContextManifest,
        query: &str,
        process_fn: F,
    ) -> ProcessingResult
    where
        F: Fn(ChunkBatch) -> Fut + Send + Sync + Clone + 'static,
        Fut: std::future::Future<Output = Vec<ChunkResult>> + Send + 'static,
    {
        let start_time = std::time::Instant::now();

        // Find available agents
        let agents = self.find_available_agents().await;

        if agents.is_empty() {
            warn!(manifest_id = %manifest.id, "No agents available for processing");
            return ProcessingResult {
                manifest_id: manifest.id.clone(),
                chunks_processed: 0,
                successful_chunks: 0,
                failed_chunks: manifest.chunk_count,
                total_tokens: 0,
                total_time_ms: 0,
                agent_costs: HashMap::new(),
                chunk_results: Vec::new(),
                aggregated_output: "Error: No agents available for chunk processing".to_string(),
            };
        }

        // Distribute chunks
        let batches = self.distribute_chunks(manifest, &agents, query);

        // Store active batches for monitoring
        {
            let mut active = self.active_batches.write().await;
            active.insert(manifest.id.clone(), batches.clone());
        }

        // Create channel for results
        let (tx, mut rx) = mpsc::channel::<Vec<ChunkResult>>(batches.len());

        // Spawn parallel tasks for each batch
        let mut handles = Vec::new();
        for batch in batches {
            let process_fn = process_fn.clone();
            let tx = tx.clone();

            let handle = tokio::spawn(async move {
                let results = process_fn(batch).await;
                if let Err(e) = tx.send(results).await {
                    error!(error = %e, "Failed to send chunk results");
                }
            });
            handles.push(handle);
        }

        // Drop the original sender so rx completes when all tasks finish
        drop(tx);

        // Collect all results
        let mut all_results = Vec::new();
        while let Some(results) = rx.recv().await {
            all_results.extend(results);
        }

        // Wait for all tasks to complete
        for handle in handles {
            if let Err(e) = handle.await {
                error!(error = %e, "Chunk processing task panicked");
            }
        }

        // Clear active batches
        {
            let mut active = self.active_batches.write().await;
            active.remove(&manifest.id);
        }

        // Aggregate results
        self.aggregate_results(
            manifest,
            all_results,
            start_time.elapsed().as_millis() as u64,
        )
    }

    /// Aggregate chunk results into final output.
    fn aggregate_results(
        &self,
        manifest: &ContextManifest,
        mut results: Vec<ChunkResult>,
        total_time_ms: u64,
    ) -> ProcessingResult {
        // Sort by chunk index for ordered output
        results.sort_by_key(|r| r.chunk_index);

        let mut agent_costs: HashMap<String, AgentCost> = HashMap::new();
        let mut total_tokens = 0;
        let mut successful_chunks = 0;
        let mut failed_chunks = 0;

        for result in &results {
            if result.success {
                successful_chunks += 1;
            } else {
                failed_chunks += 1;
            }

            total_tokens += result.tokens_used;

            // Update agent cost tracking
            let cost = agent_costs
                .entry(result.agent_id.clone())
                .or_insert_with(|| AgentCost {
                    agent_id: result.agent_id.clone(),
                    ..Default::default()
                });
            cost.chunks_processed += 1;
            cost.tokens_used += result.tokens_used;
            cost.time_ms += result.processing_time_ms;
        }

        // Combine successful outputs
        let aggregated_output: String = results
            .iter()
            .filter(|r| r.success)
            .map(|r| format!("[Chunk {}]: {}", r.chunk_index, r.output))
            .collect::<Vec<_>>()
            .join("\n\n");

        info!(
            manifest_id = %manifest.id,
            successful_chunks,
            failed_chunks,
            total_tokens,
            total_time_ms,
            agent_count = agent_costs.len(),
            "Chunk processing completed"
        );

        ProcessingResult {
            manifest_id: manifest.id.clone(),
            chunks_processed: results.len(),
            successful_chunks,
            failed_chunks,
            total_tokens,
            total_time_ms,
            agent_costs,
            chunk_results: results,
            aggregated_output,
        }
    }

    /// Get statistics about active processing.
    pub async fn get_active_stats(&self) -> HashMap<String, usize> {
        let active = self.active_batches.read().await;
        active
            .iter()
            .map(|(id, batches)| {
                let total_chunks: usize = batches.iter().map(|b| b.chunk_indices.len()).sum();
                (id.clone(), total_chunks)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_context::ChunkRef;
    use std::collections::HashMap;

    fn create_test_manifest(chunk_count: usize) -> ContextManifest {
        let chunks: Vec<ChunkRef> = (0..chunk_count)
            .map(|i| ChunkRef {
                id: format!("chunk-{i}"),
                index: i,
                token_count: 100,
                char_count: 400,
                ticket: format!("ticket-{i}"),
                preview: format!("Preview of chunk {i}"),
                hints: vec!["test".to_string()],
            })
            .collect();

        ContextManifest {
            id: "test-manifest".to_string(),
            chunks,
            total_tokens: chunk_count * 100,
            total_chars: chunk_count * 400,
            chunk_count,
            overlap_bytes: 200,
            created_at: 0,
        }
    }

    #[tokio::test]
    async fn test_find_available_agents() {
        let registry = Arc::new(AgentRegistry::new());

        // Register test agents
        registry
            .register_agent(
                "agent-1".to_string(),
                "Agent 1".to_string(),
                "Test agent".to_string(),
                vec!["llm".to_string(), "chunk_processing".to_string()],
                None,
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        registry
            .register_agent(
                "agent-2".to_string(),
                "Agent 2".to_string(),
                "Test agent".to_string(),
                vec!["llm".to_string()],
                None,
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        let processor = ChunkProcessor::with_registry(registry);
        let agents = processor.find_available_agents().await;

        assert_eq!(agents.len(), 2);
    }

    #[tokio::test]
    async fn test_distribute_chunks() {
        let registry = Arc::new(AgentRegistry::new());

        // Register 3 agents
        for i in 1..=3 {
            registry
                .register_agent(
                    format!("agent-{i}"),
                    format!("Agent {i}"),
                    "Test".to_string(),
                    vec!["llm".to_string()],
                    None,
                    HashMap::new(),
                    None,
                )
                .await
                .unwrap();
        }

        let processor = ChunkProcessor::with_registry(Arc::clone(&registry));
        let manifest = create_test_manifest(10);
        let agents = processor.find_available_agents().await;
        let batches = processor.distribute_chunks(&manifest, &agents, "test query");

        // With 10 chunks and 3 agents, expect distribution: 4, 3, 3
        let total_chunks: usize = batches.iter().map(|b| b.chunk_indices.len()).sum();
        assert_eq!(total_chunks, 10);

        // Verify all chunk indices are covered
        let mut all_indices: Vec<usize> = batches
            .iter()
            .flat_map(|b| b.chunk_indices.clone())
            .collect();
        all_indices.sort();
        assert_eq!(all_indices, (0..10).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn test_spawn_chunk_processors() {
        let registry = Arc::new(AgentRegistry::new());

        registry
            .register_agent(
                "agent-1".to_string(),
                "Agent 1".to_string(),
                "Test".to_string(),
                vec!["llm".to_string()],
                None,
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        let processor = ChunkProcessor::with_registry(registry);
        let manifest = create_test_manifest(5);

        // Mock process function
        let result = processor
            .spawn_chunk_processors(&manifest, "test query", |batch| async move {
                batch
                    .chunk_indices
                    .iter()
                    .map(|&idx| ChunkResult {
                        chunk_index: idx,
                        agent_id: batch.agent_id.clone(),
                        output: format!("Result for chunk {idx}"),
                        tokens_used: 50,
                        processing_time_ms: 100,
                        success: true,
                        error: None,
                    })
                    .collect()
            })
            .await;

        assert_eq!(result.chunks_processed, 5);
        assert_eq!(result.successful_chunks, 5);
        assert_eq!(result.failed_chunks, 0);
        assert!(result.aggregated_output.contains("[Chunk 0]:"));
        assert!(result.aggregated_output.contains("[Chunk 4]:"));
    }

    #[tokio::test]
    async fn test_aggregate_with_failures() {
        let registry = Arc::new(AgentRegistry::new());
        let processor = ChunkProcessor::with_registry(registry);
        let manifest = create_test_manifest(3);

        let results = vec![
            ChunkResult {
                chunk_index: 0,
                agent_id: "agent-1".to_string(),
                output: "Success".to_string(),
                tokens_used: 50,
                processing_time_ms: 100,
                success: true,
                error: None,
            },
            ChunkResult {
                chunk_index: 1,
                agent_id: "agent-1".to_string(),
                output: String::new(),
                tokens_used: 10,
                processing_time_ms: 50,
                success: false,
                error: Some("Timeout".to_string()),
            },
            ChunkResult {
                chunk_index: 2,
                agent_id: "agent-2".to_string(),
                output: "Also success".to_string(),
                tokens_used: 60,
                processing_time_ms: 110,
                success: true,
                error: None,
            },
        ];

        let aggregated = processor.aggregate_results(&manifest, results, 1000);

        assert_eq!(aggregated.chunks_processed, 3);
        assert_eq!(aggregated.successful_chunks, 2);
        assert_eq!(aggregated.failed_chunks, 1);
        assert_eq!(aggregated.total_tokens, 120);
        assert_eq!(aggregated.agent_costs.len(), 2);
    }
}
