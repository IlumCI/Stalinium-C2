use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::shard::{PipelinePlan, ShardAssignment};

/// Pipelined-ring inference execution engine.
///
/// Implements the PRP (Pipelined-Ring Parallelism) protocol from prima.cpp
/// (arXiv:2504.08791) adapted for C2 gossip transport:
///
///   1. Coordinator distributes model shards to assigned nodes
///   2. Inference requests flow through the ring in pipeline stages
///   3. Each node processes its assigned layers and forwards activations
///   4. Final node returns completed output to coordinator
///
/// HALO-style (arXiv:2601.11676) degradation: if a node in the ring drops
/// out, its shard is reassigned to the nearest capable neighbor with a
/// reliability penalty, preserving pipeline continuity over lossy links.

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum PipelineMessage {
    ShardDistribute {
        job_id: String,
        assignment: ShardAssignment,
        model_url: String,
    },
    ShardAck {
        job_id: String,
        shard_id: String,
        node_id: String,
        ready: bool,
    },
    InferenceRequest {
        job_id: String,
        request_id: String,
        input_tokens: Vec<u32>,
        stage_index: u32,
    },
    ActivationForward {
        job_id: String,
        request_id: String,
        stage_index: u32,
        activation_data: Vec<u8>,
    },
    InferenceResult {
        job_id: String,
        request_id: String,
        output_tokens: Vec<u32>,
        latency_ms: f64,
    },
    NodeFailure {
        job_id: String,
        failed_node: String,
        stage_index: u32,
    },
    ShardReassign {
        job_id: String,
        original_node: String,
        new_node: String,
        assignment: ShardAssignment,
    },
}

#[derive(Debug, Clone)]
pub struct StageState {
    pub assignment: ShardAssignment,
    pub ready: bool,
    pub processed_requests: u64,
    pub total_latency_ms: f64,
    pub failures: u32,
}

pub struct PipelineCoordinator {
    plan: PipelinePlan,
    stages: Arc<RwLock<HashMap<String, StageState>>>,
    pending_requests: Arc<RwLock<HashMap<String, PendingInference>>>,
}

#[derive(Debug)]
struct PendingInference {
    request_id: String,
    input_tokens: Vec<u32>,
    current_stage: u32,
    started_at: std::time::Instant,
}

impl PipelineCoordinator {
    pub fn new(plan: PipelinePlan) -> Self {
        let mut stages = HashMap::new();
        for assignment in &plan.assignments {
            stages.insert(
                assignment.shard_id.clone(),
                StageState {
                    assignment: assignment.clone(),
                    ready: false,
                    processed_requests: 0,
                    total_latency_ms: 0.0,
                    failures: 0,
                },
            );
        }

        Self {
            plan,
            stages: Arc::new(RwLock::new(stages)),
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn generate_distribute_messages(&self) -> Vec<PipelineMessage> {
        self.plan
            .assignments
            .iter()
            .map(|a| PipelineMessage::ShardDistribute {
                job_id: self.plan.model_id.clone(),
                assignment: a.clone(),
                model_url: String::new(),
            })
            .collect()
    }

    pub async fn handle_message(&self, msg: PipelineMessage) -> Option<PipelineMessage> {
        match msg {
            PipelineMessage::ShardAck {
                shard_id, ready, ..
            } => {
                let mut stages = self.stages.write().await;
                if let Some(state) = stages.get_mut(&shard_id) {
                    state.ready = ready;
                }
                None
            }

            PipelineMessage::ActivationForward {
                job_id,
                request_id,
                stage_index,
                activation_data,
            } => {
                let next_stage = stage_index + 1;
                let _stages = self.stages.read().await;

                let is_final = next_stage as usize >= self.plan.assignments.len();

                if is_final {
                    let output_tokens = decode_output_tokens(&activation_data);
                    let mut pending = self.pending_requests.write().await;
                    let latency = pending
                        .remove(&request_id)
                        .map(|p| p.started_at.elapsed().as_secs_f64() * 1000.0)
                        .unwrap_or(0.0);

                    Some(PipelineMessage::InferenceResult {
                        job_id,
                        request_id,
                        output_tokens,
                        latency_ms: latency,
                    })
                } else {
                    Some(PipelineMessage::ActivationForward {
                        job_id,
                        request_id,
                        stage_index: next_stage,
                        activation_data,
                    })
                }
            }

            PipelineMessage::NodeFailure {
                job_id,
                failed_node,
                stage_index: _,
            } => {
                let mut stages = self.stages.write().await;
                let failed_shard = stages
                    .values()
                    .find(|s| s.assignment.node_id == failed_node)
                    .map(|s| s.assignment.clone());

                if let Some(assignment) = failed_shard {
                    let fallback = self
                        .plan
                        .ring_order
                        .iter()
                        .find(|n| {
                            **n != failed_node
                                && stages
                                    .get(*n)
                                    .map(|s| s.failures < 3)
                                    .unwrap_or(true)
                        })
                        .cloned();

                    if let Some(new_node) = fallback {
                        let mut reassigned = assignment.clone();
                        reassigned.node_id = new_node.clone();
                        reassigned.criticality *= 0.8;

                        if let Some(state) = stages.get_mut(&assignment.shard_id) {
                            state.assignment = reassigned.clone();
                            state.ready = false;
                            state.failures += 1;
                        }

                        return Some(PipelineMessage::ShardReassign {
                            job_id,
                            original_node: failed_node,
                            new_node,
                            assignment: reassigned,
                        });
                    }
                }
                None
            }

            _ => None,
        }
    }

    pub async fn submit_inference(&self, request_id: String, input_tokens: Vec<u32>) -> Option<PipelineMessage> {
        let stages = self.stages.read().await;
        let all_ready = stages.values().all(|s| s.ready);
        if !all_ready {
            return None;
        }

        self.pending_requests.write().await.insert(
            request_id.clone(),
            PendingInference {
                request_id: request_id.clone(),
                input_tokens: input_tokens.clone(),
                current_stage: 0,
                started_at: std::time::Instant::now(),
            },
        );

        Some(PipelineMessage::InferenceRequest {
            job_id: self.plan.model_id.clone(),
            request_id,
            input_tokens,
            stage_index: 0,
        })
    }

    pub async fn pipeline_stats(&self) -> PipelineStats {
        let stages = self.stages.read().await;
        let pending = self.pending_requests.read().await;

        let total_processed: u64 = stages.values().map(|s| s.processed_requests).sum();
        let avg_latency = if total_processed > 0 {
            stages.values().map(|s| s.total_latency_ms).sum::<f64>() / total_processed as f64
        } else {
            0.0
        };

        PipelineStats {
            active_stages: stages.len(),
            ready_stages: stages.values().filter(|s| s.ready).count(),
            pending_requests: pending.len(),
            total_processed,
            avg_latency_ms: avg_latency,
            total_failures: stages.values().map(|s| s.failures as u64).sum(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PipelineStats {
    pub active_stages: usize,
    pub ready_stages: usize,
    pub pending_requests: usize,
    pub total_processed: u64,
    pub avg_latency_ms: f64,
    pub total_failures: u64,
}

fn decode_output_tokens(data: &[u8]) -> Vec<u32> {
    data.chunks(4)
        .filter_map(|chunk| {
            if chunk.len() == 4 {
                Some(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            } else {
                None
            }
        })
        .collect()
}
