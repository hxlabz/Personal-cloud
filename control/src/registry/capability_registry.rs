use hxnet_common::*;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error};
use anyhow::Result;
use uuid::Uuid;
use chrono::{DateTime, Utc};

pub struct CapabilityRegistry {
    pool: PgPool,
    cache: Arc<RwLock<HashMap<Uuid, CapabilityDescriptor>>>,
    etcd: Option<etcd_client::Client>,
}

impl CapabilityRegistry {
    pub async fn new(database_url: &str, etcd_endpoints: Option<Vec<String>>) -> Result<Self> {
        let pool = PgPool::connect(database_url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        
        let etcd = if let Some(endpoints) = etcd_endpoints {
            Some(etcd_client::Client::connect(endpoints, None).await?)
        } else {
            None
        };
        
        Ok(Self {
            pool,
            cache: Arc::new(RwLock::new(HashMap::new())),
            etcd,
        })
    }

    pub async fn advertise(&self, descriptor: CapabilityDescriptor) -> Result<AdvertiseResponse> {
        let node_id = descriptor.node_id;
        let expires_at = chrono::Utc::now().timestamp() + 3600;
        
        let mut tx = self.pool.begin().await?;
        
        sqlx::query!(
            r#"
            INSERT INTO node_capabilities (node_id, node_class, version, descriptor, expires_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (node_id) DO UPDATE SET
                node_class = EXCLUDED.node_class,
                version = EXCLUDED.version,
                descriptor = EXCLUDED.descriptor,
                expires_at = EXCLUDED.expires_at,
                updated_at = EXCLUDED.updated_at
            "#,
            node_id,
            descriptor.node_class as i32,
            descriptor.version,
            serde_json::to_value(&descriptor)?,
            DateTime::from_timestamp(expires_at, 0).unwrap(),
            Utc::now()
        )
        .execute(&mut *tx)
        .await?;
        
        tx.commit().await?;
        
        self.cache.write().await.insert(node_id, descriptor.clone());
        
        if let Some(ref etcd) = self.etcd {
            let key = format!("/hxnet/capabilities/{}", node_id);
            etcd.put(key, serde_json::to_vec(&descriptor)?, None).await?;
        }
        
        info!("Capability advertised for node: {}", node_id);
        
        Ok(AdvertiseResponse {
            accepted: true,
            lease_id: Uuid::new_v4(),
            expires_at,
        })
    }

    pub async fn query(&self, request: QueryRequest) -> Result<QueryResponse> {
        let mut query = "SELECT descriptor FROM node_capabilities WHERE expires_at > $1".to_string();
        let mut params: Vec<Box<dyn sqlx::Encode<'_, sqlx::Postgres> + Send + Sync>> = vec![Box::new(Utc::now())];
        
        if !request.categories.is_empty() {
            query.push_str(" AND EXISTS (SELECT 1 FROM jsonb_each_text(descriptor->'capabilities') AS cap(cat, val) WHERE cap.cat = ANY($2))");
            params.push(Box::new(request.categories.clone()));
        }
        
        if request.node_class != NodeClass::Full {
            query.push_str(" AND node_class = $3");
            params.push(Box::new(request.node_class as i32));
        }
        
        let rows = sqlx::query_with(&query, sqlx::postgres::PgArguments::default())
            .fetch_all(&self.pool)
            .await?;
        
        let mut descriptors = Vec::new();
        for row in rows {
            let descriptor: CapabilityDescriptor = row.get("descriptor");
            if self.matches_query(&descriptor, &request) {
                descriptors.push(descriptor);
            }
        }
        
        Ok(QueryResponse { descriptors })
    }

    fn matches_query(&self, descriptor: &CapabilityDescriptor, request: &QueryRequest) -> bool {
        if !request.categories.is_empty() {
            let has_category = request.categories.iter().any(|cat| descriptor.capabilities.contains_key(cat));
            if !has_category {
                return false;
            }
        }
        
        for (attr_key, attr_val) in &request.attributes {
            let found = descriptor.capabilities.values().any(|cat| {
                cat.capabilities.values().any(|cap| cap.attributes.get(attr_key) == Some(attr_val))
            });
            if !found {
                return false;
            }
        }
        
        true
    }

    pub async fn watch(&self, request: WatchRequest) -> Result<tokio::sync::mpsc::Receiver<CapabilityEvent>> {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        
        if let Some(mut etcd) = self.etcd.clone() {
            let categories = request.categories.clone();
            tokio::spawn(async move {
                let mut stream = etcd.watch("/hxnet/capabilities/", Some(etcd_client::WatchOptions::new().with_prefix())).await.unwrap();
                while let Some(resp) = stream.message().await.unwrap() {
                    for event in resp.events() {
                        if let Some(kv) = event.kv() {
                            if let Ok(descriptor) = serde_json::from_slice::<CapabilityDescriptor>(kv.value()) {
                                if categories.is_empty() || categories.iter().any(|c| descriptor.capabilities.contains_key(c)) {
                                    let _ = tx.send(CapabilityEvent {
                                        type_: CapabilityEventType::Updated,
                                        descriptor,
                                    }).await;
                                }
                            }
                        }
                    }
                }
            });
        }
        
        Ok(rx)
    }

    pub async fn get_node_health(&self, node_id: Uuid) -> Result<Option<HealthStatus>> {
        let row = sqlx::query!(
            "SELECT health_data FROM node_health WHERE node_id = $1 AND updated_at > NOW() - INTERVAL '30 seconds'",
            node_id
        )
        .fetch_optional(&self.pool)
        .await?;
        
        row.map(|r| serde_json::from_value(r.health_data).ok()).flatten()
    }

    pub async fn update_health(&self, health: HealthStatus) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO node_health (node_id, health_data, updated_at)
            VALUES ($1, $2, $3)
            ON CONFLICT (node_id) DO UPDATE SET
                health_data = EXCLUDED.health_data,
                updated_at = EXCLUDED.updated_at
            "#,
            health.node_id,
            serde_json::to_value(&health)?,
            Utc::now()
        )
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }

    pub async fn revoke_node(&self, node_id: Uuid) -> Result<()> {
        sqlx::query!("DELETE FROM node_capabilities WHERE node_id = $1", node_id)
            .execute(&self.pool)
            .await?;
        
        self.cache.write().await.remove(&node_id);
        
        if let Some(ref etcd) = self.etcd {
            let key = format!("/hxnet/capabilities/{}", node_id);
            etcd.delete(key, None).await?;
        }
        
        info!("Node revoked: {}", node_id);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AdvertiseResponse {
    pub accepted: bool,
    pub lease_id: Uuid,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct QueryRequest {
    pub categories: Vec<String>,
    pub attributes: HashMap<String, String>,
    pub node_class: NodeClass,
}

#[derive(Debug, Clone)]
pub struct QueryResponse {
    pub descriptors: Vec<CapabilityDescriptor>,
}

#[derive(Debug, Clone)]
pub struct WatchRequest {
    pub categories: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CapabilityEvent {
    pub type_: CapabilityEventType,
    pub descriptor: CapabilityDescriptor,
}

#[derive(Debug, Clone, Copy)]
pub enum CapabilityEventType {
    Added,
    Updated,
    Removed,
    Expired,
}