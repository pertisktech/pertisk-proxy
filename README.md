# pertisk-proxy

Kubernetes Ingress controller and reverse proxy built on [Pingora](https://github.com/cloudflare/pingora) (HTTP/1.1 + HTTP/2) and [Quiche](https://github.com/cloudflare/quiche) via [tokio-quiche](https://github.com/cloudflare/quiche/tree/master/tokio-quiche) (HTTP/3).

## Architecture

```
[Client] -- HTTP/1, HTTP/2 (TCP) --> [Pingora Proxy] -- HTTP --> [K8s Service]
         -- HTTP/3 (UDP/QUIC)  --> [Quiche/tokio-quiche] -- HTTP --> [K8s Service]
                                              ^
                                              |
                                    [kube-rs Ingress Watcher]
```

- **Pingora** terminates TCP TLS and proxies HTTP/1.1 and HTTP/2 to cluster backends.
- **tokio-quiche** listens on UDP for QUIC/HTTP/3, resolves routes via the same routing table, and forwards to backends over HTTP.
- **kube-rs** watches `Ingress` resources and atomically updates the shared route table (`ArcSwap`).

## Quick start

### Build locally

```bash
cargo build --release
```

### Run locally (without Kubernetes)

```bash
export ENABLE_H3=false
export LISTEN_HTTP=0.0.0.0:8080
./target/release/pertisk-proxy
```

### Run in Kubernetes

```bash
# Create namespace and TLS secret first
kubectl create namespace pertisk-proxy
kubectl -n pertisk-proxy create secret tls pertisk-proxy-tls \
  --cert=path/to/tls.crt --key=path/to/tls.key

# Deploy controller
kubectl apply -f deploy/kubernetes.yaml

# Create an Ingress
kubectl apply -f deploy/example-ingress.yaml
```

### Docker

```bash
docker build -t pertisk-proxy .
```

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `LISTEN_HTTP` | `0.0.0.0:8080` | HTTP/1 + HTTP/2 (cleartext) listen address |
| `LISTEN_HTTPS` | `0.0.0.0:8443` | HTTPS (HTTP/1 + HTTP/2) listen address |
| `LISTEN_H3_UDP` | `0.0.0.0:8443` | HTTP/3 QUIC UDP listen address |
| `ENABLE_H3` | `true` | Enable HTTP/3 listener |
| `TLS_CERT_PATH` | — | TLS certificate (required when H3 or HTTPS enabled) |
| `TLS_KEY_PATH` | — | TLS private key |
| `INGRESS_CLASS` | — | Only reconcile Ingresses with this `ingressClassName` |
| `WATCH_ALL_NAMESPACES` | `true` | Watch Ingresses cluster-wide |
| `WATCH_NAMESPACE` | — | Watch a single namespace when `WATCH_ALL_NAMESPACES=false` |
| `RUST_LOG` | `info` | Log level |

Health endpoints: `/healthz` and `/readyz`.

## Project layout

```
src/
  main.rs        # Entry point: Pingora server + background tasks
  proxy.rs       # Pingora ProxyHttp implementation
  router.rs      # Ingress → route table (ArcSwap)
  controller.rs  # kube-rs Ingress watcher
  h3/            # HTTP/3 server (tokio-quiche)
  config.rs      # Environment configuration
deploy/          # Kubernetes manifests
```

## License

Apache-2.0
