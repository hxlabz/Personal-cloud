use hxnet_common::*;
use super::bridges::{DeviceBridge, BridgedDevice};
use anyhow::Result;

pub struct BleBridge {
    adapter: Option<ble_peripheral::Adapter>,
}

impl BleBridge {
    pub async fn new() -> Result<Self> {
        Ok(Self { adapter: None })
    }
}

#[async_trait::async_trait]
impl DeviceBridge for BleBridge {
    async fn start(&self) -> Result<()> {
        info!("BLE bridge started (stub)");
        Ok(())
    }
    
    async fn discover(&self) -> Result<Vec<BridgedDevice>> {
        Ok(vec![
            BridgedDevice {
                device_id: "ble-heartrate-001".into(),
                protocol: "ble".into(),
                name: "Heart Rate Monitor".into(),
                capabilities: vec!["heart_rate".into(), "battery".into()],
            },
            BridgedDevice {
                device_id: "ble-temp-001".into(),
                protocol: "ble".into(),
                name: "Temperature Tag".into(),
                capabilities: vec!["temperature".into(), "battery".into()],
            },
        ])
    }
    
    async fn pair(&self, pairing_code: String) -> Result<()> {
        info!("Pairing BLE device with code: {}", pairing_code);
        Ok(())
    }
    
    async fn control(&self, device_id: &str, command: serde_json::Value) -> Result<serde_json::Value> {
        info!("BLE control {}: {}", device_id, command);
        Ok(serde_json::json!({ "status": "ok" }))
    }
}

mod ble_peripheral {
    pub struct Adapter;
}