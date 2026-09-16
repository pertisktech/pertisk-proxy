# Terraform provider for pertisk-proxy

Manage sites, DNS providers, access lists, and WAF policies through the pertisk-proxy management API (**proxy mode**).

```hcl
terraform {
  required_providers {
    pertisk-proxy = {
      source  = "app.terraform.io/pertisktech/pertisk-proxy"
      version = "0.1.0"
    }
  }
}

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

## Publish to HCP Terraform (org `pertisktech`)

Requires [terraform login](https://app.terraform.io/) and a local GPG key used to sign releases ([private provider docs](https://developer.hashicorp.com/terraform/cloud-docs/registry/publish-providers)).

```bash
cd terraform

# one-time GPG key (if you do not already have one)
gpg --batch --passphrase '' --quick-generate-key 'pertisktech <devops@pertisk.com>' default default never

make publish          # release artifacts + upload to app.terraform.io/pertisktech
# or: make release && make publish

# If signing picks the wrong key / needs a passphrase:
#   GPG_KEY_ID=<your-key-id> make publish
#   GPG_PASSPHRASE='…' make publish
```

Source for consumers:

`app.terraform.io/pertisktech/pertisk-proxy`

## Local install (without registry)

```bash
cd terraform
make install
```

Installs under `~/.terraform.d/plugins/registry.terraform.io/pertisktech/pertisk-proxy/…`.

For a quicker edit/test loop, use `dev_overrides` in `~/.terraformrc`:

```hcl
provider_installation {
  dev_overrides {
    "pertisktech/pertisk-proxy" = "/absolute/path/to/pertisk-proxy/terraform"
  }
  direct {}
}
```

Credentials can also come from the environment: `PERTISK_ENDPOINT`, `PERTISK_USERNAME`, `PERTISK_PASSWORD`, `PERTISK_TOKEN`, `PERTISK_TLS_INSECURE`.

## Resources

| Name | API |
|---|---|
| `pertisk_proxy_site` | GET/PUT `/api/config` (upsert by `host`) |
| `pertisk_proxy_dns_provider` | CRUD `/api/dns-providers` |
| `pertisk_proxy_access_list` | CRUD `/api/access-lists` |
| `pertisk_proxy_waf_policy` | CRUD `/api/waf-policies` |

### `pertisk_proxy_site`

Sites are not individual API resources. The provider reads the full config, merges the site (and optionally creates/updates a backend from `backend_upstream`), then PUTs the config back.

- Import: `terraform import pertisk_proxy_site.app app.example.com`
- Prefer a single Terraform workspace per proxy instance (no ETag / optimistic locking).
- Ingress mode rejects config PUT — use Kubernetes resources there instead.

### `pertisk_proxy_waf_policy`

`security_json` is a JSON object matching the Admin API `security` field, for example:

```hcl
security_json = jsonencode({
  waf = { enabled = true, use_builtin_rules = true }
  bot = { enabled = false }
})
```

## Make targets

| Target | Action |
|---|---|
| `make tidy` | `go mod tidy` |
| `make build` | Build `bin/terraform-provider-pertisk-proxy` |
| `make install` | Install into `~/.terraform.d/plugins/...` |
| `make release` | Multi-platform zips + SHA256SUMS + GPG signature in `dist/` |
| `make publish` | `release` then upload to HCP org `pertisktech` |
| `make test` | `go test ./...` |
| `make fmt` | `gofmt -w .` |
