use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use signal_layer_ipc::{FrameError, read_frame, write_frame};
use tokio::io::AsyncWrite;

/// An in-memory stream that accepts two bytes and then stops making progress.
/// This deterministically models a socket whose send buffer fills mid-frame.
#[derive(Default)]
struct PartialThenPending {
    bytes: Vec<u8>,
    first_write_done: bool,
    blocked: bool,
}

impl PartialThenPending {
    fn unblock(&mut self) {
        self.blocked = false;
    }
}

impl AsyncWrite for PartialThenPending {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        input: &[u8],
    ) -> Poll<io::Result<usize>> {
        if !self.first_write_done {
            let accepted = input.len().min(2);
            self.bytes.extend_from_slice(&input[..accepted]);
            self.first_write_done = true;
            self.blocked = true;
            return Poll::Ready(Ok(accepted));
        }

        if self.blocked {
            return Poll::Pending;
        }

        self.bytes.extend_from_slice(input);
        Poll::Ready(Ok(input.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut stream = PartialThenPending::default();

    let first = vec![7_u8; 128];
    let timed_out =
        tokio::time::timeout(Duration::from_millis(20), write_frame(&mut stream, &first)).await;

    assert!(timed_out.is_err(), "the first frame must be cancelled");
    assert_eq!(stream.bytes.len(), 2, "two prefix bytes reached the stream");
    println!("after cancellation: {:02x?}", stream.bytes);

    // Mirror reusing the same connection for the next operation.
    stream.unblock();
    write_frame(&mut stream, &vec![9_u8])
        .await
        .expect("second frame write");

    let mut wire = stream.bytes.as_slice();
    let error = read_frame(&mut wire)
        .await
        .expect_err("the concatenated stream must be desynchronised");
    assert!(matches!(error, FrameError::TooLarge(_)));
    println!("next read after connection reuse: {error}");
}
