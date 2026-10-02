//! Server write-wedge reproducer: a client that pipelines requests but never reads responses wedges the server's
//! per-connection task in an unbounded response write, the read-side slow-loris timeout can never fire, because
//! the read side stays healthy.
//!
//! Run against master to observe the bug, against the fix branch `challenge/server-write-timeout` to observe the reaping.

use std::sync::Arc;
use std::time::Duration;

use signal_layer_ipc::{
    PROTOCOL_VERSION, Request, StoreRead, TapStore, read_frame, serve, write_frame,
};
use tokio::net::{UnixListener, UnixStream};

/// Retained payload served for every read: large enough that a handful of unread responses fill the socket send buffer.
const RETAINED_LEN: usize = 32 * 1024;
/// Requests pipelined before going silent.  Each is ~6 bytes on the wire but costs the server 32 KB + framing, so
/// the server parks in its response write long before the client finishes pipelining.
/// Kept small: every request sits in the server's receive queue as a full socket-buffer entry
/// (a 6-byte frame costs ~1-2 KB of kernel memory), so pipelining hundreds would eventually stall the client too
/// 48 is enough to prove the wedge.
const PIPELINED_REQUESTS: usize = 48;
/// Mirror of the server's private `REQUEST_TIMEOUT_SECS`.
const REQUEST_TIMEOUT_SECS: u64 = 30;

struct BigStore;

impl TapStore for BigStore {
    fn resolve(&self, name: &str) -> Option<u32> {
        (name == "big").then_some(1)
    }

    fn read_retained(&self, h: u32) -> StoreRead {
        match h {
            1 => StoreRead::Value {
                timestamp_ms: 0,
                bytes: vec![7u8; RETAINED_LEN],
            },
            _ => StoreRead::InvalidHandle,
        }
    }

    fn take_event(&self, _h: u32) -> StoreRead {
        StoreRead::Empty
    }

    fn list_len(&self) -> u32 {
        1
    }

    fn list_entry(&self, index: u32) -> Option<(String, u8)> {
        (index == 0).then(|| ("big".to_owned(), 0))
    }

    fn type_id(&self, h: u32) -> Option<u32> {
        (h == 1).then_some(0)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("signal-layer.sock");

    // The REAL server, exactly as the generated pipeline runs it.
    let listener = UnixListener::bind(&path).expect("bind");
    let store: Arc<dyn TapStore> = Arc::new(BigStore);
    tokio::spawn(serve(listener, store, None));

    let mut stream = UnixStream::connect(&path).await.expect("connect");

    // Handshake.
    write_frame(
        &mut stream,
        &Request::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await
    .expect("write Hello");
    let hello_ok = read_frame(&mut stream).await.expect("read HelloOk");
    assert!(!hello_ok.is_empty(), "HelloOk frame");

    // Pipeline requests, then never read a single response. The server reads each request (its read side stays healthy,
    // so the 30s read timeout never fires) and parks in the response write once the buffers fill.
    for _ in 0..PIPELINED_REQUESTS {
        write_frame(&mut stream, &Request::TapReadRetained { handle: 1 })
            .await
            .expect("pipeline request");
    }
    println!(
        "pipelined {PIPELINED_REQUESTS} requests with {RETAINED_LEN}-byte responses; reading none"
    );

    tokio::time::sleep(Duration::from_secs(REQUEST_TIMEOUT_SECS + 5)).await;
    println!(
        "waited {}s (REQUEST_TIMEOUT_SECS is {}s)",
        REQUEST_TIMEOUT_SECS + 5,
        REQUEST_TIMEOUT_SECS
    );

    // Probe: is the wedged connection still open?
    let mut closed = false;
    for _ in 0..16 {
        if write_frame(&mut stream, &Request::TapListLen)
            .await
            .is_err()
        {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    if closed {
        println!(
            "probe write failed - the server closed the wedged connection at the write timeout"
        );
        println!("FIXED: a stalled reader releases its connection slot");
    } else {
        println!("probe write succeeded - the connection is still open");
        println!(
            "BUG PRESENT: the response write has no timeout, so the server holds this connection \
             (and one of its 64 slots) forever; 64 such clients deny the whole signal layer"
        );
    }
}
