# Finding A - Client: cancellation mid-`write_frame` desynchronises the shared connection

**Component:** `sdk/signal-layer/signal-layer-ipc` - `TapClient::send_recv`
**Kind:** robustness / protocol desync
**Severity:** Medium - one stalled write fails an unrelated later operation and forces a reconnect of the connection shared by every cell on the node.

## Summary

Upstream already fixed cancellation *between* the request write and the response read (the B2 regression test and its `TeardownGuard`). The guard,
however, is installed only **after** `write_frame` returns. A cancellation *during* the write which the 5 s `TAP_CALL_TIMEOUT` wrapping every public
operation can deliver whenever the peer's receive buffer fills mid-frame drops the future before the guard exists. The partial frame stays on the wire,
the connection stays in `inner.conn` and the next caller on that shared connection inherits a corrupted framing boundary.

## Root cause

`client.rs` (master at commit `841952a`), `send_recv`:

```rust
// Write the request.  If this fails, tear down immediately.
if write_frame(&mut conn.writer, req).await.is_err() {
    inner.conn = None;
    return None;
}

// The guard is created HERE only after the write completed.
let must_teardown = TeardownGuard { conn: &mut inner.conn };
```

`write_frame` failing is handled, `write_frame` being **dropped** is not. Every public operation runs under `bounded()` ->
`tokio::time::timeout(TAP_CALL_TIMEOUT, op)`, so a stall longer than 5s drops the operation future at whatever await point 
it is parked in including the `write_all` inside `write_frame`.

Compounding this, the doc comment on `write_frame` in `framing.rs` claims:

> a single write_all is all-or-nothing (B2b)

That is only true with respect to *errors*. `write_all` makes no cancellation-atomicity guarantee: if the future is dropped after a partial
`poll_write`, those bytes are on the wire. Cancellation during a single `write_all` is precisely the case this finding is about, so the comment
documents a safety property the code does not have.

## Reproduction

`repro/client-write-cancellation/` (standalone repro, run with `cargo run`).

An in-memory stream accepts two bytes of the first frame and then returns `Poll::Pending`, a 20ms timeout cancels `write_frame`. Reusing the stream
for a second frame then yields:

```text
after cancellation: [82, 00]
next read after connection reuse: frame length 131202 exceeds the 64 KB cap
```

The two leaked prefix bytes shift every subsequent frame boundary, the reader decodes a garbage length and fails with `FrameError::TooLarge`. Inside
`TapClient`, that decode failure is what eventually tears the connection down - meaning the observable production behavior is: the operation that timed
out returns `Unavailable` (expected), **and the next, unrelated operation on the shared connection also fails** (unexpected), plus a forced reconnect.

Since the runtime multiplexes every cell on the node over this one connection, one cell's stalled write poisons the next tap call made by any cell.

## Fix

Arm the teardown guard **before** `write_frame`, covering both the write and the response read, disarm only after a fully decoded response. Any exit before
that point - error or cancellation - leaves `inner.conn = None`, so the next call reconnects cleanly. Also correct the `write_frame` doc comment to state
the actual guarantee (atomic w.r.t. errors only).

Branch: `challenge/client-write-cancellation` (commit `9c2ccd6`) - includes the regression test
`client_tests::cancelled_mid_write_send_recv_tears_down_connection` (deterministic mid-write cancellation via an injected in-memory connection
whose buffer is smaller than one request frame), verified to fail against the pre-fix `send_recv` and to pass with the fix, plus the doc-comment
correction in `framing.rs`.

## Why this matters beyond one failed call

The Signal Layer is the only path from cells to hardware on Linux. The client's design goal, stated in its own docs, is that a stalled peer cannot
keep "any cell queued behind it on the shared connection" waiting - the timeout exists precisely to bound that. This defect lets a stalled peer reach
past the timeout and damage an operation that had already been given the all-clear, which is the failure mode the B2 fix was meant to eliminate.
