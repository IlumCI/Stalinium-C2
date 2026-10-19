use serde::{Deserialize, Serialize};
use super::capability::{ComputeScore, NodeCapability};

/// Model sharding and pipeline stage assignment.
///
/// Implements topology-aware hybrid sharding inspired by:
///   - prima.cpp pipelined-ring parallelism (arXiv:2504.08791): per-device
///     layer window sizing to minimize pipeline bubbles across heterogeneous nodes
///   - Pre-compiled pipeline shards (arXiv:2608.19147): ILP-based stage
///     placement using per-(stage, device) latency/memory profiles
///   - HALO (arXiv:2601.11676): assigning less critical neuron groups to
///     less reliable devices in lossy networks

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModelManifest {
    pub model_id: String,
    pub total_layers: u32,
    pub param_bytes_per_layer: u64,
    pub total_param_bytes: u64,
    pub architecture: String,
    pub quantization: Quantization,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Quantization {
    FP32,
    FP16,
    INT8,
    INT4,
}

impl Quantization {
    pub fn bytes_per_param(&self) -> f64 {
        match self {
            Quantization::FP32 => 4.0,
            Quantization::FP16 => 2.0,
            Quantization::INT8 => 1.0,
            Quantization::INT4 => 0.5,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShardAssignment {
    pub shard_id: String,
    pub node_id: String,
    pub layer_start: u32,
    pub layer_end: u32,
    pub estimated_latency_ms: f64,
    pub memory_required: u64,
    pub criticality: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PipelinePlan {
    pub model_id: String,
    pub assignments: Vec<ShardAssignment>,
    pub ring_order: Vec<String>,
    pub estimated_token_latency_ms: f64,
    pub rounds_per_token: u32,
}

pub fn compute_pipeline_plan(
    manifest: &ModelManifest,
    nodes: &[(NodeCapability, i32)],
) -> Option<PipelinePlan> {
    if nodes.is_empty() {
        return None;
    }

    let mut scores: Vec<ComputeScore> = nodes
        .iter()
        .map(|(cap, rep)| cap.compute_score(*rep))
        .collect();

    scores.sort_by(|a, b| b.flops_estimate.partial_cmp(&a.flops_estimate).unwrap());

    let effective_bytes = manifest.param_bytes_per_layer as f64
        * manifest.quantization.bytes_per_param()
        / 4.0;
    let eff_layer_bytes = effective_bytes as u64;

    let viable: Vec<&ComputeScore> = scores
        .iter()
        .filter(|s| s.memory_capacity >= eff_layer_bytes && s.reliability > 0.1)
        .collect();

    if viable.is_empty() {
        return None;
    }

    let total_flops: f64 = viable.iter().map(|s| s.flops_estimate * s.reliability).sum();

    let mut assignments = Vec::new();
    let mut ring_order = Vec::new();
    let mut layer_cursor: u32 = 0;

    for (i, score) in viable.iter().enumerate() {
        if layer_cursor >= manifest.total_layers {
            break;
        }

        let weight = (score.flops_estimate * score.reliability) / total_flops;
        let raw_layers = (weight * manifest.total_layers as f64).round() as u32;
        let layer_count = raw_layers.max(1);

        let layer_end = (layer_cursor + layer_count).min(manifest.total_layers);

        let mem_required = (layer_end - layer_cursor) as u64 * eff_layer_bytes;

        let max_mem = nodes
            .iter()
            .find(|(c, _)| c.node_id == score.node_id)
            .map(|(c, _)| c.gpu.as_ref().map(|g| g.vram_bytes).unwrap_or(c.ram_bytes))
            .unwrap_or(0);

        let actual_end = if mem_required > max_mem {
            let max_layers = max_mem / eff_layer_bytes.max(1);
            layer_cursor + (max_layers as u32).max(1)
        } else {
            layer_end
        };

        let actual_end = actual_end.min(manifest.total_layers);

        let latency = nodes
            .iter()
            .find(|(c, _)| c.node_id == score.node_id)
            .map(|(c, _)| {
                c.estimated_layer_latency_ms(eff_layer_bytes) * (actual_end - layer_cursor) as f64
            })
            .unwrap_or(f64::MAX);

        let criticality = 1.0 - (i as f32 / viable.len().max(1) as f32);

        let mut seed = [0u8; 8];
        let _ = getrandom::getrandom(&mut seed);

        assignments.push(ShardAssignment {
            shard_id: format!("shard-{:04x}", u16::from_le_bytes([seed[0], seed[1]])),
            node_id: score.node_id.clone(),
            layer_start: layer_cursor,
            layer_end: actual_end,
            estimated_latency_ms: latency,
            memory_required: (actual_end - layer_cursor) as u64 * eff_layer_bytes,
            criticality,
        });

        ring_order.push(score.node_id.clone());
        layer_cursor = actual_end;
    }

    if layer_cursor < manifest.total_layers && !assignments.is_empty() {
        let last = assignments.last_mut().unwrap();
        let extra_layers = manifest.total_layers - layer_cursor;
        last.layer_end = manifest.total_layers;
        last.memory_required += extra_layers as u64 * eff_layer_bytes;
    }

    let max_stage_latency = assignments
        .iter()
        .map(|a| a.estimated_latency_ms)
        .fold(0.0_f64, f64::max);

    let net_overhead: f64 = nodes
        .iter()
        .map(|(c, _)| c.net_latency_ms)
        .sum::<f64>()
        / nodes.len().max(1) as f64;

    let rounds = ((manifest.total_layers as f64 / assignments.len().max(1) as f64).ceil()) as u32;

    Some(PipelinePlan {
        model_id: manifest.model_id.clone(),
        assignments,
        ring_order,
        estimated_token_latency_ms: max_stage_latency * rounds as f64 + net_overhead,
        rounds_per_token: rounds,
    })
}
