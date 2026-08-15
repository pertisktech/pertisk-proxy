# Install GeoIP DBs (WAF needs no DB)

**WAF / bot / captcha** — built-in, no download.

**GeoIP** needs `GeoLite2-Country.mmdb` + `ip2asn-combined.tsv` (or `GeoLite2-ASN.mmdb`).

---

## Talos / Omni (285h) — recommended

Talos nodes are immutable — **do not use hostPath**. Use Helm **download** mode (init container → emptyDir).

In `deploy/helm/pertisk-ingress/285/values.yaml`:

```yaml
geoip:
  enabled: true
  download:
    enabled: true

service:
  # Prefer Cluster on Cilium/Talos (Local often causes LB 502s).
  externalTrafficPolicy: Cluster
```

**Do not** put GeoIP allow/deny on the **admin** Ingress (`…-admin`). It is marked `proxy.pertisk.tech/security-exempt: "true"`. Apply GeoIP only on app Ingresses.

With `Cluster`, the peer IP is often private (SNAT). The proxy fail-opens GeoIP for private IPs and uses `X-Forwarded-For` / `X-Real-IP` when present. For strict GeoIP on public apps, put Cloudflare (or similar) in front so those headers carry the real client IP — do **not** switch this LB to `Local` unless you have verified health checks.

Redeploy:

```bash
export KUBECONFIG="${KUBECONFIG:-$HOME/.kube/config}"
VERSION=0.1.xx ./deploy/285h.sh
# or only helm (image already pushed):
helm upgrade --install pertisk-proxy-ingress ./deploy/helm/pertisk-ingress \
  -n pertisk-proxy \
  -f ./deploy/helm/pertisk-ingress/285/values.yaml \
  --set image.tag=0.1.xx \
  --set geoip.enabled=true \
  --set geoip.download.enabled=true
```

Pods will run `geoip-download` init (~50MB), then start. Admin → Advanced should show Country/ASN **ready**.

---

## Other volume options

| Mode | When |
|------|------|
| `download.enabled: true` | Talos / no node disk (default for 285) |
| `existingClaim: geoip-pvc` | Shared NFS PVC (`nfs-client` on 285) |
| `hostPath: /var/lib/...` | Normal Linux nodes only — not Talos |

---

## Proxy mode (RPM / bare metal)

Listen dual-stack (IPv4+IPv6) with `IPV6_V6ONLY=0`:

```sh
LISTEN_HTTP=[::]:80
LISTEN_HTTPS=[::]:443
LISTEN_H3_UDP=[::]:443
PERTISK_MANAGEMENT_ADDR=[::]:9080
```

Client IPs are normalized for GeoIP (`::ffff:x.x.x.x` → IPv4, zone ids stripped). Your public IPv6 (e.g. AIS `2405:9800:…`) should map to **TH** in GeoLite2-Country.

Install DBs under `/var/lib/pertisk-proxy/geoip/` (same as above), restart `pertisk-proxy`, then Sites → Advanced → allow countries `TH`.

## Enable features

| Feature | How |
|---------|-----|
| GeoIP | Admin Advanced → Enable GeoIP |
| WAF | Admin Advanced → Enable WAF (no DB) |
| Captcha | Admin Advanced; set `PERTISK_CAPTCHA_SECRET` for stable cookies |
