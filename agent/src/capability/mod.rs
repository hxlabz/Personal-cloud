use hxnet_common::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug};
use anyhow::Result;
use uuid::Uuid;
use chrono::Utc;

pub struct CapabilityManager {
    node_id: Uuid,
    node_class: NodeClass,
    public_key: Vec<u8>,
    capabilities: Arc<RwLock<HashMap<String, CapabilityCategory>>>,
}

impl CapabilityManager {
    pub fn new(node_id: Uuid, node_class: NodeClass, public_key: Vec<u8>) -> Self {
        let mut caps = HashMap::new();
        Self::populate_default_capabilities(&mut caps, node_class);
        
        Self {
            node_id,
            node_class,
            public_key,
            capabilities: Arc::new(RwLock::new(caps)),
        }
    }
    
    fn populate_default_capabilities(caps: &mut HashMap<String, CapabilityCategory>, node_class: NodeClass) {
        let mut compute = CapabilityCategory { capabilities: HashMap::new() };
        compute.capabilities.insert("cpu".into(), Capability {
            name: "cpu".into(),
            available: true,
            version: "1.0".into(),
            attributes: [("arch".into(), "x86_64".into()), ("cores".into(), "8".into())].into(),
            constraints: CapabilityConstraints::default(),
            lease: CapabilityLease::default(),
        });
        caps.insert("compute".into(), compute);
        
        let mut storage = CapabilityCategory { capabilities: HashMap::new() };
        storage.capabilities.insert("local".into(), Capability {
            name: "local".into(),
            available: true,
            version: "1.0".into(),
            attributes: [("type".into(), "local".into()), ("free_gb".into(), "500".into())].into(),
            constraints: CapabilityConstraints::default(),
            lease: CapabilityLease::default(),
        });
        caps.insert("storage".into(), storage);
        
        let mut network = CapabilityCategory { capabilities: HashMap::new() };
        network.capabilities.insert("ethernet".into(), Capability {
            name: "ethernet".into(),
            available: true,
            version: "1.0".into(),
            attributes: [("bandwidth_mbps".into(), "1000".into())].into(),
            constraints: CapabilityConstraints::default(),
            lease: CapabilityLease::default(),
        });
        caps.insert("network".into(), network);
        
        if node_class == NodeClass::Edge || node_class == NodeClass::Full {
            let mut camera = CapabilityCategory { capabilities: HashMap::new() };
            camera.capabilities.insert("rear".into(), Capability {
                name: "rear".into(),
                available: true,
                version: "1.0".into(),
                attributes: [("megapixels".into(), "12".into())].into(),
                constraints: CapabilityConstraints::default(),
                lease: CapabilityLease::default(),
            });
            caps.insert("camera".into(), camera);
            
            let mut sensor = CapabilityCategory { capabilities: HashMap::new() };
            sensor.capabilities.insert("accelerometer".into(), Capability {
                name: "accelerometer".into(),
                available: true,
                version: "1.0".into(),
                attributes: [("hz".into(), "100".into())].into(),
                constraints: CapabilityConstraints::default(),
                lease: CapabilityLease::default(),
            });
            caps.insert("sensor".into(), sensor);
        }
        
        if node_class == NodeClass::Full {
            let mut accelerator = CapabilityCategory { capabilities: HashMap::new() };
            accelerator.capabilities.insert("gpu".into(), Capability {
                name: "gpu".into(),
                available: true,
                version: "1.0".into(),
                attributes: [("vendor".into(), "nvidia".into()), ("vram_gb".into(), "12".into())].into(),
                constraints: CapabilityConstraints::default(),
                lease: CapabilityLease::default(),
            });
            caps.insert("accelerator".into(), accelerator);
        }
    }
    
    pub async fn get_descriptor(&self) -> CapabilityDescriptor {
        let caps = self.capabilities.read().await;
        let endpoints = HashMap::from([
            ("http".into(), format!("http://localhost:8081")),
            ("quic".into(), format!("quic://localhost:8082")),
        ]);
        
        let mut descriptor = CapabilityDescriptor {
            node_id: self.node_id,
            version: "1.0.0".into(),
            timestamp: Utc::now().timestamp(),
            capabilities: caps.clone(),
            node_class: self.node_class,
            endpoints,
            public_key: self.public_key.clone(),
            signature: vec![],
        };
        
        let (signing_key, _) = hxnet_common::generate_ed25519_keypair();
        descriptor.sign(&signing_key).unwrap();
        
        descriptor
    }
    
    pub async fn update_capability(&self, category: String, name: String, capability: Capability) {
        self.capabilities.write().await
            .entry(category)
            .or_insert_with(|| CapabilityCategory { capabilities: HashMap::new() })
            .capabilities.insert(name, capability);
    }
}