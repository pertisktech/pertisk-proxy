# Terraform provider for pertisk-proxy

Manage sites, DNS providers, access lists, and WAF policies through the pertisk-proxy management API (**proxy mode**).

```hcl
terraform {
  required_providers {
    pertisk = {
      source = "pertisktech/pertisk"
    }
  }
}

provider "pertisk" {
  endpoint = "http://127.0.0.1:9080"
  username = "admin"
  password = var.pertisk_password
}

resource "pertisk_access_list" "office" {
  name              = "office"
  enabled           = true
  allow_countries   = ["TH", "SG"]
}

resource "pertisk_site" "app" {
  host             = "app.example.com"
  backend          = "app"
  backend_upstream = "http://127.0.0.1:8080"
  access_list_id   = pertisk_access_list.office.id

  routes {
    path      = "/"
    path_type = "Prefix"
  }
}
```

## Build / install

The provider is not on the Terraform Registry yet. Build and install a local plugin copy (same pattern as pertisk-vms):

```bash
cd terraform
make install
```

That places the binary under `~/.terraform.d/plugins/registry.terraform.io/pertisktech/pertisk/…`. Then run `terraform init` in your root module.

For a quicker edit/test loop, point Terraform at the directory that contains the binary with a CLI config `dev_overrides` block (`~/.terraformrc`):

```hcl
provider_installation {
  dev_overrides {
    "pertisktech/pertisk" = "/absolute/path/to/pertisk-proxy/terraform"
  }
  direct {}
}
```

With `dev_overrides`, build with `make build` and put/run the binary from that directory (or symlink `bin/terraform-provider-pertisk`).

Credentials can also come from the environment: `PERTISK_ENDPOINT`, `PERTISK_USERNAME`, `PERTISK_PASSWORD`, `PERTISK_TOKEN`, `PERTISK_TLS_INSECURE`.

## Resources

| Name | API |
|---|---|
| `pertisk_site` | GET/PUT `/api/config` (upsert by `host`) |
| `pertisk_dns_provider` | CRUD `/api/dns-providers` |
| `pertisk_access_list` | CRUD `/api/access-lists` |
| `pertisk_waf_policy` | CRUD `/api/waf-policies` |

### `pertisk_site`

Sites are not individual API resources. The provider reads the full config, merges the site (and optionally creates/updates a backend from `backend_upstream`), then PUTs the config back.

- Import: `terraform import pertisk_site.app app.example.com`
- Prefer a single Terraform workspace per proxy instance (no ETag / optimistic locking).
- Ingress mode rejects config PUT — use Kubernetes resources there instead.

### `pertisk_waf_policy`

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
| `make build` | Build `bin/terraform-provider-pertisk` |
| `make install` | Install into `~/.terraform.d/plugins/...` |
| `make test` | `go test ./...` |
| `make fmt` | `gofmt -w .` |
