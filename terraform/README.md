# Terraform provider for pertisk-proxy

Manage sites, DNS providers, access lists, and WAF policies through the pertisk-proxy management API (**proxy mode**).

## Private vs public

| Target | Who can use it | Source |
|---|---|---|
| **HCP private** (already published) | Only members of org `pertisktech` on [app.terraform.io](https://app.terraform.io/) | `app.terraform.io/pertisktech/pertisk-proxy` |
| **Public Terraform Registry** | Everyone | `pertisktech/pertisk-proxy` → `registry.terraform.io/pertisktech/pertisk-proxy` |

Develop in this monorepo ([pertisktech/pertisk-proxy](https://github.com/pertisktech/pertisk-proxy) → `terraform/`).  
Public Registry publishes from [pertisktech/terraform-provider-pertisk-proxy](https://github.com/pertisktech/terraform-provider-pertisk-proxy) ([docs](https://developer.hashicorp.com/terraform/registry/providers/publishing)).

### Sync → public provider repo

```bash
cd terraform
make sync-provider-repo
# or:
# PROVIDER_REPO=https://github.com/pertisktech/terraform-provider-pertisk-proxy.git make sync-provider-repo
```

Then cut a release for the public Registry:

```bash
make release
# Create GitHub Release v0.1.0 on terraform-provider-pertisk-proxy with all files from dist/
# First time only: registry.terraform.io → Publish → Provider (org pertisktech)
```

Keep coding here on `main`; re-run `make sync-provider-repo` before each public release.

### HCP private (org only — already works from this repo)

```bash
cd terraform
make publish
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
| `make publish-public` | Sync + GitHub Release for **public** Registry (`pertisktech/pertisk-proxy`) |
| `make sync-provider-repo` | Mirror `terraform/` → `terraform-provider-pertisk-proxy` for public Registry |
| `make test` | `go test ./...` |

<!-- registry publish 20260917T032439Z -->
