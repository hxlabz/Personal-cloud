use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;
use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer, Verifier};
use x25519_dalek::{PublicKey, EphemeralSecret};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, aead::Aead, aead::Payload};
use blake3;
use sha2::{Sha256, digest::FixedOutput};
use hkdf::Hkdf;
use rand::rngs::OsRng;
use rand::RngCore;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum NodeClass {
    Full,
    Edge,
    Lightweight,
    Gateway,
    Service,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeIdentity {
    pub node_id: Uuid,
    pub node_class: NodeClass,
    pub public_key: Vec<u8>,
    pub version: String,
    pub created_at: i64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub node_id: Uuid,
    pub version: String,
    pub timestamp: i64,
    pub capabilities: HashMap<String, CapabilityCategory>,
    pub node_class: NodeClass,
    pub endpoints: HashMap<String, String>,
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityCategory {
    pub capabilities: HashMap<String, Capability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    pub name: String,
    pub available: bool,
    pub version: String,
    pub attributes: HashMap<String, String>,
    pub constraints: CapabilityConstraints,
    pub lease: CapabilityLease,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityConstraints {
    pub max_concurrent: u32,
    pub max_duration_sec: u64,
    pub requires_grant: Vec<String>,
    pub power_budget_mw: u64,
    pub thermal_limit_c: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityLease {
    pub max_lease_sec: u64,
    pub renewable: bool,
    pub revocable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRef {
    pub category: String,
    pub name: String,
    pub version: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeAdvertisement {
    pub node_id: Uuid,
    pub node_class: NodeClass,
    pub version: String,
    pub caps: Vec<CapabilityRef>,
    pub endpoints: HashMap<String, String>,
    pub public_key: Vec<u8>,
    pub timestamp: i64,
    pub signature: Vec<u8>,
}

impl NodeAdvertisement {
    pub fn sign(&mut self, signing_key: &SigningKey) -> anyhow::Result<()> {
        let mut buf = Vec::new();
        buf.extend_from_slice(self.node_id.as_bytes());
        buf.extend_from_slice(self.version.as_bytes());
        for cap in &self.caps {
            buf.extend_from_slice(cap.category.as_bytes());
            buf.extend_from_slice(cap.name.as_bytes());
            buf.extend_from_slice(cap.version.as_bytes());
        }
        for (k, v) in &self.endpoints {
            buf.extend_from_slice(k.as_bytes());
            buf.extend_from_slice(v.as_bytes());
        }
        buf.extend_from_slice(&self.public_key);
        buf.extend_from_slice(&self.timestamp.to_le_bytes());
        
        let sig = signing_key.sign(&buf);
        self.signature = sig.to_bytes().to_vec();
        Ok(())
    }
    
    pub fn verify(&self, verifying_key: &VerifyingKey) -> bool {
        let mut buf = Vec::new();
        buf.extend_from_slice(self.node_id.as_bytes());
        buf.extend_from_slice(self.version.as_bytes());
        for cap in &self.caps {
            buf.extend_from_slice(cap.category.as_bytes());
            buf.extend_from_slice(cap.name.as_bytes());
            buf.extend_from_slice(cap.version.as_bytes());
        }
        for (k, v) in &self.endpoints {
            buf.extend_from_slice(k.as_bytes());
            buf.extend_from_slice(v.as_bytes());
        }
        buf.extend_from_slice(&self.public_key);
        buf.extend_from_slice(&self.timestamp.to_le_bytes());
        
        if self.signature.len() != 64 {
            return false;
        }
        let sig = Signature::from_bytes(&self.signature[..].try_into().unwrap());
        verifying_key.verify(&buf, &sig).is_ok()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workload {
    pub workload_id: Uuid,
    pub name: String,
    pub version: String,
    pub format: WorkloadFormat,
    pub required_capabilities: Vec<CapabilityRequirement>,
    pub inputs: Vec<WorkloadIO>,
    pub outputs: Vec<WorkloadIO>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadFormat {
    WasmComponent,
    OciContainer,
    NativeBinary,
    PlatformService,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRequirement {
    pub category: String,
    pub name: String,
    pub version: String,
    pub attributes: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadIO {
    pub name: String,
    pub data_type: String,
    pub schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Placement {
    pub workload_id: Uuid,
    pub node_id: Uuid,
    pub assigned_capabilities: HashMap<String, CapabilityAssignment>,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityAssignment {
    pub capability_ref: CapabilityRef,
    pub lease_id: Uuid,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    pub workload_id: Uuid,
    pub placement: Placement,
    pub input_data: HashMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub event_type: ExecutionEventType,
    pub workload_id: Uuid,
    pub data: Option<Vec<u8>>,
    pub output_name: Option<String>,
    pub error: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEventType {
    Started,
    Progress,
    Output,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub node_id: Uuid,
    pub status: HealthState,
    pub cpu_usage: f64,
    pub memory_usage: f64,
    pub disk_usage: f64,
    pub network_throughput_mbps: f64,
    pub battery_percent: Option<f64>,
    pub thermal_c: Option<f64>,
    pub uptime_sec: u64,
    pub capabilities_available: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Healthy,
    Degraded,
    Unhealthy,
    Offline,
}

impl Default for HealthState {
    fn default() -> Self {
        HealthState::Offline
    }
}

impl Default for CapabilityConstraints {
    fn default() -> Self {
        Self {
            max_concurrent: 1,
            max_duration_sec: 3600,
            requires_grant: Vec::new(),
            power_budget_mw: 0,
            thermal_limit_c: 85.0,
        }
    }
}

impl Default for CapabilityLease {
    fn default() -> Self {
        Self {
            max_lease_sec: 3600,
            renewable: true,
            revocable: true,
        }
    }
}

pub fn generate_ed25519_keypair() -> (SigningKey, VerifyingKey) {
    let mut csprng = OsRng;
    let mut bytes = [0u8; 32];
    csprng.fill_bytes(&mut bytes);
    let signing_key = SigningKey::from_bytes(&bytes);
    let verifying_key = signing_key.verifying_key();
    (signing_key, verifying_key)
}

pub fn generate_x25519_keypair() -> (EphemeralSecret, PublicKey) {
    let secret = EphemeralSecret::random_from_rng(OsRng);
    let public = PublicKey::from(&secret);
    (secret, public)
}

pub fn derive_shared_secret(our_secret: EphemeralSecret, their_public: &PublicKey) -> [u8; 32] {
    let shared = our_secret.diffie_hellman(their_public);
    *shared.as_bytes()
}

pub fn encrypt_aead(key: &[u8; 32], nonce: &[u8; 12], plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new(key.into());
    cipher.encrypt(nonce.into(), Payload { msg: plaintext, aad }).unwrap()
}

pub fn decrypt_aead(key: &[u8; 32], nonce: &[u8; 12], ciphertext: &[u8], aad: &[u8]) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new(key.into());
    cipher.decrypt(nonce.into(), Payload { msg: ciphertext, aad }).unwrap()
}

pub fn hkdf_sha256(ikm: &[u8], salt: &[u8], info: &[u8], out_len: usize) -> Vec<u8> {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut okm = vec![0u8; out_len];
    hk.expand(info, &mut okm).unwrap();
    okm
}

pub fn blake3_hash(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_advertisement_sign_verify() {
        let (signing_key, verifying_key) = generate_ed25519_keypair();
        let mut adv = NodeAdvertisement {
            node_id: Uuid::new_v4(),
            node_class: NodeClass::Full,
            version: "1.0.0".into(),
            caps: vec![],
            endpoints: HashMap::new(),
            public_key: verifying_key.to_bytes().to_vec(),
            timestamp: 1234567890,
            signature: vec![],
        };
        
        adv.sign(&signing_key).unwrap();
        assert!(adv.verify(&verifying_key));
        
        adv.signature[0] ^= 1;
        assert!(!adv.verify(&verifying_key));
    }
    
    #[test]
    fn test_encryption_roundtrip() {
        let key = [1u8; 32];
        let nonce = [2u8; 12];
        let plaintext = b"hello world";
        let aad = b"aad";
        
        let ct = encrypt_aead(&key, &nonce, plaintext, aad);
        let pt = decrypt_aead(&key, &nonce, &ct, aad);
        
        assert_eq!(pt, plaintext);
    }
}