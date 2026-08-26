use hxnet_common::*;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error};
use anyhow::Result;
use uuid::Uuid;
use chrono::Utc;

pub struct Scheduler {
    pool: PgPool,
    registry: Arc<CapabilityRegistry>,
    weights: SchedulerWeights,
}

#[derive(Debug, Clone)]
pub struct SchedulerWeights {
    pub w_data: f64,
    pub w_compute: f64,
    pub w_latency: f64,
    pub w_power: f64,
    pub w_privacy: f64,
    pub w_cost: f64,
}

impl Default for SchedulerWeights {
    fn default() -> Self {
        Self {
            w_data: 0.30,
            w_compute: 0.25,
            w_latency: 0.20,
            w_power: 0.15,
            w_privacy: 0.10,
            w_cost: 0.0,
        }
    }
}

impl Scheduler {
    pub fn new(pool: PgPool, registry: Arc<CapabilityRegistry>) -> Self {
        Self {
            pool,
            registry,
            weights: SchedulerWeights::default(),
        }
    }

    pub async fn place(&self, workload: Workload) -> Result<Placement> {
        let candidates = self.find_candidates(&workload).await?;
        
        if candidates.is_empty() {
            return Err(anyhow::anyhow!("No suitable nodes found for workload"));
        }
        
        let mut scored = Vec::new();
        for node in candidates {
            if let Some(score) = self.score_node(&workload, &node).await {
                scored.push((score, node));
            }
        }
        
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        
        let (best_score, best_node) = scored.into_iter().next().unwrap();
        
        let placement = self.create_placement(&workload, &best_node, best_score).await?;
        
        self.record_placement(&placement).await?;
        
        info!("Workload {} placed on node {} with score {}", workload.workload_id, best_node.descriptor.node_id, best_score);
        
        Ok(placement)
    }

    async fn find_candidates(&self, workload: &Workload) -> Result<Vec<CandidateNode>> {
        let mut candidates = Vec::new();
        
        let query = QueryRequest {
            categories: workload.required_capabilities.iter().map(|c| c.category.clone()).collect(),
            attributes: HashMap::new(),
            node_class: NodeClass::Full,
        };
        
        let response = self.registry.query(query).await?;
        
        for descriptor in response.descriptors {
            if self.satisfies_hard_constraints(workload, &descriptor) {
                let health = self.registry.get_node_health(descriptor.node_id).await?.unwrap_or_default();
                candidates.push(CandidateNode {
                    descriptor,
                    health,
                });
            }
        }
        
        Ok(candidates)
    }

    fn satisfies_hard_constraints(&self, workload: &Workload, descriptor: &CapabilityDescriptor) -> bool {
        for req in &workload.required_capabilities {
            let cat = descriptor.capabilities.get(&req.category);
            if cat.is_none() {
                return false;
            }
            let cat = cat.unwrap();
            let cap = cat.capabilities.get(&req.name);
            if cap.is_none() {
                return false;
            }
            let cap = cap.unwrap();
            if !cap.available {
                return false;
            }
            for (attr_key, attr_val) in &req.attributes {
                if cap.attributes.get(attr_key) != Some(attr_val) {
                    return false;
                }
            }
        }
        true
    }

    async fn score_node(&self, workload: &Workload, candidate: &CandidateNode) -> Option<f64> {
        let health = &candidate.health;
        let descriptor = &candidate.descriptor;
        
        let data_score = self.data_locality_score(workload, descriptor);
        let compute_score = self.compute_fit_score(workload, descriptor);
        let latency_score = self.latency_score(health);
        let power_score = self.power_score(health);
        let privacy_score = self.privacy_score(descriptor);
        let cost_score = self.cost_score(descriptor);
        
        let total = self.weights.w_data * data_score +
                    self.weights.w_compute * compute_score +
                    self.weights.w_latency * latency_score +
                    self.weights.w_power * power_score +
                    self.weights.w_privacy * privacy_score +
                    self.weights.w_cost * cost_score;
        
        Some(total)
    }

    fn data_locality_score(&self, workload: &Workload, descriptor: &CapabilityDescriptor) -> f64 {
        let mut score = 0.5;
        if let Some(storage) = descriptor.capabilities.get("storage") {
            for cap in storage.capabilities.values() {
                if cap.attributes.get("type").map(|s| s.as_str()) == Some("local") {
                    score += 0.3;
                    break;
                }
            }
        }
        score.min(1.0)
    }

    fn compute_fit_score(&self, workload: &Workload, descriptor: &CapabilityDescriptor) -> f64 {
        let mut score = 0.0;
        let mut count = 0;
        
        if let Some(compute) = descriptor.capabilities.get("compute") {
            for req in &workload.required_capabilities {
                if req.category == "compute" {
                    if let Some(cap) = compute.capabilities.get(&req.name) {
                        if let Some(cores_str) = cap.attributes.get("cores") {
                            if let Ok(cores) = cores_str.parse::<u32>() {
                                if cores >= 4 {
                                    score += 1.0;
                                } else {
                                    score += cores as f64 / 4.0;
                                }
                            }
                        }
                        count += 1;
                    }
                }
            }
        }
        
        if count > 0 { score / count as f64 } else { 0.5 }
    }

    fn latency_score(&self, health: &HealthStatus) -> f64 {
        1.0 - (health.network_throughput_mbps / 10000.0).min(1.0)
    }

    fn power_score(&self, health: &HealthStatus) -> f64 {
        if let Some(battery) = health.battery_percent {
            battery / 100.0
        } else {
            1.0
        }
    }

    fn privacy_score(&self, descriptor: &CapabilityDescriptor) -> f64 {
        if descriptor.node_class == NodeClass::Full {
            1.0
        } else {
            0.7
        }
    }

    fn cost_score(&self, _descriptor: &CapabilityDescriptor) -> f64 {
        1.0
    }

    async fn create_placement(&self, workload: &Workload, candidate: &CandidateNode, score: f64) -> Result<Placement> {
        let mut assigned = HashMap::new();
        
        for req in &workload.required_capabilities {
            if let Some(cat) = candidate.descriptor.capabilities.get(&req.category) {
                if let Some(cap) = cat.capabilities.get(&req.name) {
                    let lease_id = Uuid::new_v4();
                    assigned.insert(req.name.clone(), CapabilityAssignment {
                        capability_ref: CapabilityRef {
                            category: req.category.clone(),
                            name: req.name.clone(),
                            version: req.version.clone(),
                            available: true,
                        },
                        lease_id,
                        expires_at: Utc::now().timestamp() + cap.lease.max_lease_sec as i64,
                    });
                }
            }
        }
        
        Ok(Placement {
            workload_id: workload.workload_id,
            node_id: candidate.descriptor.node_id,
            assigned_capabilities: assigned,
            score,
        })
    }

    async fn record_placement(&self, placement: &Placement) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO workload_placements (workload_id, node_id, placement_data, created_at)
            VALUES ($1, $2, $3, $4)
            "#,
            placement.workload_id,
            placement.node_id,
            serde_json::to_value(placement)?,
            Utc::now()
        )
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }

    pub async fn evict(&self, workload_id: Uuid) -> Result<()> {
        sqlx::query!("DELETE FROM workload_placements WHERE workload_id = $1", workload_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

struct CandidateNode {
    descriptor: CapabilityDescriptor,
    health: HealthStatus,
}

impl Default for HealthStatus {
    fn default() -> Self {
        Self {
            node_id: Uuid::nil(),
            status: HealthState::Offline,
            cpu_usage: 0.0,
            memory_usage: 0.0,
            disk_usage: 0.0,
            network_throughput_mbps: 0.0,
            battery_percent: None,
            thermal_c: None,
            uptime_sec: 0,
            capabilities_available: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueryRequest {
    pub categories: Vec<String>,
    pub attributes: HashMap<String, String>,
    pub node_class: NodeClass,
}

#[derive(Debug, Clone)]
pub struct QueryResponse {
    pub descriptors: Vec<CapabilityDescriptor>,
}

#[derive(Debug, Clone)]
pub struct CapabilityRegistry;