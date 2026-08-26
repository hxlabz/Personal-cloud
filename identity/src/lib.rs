use hxnet_common::*;
use webauthn_rs::prelude::*;
use rcgen;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info};
use anyhow::Result;
use uuid::Uuid;
use chrono::{DateTime, Utc};
use qrcode::QrCode;
use base64::{Engine as _, engine::general_purpose};

pub struct IdentityService {
    webauthn: Webauthn,
    pending_registrations: Arc<RwLock<HashMap<Uuid, RegistrationState>>>,
    pending_authentications: Arc<RwLock<HashMap<Uuid, AuthenticationState>>>,
    device_credentials: Arc<RwLock<HashMap<Uuid, DeviceCredential>>>,
    root_ca: Arc<RwLock<Option<RootCA>>>,
}

#[derive(Debug, Clone)]
struct RegistrationState {
    user_id: Uuid,
    challenge: Vec<u8>,
    creation_ccr: CreationChallengeResponse,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct AuthenticationState {
    user_id: Uuid,
    challenge: Vec<u8>,
    request_challenge_response: RequestChallengeResponse,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct DeviceCredential {
    device_id: Uuid,
    user_id: Uuid,
    credential_id: Vec<u8>,
    public_key: Vec<u8>,
    sign_count: u32,
    attested: bool,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct RootCA {
    signing_key: ed25519_dalek::SigningKey,
    cert_pem: String,
}

impl IdentityService {
    pub async fn new(rp_id: &str, rp_origin: &str) -> Result<Self> {
        let webauthn = WebauthnBuilder::new(rp_id)?
            .rp_name("HXNet")
            .rp_origin(rp_origin)
            .build()?;
        
        Ok(Self {
            webauthn,
            pending_registrations: Arc::new(RwLock::new(HashMap::new())),
            pending_authentications: Arc::new(RwLock::new(HashMap::new())),
            device_credentials: Arc::new(RwLock::new(HashMap::new())),
            root_ca: Arc::new(RwLock::new(None)),
        })
    }
    
    pub async fn initialize_root_ca(&self) -> Result<()> {
        let (signing_key, verifying_key) = hxnet_common::generate_ed25519_keypair();
        let cert_pem = self.generate_ca_cert(&verifying_key).await?;
        
        *self.root_ca.write().await = Some(RootCA { signing_key, cert_pem });
        info!("Root CA initialized");
        Ok(())
    }
    
    async fn generate_ca_cert(&self, _verifying_key: &ed25519_dalek::VerifyingKey) -> Result<String> {
        let mut params = rcgen::CertificateParams::new(vec!["hxnet-root-ca".into()])?;
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        
        let key_pair = rcgen::KeyPair::generate()?;
        params.key_pair = Some(key_pair);
        
        let cert = params.self_signed()?;
        Ok(cert.pem())
    }
    
    pub async fn start_registration(&self, user_id: Uuid, device_name: &str) -> Result<RegistrationResponse> {
        let (ccr, reg_state) = self.webauthn.start_passkey_registration(user_id.as_bytes(), device_name, None)?;
        
        let reg_id = Uuid::new_v4();
        self.pending_registrations.write().await.insert(reg_id, RegistrationState {
            user_id,
            challenge: ccr.public_key.challenge.clone(),
            creation_ccr: ccr.clone(),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        });
        
        let qr_data = format!("hxnet://register/{}", general_purpose::URL_SAFE_NO_PAD.encode(reg_id.as_bytes()));
        let qr_code = QrCode::new(qr_data.as_bytes())?;
        let qr_svg = qr_code.render::<qrcode::render::svg::Color>().build();
        
        Ok(RegistrationResponse {
            registration_id: reg_id,
            creation_options: ccr,
            qr_code_svg: qr_svg,
        })
    }
    
    pub async fn finish_registration(&self, registration_id: Uuid, registration: RegisterPublicKeyCredential) -> Result<DeviceCredentialResponse> {
        let mut pending = self.pending_registrations.write().await;
        let state = pending.remove(&registration_id).ok_or(anyhow::anyhow!("Registration not found"))?;
        
        if Utc::now() > state.expires_at {
            return Err(anyhow::anyhow!("Registration expired"));
        }
        
        let credential = self.webauthn.finish_passkey_registration(&registration, &state.creation_ccr)?;
        
        let device_cred = DeviceCredential {
            device_id: Uuid::new_v4(),
            user_id: state.user_id,
            credential_id: credential.cred_id().to_vec(),
            public_key: credential.public_key().to_vec(),
            sign_count: credential.sign_count(),
            attested: true,
            created_at: Utc::now(),
        };
        
        self.device_credentials.write().await.insert(device_cred.device_id, device_cred.clone());
        
        let device_cert = self.issue_device_cert(&device_cred).await?;
        
        Ok(DeviceCredentialResponse {
            device_id: device_cred.device_id,
            credential_id: device_cred.credential_id,
            device_cert_pem: device_cert,
            root_ca_pem: self.root_ca.read().await.as_ref().map(|ca| ca.cert_pem.clone()).unwrap_or_default(),
        })
    }
    
    async fn issue_device_cert(&self, credential: &DeviceCredential) -> Result<String> {
        let root_ca = self.root_ca.read().await;
        let ca = root_ca.as_ref().ok_or(anyhow::anyhow!("Root CA not initialized"))?;
        
        let mut params = rcgen::CertificateParams::new(vec![format!("hxnet-device-{}", credential.device_id)])?;
        params.is_ca = rcgen::IsCa::NoCa;
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::DigitalSignature,
            rcgen::KeyUsagePurpose::KeyAgreement,
        ];
        params.extended_key_usages = vec![
            rcgen::ExtendedKeyUsagePurpose::ClientAuth,
            rcgen::ExtendedKeyUsagePurpose::ServerAuth,
        ];
        
        let device_key = rcgen::KeyPair::generate()?;
        params.key_pair = Some(device_key.clone());
        
        let ca_key = rcgen::KeyPair::from_der(&ca.signing_key.to_bytes())?;
        let ca_cert = rcgen::Certificate::from_der(&ca.cert_pem)?;
        
        let cert = params.signed_by(&device_key, &ca_cert, &ca_key)?;
        Ok(cert.pem())
    }
    
    pub async fn start_authentication(&self, user_id: Uuid) -> Result<AuthenticationResponse> {
        let (rcr, auth_state) = self.webauthn.start_passkey_authentication(user_id.as_bytes())?;
        
        let auth_id = Uuid::new_v4();
        self.pending_authentications.write().await.insert(auth_id, AuthenticationState {
            user_id,
            challenge: rcr.public_key.challenge.clone(),
            request_challenge_response: rcr.clone(),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        });
        
        Ok(AuthenticationResponse {
            authentication_id: auth_id,
            request_options: rcr,
        })
    }
    
    pub async fn finish_authentication(&self, authentication_id: Uuid, assertion: PublicKeyCredential) -> Result<Uuid> {
        let mut pending = self.pending_authentications.write().await;
        let state = pending.remove(&authentication_id).ok_or(anyhow::anyhow!("Authentication not found"))?;
        
        if Utc::now() > state.expires_at {
            return Err(anyhow::anyhow!("Authentication expired"));
        }
        
        let device_cred = self.device_credentials.read().await
            .values()
            .find(|c| c.user_id == state.user_id && c.credential_id == assertion.cred_id().to_vec())
            .cloned()
            .ok_or(anyhow::anyhow!("No matching credential"))?;
        
        self.webauthn.finish_passkey_authentication(&assertion, &state.request_challenge_response, &device_cred.into())?;
        
        Ok(device_cred.device_id)
    }
    
    pub async fn get_device_credential(&self, device_id: Uuid) -> Option<DeviceCredential> {
        self.device_credentials.read().await.get(&device_id).cloned()
    }
    
    pub async fn revoke_device(&self, device_id: Uuid) -> Result<()> {
        self.device_credentials.write().await.remove(&device_id);
        info!("Device revoked: {}", device_id);
        Ok(())
    }
}

#[derive(Debug, serde::Serialize)]
pub struct RegistrationResponse {
    pub registration_id: Uuid,
    pub creation_options: CreationChallengeResponse,
    pub qr_code_svg: String,
}

#[derive(Debug, serde::Serialize)]
pub struct DeviceCredentialResponse {
    pub device_id: Uuid,
    pub credential_id: Vec<u8>,
    pub device_cert_pem: String,
    pub root_ca_pem: String,
}

#[derive(Debug, serde::Serialize)]
pub struct AuthenticationResponse {
    pub authentication_id: Uuid,
    pub request_options: RequestChallengeResponse,
}

