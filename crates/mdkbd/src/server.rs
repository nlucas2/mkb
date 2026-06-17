//! The Unix-socket request server.
//!
//! Accepts connections, reads newline-delimited JSON [`mdkb_protocol::Request`]s, and
//! answers each by locking the shared [`mdkb_core::Service`] and calling the shared
//! [`mdkb_protocol::dispatch`]. All callers over the local socket are treated as
//! [`mdkb_core::Caller::Local`].

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread;

use mdkb_core::RequestContext;
use mdkb_protocol::{decode_request, dispatch, encode_response, Response};

use crate::SharedService;

/// Bind the socket and serve forever.
pub fn serve(socket: &Path, service: SharedService) -> io::Result<()> {
    let listener = UnixListener::bind(socket)?;
    eprintln!("mdkbd: listening on {}", socket.display());
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let svc = SharedService::clone(&service);
                thread::spawn(move || {
                    if let Err(e) = handle(stream, svc) {
                        eprintln!("mdkbd: connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("mdkbd: accept error: {e}"),
        }
    }
    Ok(())
}

fn handle(stream: UnixStream, service: SharedService) -> io::Result<()> {
    let ctx = RequestContext::local();
    let mut writer = stream.try_clone()?;
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match decode_request(&line) {
            Ok(req) => {
                let mut guard = service.lock().unwrap_or_else(|poison| poison.into_inner());
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
