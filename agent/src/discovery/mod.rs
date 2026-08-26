use hxnet_common::*;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use libp2p::{Swarm, kad, mdns, noise, yamux, tcp, quic, websocket, PeerId, identity};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug, warn};
use anyhow::Result;
use uuid::Uuid;
use std::time::Duration;

pub struct DiscoveryService {
    node_id: Uuid,
    node_class: NodeClass,
    public_key: Vec<u8>,
    mdns_daemon: Option<ServiceDaemon>,
    libp2p_swarm: Option<Swarm<libp2p::kad::Behaviour<libp2p::kad::store::MemoryStore>>>,
    network_transport: Arc<NetworkTransport>,
}

impl DiscoveryService {
    pub async fn new(
        node_id: Uuid,
        node_class: NodeClass,
        public_key: Vec<u8>,
        network_transport: Arc<NetworkTransport>,
    ) -> Result<Self> {
        let mdns_daemon = ServiceDaemon::new().ok();
        
        let mut swarm = libp2p::SwarmBuilder::new_ephemeral()
            .with_behaviour(|key| {
                let mut cfg = kad::Config::default();
                cfg.set_protocol_names(vec!["/hxnet/kad/1.0.0".into()]);
                kad::Behaviour::with_config(key.public().to_peer_id(), kad::store::MemoryStore::new(key.public().to_peer_id()), cfg)
            })
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();
        
        swarm.behaviour_mut().set_mode(Some(kad::Mode::Server));
        
        Ok(Self {
            node_id,
            node_class,
            public_key,
            mdns_daemon,
            libp2p_swarm: Some(swarm),
            network_transport,
        })
    }
    
    pub async fn run(&mut self) -> Result<()> {
        self.start_mdns().await?;
        self.start_libp2p().await?;
        
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            self.announce().await?;
        }
    }
    
    async fn start_mdns(&self) -> Result<()> {
        if let Some(daemon) = &self.mdns_daemon {
            let service_info = ServiceInfo::new(
                "_hxnet._tcp.local.",
                &format!("hxnet-{}", self.node_id),
                "_hxnet._tcp.local.",
                "local.",
                8081,
                &[
                    format!("node_id={}", self.node_id),
                    format!("class={:?}", self.node_class),
                    format!("pubkey={}", hex::encode(&self.public_key)),
                ],
            )?;
            
            daemon.register(service_info)?;
            info!("mDNS service registered");
        }
        Ok(())
    }
    
    async fn start_libp2p(&mut self) -> Result<()> {
        if let Some(swarm) = &mut self.libp2p_swarm {
            swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;
            swarm.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse()?)?;
            info!("libp2p listening");
        }
        Ok(())
    }
    
    async fn announce(&self) -> Result<()> {
        let advertisement = NodeAdvertisement {
            node_id: self.node_id,
            node_class: self.node_class,
            version: "1.0.0".into(),
            caps: vec![],
            endpoints: HashMap::new(),
            public_key: self.public_key.clone(),
            timestamp: chrono::Utc::now().timestamp(),
            signature: vec![],
        };
        
        if let Some(swarm) = &self.libp2p_swarm {
            let data = serde_json::to_vec(&advertisement)?;
            // In real implementation, publish to DHT
            debug!("Announced to libp2p DHT: {} bytes", data.len());
        }
        
        Ok(())
    }
}

mod hex {
    pub fn encode(data: &[u8]) -> String {
        data.iter().map(|b| format!("{:02x}", b)).collect()
    }
}