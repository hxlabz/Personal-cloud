use hxnet_common::*;
use minio::s3::Client;
use minio::s3::creds::StaticProvider;
use minio::s3::region::Region;
use minio::s3::types::S3Api;
use minio::s3::response::GetObjectResponse;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error};
use anyhow::Result;
use uuid::Uuid;
use blake3;
use bytes::Bytes;
use std::collections::HashMap;

pub struct StorageClient {
    hot_client: Arc<Client>,
    cold_client: Option<Arc<Client>>,
    hot_bucket: String,
    cold_bucket: String,
    local_cache: Arc<RwLock<HashMap<String, Vec<u8>>>>,
}

impl StorageClient {
    pub async fn new(
        hot_endpoint: &str,
        hot_access_key: &str,
        hot_secret_key: &str,
        hot_bucket: &str,
        cold_endpoint: Option<&str>,
        cold_access_key: Option<&str>,
        cold_secret_key: Option<&str>,
        cold_bucket: Option<&str>,
    ) -> Result<Self> {
        let hot_provider = StaticProvider::new(hot_access_key, hot_secret_key, None);
        let hot_region = Region::new("us-east-1");
        
        let hot_client = Arc::new(
            Client::new(hot_endpoint.parse()?)
                .provider(hot_provider)
                .region(hot_region)
                .build()?
        );
        
        let cold_client = if let (Some(endpoint), Some(access), Some(secret), Some(bucket)) = 
            (cold_endpoint, cold_access_key, cold_secret_key, cold_bucket) {
            let provider = StaticProvider::new(access, secret, None);
            let region = Region::new("us-east-1");
            Some(Arc::new(
                Client::new(endpoint.parse()?)
                    .provider(provider)
                    .region(region)
                    .build()?
            ))
        } else {
            None
        };
        
        Ok(Self {
            hot_client,
            cold_client,
            hot_bucket: hot_bucket.to_string(),
            cold_bucket: cold_bucket.unwrap_or("hxnet-cold").to_string(),
            local_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    pub async fn put(&self, key: &str, data: &[u8], tier: StorageTier) -> Result<String> {
        let hash = blake3_hash(data);
        let object_key = format!("{}/{}", hex::encode(&hash[..8]), key);
        
        let client = match tier {
            StorageTier::Hot => &self.hot_client,
            StorageTier::Cold => self.cold_client.as_ref().ok_or(anyhow::anyhow!("Cold storage not configured"))?,
        };
        
        let bucket = match tier {
            StorageTier::Hot => &self.hot_bucket,
            StorageTier::Cold => &self.cold_bucket,
        };
        
        client.put_object(bucket, &object_key, Bytes::copy_from_slice(data), None, None, None).await?;
        
        if tier == StorageTier::Hot {
            self.local_cache.write().await.insert(key.to_string(), data.to_vec());
        }
        
        info!("Stored object: {} ({} bytes, tier: {:?})", object_key, data.len(), tier);
        Ok(object_key)
    }
    
    pub async fn get(&self, key: &str, tier: StorageTier) -> Result<Vec<u8>> {
        if tier == StorageTier::Hot {
            if let Some(cached) = self.local_cache.read().await.get(key) {
                return Ok(cached.clone());
            }
        }
        
        let client = match tier {
            StorageTier::Hot => &self.hot_client,
            StorageTier::Cold => self.cold_client.as_ref().ok_or(anyhow::anyhow!("Cold storage not configured"))?,
        };
        
        let bucket = match tier {
            StorageTier::Hot => &self.hot_bucket,
            StorageTier::Cold => &self.cold_bucket,
        };
        
        let resp = client.get_object(bucket, key).send().await?;
        let data = resp.bytes().await?.to_vec();
        
        if tier == StorageTier::Hot {
            self.local_cache.write().await.insert(key.to_string(), data.clone());
        }
        
        Ok(data)
    }
    
    pub async fn delete(&self, key: &str, tier: StorageTier) -> Result<()> {
        let client = match tier {
            StorageTier::Hot => &self.hot_client,
            StorageTier::Cold => self.cold_client.as_ref().ok_or(anyhow::anyhow!("Cold storage not configured"))?,
        };
        
        let bucket = match tier {
            StorageTier::Hot => &self.hot_bucket,
            StorageTier::Cold => &self.cold_bucket,
        };
        
        client.delete_object(bucket, key).await?;
        self.local_cache.write().await.remove(key);
        
        Ok(())
    }
    
    pub async fn list(&self, prefix: &str, tier: StorageTier) -> Result<Vec<String>> {
        let client = match tier {
            StorageTier::Hot => &self.hot_client,
            StorageTier::Cold => self.cold_client.as_ref().ok_or(anyhow::anyhow!("Cold storage not configured"))?,
        };
        
        let bucket = match tier {
            StorageTier::Hot => &self.hot_bucket,
            StorageTier::Cold => &self.cold_bucket,
        };
        
        let mut objects = Vec::new();
        let mut continuation_token = None;
        
        loop {
            let resp = client.list_objects_v2(bucket)
                .prefix(prefix)
                .continuation_token(continuation_token)
                .send()
                .await?;
            
            for obj in resp.contents {
                objects.push(obj.key);
            }
            
            if let Some(token) = resp.next_continuation_token {
                continuation_token = Some(token);
            } else {
                break;
            }
        }
        
        Ok(objects)
    }
    
    pub async fn tier_object(&self, key: &str, from: StorageTier, to: StorageTier) -> Result<()> {
        let data = self.get(key, from).await?;
        self.put(key, &data, to).await?;
        self.delete(key, from).await?;
        info!("Tiered object {} from {:?} to {:?}", key, from, to);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum StorageTier {
    Hot,
    Cold,
}

mod hex {
    pub fn encode(data: &[u8]) -> String {
        data.iter().map(|b| format!("{:02x}", b)).collect()
    }
}