//! S3-compatible object storage client for backup upload.

use anyhow::{anyhow, bail, Result};
use aws_credential_types::Credentials;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::Client;

use crate::db::S3SettingsRow;

pub fn build_client(settings: &S3SettingsRow) -> Result<Client> {
    let region = if settings.region.trim().is_empty() {
        "us-east-1".to_string()
    } else {
        settings.region.trim().to_string()
    };
    let access_key = settings.access_key_id.trim();
    let secret = settings.secret_access_key.trim();
    if access_key.is_empty() {
        bail!("S3 access key id is not configured");
    }
    if secret.is_empty() {
        bail!("S3 secret access key is not configured");
    }

    let creds = Credentials::new(
        access_key.to_string(),
        secret.to_string(),
        None,
        None,
        "pertisk-proxy",
    );

    let mut builder = aws_sdk_s3::Config::builder()
        .region(Region::new(region))
        .credentials_provider(creds)
        .behavior_version_latest();

    let endpoint = settings.endpoint.trim();
    if !endpoint.is_empty() {
        builder = builder.endpoint_url(endpoint);
    }
    builder = builder.force_path_style(settings.force_path_style);

    Ok(Client::from_conf(builder.build()))
}

pub async fn put_object(
    settings: &S3SettingsRow,
    key: &str,
    body: Vec<u8>,
    content_type: &str,
) -> Result<()> {
    let bucket = settings.bucket.trim();
    if bucket.is_empty() {
        bail!("S3 bucket is not configured");
    }
    let client = build_client(settings)?;
    client
        .put_object()
        .bucket(bucket)
        .key(key)
        .content_type(content_type)
        .body(body.into())
        .send()
        .await
        .map_err(|e| anyhow!("S3 PutObject failed: {e}"))?;
    Ok(())
}

/// Validate credentials/endpoint by listing up to one object in the bucket.
pub async fn test_connection(settings: &S3SettingsRow) -> Result<()> {
    let bucket = settings.bucket.trim();
    if bucket.is_empty() {
        bail!("S3 bucket is not configured");
    }
    let client = build_client(settings)?;
    let prefix = normalize_prefix(&settings.prefix);
    let mut req = client.list_objects_v2().bucket(bucket).max_keys(1);
    if !prefix.is_empty() {
        req = req.prefix(prefix);
    }
    req.send()
        .await
        .map_err(|e| anyhow!("S3 connection test failed: {e}"))?;
    Ok(())
}

pub fn object_key(prefix: &str, filename: &str) -> String {
    let prefix = normalize_prefix(prefix);
    if prefix.is_empty() {
        filename.to_string()
    } else {
        format!("{prefix}{filename}")
    }
}

fn normalize_prefix(prefix: &str) -> String {
    let p = prefix.trim().trim_start_matches('/').to_string();
    if p.is_empty() {
        return String::new();
    }
    if p.ends_with('/') {
        p
    } else {
        format!("{p}/")
    }
}
