use hxnet_common::*;
use hxnet_wasm::WasmRuntime;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{info, warn, error};
use anyhow::Result;
use clap::Parser;
use axum::{Router, routing::{get, post}, Json, extract::State};
use tower_http::trace::TraceLayer;
use metrics_exporter_prometheus::PrometheusBuilder;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use libp2p::{Swarm, kad, mdns, noise, yamux, tcp, quic, websocket, PeerId};
use std::time::Duration;

mod discovery;
mod runtime;
mod capability;
mod transport;
mod health;

use discovery::DiscoveryService;
use capability::CapabilityManager;
use runtime::RuntimeManager;
use transport::NetworkTransport;
use health::HealthMonitor;

#[derive(Parser, Debug)]
#[command(name = "hxnet-agent")]
struct Args {
    #[arg(long, env = "NODE_CLASS", default_value = "full")]
    node_class: String,
    
    #[arg(long, env = "CONTROL_PLANE_URL", default_value = "http://localhost:8080")]
    control_plane_url: String,
    
    #[arg(long, env = "BIND_ADDR", default_value = "0.0.0.0:8081")]
    bind_addr: String,
    
    #[arg(long, env = "METRICS_ADDR", default_value = "0.0.0.0:9091")]
    metrics_addr: String,
    
    #[arg(long, env = "NODE_ID")]
    node_id: Option<String>,
}

#[derive(Clone)]
struct AppState {
    node_id: Uuid,
    node_class: NodeClass,
    capability_manager: Arc<CapabilityManager>,
    runtime_manager: Arc<RuntimeManager>,
    network_transport: Arc<NetworkTransport>,
    health_monitor: Arc<HealthMonitor>,
    control_plane_client: Arc<ControlPlaneClient>,
}

struct ControlPlaneClient {
    base_url: String,
    client: reqwest::Client,
}

impl ControlPlaneClient {
    fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }
    
    async fn advertise(&self, descriptor: &CapabilityDescriptor) -> Result<()> {
        let url = format!("{}/api/v1/capabilities/advertise", self.base_url);
        self.client.post(&url).json(descriptor).send().await?.error_for_status()?;
        Ok(())
    }
    
    async fn update_health(&self, health: &HealthStatus) -> Result<()> {
        let url = format!("{}/api/v1/nodes/health", self.base_url);
        self.client.post(&url).json(health).send().await?.error_for_status()?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();
    
    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], 9091))
        .install()?;
    
    let args = Args::parse();
    
    let node_class = match args.node_class.as_str() {
        "full" => NodeClass::Full,
        "edge" => NodeClass::Edge,
        "lightweight" => NodeClass::Lightweight,
        "gateway" => NodeClass::Gateway,
        "service" => NodeClass::Service,
        _ => NodeClass::Full,
    };
    
    let node_id = args.node_id
        .map(|s| Uuid::parse_str(&s).unwrap())
        .unwrap_or_else(Uuid::new_v4);
    
    let (signing_key, verifying_key) = hxnet_common::generate_ed25519_keypair();
    
    let capability_manager = Arc::new(CapabilityManager::new(node_id, node_class, verifying_key.to_bytes().to_vec()));
    let runtime_manager = Arc::new(RuntimeManager::new());
    let network_transport = Arc::new(NetworkTransport::new(node_id, verifying_key.to_bytes().to_vec()).await?);
    let health_monitor = Arc::new(HealthMonitor::new(node_id));
    let control_plane_client = Arc::new(ControlPlaneClient::new(args.control_plane_url));
    
    let discovery = DiscoveryService::new(
        node_id,
        node_class,
        verifying_key.to_bytes().to_vec(),
        network_transport.clone(),
    ).await?;
    
    let state = AppState {
        node_id,
        node_class,
        capability_manager: capability_manager.clone(),
        runtime_manager: runtime_manager.clone(),
        network_transport: network_transport.clone(),
        health_monitor: health_monitor.clone(),
        control_plane_client: control_plane_client.clone(),
    };
    
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/api/v1/execute", post(execute_workload))
        .route("/api/v1/capabilities", get(get_capabilities))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());
    
    let listener = tokio::net::TcpListener::bind(&args.bind_addr).await?;
    info!("HXNet Agent {} listening on {}", node_id, args.bind_addr);
    
    let advertise_handle = tokio::spawn(advertise_loop(state.clone()));
    let health_handle = tokio::spawn(health_loop(state.clone()));
    let discovery_handle = tokio::spawn(discovery.run());
    
    axum::serve(listener, app).await?;
    
    advertise_handle.abort();
    health_handle.abort();
    discovery_handle.abort();
    
    Ok(())
}

async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "healthy", "service": "hxnet-agent" }))
}

async fn execute_workload(
    State(state): State<AppState>,
    Json(request): Json<ExecutionRequest>,
) -> Result<Json<ExecutionEvent>, (axum::http::StatusCode, String)> {
    state.runtime_manager.execute(request).await
        .map(Json)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn get_capabilities(
    State(state): State<AppState>,
) -> Json<CapabilityDescriptor> {
    Json(state.capability_manager.get_descriptor().await)
}

async fn advertise_loop(state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        let descriptor = state.capability_manager.get_descriptor().await;
        if let Err(e) = state.control_plane_client.advertise(&descriptor).await {
            warn!("Failed to advertise capabilities: {}", e);
        }
    }
}

async fn health_loop(state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    loop {
        interval.tick().await;
        let health = state.health_monitor.collect().await;
        if let Err(e) = state.control_plane_client.update_health(&health).await {
            warn!("Failed to update health: {}", e);
        }
    }
}