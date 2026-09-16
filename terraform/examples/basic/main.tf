terraform {
  required_providers {
    pertisk = {
      source = "pertisktech/pertisk"
    }
  }
}

provider "pertisk" {
  endpoint = var.endpoint
  username = var.username
  password = var.password
}

variable "endpoint" {
  type    = string
  default = "http://127.0.0.1:9080"
}

variable "username" {
  type    = string
  default = "admin"
}

variable "password" {
  type      = string
  sensitive = true
}

resource "pertisk_access_list" "office" {
  name            = "office"
  description     = "Allow office countries"
  enabled         = true
  allow_countries = ["TH", "SG"]
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
