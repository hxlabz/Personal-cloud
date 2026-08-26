use hxnet_common::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error};
use anyhow::Result;
use uuid::Uuid;
use chrono::Utc;

pub struct BridgeManager {
    node_id: Uuid,
    public_key: Vec<u8>,
    bridges: Arc<RwLock<HashMap<String, Box<dyn DeviceBridge>>>>,
    virtual_nodes: Arc<RwLock<HashMap<String, VirtualNode>>>,
}

impl BridgeManager {
    pub async fn new(node_id: Uuid, public_key: Vec<u8>) -> Result<Self> {
        let mut bridges: HashMap<String, Box<dyn DeviceBridge>> = HashMap::new();
        bridges.insert("matter".into(), Box::new(MatterBridge::new().await?));
        bridges.insert("thread".into(), Box::new(ThreadBridge::new().await?));
        bridges.insert("ble".into(), Box::new(BleBridge::new().await?));
        
        Ok(Self {
            node_id,
            public_key,
            bridges: Arc::new(RwLock::new(bridges)),
            virtual_nodes: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    pub async fn run(&self) -> Result<()> {
        let bridges = self.bridges.read().await;
        for (name, bridge) in bridges.iter() {
            bridge.start().await?;
            info!("Started bridge: {}", name);
        }
        
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            self.discover_devices().await?;
        }
    }
    
    async fn discover_devices(&self) -> Result<()> {
        let bridges = self.bridges.read().await;
        for (name, bridge) in bridges.iter() {
            let devices = bridge.discover().await?;
            for device in devices {
                self.register_virtual_node(device).await?;
            }
        }
        Ok(())
    }
    
    async fn register_virtual_node(&self, device: BridgedDevice) -> Result<()> {
        let mut virtual_nodes = self.virtual_nodes.write().await;
        let virtual_id = format!("{}-{}", device.protocol, device.device_id);
        
        let mut capabilities = HashMap::new();
        for cap_name in device.capabilities {
            let mut cat = CapabilityCategory { capabilities: HashMap::new() };
            cat.capabilities.insert(cap_name.clone(), Capability {
                name: cap_name,
                available: true,
                version: "1.0".into(),
                attributes: HashMap::new(),
                constraints: CapabilityConstraints::default(),
                lease: CapabilityLease::default(),
            });
            capabilities.insert(device.protocol.clone(), cat);
        }
        
        virtual_nodes.insert(virtual_id.clone(), VirtualNode {
            virtual_id: virtual_id.clone(),
            physical_node_id: self.node_id,
            protocol: device.protocol,
            device_id: device.device_id,
            name: device.name,
            capabilities,
            last_seen: Utc::now().timestamp(),
        });
        
        info!("Registered virtual node: {}", virtual_id);
        Ok(())
    }
    
    pub async fn list_devices(&self) -> Vec<BridgedDevice> {
        let virtual_nodes = self.virtual_nodes.read().await;
        virtual_nodes.values().map(|vn| BridgedDevice {
            device_id: vn.device_id.clone(),
            protocol: vn.protocol.clone(),
            name: vn.name.clone(),
            capabilities: vn.capabilities.keys().cloned().collect(),
        }).collect()
    }
    
    pub async fn pair_device(&self, request: PairRequest) -> Result<()> {
        let bridges = self.bridges.read().await;
        if let Some(bridge) = bridges.get(&request.protocol) {
            bridge.pair(request.pairing_code).await?;
        }
        Ok(())
    }
    
    pub async fn get_descriptor(&self) -> CapabilityDescriptor {
        let virtual_nodes = self.virtual_nodes.read().await;
        let mut capabilities = HashMap::new();
        
        let mut bridge_caps = CapabilityCategory { capabilities: HashMap::new() };
        bridge_caps.capabilities.insert("matter".into(), Capability {
            name: "matter".into(),
            available: true,
            version: "1.3".into(),
            attributes: [("version".into(), "1.3".into())].into(),
            constraints: CapabilityConstraints::default(),
            lease: CapabilityLease::default(),
        });
        bridge_caps.capabilities.insert("thread".into(), Capability {
            name: "thread".into(),
            available: true,
            version: "1.2".into(),
            attributes: [("version".into(), "1.2".into())].into(),
            constraints: CapabilityConstraints::default(),
            lease: CapabilityLease::default(),
        });
        bridge_caps.capabilities.insert("ble".into(), Capability {
            name: "ble".into(),
            available: true,
            version: "5.2".into(),
            attributes: [("version".into(), "5.2".into())].into(),
            constraints: CapabilityConstraints::default(),
            lease: CapabilityLease::default(),
        });
        capabilities.insert("service".into(), bridge_caps);
        
        for vn in virtual_nodes.values() {
            for (cat_name, cat) in &vn.capabilities {
                capabilities.entry(cat_name.clone())
                    .or_insert_with(|| CapabilityCategory { capabilities: HashMap::new() })
                    .capabilities.extend(cat.capabilities.clone());
            }
        }
        
        let endpoints = HashMap::from([
            ("http".into(), format!("http://localhost:8083")),
            ("matter".into(), "matter://local".into()),
            ("thread".into(), "thread://local".into()),
        ]);
        
        let mut descriptor = CapabilityDescriptor {
            node_id: self.node_id,
            version: "1.0.0".into(),
            timestamp: Utc::now().timestamp(),
            capabilities,
            node_class: NodeClass::Gateway,
            endpoints,
            public_key: self.public_key.clone(),
            signature: vec![],
        };
        
        let (signing_key, _) = hxnet_common::generate_ed25519_keypair();
        descriptor.sign(&signing_key).unwrap();
        
        descriptor
    }
}

#[async_trait::async_trait]
pub trait DeviceBridge: Send + Sync {
    async fn start(&self) -> Result<()>;
    async fn discover(&self) -> Result<Vec<BridgedDevice>>;
    async fn pair(&self, pairing_code: String) -> Result<()>;
    async fn control(&self, device_id: &str, command: serde_json::Value) -> Result<serde_json::Value>;
}

struct VirtualNode {
    virtual_id: String,
    physical_node_id: Uuid,
    protocol: String,
    device_id: String,
    name: String,
    capabilities: HashMap<String, CapabilityCategory>,
    last_seen: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct BridgedDevice {
    pub device_id: String,
    pub protocol: String,
    pub name: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct PairRequest {
    pub protocol: String,
    pub pairing_code: String,
}