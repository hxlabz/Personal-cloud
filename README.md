# HXNet — Universal Personal Device Fabric

A user-owned universal device fabric that connects heterogeneous personal devices and exposes their available compute, storage, networking, sensors, interfaces, and services through a common capability-aware infrastructure, allowing applications and AI agents to dynamically use the entire personal environment as one coordinated system.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              HXNet Fabric                                    │
├─────────────────────────────────────────────────────────────────────────────┤
│  IDENTITY & TRUST LAYER                                                      │
│  ├── Root CA (user-controlled, offline-capable)                             │
│  ├── Device Certificates (Ed25519, auto-rotated)                            │
│  ├── Passkey/WebAuthn Onboarding (user authorizes each device)              │
│  ├── Capability-Based Permissions (OPA policies, per-capability grants)     │
│  └── Compromise Isolation (revocation lists, automatic quarantine)          │
├─────────────────────────────────────────────────────────────────────────────┤
│  CONTROL PLANE (runs on 1-3 always-on Full Nodes)                          │
│  ├── Capability Registry          ← "What can each device do?"              │
│  ├── Node Registry                ← "Which devices are online?"             │
│  ├── Policy Engine                ← "Who can access what?"                  │
│  ├── Hierarchical Scheduler       ← "Where should this workload run?"       │
│  ├── Resource Manager             ← "Track capacity, leases, health"        │
│  ├── Telemetry & Event Bus        ← "Metrics, logs, traces, alerts"         │
│  └── Distributed State (etcd/CRDT) ← "Consensus only where needed"         │
├─────────────────────────────────────────────────────────────────────────────┤
│  DATA PLANE                                                                    │
│  ├── QUIC + HTTP/3 + WebRTC      ← Primary transports                       │
│  ├── WireGuard (via Headscale)   ← Encrypted mesh, NAT traversal            │
│  ├── mDNS/DNS-SD + libp2p DHT    ← Local + remote discovery                 │
│  └── DERP Relays / TURN          ← Fallback for extreme NAT                 │
├─────────────────────────────────────────────────────────────────────────────┤
│  DEVICE CLASSES                                                                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐        │
│  │ Full Node   │  │ Edge Node   │  │ Lightweight │  │ Gateway     │        │
│  │ (K3s)       │  │ (Agent)     │  │ Node        │  │ Node        │        │
│  │ Desktop/    │  │ Phone/      │  │ Watch/      │  │ Router/     │        │
│  │ Server/     │  │ Tablet/     │  │ Sensor/     │  │ Home Hub/   │        │
│  │ Laptop/NAS  │  │ NAS         │  │ Embedded    │  │ Bridge      │        │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘        │
│         │                │                │                │                │
│         ▼                ▼                ▼                ▼                │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                    HXNet Node Agent (per device class)               │   │
│  │  ┌───────────┐ ┌─────────────┐ ┌──────────────┐ ┌────────────────┐  │   │
│  │  │ Discovery │ │ Capability  │ │ Runtime Mgr  │ │ Resource Mon   │  │   │
│  │  │  (mDNS,   │ │  Manager    │ │ (WASM, cont, │ │ (CPU, GPU,     │  │   │
│  │  │  DHT,     │ │  (advertise,│ │  native,     │ │  RAM, disk,    │  │   │
│  │  │  DERP)    │ │  lease)     │ │  platform)   │ │  net, battery) │  │   │
│  │  └───────────┘ └─────────────┘ └──────────────┘ └────────────────┘  │   │
│  │  ┌─────────────────────┐ ┌─────────────────────────────────────┐   │   │
│  │  │ Security Manager    │ │ Network Transport                   │   │   │
│  │  │ (mTLS, attestation, │ │ (QUIC, WebRTC, WireGuard,           │   │   │
│  │  │  capability tokens) │ │  WebSocket, TCP fallback)           │   │   │
│  │  └─────────────────────┘ └─────────────────────────────────────┘   │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Components

| Component | Language | Purpose |
|-----------|----------|---------|
| **control-plane** | Rust | Capability registry, scheduler, node registry, policy engine |
| **agent** | Rust | Node agent for Full/Edge/Lightweight nodes (WASM runtime, discovery, health) |
| **gateway** | Rust | Bridges for Matter, Thread, BLE, Zigbee devices |
| **wasm** | Rust | WASM Component Model runtime (wasmtime/wasmedge) |
| **storage** | Rust | Tiered object storage client (MinIO hot + Ceph cold) |
| **identity** | Rust | WebAuthn/Passkeys, device attestation, certificate management |
| **cli** | Rust | Command-line interface for fabric management |

## Quick Start

### Prerequisites
- Docker & Docker Compose
- Rust 1.75+ (for building from source)
- PostgreSQL 16+, etcd 3.5+, MinIO, Headscale

### Deploy with Docker Compose (Development)

```bash
cd deploy
docker-compose up -d
```

This starts:
- PostgreSQL (control plane database)
- etcd (distributed state)
- MinIO hot + cold (tiered object storage)
- Headscale (self-hosted Tailscale/VPN mesh)
- Control plane API (port 8080)
- Full node agent (port 8081)
- Edge node agent (port 8083)
- Gateway (port 8083)
- Identity service (port 8084)
- Prometheus (port 9090)
- Grafana (port 3000)

### Deploy with Terraform (Production)

```bash
cd deploy/terraform
terraform init
terraform apply
```

### Build from Source

```bash
cd hxnet
cargo build --release --workspace
```

## Device Onboarding

```bash
# Start device registration
hxnet identity register --user-id <uuid> --device-name "My Laptop"

# Scan QR code with authenticator app (passkey)
# Complete registration
hxnet identity complete --registration-id <uuid>

# Device automatically joins fabric via Headscale
# Capabilities advertised to control plane
```

## Workload Submission

Create a workload manifest (`workload.json`):

```json
{
  "workload_id": "auto-generated",
  "name": "ai-inference",
  "version": "1.0.0",
  "format": "wasm_component",
  "required_capabilities": [
    { "category": "compute", "name": "gpu", "version": "1.0", "attributes": { "vendor": "nvidia" } },
    { "category": "storage", "name": "local", "version": "1.0", "attributes": { "free_gb": "10" } }
  ],
  "inputs": [
    { "name": "model", "data_type": "application/octet-stream", "schema": {} },
    { "name": "input", "data_type": "application/json", "schema": {} }
  ],
  "outputs": [
    { "name": "result", "data_type": "application/json", "schema": {} }
  ],
  "metadata": {}
}
```

Submit workload:

```bash
hxnet workload submit --manifest workload.json
```

## Capability Query

```bash
# Query all GPU capabilities
hxnet capability query --category compute

# Watch for capability changes
hxnet capability watch --category camera
```

## Storage Operations

```bash
# Store in hot tier (MinIO)
hxnet storage put --key my-model.bin --file ./model.bin --tier hot

# Store in cold tier (Ceph)
hxnet storage put --key backup.tar --file ./backup.tar --tier cold

# Tier object from hot to cold
hxnet storage tier --key my-model.bin --from hot --to cold
```

## Federation

```bash
# Invite another user's fabric
hxnet fabric invite --fabric-id <uuid> --user-email friend@example.com --permissions '{"read": true, "write": false}'

# Accept invitation
hxnet fabric accept --invitation-id <uuid>
```

## Monitoring

- **Grafana**: http://localhost:3000 (admin/admin)
- **Prometheus**: http://localhost:9090
- **Metrics**: Each service exposes `/metrics` on its metrics port

## Configuration

Environment variables for each service:

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `postgres://hxnet:hxnet@localhost/hxnet` | PostgreSQL connection |
| `ETCD_ENDPOINTS` | `http://localhost:2379` | etcd cluster endpoints |
| `MINIO_HOT_ENDPOINT` | `http://localhost:9000` | Hot storage endpoint |
| `MINIO_COLD_ENDPOINT` | `http://localhost:9003` | Cold storage endpoint |
| `HEADSCALE_URL` | `http://localhost:8080` | Headscale server URL |
| `RP_ID` | `hxnet.local` | WebAuthn relying party ID |
| `RP_ORIGIN` | `https://hxnet.local` | WebAuthn relying party origin |
| `BIND_ADDR` | `0.0.0.0:8080` | Service bind address |
| `METRICS_ADDR` | `0.0.0.0:9090` | Metrics bind address |

## Security Model

- **Device Identity**: Ed25519 keys generated in TPM/Secure Enclave
- **Onboarding**: WebAuthn/Passkeys with hardware attestation
- **Transport**: QUIC with TLS 1.3, WireGuard mesh via Headscale
- **Authorization**: OPA/Rego policies, capability-based tokens
- **Revocation**: Immediate certificate revocation, Tailscale key rotation

## Development

### Running Tests

```bash
cargo test --workspace
```

### Adding a New Device Class

1. Add `NodeClass` variant in `common/src/lib.rs`
2. Implement `CapabilityManager::populate_default_capabilities` for the class
3. Create agent binary with appropriate feature flags
4. Add deployment configuration

### Adding a New Gateway Bridge

1. Implement `DeviceBridge` trait in `gateway/src/bridges/mod.rs`
2. Add bridge to `BridgeManager::new()`
3. Define capability mappings for bridged devices

## License

MIT