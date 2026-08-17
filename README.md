# pertisk-proxy

Reverse proxy and Kubernetes Ingress controller built on [Pingora](https://github.com/cloudflare/pingora) (HTTP/1.1 + HTTP/2) and [Quiche](https://github.com/cloudflare/quiche) / [tokio-quiche](https://github.com/cloudflare/quiche/tree/master/tokio-quiche) (HTTP/3).

Structured like [pertisk-rproxy](https://github.com/pertisktech/pertisk-rproxy): **two binaries**, separate configs, and packaging via `build/package.sh`.

## Admin UI

Web console for sites, TLS, access control, WAF, certificates, DNS, logs, metrics, backup, and settings. The **Sites** list shows domain, protocol (HTTP/HTTPS), upstream, routes, and SSL status.

Default management API listen: `[::]:9080` (IPv4 + IPv6 dual-stack).

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

## GeoIP + WAF / bot / captcha

Edge filtering runs after route match: **GeoIP → captcha endpoints → WAF → bot score → challenge/block**.

### 1. Install GeoIP databases

Country lookup needs a MaxMind MMDB. ASN lookup accepts MaxMind `GeoLite2-ASN.mmdb` **or** iptoasn `ip2asn-combined.tsv`.

```bash
mkdir -p /var/lib/pertisk-proxy/geoip
cd /tmp

# Country (MaxMind GeoLite2)
wget https://cdn.jsdelivr.net/npm/geolite2-country/GeoLite2-Country.mmdb.gz
gzip -d GeoLite2-Country.mmdb.gz
cp GeoLite2-Country.mmdb /var/lib/pertisk-proxy/geoip/

# ASN (iptoasn TSV — no MaxMind license needed)
wget https://iptoasn.com/data/ip2asn-combined.tsv.gz
gzip -d ip2asn-combined.tsv.gz
cp ip2asn-combined.tsv /var/lib/pertisk-proxy/geoip/

ls -la /var/lib/pertisk-proxy/geoip/
```

Default search paths (override with env):

| Variable | Default candidates |
|----------|-------------------|
| `PERTISK_GEOIP_COUNTRY_DB` | `/var/lib/pertisk-proxy/geoip/GeoLite2-Country.mmdb`, `/usr/share/GeoIP/...` |
| `PERTISK_GEOIP_ASN_DB` | `GeoLite2-ASN.mmdb` or `ip2asn-combined.tsv` under the same dirs |

Optional: `PERTISK_CAPTCHA_SECRET` (or `PERTISK_AUTH_SIGNING_SECRET`) for stable captcha cookies across restarts.

### 2. Proxy mode (admin Sites → Advanced)

1. Install DBs on the host (path above).
2. Restart `pertisk-proxy`.
3. In Admin → **Sites** → edit site → **Advanced**, enable GeoIP / WAF / bot / captcha.

### 3. Ingress mode (Helm + annotations)

Mount DBs into controller pods, then enable per Ingress/HTTPRoute (Admin **Advanced** tab writes these annotations).

```yaml
# Helm values snippet
geoip:
  enabled: true
  hostPath: /var/lib/pertisk-proxy/geoip   # or existingClaim: geoip-pvc
  # optional overrides:
  # countryDb: /var/lib/pertisk-proxy/geoip/GeoLite2-Country.mmdb
  # asnDb: /var/lib/pertisk-proxy/geoip/ip2asn-combined.tsv
```

```bash
helm upgrade --install pertisk-proxy-ingress ./deploy/helm/pertisk-ingress \
  -n pertisk-proxy -f your-values.yaml
```

Example Ingress annotations (also settable from Admin → Ingress → Advanced):

```yaml
metadata:
  annotations:
    proxy.pertisk.tech/geoip-enabled: "true"
    proxy.pertisk.tech/geoip-allow-countries: "TH,US"
    proxy.pertisk.tech/geoip-deny-countries: "CN"
    proxy.pertisk.tech/geoip-allow-asns: "13335,AS15169"
    proxy.pertisk.tech/geoip-deny-asns: "12345"
    proxy.pertisk.tech/waf-enabled: "true"
    proxy.pertisk.tech/waf-builtin: "true"
    proxy.pertisk.tech/bot-enabled: "true"
    proxy.pertisk.tech/bot-challenge-score: "40"
    proxy.pertisk.tech/bot-block-score: "80"
    proxy.pertisk.tech/captcha-enabled: "true"
```

Challenge page: `/.pertisk/captcha`. Metrics: `pertisk_geoip_blocked_total`, `pertisk_waf_*`, `pertisk_bot_*`, `pertisk_captcha_*`.

## Packaging

Like pertisk-rproxy's `build/package.sh`:

```bash
./build/package.sh amd64              # both binaries
./build/package.sh amd64 0.1.0 proxy  # proxy only
./build/package.sh amd64 0.1.0 ingress # ingress only
make package-amd64
make package-helm VERSION=0.1.0       # Helm chart → release/*.tgz
make release-helm VERSION=0.1.0       # package + publish chart repo
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
| `PERTISK_GEOIP_COUNTRY_DB` | Country MMDB path |
| `PERTISK_GEOIP_ASN_DB` | ASN MMDB or `ip2asn-combined.tsv` path |
| `PERTISK_CAPTCHA_SECRET` | Captcha cookie signing secret |

Health: `/healthz`, `/readyz`

### Performance tuning

The admin dashboard reports effective runtime, connection-pool, CPU-affinity,
file-limit, and Linux network settings. See
[`docs/performance-tuning.md`](docs/performance-tuning.md) for production
defaults, systemd/sysctl setup, HTTP/3 offload, and benchmark guidance.

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
  geoip.rs                # Country/ASN lookup + policy
  security/               # WAF / bot / captcha
admin/                    # React admin UI (Sites, TLS, WAF, …)
docs/
  tunnel.md               # Reverse tunnel (local → VPS → Sites)
  geoip-asn.md
  performance-tuning.md
tunnel/                   # pertisk-tunnel-server + client (QUIC)
build/
  package.sh              # DEB/RPM/tar packaging
  pertisk-proxy.conf
  pertisk-proxy-ingress.conf
deploy/                   # Kubernetes manifests + Helm
```

## License

Apache-2.0

## Reverse tunnel

Expose a local app through this VPS: see [docs/tunnel.md](docs/tunnel.md).

```bash
make tunnel                          # build binaries
make package-tunnel VERSION=0.1.80   # RPM+DEB for server and client
make deploy-rpm-tunnel DEPLOY_HOST=user@vps VERSION=0.1.80
make deploy-rpm-tunnel-server DEPLOY_HOST=user@vps
make deploy-rpm-tunnel-client DEPLOY_HOST=user@laptop
```
