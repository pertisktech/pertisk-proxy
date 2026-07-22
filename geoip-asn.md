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
export KUBECONFIG=/Users/nat/.kube/omni-proxmox-285h-kubeconfig.yaml
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

## Proxy / bare-metal host

```sh
sudo mkdir -p /var/lib/pertisk-proxy/geoip && cd /tmp
wget -q https://cdn.jsdelivr.net/npm/geolite2-country/GeoLite2-Country.mmdb.gz && gzip -df GeoLite2-Country.mmdb.gz
sudo cp GeoLite2-Country.mmdb /var/lib/pertisk-proxy/geoip/
wget -q https://iptoasn.com/data/ip2asn-combined.tsv.gz && gzip -df ip2asn-combined.tsv.gz
sudo cp ip2asn-combined.tsv /var/lib/pertisk-proxy/geoip/
sudo systemctl restart pertisk-proxy
```

## Enable features

| Feature | How |
|---------|-----|
| GeoIP | Admin Advanced → Enable GeoIP |
| WAF | Admin Advanced → Enable WAF (no DB) |
| Captcha | Admin Advanced; set `PERTISK_CAPTCHA_SECRET` for stable cookies |
