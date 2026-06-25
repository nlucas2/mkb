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

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mdkb_core::{Caller, RequestContext, Scope};
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

/// A held lease's ttl is clamped to this range, so a misbehaving client can neither pin the
/// daemon forever (no upper bound) nor thrash with a near-zero ttl.
const MIN_LEASE_TTL: Duration = Duration::from_secs(1);
const MAX_LEASE_TTL: Duration = Duration::from_secs(300);

/// Active **interactive leases**. A long-lived client (the desktop app) holds a lease to
/// keep an auto-started daemon alive while it is open; momentary clients (CLI/MCP) don't need one.
///
/// A lease is acquired-or-renewed by a heartbeat and expires `ttl` after the last heartbeat, so a
/// crashed client never pins the daemon (the lease simply ages out). Shared (via `Arc`) between
/// the connection handlers and the idle watchdog.
#[derive(Clone, Default)]
pub struct Leases {
    inner: Arc<Mutex<HashMap<String, Instant>>>,
}

impl Leases {
    fn guard(&self) -> std::sync::MutexGuard<'_, HashMap<String, Instant>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Acquire-or-renew `lease`, expiring `ttl` (clamped) from now. Idempotent in `lease`.
    pub fn heartbeat(&self, lease: &str, ttl: Duration) {
        let ttl = ttl.clamp(MIN_LEASE_TTL, MAX_LEASE_TTL);
        self.guard().insert(lease.to_string(), Instant::now() + ttl);
    }

    /// Drop `lease` (clean release). Unknown leases are ignored.
    pub fn release(&self, lease: &str) {
        self.guard().remove(lease);
    }

    /// Whether any unexpired lease is held. Prunes expired leases as a side effect.
    pub fn any_active(&self) -> bool {
        let now = Instant::now();
        let mut g = self.guard();
        g.retain(|_, exp| *exp > now);
        !g.is_empty()
    }
}

/// Whether an auto-started daemon should reap itself now: it must be **both** idle for at least
/// `timeout` **and** holding no interactive lease. Factored out so the decision is unit-testable
/// without spawning the watchdog thread (which calls `process::exit`).
fn should_reap(idle: Duration, timeout: Duration, leases: &Leases) -> bool {
    idle >= timeout && !leases.any_active()
}

/// Watch for inactivity and self-terminate once the daemon has been idle for `timeout` **and** no
/// interactive lease is held.
///
/// Only armed when a client auto-starts the daemon (it passes `--idle-timeout`); a manually-run
/// or remote daemon never gets one and runs forever. On reap we remove the socket file (so the
/// next client cold-starts cleanly) and exit; the OS releases the vault lock on exit.
fn spawn_idle_watchdog(activity: Activity, timeout: Duration, leases: Leases, socket: PathBuf) {
    // Re-check on a fraction of the timeout so we never overshoot by much (bounded to 1..=30s).
    let tick = timeout
        .div_f32(10.0)
        .clamp(Duration::from_secs(1), Duration::from_secs(30));
    thread::spawn(move || loop {
        thread::sleep(tick);
        if should_reap(activity.idle_for(), timeout, &leases) {
            eprintln!(
                "mdkbd: idle for {:?} (>= {:?}) and no interactive lease; shutting down {}",
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
    //
    // An interactive client (desktop app) holds a lease that keeps the daemon alive
    // while it's open; the watchdog only reaps when idle **and** no lease is held. Momentary
    // clients (CLI/MCP) don't lease — their request activity already defers the timer. The lease
    // registry exists regardless (it's cheap); only the watchdog consults it.
    let leases = Leases::default();
    let activity = idle_timeout.map(|timeout| {
        let activity = Activity::new();
        spawn_idle_watchdog(
            activity.clone(),
            timeout,
            leases.clone(),
            socket.to_path_buf(),
        );
        eprintln!("mdkbd: idle self-shutdown armed ({timeout:?})");
        activity
    });

    if let Some(net) = net {
        let svc = SharedService::clone(&service);
        let addr = net.addr.clone();
        let act = activity.clone();
        let lz = leases.clone();
        match TcpListener::bind(&addr) {
            Ok(listener) => {
                eprintln!("mdkbd: network listener on {addr} (token auth required)");
                thread::spawn(move || serve_tcp(listener, net, svc, act, lz));
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
    let sock = socket.to_path_buf();
    for stream in listener {
        match stream {
            Ok(stream) => {
                let svc = SharedService::clone(&service);
                let act = activity.clone();
                let lz = leases.clone();
                let sock = sock.clone();
                thread::spawn(move || {
                    // `&Stream` is both Read and Write, so one stream serves reader and writer.
                    if let Err(e) = handle(
                        &stream,
                        &stream,
                        RequestContext::local(),
                        None,
                        svc,
                        act.as_ref(),
                        &lz,
                        Some(sock.clone()),
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
    leases: Leases,
) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let svc = SharedService::clone(&service);
                let token = net.token.clone();
                let act = activity.clone();
                let lz = leases.clone();
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
                    if let Err(e) = handle(
                        stream,
                        writer,
                        ctx,
                        Some(token),
                        svc,
                        act.as_ref(),
                        &lz,
                        None,
                    ) {
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
// One param per connection concern (transport identity, service, idle tracker, lease registry,
// socket-for-shutdown); grouping them into a struct would add lifetime ceremony for no real gain.
#[allow(clippy::too_many_arguments)]
fn handle(
    reader: impl Read,
    mut writer: impl Write,
    mut ctx: RequestContext,
    token: Option<String>,
    service: SharedService,
    activity: Option<&Activity>,
    leases: &Leases,
    socket: Option<PathBuf>,
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
        let mut shutting_down = false;
        let response = match decode_request(line) {
            Ok(Request::Authenticate { token: presented }) => {
                if let Some(a) = activity {
                    a.touch();
                }
                authenticate(&mut ctx, token.as_deref(), &presented)
            }
            // Lease ops are a daemon-lifecycle concern, handled here rather than dispatched to
            // core: a heartbeat acquires-or-renews an interactive lease (keeping the daemon alive
            // while a long-lived client is open); release drops it. Both also count as activity.
            Ok(Request::Heartbeat { lease, ttl_ms }) => {
                if let Some(a) = activity {
                    a.touch();
                }
                leases.heartbeat(&lease, Duration::from_millis(ttl_ms));
                Response::Ok
            }
            Ok(Request::ReleaseLease { lease }) => {
                if let Some(a) = activity {
                    a.touch();
                }
                leases.release(&lease);
                Response::Ok
            }
            // Scope upgrade for the desktop app: grant lock management, but only over the local
            // (trusted) transport. On a remote/authenticated connection it's refused — lock
            // management is a local human surface, not something a network token confers.
            Ok(Request::AnnounceApp) => {
                if let Some(a) = activity {
                    a.touch();
                }
                announce_app(&mut ctx)
            }
            // Explicit shutdown: local connections only (a remote caller can't take down a shared
            // daemon). We respond first, then remove the socket and exit after the flush below —
            // the same cleanup the idle watchdog performs.
            Ok(Request::Shutdown) => match ctx.caller {
                Caller::Local => {
                    shutting_down = true;
                    Response::Ok
                }
                _ => Response::Error {
                    message: "shutdown is available only to the local desktop app".to_string(),
                },
            },
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
        if shutting_down {
            eprintln!("mdkbd: shutdown requested; removing socket and exiting");
            if let Some(s) = &socket {
                let _ = std::fs::remove_file(s);
            }
            std::process::exit(0);
        }
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

/// Grant the **app** scope (lock management) to a connection — but only a local, trusted one.
/// Lock/unlock is a local human surface: a remote connection (even an authenticated one) is
/// refused, so a network token never confers the ability to toggle locks. This is a local-trust
/// guardrail, not a security boundary.
fn announce_app(ctx: &mut RequestContext) -> Response {
    match ctx.caller {
        Caller::Local => {
            ctx.scope = Scope::APP;
            Response::Ok
        }
        _ => Response::Error {
            message: "lock management is available only to the local desktop app".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_holds_then_expires() {
        let leases = Leases::default();
        assert!(!leases.any_active(), "no leases initially");
        leases.heartbeat("app", Duration::from_secs(60));
        assert!(leases.any_active(), "an unexpired lease is active");
        // A near-zero ttl is clamped up to MIN_LEASE_TTL, so it doesn't vanish instantly; but a
        // ttl in the past is impossible to request — expiry is tested via release below.
        leases.release("app");
        assert!(!leases.any_active(), "released lease is gone");
    }

    #[test]
    fn heartbeat_is_idempotent_renew() {
        let leases = Leases::default();
        leases.heartbeat("app", Duration::from_secs(10));
        leases.heartbeat("app", Duration::from_secs(10)); // renew same id, not a second lease
                                                          // Only one lease id is tracked.
        assert_eq!(leases.guard().len(), 1);
        assert!(leases.any_active());
    }

    #[test]
    fn expired_lease_is_pruned_and_not_active() {
        let leases = Leases::default();
        // Insert an already-expired entry directly to simulate a stale lease (no real sleep).
        leases
            .guard()
            .insert("stale".to_string(), Instant::now() - Duration::from_secs(1));
        assert!(
            !leases.any_active(),
            "expired lease must not keep the daemon alive"
        );
        assert_eq!(leases.guard().len(), 0, "any_active prunes expired leases");
    }

    #[test]
    fn should_reap_requires_idle_and_no_lease() {
        let leases = Leases::default();
        let timeout = Duration::from_secs(120);
        // Idle past the grace, no lease → reap.
        assert!(should_reap(Duration::from_secs(121), timeout, &leases));
        // Not idle enough → keep.
        assert!(!should_reap(Duration::from_secs(10), timeout, &leases));
        // Idle past the grace BUT a lease is held → keep (interactive client attached).
        leases.heartbeat("app", Duration::from_secs(60));
        assert!(!should_reap(Duration::from_secs(121), timeout, &leases));
    }
}
