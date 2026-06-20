# pertisk-proxy

High-performance reverse proxy built on [Pingora](https://github.com/cloudflare/pingora) (HTTP/1.1 + HTTP/2) and [Quiche](https://github.com/cloudflare/quiche) / [tokio-quiche](https://github.com/cloudflare/quiche/tree/master/tokio-quiche) (HTTP/3).

Run it in one of four modes — each mode defines **what the app does**, not which config file format you paste in.

## Operating modes

| Mode | `MODE` | What it does |
|------|--------|--------------|
| **Ingress** | `ingress` | Kubernetes Ingress controller — watches `Ingress` resources |
| **Nginx** | `nginx` | Static reverse proxy (nginx-like): manual TLS, no hot reload by default |
| **Caddy** | `caddy` | Simple reverse proxy (caddy-like): automatic HTTP→HTTPS redirect |
| **Traefik** | `traefik` | Dynamic reverse proxy (traefik-like): hot reload, middleware, API dashboard |

Proxy modes (`nginx`, `caddy`, `traefik`) load routes from a **pertisk routes file** (YAML/JSON), not from nginx.conf / Caddyfile / traefik.yaml.

```
                    ┌─────────────────────────────────────┐
                    │         pertisk-proxy engine        │
                    │   Pingora (HTTP/1+2) + Quiche (H3)  │
                    └─────────────────────────────────────┘
                          ▲              ▲              ▲
              ingress     │   nginx      │   caddy      │  traefik
              mode        │   mode       │   mode       │  mode
                          │              │              │
                   K8s Ingress      static routes   auto HTTPS    hot reload
                   watcher          file            redirect      + middleware
```

## Quick start

```bash
cargo build --release
```

### Ingress mode (Kubernetes)

```bash
export MODE=ingress
export INGRESS_CLASS=pertisk
kubectl apply -f deploy/kubernetes.yaml
```

### Nginx mode (static reverse proxy)

```bash
export MODE=nginx
export ROUTES_CONFIG=./config/examples/routes.yaml
export TLS_CERT_PATH=./cert.pem
export TLS_KEY_PATH=./key.pem
export ENABLE_H3=false
./target/release/pertisk-proxy
```

### Caddy mode (automatic HTTPS)

```bash
export MODE=caddy
export ROUTES_CONFIG=./config/examples/routes.yaml
export TLS_CERT_PATH=./cert.pem
export TLS_KEY_PATH=./key.pem
# HTTP :80 redirects to HTTPS :443 automatically (AUTO_HTTPS=true by default)
./target/release/pertisk-proxy
```

### Traefik mode (dynamic + middleware)

```bash
export MODE=traefik
export ROUTES_CONFIG=./config/examples/routes.yaml
export ROUTES_WATCH=true          # hot reload (default for traefik)
export TLS_CERT_PATH=./cert.pem
export TLS_KEY_PATH=./key.pem
./target/release/pertisk-proxy

# Dashboard-style API
curl http://localhost/api/http/routers
curl http://localhost/api/overview
```

## Routes file format

`config/examples/routes.yaml`:

```yaml
routes:
  - host: app.example.com
    path: /api
    path_type: prefix        # prefix | exact
    upstream: http://backend:8080
    middlewares:             # traefik mode only
      - type: stripPrefix
        prefix: /api
      - type: requestHeaders
        headers:
          X-Custom: value
      - type: responseHeaders
        headers:
          X-Frame-Options: DENY
```

## Mode comparison

| Feature | Ingress | Nginx | Caddy | Traefik |
|---------|---------|-------|-------|---------|
| Route source | K8s Ingress | routes file | routes file | routes file |
| Hot reload | yes (watch) | no (default) | yes (default) | yes (default) |
| Auto HTTPS redirect | — | — | yes | — |
| Middleware | — | — | — | yes |
| API dashboard | — | — | — | `/api/*` |
| Default ports | 8080/8443 | 80/443 | 80/443 | 80/443 |

## Configuration

| Variable | Description |
|----------|-------------|
| `MODE` | `ingress`, `nginx`, `caddy`, or `traefik` |
| `ROUTES_CONFIG` | Routes file path (required for proxy modes) |
| `ROUTES_WATCH` | Reload routes on change (default: off for nginx, on for caddy/traefik) |
| `AUTO_HTTPS` | HTTP→HTTPS redirect (default: on for caddy only) |
| `LISTEN_HTTP` / `LISTEN_HTTPS` / `LISTEN_H3_UDP` | Listen addresses |
| `TLS_CERT_PATH` / `TLS_KEY_PATH` | TLS certificate and key |
| `ENABLE_H3` | Enable HTTP/3 (default: true) |
| `INGRESS_CLASS` | Ingress mode: filter by class name |

Health: `/healthz`, `/readyz`

## License

Apache-2.0
