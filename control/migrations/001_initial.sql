-- HXNet Control Plane Database Migrations
-- Migration 001: Initial schema

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- Node capabilities table
CREATE TABLE node_capabilities (
    node_id UUID PRIMARY KEY,
    node_class INTEGER NOT NULL,
    version VARCHAR(64) NOT NULL,
    descriptor JSONB NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_node_capabilities_expires ON node_capabilities(expires_at);
CREATE INDEX idx_node_capabilities_class ON node_capabilities(node_class);
CREATE INDEX idx_node_capabilities_descriptor_gin ON node_capabilities USING GIN(descriptor);

-- Node health table
CREATE TABLE node_health (
    node_id UUID PRIMARY KEY,
    health_data JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Workload placements table
CREATE TABLE workload_placements (
    workload_id UUID PRIMARY KEY,
    node_id UUID NOT NULL REFERENCES node_capabilities(node_id) ON DELETE CASCADE,
    placement_data JSONB NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'placed',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_workload_placements_node ON workload_placements(node_id);
CREATE INDEX idx_workload_placements_status ON workload_placements(status);

-- Device identity table
CREATE TABLE device_identities (
    device_id UUID PRIMARY KEY,
    user_id UUID NOT NULL,
    node_id UUID REFERENCES node_capabilities(node_id) ON DELETE SET NULL,
    credential_id BYTEA NOT NULL,
    public_key BYTEA NOT NULL,
    sign_count INTEGER NOT NULL DEFAULT 0,
    attested BOOLEAN NOT NULL DEFAULT FALSE,
    cert_pem TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ
);

CREATE INDEX idx_device_identities_user ON device_identities(user_id);
CREATE INDEX idx_device_identities_node ON device_identities(node_id);
CREATE INDEX idx_device_identities_credential ON device_identities(credential_id);

-- Users table
CREATE TABLE users (
    user_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username VARCHAR(255) UNIQUE NOT NULL,
    display_name VARCHAR(255),
    webauthn_user_id BYTEA UNIQUE NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Fabrics table (for federation)
CREATE TABLE fabrics (
    fabric_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    public_key BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Fabric members (devices in a fabric)
CREATE TABLE fabric_members (
    fabric_id UUID NOT NULL REFERENCES fabrics(fabric_id) ON DELETE CASCADE,
    node_id UUID NOT NULL REFERENCES node_capabilities(node_id) ON DELETE CASCADE,
    role VARCHAR(32) NOT NULL DEFAULT 'member',
    joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (fabric_id, node_id)
);

-- Federation invitations
CREATE TABLE federation_invitations (
    invitation_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    from_fabric_id UUID NOT NULL REFERENCES fabrics(fabric_id) ON DELETE CASCADE,
    to_fabric_id UUID REFERENCES fabrics(fabric_id) ON DELETE CASCADE,
    to_user_id UUID REFERENCES users(user_id) ON DELETE CASCADE,
    status VARCHAR(32) NOT NULL DEFAULT 'pending',
    permissions JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

-- Storage objects metadata
CREATE TABLE storage_objects (
    object_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    fabric_id UUID NOT NULL REFERENCES fabrics(fabric_id) ON DELETE CASCADE,
    bucket VARCHAR(255) NOT NULL,
    key VARCHAR(1024) NOT NULL,
    size_bytes BIGINT NOT NULL,
    hash_blake3 BYTEA NOT NULL,
    tier VARCHAR(16) NOT NULL DEFAULT 'hot',
    content_type VARCHAR(255),
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_storage_objects_fabric ON storage_objects(fabric_id);
CREATE INDEX idx_storage_objects_bucket_key ON storage_objects(bucket, key);
CREATE INDEX idx_storage_objects_tier ON storage_objects(tier);

-- Updated_at trigger function
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

CREATE TRIGGER update_node_capabilities_updated_at BEFORE UPDATE ON node_capabilities
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_workload_placements_updated_at BEFORE UPDATE ON workload_placements
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_device_identities_updated_at BEFORE UPDATE ON device_identities
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_users_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_fabrics_updated_at BEFORE UPDATE ON fabrics
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_storage_objects_updated_at BEFORE UPDATE ON storage_objects
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();