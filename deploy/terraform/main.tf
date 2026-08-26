terraform {
  required_version = ">= 1.5"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
    docker = {
      source  = "kreuzwerker/docker"
      version = "~> 3.0"
    }
    libvirt = {
      source  = "dmacvicar/libvirt"
      version = "~> 0.7"
    }
  }
}

variable "provider" {
  description = "Cloud provider: aws, libvirt, or docker"
  type        = string
  default     = "docker"
}

variable "region" {
  description = "AWS region"
  type        = string
  default     = "us-east-1"
}

variable "instance_type" {
  description = "Instance type for control plane"
  type        = string
  default     = "t3.medium"
}

variable "node_count" {
  description = "Number of edge nodes"
  type        = number
  default     = 3
}

locals {
  common_tags = {
    Project     = "hxnet"
    Environment = "production"
    ManagedBy   = "terraform"
  }
}

provider "aws" {
  region = var.region
}

provider "docker" {
  host = "unix:///var/run/docker.sock"
}

provider "libvirt" {
  uri = "qemu:///system"
}

resource "random_password" "postgres" {
  length  = 32
  special = false
}

resource "random_password" "minio" {
  length  = 32
  special = false
}

resource "docker_network" "hxnet" {
  name   = "hxnet"
  driver = "bridge"
}

resource "docker_volume" "postgres_data" {
  name = "hxnet_postgres_data"
}

resource "docker_volume" "etcd_data" {
  name = "hxnet_etcd_data"
}

resource "docker_volume" "minio_hot_data" {
  name = "hxnet_minio_hot_data"
}

resource "docker_volume" "minio_cold_data" {
  name = "hxnet_minio_cold_data"
}

resource "docker_volume" "headscale_data" {
  name = "hxnet_headscale_data"
}

resource "docker_volume" "prometheus_data" {
  name = "hxnet_prometheus_data"
}

resource "docker_volume" "grafana_data" {
  name = "hxnet_grafana_data"
}

resource "docker_container" "postgres" {
  name  = "hxnet-postgres"
  image = "postgres:16-alpine"
  
  env = [
    "POSTGRES_DB=hxnet",
    "POSTGRES_USER=hxnet",
    "POSTGRES_PASSWORD=${random_password.postgres.result}",
  ]
  
  ports {
    internal = 5432
    external = 5432
  }
  
  volumes {
    volume_name = docker_volume.postgres_data.name
    container_path = "/var/lib/postgresql/data"
  }
  
  networks_advanced {
    name = docker_network.hxnet.name
  }
  
  healthcheck {
    test     = ["CMD-SHELL", "pg_isready -U hxnet -d hxnet"]
    interval = "5s"
    timeout  = "5s"
    retries  = 5
  }
}

resource "docker_container" "etcd" {
  name  = "hxnet-etcd"
  image = "quay.io/coreos/etcd:v3.5.12"
  
  command = [
    "/usr/local/bin/etcd",
    "--name=hxnet-etcd",
    "--data-dir=/etcd-data",
    "--listen-client-urls=http://0.0.0.0:2379",
    "--advertise-client-urls=http://etcd:2379",
    "--listen-peer-urls=http://0.0.0.0:2380",
    "--initial-cluster=hxnet-etcd=http://etcd:2380",
    "--initial-cluster-token=hxnet-cluster",
    "--initial-cluster-state=new",
  ]
  
  ports {
    internal = 2379
    external = 2379
  }
  
  ports {
    internal = 2380
    external = 2380
  }
  
  volumes {
    volume_name = docker_volume.etcd_data.name
    container_path = "/etcd-data"
  }
  
  networks_advanced {
    name = docker_network.hxnet.name
  }
}

resource "docker_container" "minio_hot" {
  name  = "hxnet-minio-hot"
  image = "minio/minio:RELEASE.2024-01-16"
  
  command = ["server", "/data", "--console-address", ":9001"]
  
  env = [
    "MINIO_ROOT_USER=hxnet",
    "MINIO_ROOT_PASSWORD=${random_password.minio.result}",
  ]
  
  ports {
    internal = 9000
    external = 9000
  }
  
  ports {
    internal = 9001
    external = 9001
  }
  
  volumes {
    volume_name = docker_volume.minio_hot_data.name
    container_path = "/data"
  }
  
  networks_advanced {
    name = docker_network.hxnet.name
  }
  
  healthcheck {
    test     = ["CMD", "curl", "-f", "http://localhost:9000/minio/health/live"]
    interval = "30s"
    timeout  = "20s"
    retries  = 3
  }
}

resource "docker_container" "minio_cold" {
  name  = "hxnet-minio-cold"
  image = "minio/minio:RELEASE.2024-01-16"
  
  command = ["server", "/data", "--console-address", ":9002"]
  
  env = [
    "MINIO_ROOT_USER=hxnet",
    "MINIO_ROOT_PASSWORD=${random_password.minio.result}",
  ]
  
  ports {
    internal = 9000
    external = 9003
  }
  
  ports {
    internal = 9001
    external = 9004
  }
  
  volumes {
    volume_name = docker_volume.minio_cold_data.name
    container_path = "/data"
  }
  
  networks_advanced {
    name = docker_network.hxnet.name
  }
}

resource "docker_container" "headscale" {
  name  = "hxnet-headscale"
  image = "headscale/headscale:0.22.3"
  
  command = ["headscale", "serve"]
  
  env = [
    "HEADSCALE_SERVER_URL=http://headscale:8080",
    "HEADSCALE_DATABASE_TYPE=sqlite3",
    "HEADSCALE_DATABASE_URL=file:/data/headscale.db",
  ]
  
  ports {
    internal = 8080
    external = 8080
  }
  
  ports {
    internal = 9090
    external = 9090
  }
  
  volumes {
    volume_name = docker_volume.headscale_data.name
    container_path = "/data"
  }
  
  networks_advanced {
    name = docker_network.hxnet.name
  }
  
  depends_on = [docker_container.postgres, docker_container.etcd]
}

resource "docker_container" "control_plane" {
  name  = "hxnet-control-plane"
  image = "hxnet/control-plane:latest"
  
  env = [
    "DATABASE_URL=postgres://hxnet:${random_password.postgres.result}@postgres:5432/hxnet",
    "ETCD_ENDPOINTS=http://etcd:2379",
    "MINIO_HOT_ENDPOINT=http://minio-hot:9000",
    "MINIO_HOT_ACCESS_KEY=hxnet",
    "MINIO_HOT_SECRET_KEY=${random_password.minio.result}",
    "MINIO_HOT_BUCKET=hxnet-hot",
    "MINIO_COLD_ENDPOINT=http://minio-cold:9000",
    "MINIO_COLD_ACCESS_KEY=hxnet",
    "MINIO_COLD_SECRET_KEY=${random_password.minio.result}",
    "MINIO_COLD_BUCKET=hxnet-cold",
    "HEADSCALE_URL=http://headscale:8080",
    "BIND_ADDR=0.0.0.0:8080",
    "METRICS_ADDR=0.0.0.0:9090",
    "RUST_LOG=info",
  ]
  
  ports {
    internal = 8080
    external = 8080
  }
  
  ports {
    internal = 9090
    external = 9090
  }
  
  networks_advanced {
    name = docker_network.hxnet.name
  }
  
  depends_on = [
    docker_container.postgres,
    docker_container.etcd,
    docker_container.minio_hot,
    docker_container.headscale,
  ]
}

resource "docker_container" "agent_full" {
  name  = "hxnet-agent-full"
  image = "hxnet/agent:latest"
  
  env = [
    "NODE_CLASS=full",
    "CONTROL_PLANE_URL=http://control-plane:8080",
    "BIND_ADDR=0.0.0.0:8081",
    "METRICS_ADDR=0.0.0.0:9091",
    "RUST_LOG=info",
  ]
  
  ports {
    internal = 8081
    external = 8081
  }
  
  ports {
    internal = 8082
    external = 8082
    protocol = "udp"
  }
  
  ports {
    internal = 9091
    external = 9091
  }
  
  networks_advanced {
    name = docker_network.hxnet.name
  }
  
  depends_on = [docker_container.control_plane]
  
  privileged = true
}

resource "docker_container" "prometheus" {
  name  = "hxnet-prometheus"
  image = "prom/prometheus:v2.48.0"
  
  command = [
    "--config.file=/etc/prometheus/prometheus.yml",
    "--storage.tsdb.path=/prometheus",
  ]
  
  ports {
    internal = 9090
    external = 9090
  }
  
  volumes {
    volume_name = docker_volume.prometheus_data.name
    container_path = "/prometheus"
  }
  
  networks_advanced {
    name = docker_network.hxnet.name
  }
  
  depends_on = [docker_container.control_plane]
}

resource "docker_container" "grafana" {
  name  = "hxnet-grafana"
  image = "grafana/grafana:10.2.0"
  
  env = [
    "GF_SECURITY_ADMIN_USER=admin",
    "GF_SECURITY_ADMIN_PASSWORD=admin",
    "GF_USERS_ALLOW_SIGN_UP=false",
  ]
  
  ports {
    internal = 3000
    external = 3000
  }
  
  volumes {
    volume_name = docker_volume.grafana_data.name
    container_path = "/var/lib/grafana"
  }
  
  networks_advanced {
    name = docker_network.hxnet.name
  }
  
  depends_on = [docker_container.prometheus]
}

output "control_plane_url" {
  value = "http://localhost:8080"
}

output "grafana_url" {
  value = "http://localhost:3000"
}

output "postgres_password" {
  value     = random_password.postgres.result
  sensitive = true
}

output "minio_password" {
  value     = random_password.minio.result
  sensitive = true
}