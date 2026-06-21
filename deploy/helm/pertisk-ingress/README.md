# pertisk-ingress Helm Chart

Deploy **pertisk-proxy-ingress** as a Kubernetes Ingress + Gateway API controller.

## Prerequisites

- Kubernetes 1.28+
- For Gateway API mode: install [Gateway API CRDs](https://gateway-api.sigs.k8s.io/guides/) before enabling `gatewayApi.enabled=true`

## Install

**Helm only** (image must already exist in registry):

```bash
make deploy-ingress-helm VERSION=0.1.0
```

**Full pipeline** — build multi-arch image, push, deploy:

```bash
make deploy-ingress VERSION=0.1.0
```

**Docker only:**

```bash
make docker-ingress-multi VERSION=0.1.0   # linux/amd64 + linux/arm64 → registry
make docker-ingress VERSION=0.1.0         # local single-arch load
make docker-ingress-push VERSION=0.1.0    # single-arch push
```

Override registry: `HARBOR_INGRESS_IMAGE=my.registry/pertisk-proxy/ingress make deploy-ingress`

### Migrating from pertisk-rproxy

If install fails with `ClusterRole "pertisk-ingress" ... release-namespace must equal "pertisk-proxy": current value is "pertisk-rproxy"`, uninstall the old release first:

```bash
make uninstall-legacy-ingress-helm
# then
make deploy-ingress-helm
```

This chart uses `fullnameOverride: pertisk-proxy-ingress` so cluster-scoped RBAC does not collide with the old `pertisk-rproxy` chart.

## Modes

| Mode | Values | User creates |
|------|--------|--------------|
| Ingress only | `gatewayApi.enabled: false` | `Ingress` with `ingressClassName: pertisk` |
| Gateway API only | `ingressClassResource.enabled: false`, `gatewayApi.enabled: true` | `Gateway` + `HTTPRoute` |
| Both | `gatewayApi.enabled: true` | Either or both |

## Admin UI

Management API (viewer-only in ingress mode) is exposed on port **9080**. Set `adminIngress.enabled: true` to expose via an Ingress, or port-forward:

```bash
kubectl port-forward svc/pertisk-ingress 9080:9080 -n pertisk-proxy
```

Default auth (when `auth.createSecret: true`): `admin` / `admin` — change in values before production.

## Example Gateway API

See [gateway-api-example.yaml](./gateway-api-example.yaml).

## Key environment variables (set by chart)

- `PERTISK_INGRESS_CLASS` / `PERTISK_GATEWAY_CLASS`
- `PERTISK_GATEWAY_API_ENABLED`
- `PERTISK_MANAGEMENT_ADDR` — admin UI bind address
- `PERTISK_PASSWORD` — admin login (via Secret)
