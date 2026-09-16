# Terraform provider for pertisk-proxy

Manage sites, DNS providers, access lists, and WAF policies through the pertisk-proxy management API (**proxy mode**).

## Private vs public

| Target | Who can use it | Source |
|---|---|---|
| **HCP private** (already published) | Only members of org `pertisktech` on [app.terraform.io](https://app.terraform.io/) | `app.terraform.io/pertisktech/pertisk-proxy` |
| **Public Terraform Registry** | Everyone | `pertisktech/pertisk-proxy` → `registry.terraform.io/pertisktech/pertisk-proxy` |

The HCP upload is **not** public. To let everyone use it, publish to the [public Terraform Registry](https://developer.hashicorp.com/terraform/registry/providers/publishing).

### Publish publicly (everyone)

HashiCorp requires a **public** GitHub repo named exactly:

`github.com/pertisktech/terraform-provider-pertisk-proxy`

(not the monorepo `pertisk-proxy`).

1. Create that public repo under the `pertisktech` GitHub org.
2. Copy/push this `terraform/` provider tree into it (keep `main.go`, `internal/`, `go.mod`, `Makefile`, `docs/` …).
3. Add your GPG public key at [registry.terraform.io](https://registry.terraform.io/) → **User Settings → Signing Keys** (org `pertisktech`).
4. Build release assets:

```bash
cd terraform
make release   # writes signed zips + manifest into dist/
```

5. Create GitHub Release **`v0.1.0`** on `terraform-provider-pertisk-proxy` and upload **all** files from `dist/` (zips, `_manifest.json`, `_SHA256SUMS`, `_SHA256SUMS.sig`).
6. On [registry.terraform.io](https://registry.terraform.io/) → **Publish → Provider** → select org `pertisktech` → repo `terraform-provider-pertisk-proxy`.

After that, anyone can use:

```hcl
terraform {
  required_providers {
    pertisk-proxy = {
      source  = "pertisktech/pertisk-proxy"
      version = "0.1.0"
    }
  }
}
```

### HCP private (org only)

```bash
cd terraform
make publish   # already done for 0.1.0 under org pertisktech
```

```hcl
source  = "app.terraform.io/pertisktech/pertisk-proxy"
version = "0.1.0"
```

## Example

```hcl
provider "pertisk-proxy" {
  endpoint = "http://127.0.0.1:9080"
  username = "admin"
  password = var.pertisk_password
}

resource "pertisk_proxy_access_list" "office" {
  name            = "office"
  enabled         = true
  allow_countries = ["TH", "SG"]
}

resource "pertisk_proxy_site" "app" {
  host             = "app.example.com"
  backend          = "app"
  backend_upstream = "http://127.0.0.1:8080"
  access_list_id   = pertisk_proxy_access_list.office.id

  routes {
    path      = "/"
    path_type = "Prefix"
  }
}
```

## Local install (dev)

```bash
cd terraform
make install
```

Credentials env: `PERTISK_ENDPOINT`, `PERTISK_USERNAME`, `PERTISK_PASSWORD`, `PERTISK_TOKEN`, `PERTISK_TLS_INSECURE`.

## Resources

| Name | API |
|---|---|
| `pertisk_proxy_site` | GET/PUT `/api/config` (upsert by `host`) |
| `pertisk_proxy_dns_provider` | CRUD `/api/dns-providers` |
| `pertisk_proxy_access_list` | CRUD `/api/access-lists` |
| `pertisk_proxy_waf_policy` | CRUD `/api/waf-policies` |

## Make targets

| Target | Action |
|---|---|
| `make build` / `make install` | Local plugin binary |
| `make release` | Multi-platform zips + manifest + GPG signature in `dist/` |
| `make publish` | Upload `dist/` to HCP **private** registry (`pertisktech`) |
| `make test` | `go test ./...` |
