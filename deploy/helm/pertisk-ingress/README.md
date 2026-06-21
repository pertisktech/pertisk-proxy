# pertisk-ingress Helm Chart

Deploy **pertisk-proxy-ingress** as a Kubernetes Ingress + Gateway API controller.

## Prerequisites

- Kubernetes 1.28+
- For Gateway API mode: install [Gateway API CRDs](https://gateway-api.sigs.k8s.io/guides/) before enabling `gatewayApi.enabled=true`

## Install

```bash
helm upgrade --install pertisk-ingress ./deploy/helm/pertisk-ingress \
  -n pertisk-proxy --create-namespace
```

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
