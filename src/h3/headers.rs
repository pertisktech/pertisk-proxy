use http::header::{HeaderName, HeaderValue, HOST};
use http::uri::{PathAndQuery, Uri};
use http::{Request, Response, StatusCode};
use quiche::h3::{Header, NameValue};
use quiche::h3::Header as H3Header;

/// Extract `:authority` host (without port).
pub fn pseudo_authority(headers: &[Header]) -> Option<&[u8]> {
    headers
        .iter()
        .find(|h| h.name() == b":authority")
        .map(|h| authority_host_only(h.value()))
}

fn authority_host_only(value: &[u8]) -> &[u8] {
    value.split(|&b| b == b':').next().unwrap_or(value)
}

/// Extract `:method` and `:path` without building a full HTTP request.
pub fn pseudo_method_path(headers: &[Header]) -> Option<(&[u8], &[u8])> {
    let mut method = None;
    let mut path = None;

    for header in headers {
        match header.name() {
            b":method" => method = Some(header.value()),
            b":path" => path = Some(path_only(header.value())),
            _ => {}
        }
    }

    Some((method?, path?))
}

fn path_only(value: &[u8]) -> &[u8] {
    value.split(|&b| b == b'?').next().unwrap_or(value)
}

pub fn h3_to_request(headers: Vec<Header>) -> anyhow::Result<Request<()>> {
    let mut method = http::Method::GET;
    let mut uri_builder = Uri::builder();
    let mut req_builder = Request::builder();

    for header in headers {
        let name = header.name();
        let value = header.value();

        if name.first() == Some(&b':') {
            match name {
                b":method" => {
                    method = http::Method::from_bytes(value)?;
                }
                b":scheme" => {
                    uri_builder = uri_builder.scheme(value);
                }
                b":authority" => {
                    let host = HeaderValue::from_bytes(value)?;
                    uri_builder = uri_builder.authority(host.as_bytes());
                    req_builder = req_builder.header(HOST, host);
                }
                b":path" => {
                    let path = PathAndQuery::try_from(value)?;
                    uri_builder = uri_builder.path_and_query(path);
                }
                _ => {}
            }
        } else {
            req_builder = req_builder.header(
                HeaderName::from_bytes(name)?,
                HeaderValue::from_bytes(value)?,
            );
        }
    }

    let uri = uri_builder.build()?;
    Ok(req_builder.method(method).uri(uri).body(())?)
}

pub fn response_to_h3(res: &Response<Vec<u8>>) -> Vec<H3Header> {
    let mut headers = vec![H3Header::new(
        b":status",
        res.status().as_str().as_bytes(),
    )];

    for (name, value) in res.headers().iter() {
        headers.push(H3Header::new(name.as_ref(), value.as_bytes()));
    }

    headers
}

pub fn error_response(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .header("server", "pertisk-proxy/h3")
        .header("x-app-name", crate::app_name())
        .body(message.as_bytes().to_vec())
        .unwrap()
}

pub fn request_host<B>(req: &Request<B>) -> String {
    if let Some(host) = req.headers().get(HOST).and_then(|v| v.to_str().ok()) {
        let host = host.trim();
        if !host.is_empty() {
            return host.split(':').next().unwrap_or(host).to_string();
        }
    }
    if let Some(authority) = req.uri().authority() {
        return authority.host().to_string();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quiche::h3::Header as H3Header;

    #[test]
    fn pseudo_authority_strips_port() {
        let headers = vec![H3Header::new(b":authority", b"api.example.com:443")];
        assert_eq!(pseudo_authority(&headers), Some(&b"api.example.com"[..]));
    }

    #[test]
    fn pseudo_method_path_extracts_path_without_query() {
        let headers = vec![
            H3Header::new(b":method", b"GET"),
            H3Header::new(b":path", b"/api/health?verbose=1"),
        ];
        let (method, path) = pseudo_method_path(&headers).unwrap();
        assert_eq!(method, b"GET");
        assert_eq!(path, b"/api/health");
    }
}
