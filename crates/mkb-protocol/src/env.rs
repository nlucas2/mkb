//! Canonical environment-variable names mkb honours.
//!
//! Defining each name **once** here means the daemon and every client read the same variables
//! without repeating string literals (which is how a typo silently makes one client ignore an
//! override the others honour). Clients read these via [`EnvSnapshot`](crate::connect::EnvSnapshot)
//! and the daemon reads the relevant subset directly; the *names* live only here.

/// Vault directory a client connects to / the daemon serves. Supports a leading `~`.
pub const VAULT: &str = "MKB_VAULT";

/// `host:port` of a remote daemon to connect to over TCP (client side).
pub const REMOTE: &str = "MKB_REMOTE";

/// Shared token: presented by a client to a remote daemon, or required by a `--listen` daemon.
pub const TOKEN: &str = "MKB_TOKEN";

/// Explicit local socket path to dial instead of deriving one from the vault (client side).
pub const SOCKET: &str = "MKB_SOCKET";

/// Base directory for the machine-local per-vault index dirs (overrides the OS local-data dir).
pub const INDEX_DIR: &str = "MKB_INDEX_DIR";

/// Per-user config directory holding the client's `vaults.json` registry.
pub const CONFIG_DIR: &str = "MKB_CONFIG_DIR";

/// Seconds a client waits for a freshly auto-started daemon to answer its first ping.
pub const READY_TIMEOUT_SECS: &str = "MKB_READY_TIMEOUT_SECS";

/// Directory holding an on-disk embedding model that overrides the compiled-in one.
pub const BUNDLED_MODEL_DIR: &str = "MKB_BUNDLED_MODEL_DIR";
