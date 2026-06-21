# pertisk-proxy

Reverse proxy and Kubernetes Ingress controller built on [Pingora](https://github.com/cloudflare/pingora) (HTTP/1.1 + HTTP/2) and [Quiche](https://github.com/cloudflare/quiche) / [tokio-quiche](https://github.com/cloudflare/quiche/tree/master/tokio-quiche) (HTTP/3).

Structured like [pertisk-rproxy](https://github.com/pertisktech/pertisk-rproxy): **two binaries**, separate configs, and packaging via `build/package.sh`.

## Binaries

| Binary | Mode | Purpose |
|--------|------|---------|
| `pertisk-proxy` | **proxy** | Standalone reverse proxy — routes from `ROUTES_CONFIG` |
| `pertisk-proxy-ingress` | **ingress** | Kubernetes Ingress controller — watches `Ingress` resources |

Both share the same Pingora + HTTP/3 data plane.

## Quick start

```bash
cargo build --release --bin pertisk-proxy
cargo build --release --bin pertisk-proxy-ingress --features ingress
```

### Proxy mode

```bash
make run
# or
export ROUTES_CONFIG=./config/examples/routes.yaml
export ENABLE_H3=false
cargo run --bin pertisk-proxy
```

Config: `/etc/pertisk-proxy/pertisk-proxy.conf` (see `build/pertisk-proxy.conf`)

### Ingress mode

```bash
make run-ingress
# or
export INGRESS_CLASS=pertisk
cargo run --bin pertisk-proxy-ingress --features ingress
```

Config: `/etc/pertisk-proxy/pertisk-proxy-ingress.conf` (see `build/pertisk-proxy-ingress.conf`)

### Kubernetes

```bash
# Multi-arch build + push + Helm deploy
make deploy-ingress VERSION=0.1.0

# Or step by step:
make docker-ingress-multi VERSION=0.1.0
make deploy-ingress-helm VERSION=0.1.0

# Local single-arch image only:
make docker-ingress VERSION=0.1.0
```

## Packaging

Like pertisk-rproxy's `build/package.sh`:

```bash
./build/package.sh amd64              # both binaries
./build/package.sh amd64 0.1.0 proxy  # proxy only
./build/package.sh amd64 0.1.0 ingress # ingress only
make package-amd64
```

Installs:
- `/usr/bin/pertisk-proxy` + `pertisk-proxy.service` + `pertisk-proxy.conf`
- `/usr/bin/pertisk-proxy-ingress` + `pertisk-proxy-ingress.service` + `pertisk-proxy-ingress.conf`

## Routes file (proxy mode)

`config/examples/routes.yaml`:

```yaml
routes:
  - host: app.example.com
    path: /api
    path_type: prefix
    upstream: http://backend:8080
```

## Configuration

### Proxy (`pertisk-proxy`)

| Variable | Default | Description |
|----------|---------|-------------|
| `ROUTES_CONFIG` | — | Routes YAML/JSON path (required) |
| `ROUTES_WATCH` | `true` | Hot reload routes file |
| `AUTO_HTTPS` | `false` | HTTP→HTTPS redirect |
| `LISTEN_HTTP` | `0.0.0.0:80` | HTTP listen address |
| `LISTEN_HTTPS` | `0.0.0.0:443` | HTTPS listen address |
| `PERTISK_PROXY_MODE` | `auto` | Runtime: `auto`, `standard`, `performance` |

### Ingress (`pertisk-proxy-ingress`)

| Variable | Default | Description |
|----------|---------|-------------|
| `INGRESS_CLASS` | — | Filter by `ingressClassName` |
| `WATCH_ALL_NAMESPACES` | `true` | Watch cluster-wide |
| `LISTEN_HTTP` | `0.0.0.0:8080` | HTTP listen address |
| `PERTISK_INGRESS_MODE` | `auto` | Runtime tuning |

### Shared

| Variable | Description |
|----------|-------------|
| `TLS_CERT_PATH` / `TLS_KEY_PATH` | TLS certificate and key |
| `ENABLE_H3` | Enable HTTP/3 (default: true) |
| `LISTEN_H3_UDP` | HTTP/3 UDP listen address |

Health: `/healthz`, `/readyz`

## Project layout

```
src/
  main.rs                 # pertisk-proxy (proxy mode)
  bin/ingress.rs          # pertisk-proxy-ingress (ingress mode)
  config/                 # ProxyConfig / IngressConfig
  controller.rs           # Ingress watcher
  proxy/routes.rs         # Routes file loader
  server.rs               # Shared Pingora startup
  runtime.rs              # PERTISK_*_MODE tuning
build/
  package.sh              # DEB/RPM/tar packaging
  pertisk-proxy.conf
  pertisk-proxy-ingress.conf
deploy/                   # Kubernetes manifests
```

## License

Apache-2.0
