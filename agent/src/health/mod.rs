use hxnet_common::*;
use sysinfo::{System, SystemExt, CpuExt, DiskExt, NetworkExt, ComponentExt};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug};
use anyhow::Result;
use uuid::Uuid;
use std::time::Duration;

pub struct HealthMonitor {
    node_id: Uuid,
    system: Arc<RwLock<System>>,
    start_time: std::time::Instant,
}

impl HealthMonitor {
    pub fn new(node_id: Uuid) -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        
        Self {
            node_id,
            system: Arc::new(RwLock::new(sys)),
            start_time: std::time::Instant::now(),
        }
    }
    
    pub async fn collect(&self) -> HealthStatus {
        let mut sys = self.system.write().await;
        sys.refresh_cpu();
        sys.refresh_memory();
        sys.refresh_disks();
        sys.refresh_networks();
        sys.refresh_components();
        
        let cpu_usage = sys.global_cpu_info().cpu_usage() as f64;
        let total_mem = sys.total_memory();
        let used_mem = sys.used_memory();
        let memory_usage = if total_mem > 0 { used_mem as f64 / total_mem as f64 * 100.0 } else { 0.0 };
        
        let total_disk: u64 = sys.disks().iter().map(|d| d.total_space()).sum();
        let used_disk: u64 = sys.disks().iter().map(|d| d.total_space() - d.available_space()).sum();
        let disk_usage = if total_disk > 0 { used_disk as f64 / total_disk as f64 * 100.0 } else { 0.0 };
        
        let (tx_bytes, rx_bytes): (u64, u64) = sys.networks().iter()
            .map(|(_, net)| (net.transmitted(), net.received()))
            .fold((0, 0), |(tx, rx), (t, r)| (tx + t, rx + r));
        let network_throughput_mbps = (tx_bytes + rx_bytes) as f64 / 1_000_000.0;
        
        let thermal_c = sys.components().iter()
            .map(|c| c.temperature())
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);
        
        let battery_percent = None;
        
        HealthStatus {
            node_id: self.node_id,
            status: if cpu_usage > 90.0 || memory_usage > 90.0 { HealthState::Degraded } else { HealthState::Healthy },
            cpu_usage,
            memory_usage,
            disk_usage,
            network_throughput_mbps,
            battery_percent,
            thermal_c: Some(thermal_c),
            uptime_sec: self.start_time.elapsed().as_secs(),
            capabilities_available: vec![],
        }
    }
}