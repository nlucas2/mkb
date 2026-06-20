//! The request server: a local socket and, optionally, a network (TCP) listener.
//!
//! Connections speak newline-delimited JSON [`mdkb_protocol::Request`]s. Each is answered by
//! locking the shared [`mdkb_core::Service`] and calling the shared
//! [`mdkb_protocol::dispatch`].
//!
//! The local socket is cross-platform via [`mdkb_protocol::transport`] (a Unix-domain socket
//! on Unix, a named pipe on Windows).
//!
//! **Auth model (plan Decision #9):**
//! - Local socket connections are `Caller::Local` — trusted (OS-level access control is the
//!   gate). This is the default and only transport unless a network listener is enabled.
//! - TCP connections start unauthenticated (`Caller::Remote`) and **fail closed**: every
//!   data request is rejected until the client sends `Authenticate { token }` with the
//!   daemon's shared token, which upgrades the connection to `Caller::Authenticated`.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use mdkb_core::RequestContext;
use mdkb_protocol::{decode_request, dispatch, encode_response, transport, Request, Response};

use crate::SharedService;

/// Maximum length of a single request line. Caps memory a connection can force us to buffer
/// before a request is even decoded (pre-auth on the network listener).
const MAX_LINE_BYTES: u64 = 8 * 1024 * 1024;

/// Idle/read timeout for network connections (slowloris mitigation). The local Unix socket
/// is trusted and left without a timeout.
const TCP_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Tracks the time of the most recent request so an idle daemon can reap itself.
///
/// Shared (via `Arc`) between every connection handler and the idle watchdog. A monotonic
/// millisecond clock (ms since process start) is good enough — we only compare elapsed time.
#[derive(Clone)]
pub struct Activity {
    last_ms: Arc<AtomicU64>,
    epoch: Instant,
}

impl Activity {
    /// Create an activity tracker, marking "now" as the last activity.
    pub fn new() -> Self {
        let me = Activity {
            last_ms: Arc::new(AtomicU64::new(0)),
            epoch: Instant::now(),
        };
        me.touch();
        me
    }

    /// Record that a request just happened.
    pub fn touch(&self) {
        let ms = self.epoch.elapsed().as_millis() as u64;
        self.last_ms.store(ms, Ordering::Relaxed);
    }

    /// How long since the last recorded activity.
    pub fn idle_for(&self) -> Duration {
        let now = self.epoch.elapsed().as_millis() as u64;
        let last = self.last_ms.load(Ordering::Relaxed);
        Duration::from_millis(now.saturating_sub(last))
    }
}

impl Default for Activity {
    fn default() -> Self {
        Self::new()
    }
}

/// Watch for inactivity and self-terminate once the daemon has been idle for `timeout`.
///
/// Only armed when a client auto-starts the daemon (it passes `--idle-timeout`); a manually-run
/// or remote daemon never gets one and runs forever. On reap we remove the socket file (so the
/// next client cold-starts cleanly) and exit; the OS releases the vault lock on exit.
fn spawn_idle_watchdog(activity: Activity, timeout: Duration, socket: PathBuf) {
    // Re-check on a fraction of the timeout so we never overshoot by much (bounded to 1..=30s).
    let tick = timeout
        .div_f32(10.0)
        .clamp(Duration::from_secs(1), Duration::from_secs(30));
    thread::spawn(move || loop {
        thread::sleep(tick);
        if activity.idle_for() >= timeout {
            eprintln!(
                "mdkbd: idle for {:?} (>= {:?}); shutting down {}",
                activity.idle_for(),
                timeout,
                socket.display()
            );
            let _ = std::fs::remove_file(&socket);
            std::process::exit(0);
        }
    });
}

/// Network listener configuration.
#[derive(Clone)]
pub struct NetConfig {
    /// Address to bind (e.g. `0.0.0.0:7820`).
    pub addr: String,
    /// Shared token clients must present via `Authenticate`.
    pub token: String,
}

/// Serve the local socket (always) and, if configured, a TCP listener (in a side thread).
pub fn serve(
    socket: &Path,
    net: Option<NetConfig>,
    service: SharedService,
    idle_timeout: Option<Duration>,
) -> io::Result<()> {
    // When a client auto-starts us it passes an idle timeout; arm a watchdog so an unused
    // vault's daemon reaps itself instead of leaking. Every request (local or network) touches
    // the shared tracker. `None` (manual/remote daemon) → no tracker, runs forever.
    let activity = idle_timeout.map(|timeout| {
        let activity = Activity::new();
        spawn_idle_watchdog(activity.clone(), timeout, socket.to_path_buf());
        eprintln!("mdkbd: idle self-shutdown armed ({timeout:?})");
        activity
    });

    if let Some(net) = net {
        let svc = SharedService::clone(&service);
        let addr = net.addr.clone();
        let act = activity.clone();
        match TcpListener::bind(&addr) {
            Ok(listener) => {
                eprintln!("mdkbd: network listener on {addr} (token auth required)");
                thread::spawn(move || serve_tcp(listener, net, svc, act));
            }
            Err(e) => eprintln!("mdkbd: failed to bind network listener {addr}: {e}"),
        }
    }

    let listener = transport::bind_local(socket)?;
    // The local socket is the trusted control plane. On Unix it is a filesystem socket, so
    // restrict it to the owner; on Windows it is a named pipe (no file to chmod). Best-effort.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600));
    }
    eprintln!("mdkbd: listening on {}", socket.display());
    for stream in listener {
        match stream {
            Ok(stream) => {
                let svc = SharedService::clone(&service);
                let act = activity.clone();
                thread::spawn(move || {
                    // `&Stream` is both Read and Write, so one stream serves reader and writer.
                    if let Err(e) = handle(
                        &stream,
                        &stream,
                        RequestContext::local(),
                        None,
                        svc,
                        act.as_ref(),
                    ) {
                        eprintln!("mdkbd: connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("mdkbd: accept error: {e}"),
        }
    }
    Ok(())
}

fn serve_tcp(
    listener: TcpListener,
    net: NetConfig,
    service: SharedService,
    activity: Option<Activity>,
) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let svc = SharedService::clone(&service);
                let token = net.token.clone();
                let act = activity.clone();
                let peer = stream
                    .peer_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|_| "unknown".to_string());
                // Bound how long a connection may stall mid-request (slowloris).
                let _ = stream.set_read_timeout(Some(TCP_READ_TIMEOUT));
                thread::spawn(move || {
                    let writer = match stream.try_clone() {
                        Ok(w) => w,
                        Err(e) => {
                            eprintln!("mdkbd: tcp clone error: {e}");
                            return;
                        }
                    };
                    let ctx = RequestContext::remote(peer);
                    if let Err(e) = handle(stream, writer, ctx, Some(token), svc, act.as_ref()) {
                        eprintln!("mdkbd: tcp connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("mdkbd: tcp accept error: {e}"),
        }
    }
}

/// Handle one connection. `token`, when `Some`, requires the client to authenticate before
/// any data request is honoured; the connection context is upgraded on success. Each decoded
/// request marks `activity` (when present) so the idle watchdog sees the daemon is in use.
fn handle(
    reader: impl Read,
    mut writer: impl Write,
    mut ctx: RequestContext,
    token: Option<String>,
    service: SharedService,
    activity: Option<&Activity>,
) -> io::Result<()> {
    let mut reader = BufReader::new(reader);
    loop {
        // Read one line with a hard cap so a peer can't force unbounded buffering before we
        // even decode (and authenticate) the request.
        let mut buf = Vec::new();
        let n = (&mut reader)
            .take(MAX_LINE_BYTES)
            .read_until(b'\n', &mut buf)?;
        if n == 0 {
            break; // EOF
        }
        if !buf.ends_with(b"\n") && (n as u64) >= MAX_LINE_BYTES {
            writer.write_all(
                encode_response(&Response::Error {
                    message: "request line exceeds maximum length".to_string(),
                })?
                .as_bytes(),
            )?;
            writer.flush()?;
            break;
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let response = match decode_request(line) {
            Ok(Request::Authenticate { token: presented }) => {
                if let Some(a) = activity {
                    a.touch();
                }
                authenticate(&mut ctx, token.as_deref(), &presented)
            }
            Ok(req) => {
                // A real request — including a Ping liveness check — counts as use and defers
                // idle shutdown.
                if let Some(a) = activity {
                    a.touch();
                }
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
