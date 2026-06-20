//! DNS-01 challenge: set TXT record via DNS provider APIs.
//! Supported: Cloudflare, DigitalOcean, Route53, GoDaddy, Linode, Hetzner, DuckDNS,
//! Namecheap, OVH, Google Cloud DNS, Azure DNS, Gandi.

use std::collections::HashMap;


#[cfg(feature = "dns-challenge")]
use aws_credential_types::Credentials as AwsCredentials;

#[cfg(feature = "dns-challenge")]
use aws_sdk_route53::types::{
    Change, ChangeAction, ChangeBatch, ResourceRecord, ResourceRecordSet, RrType,
};

#[cfg(feature = "dns-challenge")]
use aws_sdk_route53::Client as Route53Client;

#[cfg(feature = "dns-challenge")]
use aws_types::region::Region;

#[cfg(feature = "dns-challenge")]
use quick_xml::events::Event;

#[cfg(feature = "dns-challenge")]
use quick_xml::Reader;

#[cfg(feature = "dns-challenge")]
use sha1::{Digest, Sha1};

/// Solver for DNS-01 challenge: create _acme-challenge.<domain> TXT record.
#[cfg_attr(feature = "dns-challenge", async_trait::async_trait)]
pub trait Dns01Solver: Send + Sync {
    /// Create or update TXT record for the given FQDN (e.g. _acme-challenge.example.com).
    async fn set_txt(&self, fqdn: &str, value: &str) -> Result<(), String>;
}

/// Build a solver from provider type and credentials (from DB).
#[cfg(feature = "dns-challenge")]
pub fn solver_for_provider(
    provider_type: &str,
    credentials: &HashMap<String, String>,
) -> Result<Box<dyn Dns01Solver>, String> {
    let t = provider_type.to_lowercase();
    if t == "cloudflare" {
        let api_token = credentials
            .get("api_token")
            .or_else(|| credentials.get("APIToken"))
            .map(String::as_str)
            .ok_or("cloudflare: missing api_token")?;
        let zone_id = credentials
            .get("zone_id")
            .or_else(|| credentials.get("ZoneId"))
            .filter(|s| !s.trim().is_empty() && !s.contains('@'))
            .map(|s| s.trim().to_string());
        Ok(Box::new(CloudflareSolver::new(
            api_token.to_string(),
            zone_id,
        )))
    } else if t == "digitalocean" {
        let token = credentials
            .get("token")
            .or_else(|| credentials.get("api_token"))
            .map(String::as_str)
            .ok_or("digitalocean: missing token")?;
        Ok(Box::new(DigitalOceanSolver::new(token.to_string())))
    } else if t == "route53" || t == "aws" {
        let access_key_id = credentials
            .get("access_key_id")
            .or_else(|| credentials.get("access_key"))
            .map(String::as_str)
            .ok_or("route53: missing access_key_id")?;
        let secret_access_key = credentials
            .get("secret_access_key")
            .or_else(|| credentials.get("secret_key"))
            .map(String::as_str)
            .ok_or("route53: missing secret_access_key")?;
        let session_token = credentials
            .get("session_token")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let region = credentials
            .get("region")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "us-east-1".to_string());
        let zone_id = credentials
            .get("zone_id")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let zone_name = credentials
            .get("zone_name")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Ok(Box::new(Route53Solver::new(
            access_key_id.to_string(),
            secret_access_key.to_string(),
            session_token,
            region,
            zone_id,
            zone_name,
        )))
    } else if t == "godaddy" {
        let api_key = credentials
            .get("api_key")
            .map(String::as_str)
            .ok_or("godaddy: missing api_key")?;
        let api_secret = credentials
            .get("api_secret")
            .map(String::as_str)
            .ok_or("godaddy: missing api_secret")?;
        Ok(Box::new(GoDaddySolver::new(
            api_key.to_string(),
            api_secret.to_string(),
        )))
    } else if t == "linode" {
        let api_token = credentials
            .get("api_token")
            .or_else(|| credentials.get("token"))
            .map(String::as_str)
            .ok_or("linode: missing api_token")?;
        Ok(Box::new(LinodeSolver::new(api_token.to_string())))
    } else if t == "hetzner" {
        let api_token = credentials
            .get("api_token")
            .or_else(|| credentials.get("token"))
            .map(String::as_str)
            .ok_or("hetzner: missing api_token")?;
        Ok(Box::new(HetznerSolver::new(api_token.to_string())))
    } else if t == "duckdns" {
        let token = credentials
            .get("token")
            .map(String::as_str)
            .ok_or("duckdns: missing token")?;
        let domain = credentials
            .get("domain")
            .map(String::as_str)
            .ok_or("duckdns: missing domain")?;
        Ok(Box::new(DuckDnsSolver::new(
            token.to_string(),
            domain.to_string(),
        )))
    } else if t == "namecheap" {
        let api_user = credentials
            .get("api_user")
            .map(String::as_str)
            .ok_or("namecheap: missing api_user")?;
        let api_key = credentials
            .get("api_key")
            .map(String::as_str)
            .ok_or("namecheap: missing api_key")?;
        let username = credentials
            .get("username")
            .map(String::as_str)
            .ok_or("namecheap: missing username")?;
        let client_ip = credentials
            .get("client_ip")
            .map(String::as_str)
            .ok_or("namecheap: missing client_ip")?;
        let domain = credentials
            .get("domain")
            .map(String::as_str)
            .ok_or("namecheap: missing domain")?;
        Ok(Box::new(NamecheapSolver::new(
            api_user.to_string(),
            api_key.to_string(),
            username.to_string(),
            client_ip.to_string(),
            domain.to_string(),
        )))
    } else if t == "ovh" {
        let application_key = credentials
            .get("application_key")
            .map(String::as_str)
            .ok_or("ovh: missing application_key")?;
        let application_secret = credentials
            .get("application_secret")
            .map(String::as_str)
            .ok_or("ovh: missing application_secret")?;
        let consumer_key = credentials
            .get("consumer_key")
            .map(String::as_str)
            .ok_or("ovh: missing consumer_key")?;
        Ok(Box::new(OvhSolver::new(
            application_key.to_string(),
            application_secret.to_string(),
            consumer_key.to_string(),
        )))
    } else if t == "googleclouddns" {
        let project_id = credentials
            .get("project_id")
            .map(String::as_str)
            .ok_or("googleclouddns: missing project_id")?;
        let service_account_json = credentials
            .get("service_account_json")
            .or_else(|| credentials.get("service_account_key"))
            .map(String::as_str)
            .ok_or("googleclouddns: missing service_account_json")?;
        let managed_zone = credentials
            .get("managed_zone")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Ok(Box::new(GoogleCloudDnsSolver::new(
            project_id.to_string(),
            service_account_json.to_string(),
            managed_zone,
        )))
    } else if t == "azure" {
        let tenant_id = credentials
            .get("tenant_id")
            .map(String::as_str)
            .ok_or("azure: missing tenant_id")?;
        let client_id = credentials
            .get("client_id")
            .map(String::as_str)
            .ok_or("azure: missing client_id")?;
        let client_secret = credentials
            .get("client_secret")
            .map(String::as_str)
            .ok_or("azure: missing client_secret")?;
        let subscription_id = credentials
            .get("subscription_id")
            .map(String::as_str)
            .ok_or("azure: missing subscription_id")?;
        let resource_group = credentials
            .get("resource_group")
            .map(String::as_str)
            .ok_or("azure: missing resource_group")?;
        let zone_name = credentials
            .get("zone_name")
            .map(String::as_str)
            .ok_or("azure: missing zone_name")?;
        Ok(Box::new(AzureDnsSolver::new(
            tenant_id.to_string(),
            client_id.to_string(),
            client_secret.to_string(),
            subscription_id.to_string(),
            resource_group.to_string(),
            zone_name.to_string(),
        )))
    } else if t == "gandi" {
        let api_token = credentials
            .get("api_token")
            .or_else(|| credentials.get("token"))
            .map(String::as_str)
            .ok_or("gandi: missing api_token")?;
        Ok(Box::new(GandiSolver::new(api_token.to_string())))
    } else {
        Err(format!(
            "unsupported DNS provider type: {} (supported: cloudflare, route53, digitalocean, godaddy, linode, hetzner, duckdns, namecheap, ovh, googleclouddns, azure, gandi)",
            provider_type
        ))
    }
}

#[cfg(feature = "dns-challenge")]
fn normalize_fqdn(fqdn: &str) -> String {
    fqdn.trim_end_matches('.').to_string()
}

#[cfg(feature = "dns-challenge")]
fn best_zone_for_fqdn<'a>(fqdn: &str, zones: impl Iterator<Item = &'a str>) -> Option<String> {
    let fqdn = normalize_fqdn(fqdn);
    let mut best: Option<String> = None;
    for zone in zones {
        let z = zone.trim_end_matches('.');
        if fqdn == z || fqdn.ends_with(&format!(".{}", z)) {
            if best.as_ref().map(|b| z.len() > b.len()).unwrap_or(true) {
                best = Some(z.to_string());
            }
        }
    }
    best
}

#[cfg(feature = "dns-challenge")]
fn relative_record_name(fqdn: &str, zone: &str) -> String {
    let fqdn = normalize_fqdn(fqdn);
    let zone = zone.trim_end_matches('.');
    if fqdn == zone {
        return "".to_string();
    }
    fqdn.trim_end_matches(&format!(".{}", zone))
        .trim_end_matches('.')
        .to_string()
}

#[cfg(feature = "dns-challenge")]
struct CloudflareSolver {
    api_token: String,
    zone_id: Option<String>,
}

#[cfg(feature = "dns-challenge")]
impl CloudflareSolver {
    fn new(api_token: String, zone_id: Option<String>) -> Self {
        Self {
            api_token,
            zone_id,
        }
    }
}

#[cfg(feature = "dns-challenge")]
#[async_trait::async_trait]
impl Dns01Solver for CloudflareSolver {
    async fn set_txt(&self, fqdn: &str, value: &str) -> Result<(), String> {
        let fqdn = fqdn.trim_end_matches('.');
        let client = reqwest::Client::new();
        let zone_id = if let Some(ref z) = self.zone_id {
            if z.contains('@') {
                None
            } else {
                Some(z.clone())
            }
        } else {
            None
        };
        let zone_id = if let Some(z) = zone_id {
            z
        } else {
            let domain = fqdn
                .strip_prefix("_acme-challenge.")
                .unwrap_or(fqdn)
                .trim_end_matches('.');
            let parts: Vec<&str> = domain.split('.').filter(|s| !s.is_empty()).collect();
            let zone_name = if parts.len() >= 2 {
                parts[parts.len() - 2..].join(".")
            } else {
                domain.to_string()
            };
            let list: serde_json::Value = client
                .get("https://api.cloudflare.com/client/v4/zones")
                .query(&[("name", &zone_name)])
                .header("Authorization", format!("Bearer {}", self.api_token))
                .send()
                .await
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|e| e.to_string())?;
            let results = list
                .get("result")
                .and_then(|r| r.as_array())
                .ok_or("invalid response")?;
            let first = results
                .first()
                .ok_or_else(|| format!("zone not found for {}", zone_name))?;
            first
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("zone id missing")?
                .to_string()
        };
        let list: serde_json::Value = client
            .get(format!(
                "https://api.cloudflare.com/client/v4/zones/{}/dns_records",
                zone_id
            ))
            .query(&[("type", "TXT"), ("name", fqdn)])
            .header("Authorization", format!("Bearer {}", self.api_token))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let empty: Vec<serde_json::Value> = vec![];
        let records = list
            .get("result")
            .and_then(|r| r.as_array())
            .unwrap_or(&empty);
        for rec in records.iter() {
            if let Some(id) = rec.get("id").and_then(|v| v.as_str()) {
                client
                    .delete(format!(
                        "https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}",
                        zone_id, id
                    ))
                    .header("Authorization", format!("Bearer {}", self.api_token))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        let body = serde_json::json!({
            "type": "TXT",
            "name": fqdn,
            "content": value,
            "ttl": 60
        });
        let resp: serde_json::Value = client
            .post(format!(
                "https://api.cloudflare.com/client/v4/zones/{}/dns_records",
                zone_id
            ))
            .header("Authorization", format!("Bearer {}", self.api_token))
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        if let Some(success) = resp.get("success").and_then(|v| v.as_bool()) {
            if !success {
                let err = resp
                    .get("errors")
                    .and_then(|e| e.as_array())
                    .and_then(|a| a.first())
                    .and_then(|o| o.get("message").and_then(|m| m.as_str()))
                    .unwrap_or("unknown Cloudflare error");
                return Err(err.to_string());
            }
        }
        Ok(())
    }
}

#[cfg(feature = "dns-challenge")]
struct DigitalOceanSolver {
    token: String,
}

#[cfg(feature = "dns-challenge")]
impl DigitalOceanSolver {
    fn new(token: String) -> Self {
        Self { token }
    }
}

#[cfg(feature = "dns-challenge")]
#[async_trait::async_trait]
impl Dns01Solver for DigitalOceanSolver {
    async fn set_txt(&self, fqdn: &str, value: &str) -> Result<(), String> {
        let client = reqwest::Client::new();
        let domain = fqdn
            .strip_prefix("_acme-challenge.")
            .unwrap_or(fqdn);
        let list: serde_json::Value = client
            .get("https://api.digitalocean.com/v2/domains")
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let domains = list
            .get("domains")
            .and_then(|d| d.as_array())
            .ok_or("invalid response")?;

        // Pick the most specific matching zone from account domains.
        // This supports delegated subzones (e.g. amd.pertisk.com) as well as parent zones (pertisk.com).
        let zone_name = best_zone_for_fqdn(
            domain,
            domains
                .iter()
                .filter_map(|d| d.get("name").and_then(|n| n.as_str())),
        )
        .ok_or_else(|| {
            format!(
                "domain not found in DigitalOcean account for {}",
                domain
            )
        })?;

        let domain_obj = domains
            .iter()
            .find(|d| d.get("name").and_then(|n| n.as_str()) == Some(zone_name.as_str()))
            .ok_or_else(|| format!("zone {} not found", zone_name))?;
        let domain_name = domain_obj
            .get("name")
            .and_then(|n| n.as_str())
            .ok_or("domain name missing")?;
        let body = serde_json::json!({
            "type": "TXT",
            "name": relative_record_name(fqdn, domain_name),
            "data": value,
            "ttl": 60
        });
        let _: serde_json::Value = client
            .post(format!(
                "https://api.digitalocean.com/v2/domains/{}/records",
                domain_name
            ))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(feature = "dns-challenge")]
struct Route53Solver {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    region: String,
    zone_id: Option<String>,
    zone_name: Option<String>,
}

#[cfg(feature = "dns-challenge")]
impl Route53Solver {
    fn new(
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
        region: String,
        zone_id: Option<String>,
        zone_name: Option<String>,
    ) -> Self {
        Self {
            access_key_id,
            secret_access_key,
            session_token,
            region,
            zone_id,
            zone_name,
        }
    }

    async fn resolve_zone_id(&self, client: &Route53Client, fqdn: &str) -> Result<String, String> {
        if let Some(zone_id) = &self.zone_id {
            return Ok(zone_id.trim_start_matches("/hostedzone/").to_string());
        }
        let zone_name = if let Some(zone_name) = &self.zone_name {
            zone_name.clone()
        } else {
            let fqdn = normalize_fqdn(fqdn);
            let parts: Vec<&str> = fqdn.split('.').filter(|s| !s.is_empty()).collect();
            if parts.len() < 2 {
                return Err("route53: unable to infer zone name".to_string());
            }
            parts[parts.len() - 2..].join(".")
        };
        let mut req = client.list_hosted_zones_by_name();
        req = req.dns_name(zone_name.clone());
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let zones = resp.hosted_zones();
        for z in zones {
            let name = z.name().trim_end_matches('.');
            if name == zone_name.trim_end_matches('.') {
                let id = z.id();
                return Ok(id.trim_start_matches("/hostedzone/").to_string());
            }
        }
        Err(format!("route53: hosted zone not found for {}", zone_name))
    }
}

#[cfg(feature = "dns-challenge")]
#[async_trait::async_trait]
impl Dns01Solver for Route53Solver {
    async fn set_txt(&self, fqdn: &str, value: &str) -> Result<(), String> {
        let creds = AwsCredentials::new(
            self.access_key_id.clone(),
            self.secret_access_key.clone(),
            self.session_token.clone(),
            None,
            "pertisk-rproxy",
        );
        let conf = aws_sdk_route53::Config::builder()
            .region(Region::new(self.region.clone()))
            .credentials_provider(creds)
            .build();
        let client = Route53Client::from_conf(conf);
        let zone_id = self.resolve_zone_id(&client, fqdn).await?;
        let fqdn = normalize_fqdn(fqdn);
        let name = format!("{}.", fqdn);
        let rrset = ResourceRecordSet::builder()
            .name(name)
            .r#type(RrType::Txt)
            .ttl(60)
            .resource_records(
                ResourceRecord::builder()
                    .value(format!("\"{}\"", value))
                    .build()
                    .map_err(|e| e.to_string())?,
            )
            .build()
            .map_err(|e| e.to_string())?;
        let change = Change::builder()
            .action(ChangeAction::Upsert)
            .resource_record_set(rrset)
            .build()
            .map_err(|e| e.to_string())?;
        let batch = ChangeBatch::builder()
            .changes(change)
            .build()
            .map_err(|e| e.to_string())?;
        client
            .change_resource_record_sets()
            .hosted_zone_id(zone_id)
            .change_batch(batch)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(feature = "dns-challenge")]
struct GoDaddySolver {
    api_key: String,
    api_secret: String,
}

#[cfg(feature = "dns-challenge")]
impl GoDaddySolver {
    fn new(api_key: String, api_secret: String) -> Self {
        Self { api_key, api_secret }
    }

    fn auth_header(&self) -> String {
        format!("sso-key {}:{}", self.api_key, self.api_secret)
    }
}

#[cfg(feature = "dns-challenge")]
#[async_trait::async_trait]
impl Dns01Solver for GoDaddySolver {
    async fn set_txt(&self, fqdn: &str, value: &str) -> Result<(), String> {
        let client = reqwest::Client::new();
        let domains: serde_json::Value = client
            .get("https://api.godaddy.com/v1/domains")
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let list = domains.as_array().ok_or("godaddy: invalid response")?;
        let zone = best_zone_for_fqdn(
            fqdn,
            list.iter().filter_map(|d| d.get("domain").and_then(|v| v.as_str())),
        )
        .ok_or("godaddy: zone not found")?;
        let mut record_name = relative_record_name(fqdn, &zone);
        if record_name.is_empty() {
            record_name = "@".to_string();
        }
        let body = serde_json::json!([
            {
                "data": value,
                "ttl": 600
            }
        ]);
        client
            .put(format!(
                "https://api.godaddy.com/v1/domains/{}/records/TXT/{}",
                zone, record_name
            ))
            .header("Authorization", self.auth_header())
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(feature = "dns-challenge")]
struct LinodeSolver {
    api_token: String,
}

#[cfg(feature = "dns-challenge")]
impl LinodeSolver {
    fn new(api_token: String) -> Self {
        Self { api_token }
    }
}

#[cfg(feature = "dns-challenge")]
#[async_trait::async_trait]
impl Dns01Solver for LinodeSolver {
    async fn set_txt(&self, fqdn: &str, value: &str) -> Result<(), String> {
        let client = reqwest::Client::new();
        let list: serde_json::Value = client
            .get("https://api.linode.com/v4/domains")
            .query(&[("page_size", "500")])
            .bearer_auth(&self.api_token)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let domains = list
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or("linode: invalid response")?;
        let zone = best_zone_for_fqdn(
            fqdn,
            domains.iter().filter_map(|d| d.get("domain").and_then(|v| v.as_str())),
        )
        .ok_or("linode: zone not found")?;
        let zone_id = domains
            .iter()
            .find(|d| d.get("domain").and_then(|v| v.as_str()) == Some(zone.as_str()))
            .and_then(|d| d.get("id").and_then(|v| v.as_i64()))
            .ok_or("linode: zone id missing")?;
        let record_name = relative_record_name(fqdn, &zone);
        let existing: serde_json::Value = client
            .get(format!("https://api.linode.com/v4/domains/{}/records", zone_id))
            .query(&[("type", "TXT"), ("name", record_name.as_str())])
            .bearer_auth(&self.api_token)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        if let Some(records) = existing.get("data").and_then(|d| d.as_array()) {
            for rec in records {
                if let Some(id) = rec.get("id").and_then(|v| v.as_i64()) {
                    let _ = client
                        .delete(format!(
                            "https://api.linode.com/v4/domains/{}/records/{}",
                            zone_id, id
                        ))
                        .bearer_auth(&self.api_token)
                        .send()
                        .await;
                }
            }
        }
        let body = serde_json::json!({
            "type": "TXT",
            "name": record_name,
            "target": value,
            "ttl_sec": 60
        });
        client
            .post(format!("https://api.linode.com/v4/domains/{}/records", zone_id))
            .bearer_auth(&self.api_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(feature = "dns-challenge")]
struct HetznerSolver {
    api_token: String,
}

#[cfg(feature = "dns-challenge")]
impl HetznerSolver {
    fn new(api_token: String) -> Self {
        Self { api_token }
    }
}

#[cfg(feature = "dns-challenge")]
#[async_trait::async_trait]
impl Dns01Solver for HetznerSolver {
    async fn set_txt(&self, fqdn: &str, value: &str) -> Result<(), String> {
        let client = reqwest::Client::new();
        let list: serde_json::Value = client
            .get("https://dns.hetzner.com/api/v1/zones")
            .header("Auth-API-Token", &self.api_token)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let zones = list
            .get("zones")
            .and_then(|z| z.as_array())
            .ok_or("hetzner: invalid response")?;
        let zone_name = best_zone_for_fqdn(
            fqdn,
            zones.iter().filter_map(|z| z.get("name").and_then(|v| v.as_str())),
        )
        .ok_or("hetzner: zone not found")?;
        let zone_id = zones
            .iter()
            .find(|z| z.get("name").and_then(|v| v.as_str()) == Some(zone_name.as_str()))
            .and_then(|z| z.get("id").and_then(|v| v.as_str()))
            .ok_or("hetzner: zone id missing")?;
        let record_name = relative_record_name(fqdn, &zone_name);
        let records: serde_json::Value = client
            .get("https://dns.hetzner.com/api/v1/records")
            .query(&[("zone_id", zone_id)])
            .header("Auth-API-Token", &self.api_token)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        if let Some(records) = records.get("records").and_then(|r| r.as_array()) {
            for rec in records {
                if rec.get("type").and_then(|v| v.as_str()) != Some("TXT") {
                    continue;
                }
                if rec.get("name").and_then(|v| v.as_str()) != Some(record_name.as_str()) {
                    continue;
                }
                if let Some(id) = rec.get("id").and_then(|v| v.as_str()) {
                    let _ = client
                        .delete(format!("https://dns.hetzner.com/api/v1/records/{}", id))
                        .header("Auth-API-Token", &self.api_token)
                        .send()
                        .await;
                }
            }
        }
        let body = serde_json::json!({
            "zone_id": zone_id,
            "type": "TXT",
            "name": record_name,
            "value": value,
            "ttl": 60
        });
        client
            .post("https://dns.hetzner.com/api/v1/records")
            .header("Auth-API-Token", &self.api_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(feature = "dns-challenge")]
struct DuckDnsSolver {
    token: String,
    domain: String,
}

#[cfg(feature = "dns-challenge")]
impl DuckDnsSolver {
    fn new(token: String, domain: String) -> Self {
        Self { token, domain }
    }
}

#[cfg(feature = "dns-challenge")]
#[async_trait::async_trait]
impl Dns01Solver for DuckDnsSolver {
    async fn set_txt(&self, _fqdn: &str, value: &str) -> Result<(), String> {
        let client = reqwest::Client::new();
        let resp = client
            .get("https://www.duckdns.org/update")
            .query(&[
                ("domains", self.domain.as_str()),
                ("token", self.token.as_str()),
                ("txt", value),
                ("clear", "true"),
            ])
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;
        if resp.trim() != "OK" {
            return Err(format!("duckdns: update failed: {}", resp.trim()));
        }
        Ok(())
    }
}

#[cfg(feature = "dns-challenge")]
#[derive(Clone)]
struct NamecheapHostRecord {
    name: String,
    record_type: String,
    address: String,
    ttl: String,
    mx_pref: String,
}

#[cfg(feature = "dns-challenge")]
struct NamecheapSolver {
    api_user: String,
    api_key: String,
    username: String,
    client_ip: String,
    domain: String,
}

#[cfg(feature = "dns-challenge")]
impl NamecheapSolver {
    fn new(api_user: String, api_key: String, username: String, client_ip: String, domain: String) -> Self {
        Self {
            api_user,
            api_key,
            username,
            client_ip,
            domain,
        }
    }

    fn sld_tld(&self) -> Result<(String, String), String> {
        let domain = self.domain.trim_end_matches('.');
        let mut parts = domain.split('.').collect::<Vec<_>>();
        if parts.len() < 2 {
            return Err("namecheap: invalid domain".to_string());
        }
        let sld = parts.remove(0).to_string();
        let tld = parts.join(".");
        Ok((sld, tld))
    }

    async fn get_hosts(&self, client: &reqwest::Client) -> Result<Vec<NamecheapHostRecord>, String> {
        let (sld, tld) = self.sld_tld()?;
        let params = vec![
            ("ApiUser", self.api_user.as_str()),
            ("ApiKey", self.api_key.as_str()),
            ("UserName", self.username.as_str()),
            ("ClientIp", self.client_ip.as_str()),
            ("Command", "namecheap.domains.dns.getHosts"),
            ("SLD", sld.as_str()),
            ("TLD", tld.as_str()),
        ];
        let resp = client
            .get("https://api.namecheap.com/xml.response")
            .query(&params)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;
        let mut reader = Reader::from_str(&resp);
        let mut buf = Vec::new();
        let mut records = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                    if e.name().as_ref() == b"host" {
                        let mut name = String::new();
                        let mut record_type = String::new();
                        let mut address = String::new();
                        let mut ttl = "60".to_string();
                        let mut mx_pref = "10".to_string();
                        for attr in e.attributes().flatten() {
                            let key = attr.key.as_ref();
                            let value = attr.unescape_value().unwrap_or_default().to_string();
                            match key {
                                b"Name" => name = value,
                                b"Type" => record_type = value,
                                b"Address" => address = value,
                                b"TTL" => ttl = value,
                                b"MXPref" => mx_pref = value,
                                _ => {}
                            }
                        }
                        if !name.is_empty() && !record_type.is_empty() {
                            records.push(NamecheapHostRecord {
                                name,
                                record_type,
                                address,
                                ttl,
                                mx_pref,
                            });
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(format!("namecheap: failed to parse response: {}", e)),
                _ => {}
            }
            buf.clear();
        }
        Ok(records)
    }

    async fn set_hosts(
        &self,
        client: &reqwest::Client,
        hosts: &[NamecheapHostRecord],
    ) -> Result<(), String> {
        let (sld, tld) = self.sld_tld()?;
        let mut params: Vec<(String, String)> = vec![
            ("ApiUser".to_string(), self.api_user.clone()),
            ("ApiKey".to_string(), self.api_key.clone()),
            ("UserName".to_string(), self.username.clone()),
            ("ClientIp".to_string(), self.client_ip.clone()),
            ("Command".to_string(), "namecheap.domains.dns.setHosts".to_string()),
            ("SLD".to_string(), sld),
            ("TLD".to_string(), tld),
        ];
        for (idx, host) in hosts.iter().enumerate() {
            let i = idx + 1;
            params.push((format!("HostName{}", i), host.name.clone()));
            params.push((format!("RecordType{}", i), host.record_type.clone()));
            params.push((format!("Address{}", i), host.address.clone()));
            params.push((format!("TTL{}", i), host.ttl.clone()));
            params.push((format!("MXPref{}", i), host.mx_pref.clone()));
        }
        client
            .get("https://api.namecheap.com/xml.response")
            .query(&params)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(feature = "dns-challenge")]
#[async_trait::async_trait]
impl Dns01Solver for NamecheapSolver {
    async fn set_txt(&self, fqdn: &str, value: &str) -> Result<(), String> {
        let client = reqwest::Client::new();
        let record_name = relative_record_name(fqdn, &self.domain);
        let record_name = if record_name.is_empty() {
            "@".to_string()
        } else {
            record_name
        };
        let mut hosts = self.get_hosts(&client).await?;
        hosts.retain(|h| !(h.record_type == "TXT" && h.name == record_name));
        hosts.push(NamecheapHostRecord {
            name: record_name,
            record_type: "TXT".to_string(),
            address: value.to_string(),
            ttl: "60".to_string(),
            mx_pref: "10".to_string(),
        });
        self.set_hosts(&client, &hosts).await
    }
}

#[cfg(feature = "dns-challenge")]
struct OvhSolver {
    application_key: String,
    application_secret: String,
    consumer_key: String,
}

#[cfg(feature = "dns-challenge")]
impl OvhSolver {
    fn new(application_key: String, application_secret: String, consumer_key: String) -> Self {
        Self {
            application_key,
            application_secret,
            consumer_key,
        }
    }

    async fn ovh_time(&self, client: &reqwest::Client) -> Result<String, String> {
        client
            .get("https://api.ovh.com/1.0/auth/time")
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())
    }

    fn signature(&self, method: &str, url: &str, body: &str, timestamp: &str) -> String {
        let payload = format!(
            "{}+{}+{}+{}+{}+{}",
            self.application_secret, self.consumer_key, method, url, body, timestamp
        );
        let mut hasher = Sha1::new();
        hasher.update(payload.as_bytes());
        let result = hasher.finalize();
        format!("$1${}", hex::encode(result))
    }

    async fn request(
        &self,
        client: &reqwest::Client,
        method: reqwest::Method,
        url: &str,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response, String> {
        let timestamp = self.ovh_time(client).await?;
        let body_str = body
            .as_ref()
            .map(|b| b.to_string())
            .unwrap_or_default();
        let signature = self.signature(method.as_str(), url, &body_str, &timestamp);
        let mut req = client
            .request(method, url)
            .header("X-Ovh-Application", &self.application_key)
            .header("X-Ovh-Consumer", &self.consumer_key)
            .header("X-Ovh-Timestamp", &timestamp)
            .header("X-Ovh-Signature", signature);
        if let Some(body) = body {
            req = req.json(&body);
        }
        req.send().await.map_err(|e| e.to_string())
    }
}

#[cfg(feature = "dns-challenge")]
#[async_trait::async_trait]
impl Dns01Solver for OvhSolver {
    async fn set_txt(&self, fqdn: &str, value: &str) -> Result<(), String> {
        let client = reqwest::Client::new();
        let zones_resp = self
            .request(
                &client,
                reqwest::Method::GET,
                "https://api.ovh.com/1.0/domain/zone",
                None,
            )
            .await?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| e.to_string())?;
        let zones = zones_resp
            .as_array()
            .ok_or("ovh: invalid response")?;
        let zone = best_zone_for_fqdn(
            fqdn,
            zones.iter().filter_map(|z| z.as_str()),
        )
        .ok_or("ovh: zone not found")?;
        let record_name = relative_record_name(fqdn, &zone);
        let list_url = format!(
            "https://api.ovh.com/1.0/domain/zone/{}/record?fieldType=TXT&subDomain={}",
            zone, record_name
        );
        let existing = self
            .request(&client, reqwest::Method::GET, &list_url, None)
            .await?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| e.to_string())?;
        if let Some(ids) = existing.as_array() {
            for id in ids {
                if let Some(id) = id.as_i64() {
                    let del_url = format!(
                        "https://api.ovh.com/1.0/domain/zone/{}/record/{}",
                        zone, id
                    );
                    let _ = self
                        .request(&client, reqwest::Method::DELETE, &del_url, None)
                        .await;
                }
            }
        }
        let body = serde_json::json!({
            "fieldType": "TXT",
            "subDomain": record_name,
            "target": value,
            "ttl": 60
        });
        let add_url = format!("https://api.ovh.com/1.0/domain/zone/{}/record", zone);
        self.request(&client, reqwest::Method::POST, &add_url, Some(body))
            .await?;
        let refresh_url = format!("https://api.ovh.com/1.0/domain/zone/{}/refresh", zone);
        let _ = self
            .request(&client, reqwest::Method::POST, &refresh_url, None)
            .await?;
        Ok(())
    }
}

#[cfg(feature = "dns-challenge")]
struct GoogleCloudDnsSolver {
    project_id: String,
    service_account_json: String,
    managed_zone: Option<String>,
}

#[cfg(feature = "dns-challenge")]
impl GoogleCloudDnsSolver {
    fn new(project_id: String, service_account_json: String, managed_zone: Option<String>) -> Self {
        Self {
            project_id,
            service_account_json,
            managed_zone,
        }
    }

    async fn access_token(&self) -> Result<String, String> {
        let key: yup_oauth2::ServiceAccountKey = serde_json::from_str(&self.service_account_json)
            .map_err(|e| format!("googleclouddns: invalid service_account_json: {}", e))?;
        let auth = yup_oauth2::ServiceAccountAuthenticator::builder(key)
            .build()
            .await
            .map_err(|e| e.to_string())?;
        let token = auth
            .token(&["https://www.googleapis.com/auth/ndev.clouddns.readwrite"])
            .await
            .map_err(|e| e.to_string())?;
        let token = token.token().unwrap_or("");
        if token.is_empty() {
            return Err("googleclouddns: token missing".to_string());
        }
        Ok(token.to_string())
    }

    async fn resolve_zone(&self, fqdn: &str, token: &str) -> Result<String, String> {
        if let Some(zone) = &self.managed_zone {
            return Ok(zone.clone());
        }
        let fqdn = normalize_fqdn(fqdn);
        let parts: Vec<&str> = fqdn.split('.').filter(|s| !s.is_empty()).collect();
        if parts.len() < 2 {
            return Err("googleclouddns: unable to infer zone name".to_string());
        }
        let zone_guess = format!("{}.", parts[parts.len() - 2..].join("."));
        let client = reqwest::Client::new();
        let resp: serde_json::Value = client
            .get(format!(
                "https://dns.googleapis.com/dns/v1/projects/{}/managedZones",
                self.project_id
            ))
            .query(&[("dnsName", zone_guess.as_str())])
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let zones = resp
            .get("managedZones")
            .and_then(|z| z.as_array())
            .ok_or("googleclouddns: invalid response")?;
        let best = zones
            .iter()
            .filter_map(|z| z.get("dnsName").and_then(|v| v.as_str()))
            .map(|z| z.trim_end_matches('.').to_string())
            .collect::<Vec<_>>();
        let best = best_zone_for_fqdn(fqdn.as_str(), best.iter().map(|s| s.as_str()))
            .ok_or("googleclouddns: zone not found")?;
        let zone = zones
            .iter()
            .find(|z| {
                z.get("dnsName")
                    .and_then(|v| v.as_str())
                    .map(|v| v.trim_end_matches('.') == best)
                    .unwrap_or(false)
            })
            .and_then(|z| z.get("name").and_then(|v| v.as_str()))
            .ok_or("googleclouddns: zone id missing")?;
        Ok(zone.to_string())
    }
}

#[cfg(feature = "dns-challenge")]
#[async_trait::async_trait]
impl Dns01Solver for GoogleCloudDnsSolver {
    async fn set_txt(&self, fqdn: &str, value: &str) -> Result<(), String> {
        let token = self.access_token().await?;
        let zone = self.resolve_zone(fqdn, &token).await?;
        let client = reqwest::Client::new();
        let fqdn = format!("{}.", normalize_fqdn(fqdn));
        let existing: serde_json::Value = client
            .get(format!(
                "https://dns.googleapis.com/dns/v1/projects/{}/managedZones/{}/rrsets",
                self.project_id, zone
            ))
            .query(&[("name", fqdn.as_str()), ("type", "TXT")])
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let deletions = existing
            .get("rrsets")
            .and_then(|r| r.as_array())
            .and_then(|arr| arr.first().cloned());
        let mut body = serde_json::json!({
            "additions": [
                {
                    "name": fqdn,
                    "type": "TXT",
                    "ttl": 60,
                    "rrdatas": [format!("\"{}\"", value)]
                }
            ]
        });
        if let Some(del) = deletions {
            body["deletions"] = serde_json::Value::Array(vec![del]);
        }
        client
            .post(format!(
                "https://dns.googleapis.com/dns/v1/projects/{}/managedZones/{}/changes",
                self.project_id, zone
            ))
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(feature = "dns-challenge")]
struct AzureDnsSolver {
    tenant_id: String,
    client_id: String,
    client_secret: String,
    subscription_id: String,
    resource_group: String,
    zone_name: String,
}

#[cfg(feature = "dns-challenge")]
impl AzureDnsSolver {
    fn new(
        tenant_id: String,
        client_id: String,
        client_secret: String,
        subscription_id: String,
        resource_group: String,
        zone_name: String,
    ) -> Self {
        Self {
            tenant_id,
            client_id,
            client_secret,
            subscription_id,
            resource_group,
            zone_name,
        }
    }

    async fn token(&self) -> Result<String, String> {
        let client = reqwest::Client::new();
        let url = format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            self.tenant_id
        );
        let resp: serde_json::Value = client
            .post(url)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("scope", "https://management.azure.com/.default"),
                ("grant_type", "client_credentials"),
            ])
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        resp.get("access_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or("azure: token missing".to_string())
    }
}

#[cfg(feature = "dns-challenge")]
#[async_trait::async_trait]
impl Dns01Solver for AzureDnsSolver {
    async fn set_txt(&self, fqdn: &str, value: &str) -> Result<(), String> {
        let token = self.token().await?;
        let mut record_name = relative_record_name(fqdn, &self.zone_name);
        if record_name.is_empty() {
            record_name = "@".to_string();
        }
        let url = format!(
            "https://management.azure.com/subscriptions/{}/resourceGroups/{}/providers/Microsoft.Network/dnsZones/{}/TXT/{}?api-version=2018-05-01",
            self.subscription_id, self.resource_group, self.zone_name, record_name
        );
        let body = serde_json::json!({
            "properties": {
                "TTL": 60,
                "TXTRecords": [
                    {
                        "value": [value]
                    }
                ]
            }
        });
        let client = reqwest::Client::new();
        client
            .put(url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(feature = "dns-challenge")]
struct GandiSolver {
    api_token: String,
}

#[cfg(feature = "dns-challenge")]
impl GandiSolver {
    fn new(api_token: String) -> Self {
        Self { api_token }
    }
}

#[cfg(feature = "dns-challenge")]
#[async_trait::async_trait]
impl Dns01Solver for GandiSolver {
    async fn set_txt(&self, fqdn: &str, value: &str) -> Result<(), String> {
        let client = reqwest::Client::new();
        let list: serde_json::Value = client
            .get("https://api.gandi.net/v5/livedns/domains")
            .header("Authorization", format!("Apikey {}", self.api_token))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let domains = list.as_array().ok_or("gandi: invalid response")?;
        let zone = best_zone_for_fqdn(
            fqdn,
            domains.iter().filter_map(|d| d.get("fqdn").and_then(|v| v.as_str())),
        )
        .ok_or("gandi: zone not found")?;
        let mut record_name = relative_record_name(fqdn, &zone);
        if record_name.is_empty() {
            record_name = "@".to_string();
        }
        let body = serde_json::json!({
            "rrset_ttl": 60,
            "rrset_values": [value]
        });
        client
            .put(format!(
                "https://api.gandi.net/v5/livedns/domains/{}/records/{}/TXT",
                zone, record_name
            ))
            .header("Authorization", format!("Apikey {}", self.api_token))
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
