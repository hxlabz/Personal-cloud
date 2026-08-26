use hxnet_common::*;
use quinn::{Endpoint, ServerConfig, ClientConfig, TransportConfig};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug, warn};
use anyhow::Result;
use uuid::Uuid;
use std::net::SocketAddr;
use rcgen::{Certificate, CertifiedKey};

pub struct NetworkTransport {
    node_id: Uuid,
    public_key: Vec<u8>,
    quinn_endpoint: Option<Arc<Endpoint>>,
    tls_config: Option<Arc<rustls::ServerConfig>>,
}

impl NetworkTransport {
    pub async fn new(node_id: Uuid, public_key: Vec<u8>) -> Result<Self> {
        let cert = Self::generate_self_signed_cert(&public_key)?;
        let server_config = Self::make_server_config(cert)?;
        let client_config = Self::make_client_config()?;
        
        let mut transport = TransportConfig::default();
        transport.keep_alive_interval(Some(std::time::Duration::from_secs(30)));
        
        let mut endpoint = Endpoint::server(server_config, "0.0.0.0:8082".parse()?)?;
        endpoint.set_default_client_config(client_config);
        
        Ok(Self {
            node_id,
            public_key,
            quinn_endpoint: Some(Arc::new(endpoint)),
            tls_config: None,
        })
    }
    
    fn generate_self_signed_cert(public_key: &[u8]) -> Result<CertifiedKey> {
        let mut params = rcgen::CertificateParams::new(vec!["hxnet.local".into()])?;
        params.key_pair = Some(rcgen::KeyPair::from_bytes(&rcgen::KeyPair::generate()?.serialize_der())?);
        let cert = params.self_signed()?;
        Ok(cert)
    }
    
    fn make_server_config(cert: CertifiedKey) -> Result<ServerConfig> {
        let mut config = ServerConfig::with_single_cert(vec![cert.cert.der().clone()], cert.key_pair.serialize_der().into())?;
        config.transport = Arc::new(TransportConfig::default());
        Ok(config)
    }
    
    fn make_client_config() -> Result<ClientConfig> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| rustls::OwnedTrustAnchor::from_subject_spki_name_constraints(ta.subject, ta.spki, ta.name_constraints)));
        let config = ClientConfig::new(Arc::new(rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth()));
        Ok(config)
    }
    
    pub async fn connect(&self, addr: SocketAddr) -> Result<quinn::Connection> {
        if let Some(endpoint) = &self.quinn_endpoint {
            let conn = endpoint.connect(addr, "hxnet.local")?.await?;
            Ok(conn)
        } else {
            Err(anyhow::anyhow!("Endpoint not initialized"))
        }
    }
    
    pub async fn send(&self, conn: &quinn::Connection, data: &[u8]) -> Result<()> {
        let (mut send, _) = conn.open_bi().await?;
        send.write_all(data).await?;
        send.finish().await?;
        Ok(())
    }
    
    pub async fn recv(&self, conn: &quinn::Connection) -> Result<Vec<u8>> {
        let (mut send, mut recv) = conn.open_bi().await?;
        let mut buf = Vec::new();
        recv.read_to_end(&mut buf).await?;
        Ok(buf)
    }
}