//! The request server: a local Unix socket and, optionally, a network (TCP) listener.
//!
//! Connections speak newline-delimited JSON [`mdkb_protocol::Request`]s. Each is answered by
//! locking the shared [`mdkb_core::Service`] and calling the shared
//! [`mdkb_protocol::dispatch`].
//!
//! **Auth model (plan Decision #9):**
//! - Unix socket connections are `Caller::Local` — trusted (filesystem permissions are the
//!   gate). This is the default and only transport unless a network listener is enabled.
//! - TCP connections start unauthenticated (`Caller::Remote`) and **fail closed**: every
//!   data request is rejected until the client sends `Authenticate { token }` with the
//!   daemon's shared token, which upgrades the connection to `Caller::Authenticated`.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::thread;

use mdkb_core::RequestContext;
use mdkb_protocol::{decode_request, dispatch, encode_response, Request, Response};

use crate::SharedService;

/// Network listener configuration.
#[derive(Clone)]
pub struct NetConfig {
    /// Address to bind (e.g. `0.0.0.0:7820`).
    pub addr: String,
    /// Shared token clients must present via `Authenticate`.
    pub token: String,
}

/// Serve the Unix socket (always) and, if configured, a TCP listener (in a side thread).
pub fn serve(socket: &Path, net: Option<NetConfig>, service: SharedService) -> io::Result<()> {
    if let Some(net) = net {
        let svc = SharedService::clone(&service);
        let addr = net.addr.clone();
        match TcpListener::bind(&addr) {
            Ok(listener) => {
                eprintln!("mdkbd: network listener on {addr} (token auth required)");
                thread::spawn(move || serve_tcp(listener, net, svc));
            }
            Err(e) => eprintln!("mdkbd: failed to bind network listener {addr}: {e}"),
        }
    }

    let listener = UnixListener::bind(socket)?;
    eprintln!("mdkbd: listening on {}", socket.display());
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let svc = SharedService::clone(&service);
                thread::spawn(move || {
                    let writer = match stream.try_clone() {
                        Ok(w) => w,
                        Err(e) => {
                            eprintln!("mdkbd: clone error: {e}");
                            return;
                        }
                    };
                    if let Err(e) = handle(stream, writer, RequestContext::local(), None, svc) {
                        eprintln!("mdkbd: connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("mdkbd: accept error: {e}"),
        }
    }
    Ok(())
}

fn serve_tcp(listener: TcpListener, net: NetConfig, service: SharedService) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let svc = SharedService::clone(&service);
                let token = net.token.clone();
                let peer = stream
                    .peer_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|_| "unknown".to_string());
                thread::spawn(move || {
                    let writer = match stream.try_clone() {
                        Ok(w) => w,
                        Err(e) => {
                            eprintln!("mdkbd: tcp clone error: {e}");
                            return;
                        }
                    };
                    let ctx = RequestContext::remote(peer);
                    if let Err(e) = handle(stream, writer, ctx, Some(token), svc) {
                        eprintln!("mdkbd: tcp connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("mdkbd: tcp accept error: {e}"),
        }
    }
}

/// Handle one connection. `token`, when `Some`, requires the client to authenticate before
/// any data request is honoured; the connection context is upgraded on success.
fn handle(
    reader: impl Read,
    mut writer: impl Write,
    mut ctx: RequestContext,
    token: Option<String>,
    service: SharedService,
) -> io::Result<()> {
    let reader = BufReader::new(reader);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match decode_request(&line) {
            Ok(Request::Authenticate { token: presented }) => {
                authenticate(&mut ctx, token.as_deref(), &presented)
            }
            Ok(req) => {
                let mut guard = service.lock().unwrap_or_else(|p| p.into_inner());
                dispatch(&mut guard, &ctx, req)
            }
            Err(e) => Response::Error {
                message: format!("invalid request: {e}"),
            },
        };
        writer.write_all(encode_response(&response)?.as_bytes())?;
        writer.flush()?;
    }
    Ok(())
}

fn authenticate(ctx: &mut RequestContext, expected: Option<&str>, presented: &str) -> Response {
    match expected {
        // No token configured for this transport (e.g. the Unix socket): already trusted.
        None => Response::Ok,
        Some(exp) if constant_time_eq(exp.as_bytes(), presented.as_bytes()) => {
            *ctx = RequestContext::authenticated("token");
            Response::Ok
        }
        Some(_) => Response::Error {
            message: "authentication failed".to_string(),
        },
    }
}

/// Constant-time comparison to avoid leaking the token via timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
