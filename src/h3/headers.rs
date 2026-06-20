use http::header::{HeaderName, HeaderValue, HOST};
use http::uri::{PathAndQuery, Uri};
use http::{Request, Response, StatusCode};
use quiche::h3::{Header, NameValue};
use quiche::h3::Header as H3Header;

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
        .body(message.as_bytes().to_vec())
        .unwrap()
}
