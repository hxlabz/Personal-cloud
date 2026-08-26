use hxnet_common::*;
use uuid::Uuid;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error};
use anyhow::Result;
use clap::Parser;
use reqwest::Client as ReqwestClient;
use axum::{Router, routing::{get, post}, Json, extract::State};
use tower_http::trace::TraceLayer;
use metrics_exporter_prometheus::PrometheusBuilder;
use matter::{MatterClient, Device as MatterDevice};
use ot::{OpenThreadBorderRouter, ThreadDevice};

mod bridges;
mod matter;
mod thread;
mod ble;

use bridges::{BridgeManager, DeviceBridge};
use matter::MatterBridge;
use thread::ThreadBridge;
use ble::BleBridge;

#[derive(Parser, Debug)]
#[command(name = "hxnet-gateway")]
struct Args {
    #[arg(long, env = "CONTROL_PLANE_URL", default_value = "http://localhost:8080")]
    control_plane_url: String,
    
    #[arg(long, env = "BIND_ADDR", default_value = "0.0.0.0:8083")]
    bind_addr: String,
    
    #[arg(long, env = "METRICS_ADDR", default_value = "0.0.0.0:9092")]
    metrics_addr: String,
    
    #[arg(long, env = "NODE_ID")]
    node_id: Option<String>,
}

#[derive(Clone)]
struct AppState {
    node_id: Uuid,
    bridge_manager: Arc<BridgeManager>,
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
        .with_http_listener(([0, 0, 0, 0], 9092))
        .install()?;
    
    let args = Args::parse();
    
    let node_id = args.node_id
        .map(|s| Uuid::parse_str(&s).unwrap())
        .unwrap_or_else(Uuid::new_v4);
    
    let (signing_key, verifying_key) = hxnet_common::generate_ed25519_keypair();
    
    let bridge_manager = Arc::new(BridgeManager::new(node_id, verifying_key.to_bytes().to_vec()).await?);
    let control_plane_client = Arc::new(ControlPlaneClient::new(args.control_plane_url));
    
    let state = AppState {
        node_id,
        bridge_manager: bridge_manager.clone(),
        control_plane_client: control_plane_client.clone(),
    };
    
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/api/v1/bridges/devices", get(list_bridged_devices))
        .route("/api/v1/bridges/pair", post(pair_device))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());
    
    let listener = tokio::net::TcpListener::bind(&args.bind_addr).await?;
    info!("HXNet Gateway {} listening on {}", node_id, args.bind_addr);
    
    let advertise_handle = tokio::spawn(advertise_loop(state.clone()));
    let bridge_handle = tokio::spawn(bridge_manager.run());
    
    axum::serve(listener, app).await?;
    
    advertise_handle.abort();
    bridge_handle.abort();
    
    Ok(())
}

async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "healthy", "service": "hxnet-gateway" }))
}

async fn list_bridged_devices(
    State(state): State<AppState>,
) -> Json<Vec<BridgedDevice>> {
    Json(state.bridge_manager.list_devices().await)
}

async fn pair_device(
    State(state): State<AppState>,
    Json(request): Json<PairRequest>,
) -> Result<Json<()>, (axum::http::StatusCode, String)> {
    state.bridge_manager.pair_device(request).await
        .map(|_| Json(()))
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn advertise_loop(state: AppState) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        interval.tick().await;
        let descriptor = state.bridge_manager.get_descriptor().await;
        if let Err(e) = state.control_plane_client.advertise(&descriptor).await {
            warn!("Failed to advertise gateway capabilities: {}", e);
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct BridgedDevice {
    device_id: String,
    protocol: String,
    name: String,
    capabilities: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct PairRequest {
    protocol: String,
    pairing_code: String,
}