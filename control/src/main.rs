use hxnet_common::*;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{info, error};
use anyhow::Result;
use clap::Parser;
use axum::{Router, routing::{get, post}, Json, extract::State};
use tower_http::trace::TraceLayer;
use metrics_exporter_prometheus::PrometheusBuilder;

mod registry;
mod scheduler;

use registry::CapabilityRegistry;
use scheduler::Scheduler;

#[derive(Parser, Debug)]
#[command(name = "hxnet-control")]
struct Args {
    #[arg(long, env = "DATABASE_URL", default_value = "postgres://hxnet:hxnet@localhost/hxnet")]
    database_url: String,
    
    #[arg(long, env = "ETCD_ENDPOINTS")]
    etcd_endpoints: Option<String>,
    
    #[arg(long, env = "BIND_ADDR", default_value = "0.0.0.0:8080")]
    bind_addr: String,
    
    #[arg(long, env = "METRICS_ADDR", default_value = "0.0.0.0:9090")]
    metrics_addr: String,
}

#[derive(Clone)]
struct AppState {
    registry: Arc<CapabilityRegistry>,
    scheduler: Arc<Scheduler>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();
    
    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], 9090))
        .install()?;
    
    let args = Args::parse();
    
    let etcd_endpoints = args.etcd_endpoints
        .map(|s| s.split(',').map(|s| s.to_string()).collect());
    
    let registry = Arc::new(CapabilityRegistry::new(&args.database_url, etcd_endpoints).await?);
    let scheduler = Arc::new(Scheduler::new(
        PgPool::connect(&args.database_url).await?,
        registry.clone(),
    ));
    
    let state = AppState { registry, scheduler };
    
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/api/v1/capabilities/advertise", post(advertise_capabilities))
        .route("/api/v1/capabilities/query", post(query_capabilities))
        .route("/api/v1/workloads/place", post(place_workload))
        .route("/api/v1/workloads/evict", post(evict_workload))
        .route("/api/v1/nodes/health", post(update_health))
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    
    let listener = tokio::net::TcpListener::bind(&args.bind_addr).await?;
    info!("HXNet Control Plane listening on {}", args.bind_addr);
    
    axum::serve(listener, app).await?;
    
    Ok(())
}

async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "healthy", "service": "hxnet-control" }))
}

async fn advertise_capabilities(
    State(state): State<AppState>,
    Json(descriptor): Json<CapabilityDescriptor>,
) -> Result<Json<AdvertiseResponse>, (axum::http::StatusCode, String)> {
    state.registry.advertise(descriptor).await
        .map(Json)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn query_capabilities(
    State(state): State<AppState>,
    Json(request): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, (axum::http::StatusCode, String)> {
    state.registry.query(request).await
        .map(Json)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn place_workload(
    State(state): State<AppState>,
    Json(workload): Json<Workload>,
) -> Result<Json<Placement>, (axum::http::StatusCode, String)> {
    state.scheduler.place(workload).await
        .map(Json)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn evict_workload(
    State(state): State<AppState>,
    Json(request): Json<EvictRequest>,
) -> Result<Json<()>, (axum::http::StatusCode, String)> {
    state.scheduler.evict(request.workload_id).await
        .map(|_| Json(()))
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn update_health(
    State(state): State<AppState>,
    Json(health): Json<HealthStatus>,
) -> Result<Json<()>, (axum::http::StatusCode, String)> {
    state.registry.update_health(health).await
        .map(|_| Json(()))
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(Debug, serde::Deserialize)]
struct EvictRequest {
    workload_id: Uuid,
}

#[derive(Debug, serde::Serialize)]
struct AdvertiseResponse {
    accepted: bool,
    lease_id: Uuid,
    expires_at: i64,
}

#[derive(Debug, serde::Deserialize)]
struct QueryRequest {
    categories: Vec<String>,
    attributes: std::collections::HashMap<String, String>,
    node_class: NodeClass,
}

#[derive(Debug, serde::Serialize)]
struct QueryResponse {
    descriptors: Vec<CapabilityDescriptor>,
}