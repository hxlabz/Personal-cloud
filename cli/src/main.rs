use hxnet_common::*;
use clap::{Parser, Subcommand};
use reqwest::Client;
use uuid::Uuid;
use qrcode::QrCode;
use base64::{Engine as _, engine::general_purpose};
use indicatif::{ProgressBar, ProgressStyle};
use anyhow::Result;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "hxnet", version, about = "HXNet CLI - Universal Personal Device Fabric")]
struct Cli {
    #[arg(long, env = "HXNET_CONTROL_URL", default_value = "http://localhost:8080")]
    control_url: String,
    
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Device management
    Device {
        #[command(subcommand)]
        action: DeviceAction,
    },
    /// Workload management
    Workload {
        #[command(subcommand)]
        action: WorkloadAction,
    },
    /// Capability queries
    Capability {
        #[command(subcommand)]
        action: CapabilityAction,
    },
    /// Storage operations
    Storage {
        #[command(subcommand)]
        action: StorageAction,
    },
    /// Identity and onboarding
    Identity {
        #[command(subcommand)]
        action: IdentityAction,
    },
    /// Fabric status
    Status,
}

#[derive(Subcommand, Debug)]
enum DeviceAction {
    /// List all devices in fabric
    List,
    /// Show device details
    Show { node_id: String },
    /// Onboard new device
    Onboard { name: String },
    /// Revoke device
    Revoke { node_id: String },
}

#[derive(Subcommand, Debug)]
enum WorkloadAction {
    /// Submit workload for execution
    Submit { manifest: String },
    /// List workloads
    List,
    /// Show workload status
    Status { workload_id: String },
    /// Cancel workload
    Cancel { workload_id: String },
}

#[derive(Subcommand, Debug)]
enum CapabilityAction {
    /// Query capabilities
    Query { category: Option<String> },
    /// Watch for capability changes
    Watch { category: Option<String> },
}

#[derive(Subcommand, Debug)]
enum StorageAction {
    /// Put object
    Put { key: String, file: String, tier: String },
    /// Get object
    Get { key: String, output: String, tier: String },
    /// List objects
    List { prefix: String, tier: String },
    /// Tier object
    Tier { key: String, from: String, to: String },
}

#[derive(Subcommand, Debug)]
enum IdentityAction {
    /// Start device registration
    Register { user_id: String, device_name: String },
    /// Complete registration with passkey
    Complete { registration_id: String },
    /// Authenticate user
    Auth { user_id: String },
    /// List user devices
    Devices { user_id: String },
    /// Revoke device
    Revoke { device_id: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();
    
    let cli = Cli::parse();
    let client = Client::new();
    
    match cli.command {
        Commands::Device { action } => handle_device(&client, &cli.control_url, action).await?,
        Commands::Workload { action } => handle_workload(&client, &cli.control_url, action).await?,
        Commands::Capability { action } => handle_capability(&client, &cli.control_url, action).await?,
        Commands::Storage { action } => handle_storage(&client, &cli.control_url, action).await?,
        Commands::Identity { action } => handle_identity(&client, &cli.control_url, action).await?,
        Commands::Status => handle_status(&client, &cli.control_url).await?,
    }
    
    Ok(())
}

async fn handle_device(client: &Client, base_url: &str, action: DeviceAction) -> Result<()> {
    match action {
        DeviceAction::List => {
            let resp = client.get(&format!("{}/api/v1/nodes", base_url)).send().await?;
            let devices: serde_json::Value = resp.json().await?;
            println!("{}", serde_json::to_string_pretty(&devices)?);
        }
        DeviceAction::Show { node_id } => {
            let resp = client.get(&format!("{}/api/v1/nodes/{}", base_url, node_id)).send().await?;
            let device: serde_json::Value = resp.json().await?;
            println!("{}", serde_json::to_string_pretty(&device)?);
        }
        DeviceAction::Onboard { name } => {
            println!("Starting onboarding for device: {}", name);
            let resp = client.post(&format!("{}/api/v1/identity/register", base_url))
                .json(&serde_json::json!({ "device_name": name }))
                .send().await?;
            let result: serde_json::Value = resp.json().await?;
            println!("Registration ID: {}", result["registration_id"]);
            println!("QR Code: {}", result["qr_code_svg"]);
        }
        DeviceAction::Revoke { node_id } => {
            client.delete(&format!("{}/api/v1/nodes/{}", base_url, node_id)).send().await?;
            println!("Device {} revoked", node_id);
        }
    }
    Ok(())
}

async fn handle_workload(client: &Client, base_url: &str, action: WorkloadAction) -> Result<()> {
    match action {
        WorkloadAction::Submit { manifest } => {
            let manifest_content = std::fs::read_to_string(manifest)?;
            let workload: Workload = serde_json::from_str(&manifest_content)?;
            
            let resp = client.post(&format!("{}/api/v1/workloads/place", base_url))
                .json(&workload)
                .send().await?;
            let placement: Placement = resp.json().await?;
            println!("Workload placed on node: {}", placement.node_id);
            println!("Score: {}", placement.score);
        }
        WorkloadAction::List => {
            let resp = client.get(&format!("{}/api/v1/workloads", base_url)).send().await?;
            let workloads: serde_json::Value = resp.json().await?;
            println!("{}", serde_json::to_string_pretty(&workloads)?);
        }
        WorkloadAction::Status { workload_id } => {
            let resp = client.get(&format!("{}/api/v1/workloads/{}", base_url, workload_id)).send().await?;
            let status: serde_json::Value = resp.json().await?;
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        WorkloadAction::Cancel { workload_id } => {
            client.post(&format!("{}/api/v1/workloads/{}/cancel", base_url, workload_id))
                .send().await?;
            println!("Workload {} cancelled", workload_id);
        }
    }
    Ok(())
}

async fn handle_capability(client: &Client, base_url: &str, action: CapabilityAction) -> Result<()> {
    match action {
        CapabilityAction::Query { category } => {
            let req = QueryRequest {
                categories: category.map(|c| vec![c]).unwrap_or_default(),
                attributes: std::collections::HashMap::new(),
                node_class: NodeClass::Full,
            };
            
            let resp = client.post(&format!("{}/api/v1/capabilities/query", base_url))
                .json(&req)
                .send().await?;
            let result: QueryResponse = resp.json().await?;
            println!("Found {} nodes", result.descriptors.len());
            for desc in result.descriptors {
                println!("  Node: {} ({:?})", desc.node_id, desc.node_class);
                for (cat_name, cat) in desc.capabilities {
                    for (cap_name, cap) in cat.capabilities {
                        println!("    {}.{}: available={}", cat_name, cap_name, cap.available);
                    }
                }
            }
        }
        CapabilityAction::Watch { category } => {
            println!("Watching for capability changes... (Ctrl+C to stop)");
            // Would connect to WebSocket/Server-Sent Events
        }
    }
    Ok(())
}

async fn handle_storage(client: &Client, base_url: &str, action: StorageAction) -> Result<()> {
    match action {
        StorageAction::Put { key, file, tier } => {
            let data = std::fs::read(file)?;
            let tier = match tier.as_str() { "hot" => StorageTier::Hot, "cold" => StorageTier::Cold, _ => StorageTier::Hot };
            
            let pb = ProgressBar::new(data.len() as u64);
            pb.set_style(ProgressStyle::default_bar().template("{bar:40.cyan/blue} {bytes}/{total_bytes} ({eta})")?);
            
            // In real impl, would stream upload
            let resp = client.post(&format!("{}/api/v1/storage/put", base_url))
                .json(&serde_json::json!({ "key": key, "data": general_purpose::STANDARD.encode(&data), "tier": tier }))
                .send().await?;
            
            pb.finish_with_message("Upload complete");
            let result: serde_json::Value = resp.json().await?;
            println!("Stored at: {}", result["object_key"]);
        }
        StorageAction::Get { key, output, tier } => {
            let tier = match tier.as_str() { "hot" => StorageTier::Hot, "cold" => StorageTier::Cold, _ => StorageTier::Hot };
            
            let resp = client.post(&format!("{}/api/v1/storage/get", base_url))
                .json(&serde_json::json!({ "key": key, "tier": tier }))
                .send().await?;
            let result: serde_json::Value = resp.json().await?;
            let data = general_purpose::STANDARD.decode(result["data"].as_str().unwrap())?;
            std::fs::write(output, data)?;
            println!("Downloaded to: {}", output);
        }
        StorageAction::List { prefix, tier } => {
            let tier = match tier.as_str() { "hot" => StorageTier::Hot, "cold" => StorageTier::Cold, _ => StorageTier::Hot };
            
            let resp = client.post(&format!("{}/api/v1/storage/list", base_url))
                .json(&serde_json::json!({ "prefix": prefix, "tier": tier }))
                .send().await?;
            let objects: Vec<String> = resp.json().await?;
            for obj in objects {
                println!("{}", obj);
            }
        }
        StorageAction::Tier { key, from, to } => {
            let from = match from.as_str() { "hot" => StorageTier::Hot, "cold" => StorageTier::Cold, _ => StorageTier::Hot };
            let to = match to.as_str() { "hot" => StorageTier::Hot, "cold" => StorageTier::Cold, _ => StorageTier::Cold };
            
            let resp = client.post(&format!("{}/api/v1/storage/tier", base_url))
                .json(&serde_json::json!({ "key": key, "from": from, "to": to }))
                .send().await?;
            println!("Tiered: {}", resp.text().await?);
        }
    }
    Ok(())
}

async fn handle_identity(client: &Client, base_url: &str, action: IdentityAction) -> Result<()> {
    match action {
        IdentityAction::Register { user_id, device_name } => {
            let resp = client.post(&format!("{}/api/v1/identity/register", base_url))
                .json(&serde_json::json!({ "user_id": user_id, "device_name": device_name }))
                .send().await?;
            let result: serde_json::Value = resp.json().await?;
            println!("Registration ID: {}", result["registration_id"]);
            println!("Scan QR code with authenticator app");
        }
        IdentityAction::Complete { registration_id } => {
            println!("Complete registration using authenticator app");
        }
        IdentityAction::Auth { user_id } => {
            let resp = client.post(&format!("{}/api/v1/identity/auth/start", base_url))
                .json(&serde_json::json!({ "user_id": user_id }))
                .send().await?;
            let result: serde_json::Value = resp.json().await?;
            println!("Authentication ID: {}", result["authentication_id"]);
            println!("Use passkey to authenticate");
        }
        IdentityAction::Devices { user_id } => {
            let resp = client.get(&format!("{}/api/v1/identity/devices/{}", base_url, user_id)).send().await?;
            let devices: serde_json::Value = resp.json().await?;
            println!("{}", serde_json::to_string_pretty(&devices)?);
        }
        IdentityAction::Revoke { device_id } => {
            client.delete(&format!("{}/api/v1/identity/devices/{}", base_url, device_id)).send().await?;
            println!("Device {} revoked", device_id);
        }
    }
    Ok(())
}

async fn handle_status(client: &Client, base_url: &str) -> Result<()> {
    let resp = client.get(&format!("{}/health", base_url)).send().await?;
    let health: serde_json::Value = resp.json().await?;
    println!("Control Plane: {}", health["status"]);
    
    let resp = client.get(&format!("{}/api/v1/nodes", base_url)).send().await?;
    let nodes: serde_json::Value = resp.json().await?;
    println!("Nodes in fabric: {}", nodes.as_array().unwrap_or(&vec![]).len());
    
    Ok(())
}

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
struct QueryRequest {
    categories: Vec<String>,
    attributes: HashMap<String, String>,
    node_class: NodeClass,
}

#[derive(Debug, Serialize, Deserialize)]
struct QueryResponse {
    descriptors: Vec<CapabilityDescriptor>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum StorageTier {
    Hot,
    Cold,
}