//! Cross-platform local IPC for the daemon's trusted control plane.
//!
//! The daemon and its local clients talk over a **local socket** identified by a filesystem
//! path (`DaemonPaths.socket`). The concrete mechanism is platform-specific:
//!
//! - **Unix** → a Unix-domain socket at that path.
//! - **Windows** → a named pipe whose name is derived *deterministically* from that path
//!   (Windows named pipes live in a flat namespace, not the filesystem).
//!
//! Both ends compute the name from the same path via [`local_name`], so they always agree.
//! TCP (the optional network listener) is handled separately in the daemon — this module is
//! only the local, trusted transport. Keeping the naming in one place upholds the
//! "no divergence" rule in `AGENTS.md`: client and server can never derive different names.

use std::io;
use std::path::Path;

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{ListenerOptions, Name};

pub use interprocess::local_socket::{Listener, Stream};

/// Derive the platform-appropriate local-socket name from a filesystem socket path.
fn local_name(path: &Path) -> io::Result<Name<'static>> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::{GenericFilePath, ToFsName};
        // The socket lives at the literal filesystem path under `<vault>/.mdkb/`.
        path.to_path_buf().to_fs_name::<GenericFilePath>()
    }
    #[cfg(windows)]
    {
        use interprocess::local_socket::{GenericNamespaced, ToNsName};
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        // Named pipes have no filesystem path, so map the socket path to a stable, unique pipe
        // name. Both client and server hash the same path and therefore agree.
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        format!("mdkb_{:016x}", hasher.finish()).to_ns_name::<GenericNamespaced>()
    }
}

/// Connect to the daemon's local socket at `path`.
pub fn connect_local(path: &Path) -> io::Result<Stream> {
    Stream::connect(local_name(path)?)
}

/// Bind the daemon's local-socket listener at `path`. The returned [`Listener`] yields
/// `io::Result<Stream>` via [`Iterator`], like `UnixListener::incoming`.
pub fn bind_local(path: &Path) -> io::Result<Listener> {
    ListenerOptions::new().name(local_name(path)?).create_sync()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_name_is_derivable() {
        // Both Unix (path) and Windows (hashed namespaced name) must produce a valid name.
        assert!(local_name(Path::new("/tmp/v/.mdkb/mdkbd.sock")).is_ok());
    }

    #[test]
    fn bind_connect_roundtrip() {
        use std::io::{BufRead, BufReader, Write};
        let dir = std::env::temp_dir().join(format!("mdkb-tp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("t.sock");
        let _ = std::fs::remove_file(&sock);

        let listener = bind_local(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let stream = listener.into_iter().next().unwrap().unwrap();
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut w = &stream;
            w.write_all(format!("echo:{line}").as_bytes()).unwrap();
            w.flush().unwrap();
        });

        let stream = connect_local(&sock).unwrap();
        let mut w = &stream;
        w.write_all(b"hello\n").unwrap();
        w.flush().unwrap();
        let mut reader = BufReader::new(&stream);
        let mut got = String::new();
        reader.read_line(&mut got).unwrap();
        assert_eq!(got, "echo:hello\n");
        server.join().unwrap();
        let _ = std::fs::remove_file(&sock);
    }
}
