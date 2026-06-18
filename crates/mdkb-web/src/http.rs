//! Minimal HTTP request parsing and response building (std-only).
//!
//! Just enough HTTP/1.1 to serve a local single-user UI. Not a general-purpose server.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};

/// A parsed HTTP request line + headers (body is ignored; the UI is read-only GET).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// HTTP method (e.g. `GET`).
    pub method: String,
    /// Path without the query string (percent-decoded).
    pub path: String,
    /// Parsed query parameters (percent-decoded).
    pub query: HashMap<String, String>,
}

/// An HTTP response to write back.
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// Reason phrase.
    pub reason: &'static str,
    /// Content-Type header value.
    pub content_type: &'static str,
    /// Body bytes.
    pub body: Vec<u8>,
    /// Redirect target for 3xx responses.
    pub location: Option<String>,
}

impl HttpResponse {
    /// A `200 OK` HTML response.
    pub fn html(body: impl Into<String>) -> Self {
        HttpResponse {
            status: 200,
            reason: "OK",
            content_type: "text/html; charset=utf-8",
            body: body.into().into_bytes(),
            location: None,
        }
    }

    /// A `404 Not Found` HTML response.
    pub fn not_found(body: impl Into<String>) -> Self {
        HttpResponse {
            status: 404,
            reason: "Not Found",
            content_type: "text/html; charset=utf-8",
            body: body.into().into_bytes(),
            location: None,
        }
    }

    /// A `302 Found` redirect.
    pub fn redirect(location: &str) -> Self {
        HttpResponse {
            status: 302,
            reason: "Found",
            content_type: "text/html; charset=utf-8",
            body: format!("<a href=\"{location}\">{location}</a>").into_bytes(),
            location: Some(location.to_string()),
        }
    }

    /// Serialize the full HTTP/1.1 response to bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.status,
            self.reason,
            self.content_type,
            self.body.len()
        );
        if let Some(loc) = &self.location {
            head.push_str(&format!("Location: {loc}\r\n"));
        }
        head.push_str("\r\n");
        let mut out = head.into_bytes();
        out.extend_from_slice(&self.body);
        out
    }
}

/// Percent-decode a URL component.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1]);
                let lo = hex_val(bytes[i + 2]);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push(h * 16 + l);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Parse the path + query string of a request target.
pub fn parse_target(target: &str) -> (String, HashMap<String, String>) {
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };
    let mut params = HashMap::new();
    for pair in query.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        params.insert(percent_decode(k), percent_decode(v));
    }
    (percent_decode(path), params)
}

/// Maximum bytes for the request line, and a cap on header lines, so a malicious or stuck
/// client cannot force unbounded buffering / looping.
const MAX_REQUEST_LINE: u64 = 64 * 1024;
const MAX_HEADER_LINES: usize = 200;

/// Read and parse an HTTP request from a stream (request line + headers; body discarded).
pub fn read_request(stream: &mut impl Read) -> std::io::Result<Option<HttpRequest>> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if (&mut reader)
        .take(MAX_REQUEST_LINE)
        .read_line(&mut request_line)?
        == 0
    {
        return Ok(None);
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    // Drain headers (until blank line), bounded so a client that never sends the blank
    // terminator can't loop us forever.
    for _ in 0..MAX_HEADER_LINES {
        let mut line = String::new();
        if (&mut reader).take(MAX_REQUEST_LINE).read_line(&mut line)? == 0
            || line == "\r\n"
            || line == "\n"
        {
            break;
        }
    }
    let (path, query) = parse_target(&target);
    Ok(Some(HttpRequest {
        method,
        path,
        query,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_path_and_query() {
        let (path, q) = parse_target("/search?q=restart%20nginx&limit=5");
        assert_eq!(path, "/search");
        assert_eq!(q.get("q").unwrap(), "restart nginx");
        assert_eq!(q.get("limit").unwrap(), "5");
    }

    #[test]
    fn percent_and_plus_decode() {
        assert_eq!(percent_decode("a+b%2Fc"), "a b/c");
    }

    #[test]
    fn reads_request_line() {
        let mut c = Cursor::new(b"GET /page/notes/arch.md HTTP/1.1\r\nHost: x\r\n\r\n".to_vec());
        let req = read_request(&mut c).unwrap().unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/page/notes/arch.md");
    }

    #[test]
    fn response_serializes_with_headers() {
        let bytes = HttpResponse::html("<p>hi</p>").to_bytes();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("Content-Length: 9"));
        assert!(s.ends_with("<p>hi</p>"));
    }

    #[test]
    fn redirect_sets_location() {
        let r = HttpResponse::redirect("/page/a.md");
        assert_eq!(r.status, 302);
        assert_eq!(r.location.as_deref(), Some("/page/a.md"));
        assert!(String::from_utf8_lossy(&r.to_bytes()).contains("Location: /page/a.md"));
    }
}
