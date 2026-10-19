use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Hardware capability profile reported by each bot in the mesh.
///
/// Inspired by prima.cpp's (arXiv:2504.08791) heterogeneous device profiling
/// and the pre-compiled pipeline shard placement solver from arXiv:2608.19147.
/// Each node self-profiles and announces capabilities via the gossip layer so
/// the coordinator can run topology-aware stage assignment.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NodeCapability {
    pub node_id: String,
    pub cpu_cores: u32,
    pub cpu_freq_mhz: u32,
    pub ram_bytes: u64,
    pub gpu: Option<GpuInfo>,
    pub disk_read_mbps: f64,
    pub net_bandwidth_mbps: f64,
    pub net_latency_ms: f64,
    pub os: String,
    pub arch: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GpuInfo {
    pub name: String,
    pub vram_bytes: u64,
    pub compute_flops: f64,
    pub supports_fp16: bool,
    pub supports_int8: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ComputeScore {
    pub node_id: String,
    pub flops_estimate: f64,
    pub memory_capacity: u64,
    pub io_throughput: f64,
    pub reliability: f64,
}

impl NodeCapability {
    pub fn compute_score(&self, reputation: i32) -> ComputeScore {
        let cpu_flops = self.cpu_cores as f64 * self.cpu_freq_mhz as f64 * 1e6 * 4.0;
        let gpu_flops = self.gpu.as_ref().map(|g| g.compute_flops).unwrap_or(0.0);
        let flops = cpu_flops + gpu_flops;

        let mem = self.gpu.as_ref().map(|g| g.vram_bytes).unwrap_or(0) + self.ram_bytes;

        let io = self.disk_read_mbps.min(self.net_bandwidth_mbps);

        let reliability = ((reputation as f64 + 100.0) / 200.0).clamp(0.05, 1.0);

        ComputeScore {
            node_id: self.node_id.clone(),
            flops_estimate: flops,
            memory_capacity: mem,
            io_throughput: io,
            reliability,
        }
    }

    pub fn can_hold_shard_bytes(&self, shard_bytes: u64) -> bool {
        let available = self.gpu.as_ref().map(|g| g.vram_bytes).unwrap_or(self.ram_bytes);
        available >= shard_bytes
    }

    pub fn estimated_layer_latency_ms(&self, layer_param_bytes: u64) -> f64 {
        let flops = self.gpu.as_ref().map(|g| g.compute_flops)
            .unwrap_or_else(|| self.cpu_cores as f64 * self.cpu_freq_mhz as f64 * 1e6 * 4.0);
        if flops <= 0.0 {
            return f64::MAX;
        }
        let ops_per_layer = layer_param_bytes as f64 * 2.0;
        (ops_per_layer / flops) * 1000.0
    }
}

pub fn profile_local() -> NodeCapability {
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);

    let (os, arch) = (std::env::consts::OS.to_string(), std::env::consts::ARCH.to_string());

    let mut seed = [0u8; 8];
    let _ = getrandom::getrandom(&mut seed);
    let node_id = format!("{:016x}", u64::from_le_bytes(seed));

    NodeCapability {
        node_id,
        cpu_cores,
        cpu_freq_mhz: 2400,
        ram_bytes: estimate_ram(),
        gpu: detect_gpu(),
        disk_read_mbps: 100.0,
        net_bandwidth_mbps: 50.0,
        net_latency_ms: 10.0,
        os,
        arch,
    }
}

fn estimate_ram() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(kb_str) = parts.get(1) {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
        }
    }
    4 * 1024 * 1024 * 1024
}

fn detect_gpu() -> Option<GpuInfo> {
    #[cfg(target_os = "linux")]
    {
        if std::path::Path::new("/dev/nvidia0").exists() {
            return Some(GpuInfo {
                name: "nvidia-detected".to_string(),
                vram_bytes: 4 * 1024 * 1024 * 1024,
                compute_flops: 5e12,
                supports_fp16: true,
                supports_int8: true,
            });
        }
    }
    None
}
