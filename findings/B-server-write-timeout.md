# Finding B - Server: response write has no timeout → permanent connection-slot wedge

**Component:** `sdk/signal-layer/signal-layer-ipc` - `serve` per-connection handler
**Kind:** robustness / denial of service
**Severity:** Medium-High - a single local client can permanently exhaust all 64 connection slots, the whole Signal Layer 
stays down until the pipeline process is restarted. No data-plane error is ever surfaced.

## Summary

The server's per-connection loop bounds the **read** side carefully: a 5s handshake timeout, then a 30 s `REQUEST_TIMEOUT_SECS` 
per `read_frame`, so a slow-loris client is reaped. The **write** side has no bound at all: the `write_frame` of the response 
is awaited unbounded. A client that keeps the read side busy with cheap pipelined requests while never reading responses
parks the server task in `write_all` forever once the socket buffers fill.
The task holds its connection semaphore permit while parked, so the read-timeout slow-loris mitigation never fires for it. 
64 such clients (or 64 connections from one client) exhaust `MAX_CONNECTIONS` **permanently** - unlike a slow-loris, 
this condition never self-heals.

## Root cause

`server.rs` (master at commit `841952a`), per-connection handler (abridged):

```rust
let req = timeout(REQUEST_TIMEOUT_SECS, read_frame(&mut reader)).await ...; // bounded
...
write_frame(&mut writer, &resp).await ...;   // unbounded
```

The read timeout cannot fire while requests keep arriving - and they keep arriving, because pipelining requests costs 
the client nothing (a `TapListLen` request is a few bytes). The server reads each one, produces a response and parks in 
`write_all` once the client's receive buffer and its own send buffer are full. From that moment:

- the connection task never finishes, so its `MAX_CONNECTIONS` permit is never released
- no timeout in the codebase covers this state
- nothing is logged, the pipeline process looks healthy.

## Reproduction

`repro/server-write-wedge/` (standalone repro, run against master with `cargo run`).

It starts the real `serve()` server on a real unix socket with a tap holding a large retained value, then runs a client
that pipelines requests and never reads. Observed behaviour on master: the server keeps accepting requests (the read side 
is healthy) while the connection is never closed - well past `REQUEST_TIMEOUT_SECS` - because the write side is parked. 
The README in the repro explains how this multiplies to full slot exhaustion with 64 connections.

## Fix

Bound the response `write_frame` with the same `REQUEST_TIMEOUT_SECS` (a peer that cannot accept a response for 30s is 
indistinguishable from a dead one) and take the normal fail-closed path on expiry: drop the connection, releasing the permit.

Branch: `challenge/server-write-timeout` (commit `4203748`) - includes the deterministic regression test
`server_tests::wedged_response_write_closes_connection_within_timeout` (in-memory 64-byte duplex pipe, a 4096-byte 
retained response, a client that never reads, with paused time the handler is shown to close the connection within the bound), 
verified to fail against the unbounded write and to pass with it.  The standalone repro in `repro/server-write-wedge/` 
shows the same wedge against the real `serve()` on a real socket (about 40 seconds): on `master` the probe write still 
succeeds 35s after the wedge, on the fix branch the connection is reaped.

## Relationship to Finding A

These are the same defect on opposite sides of one protocol: **the write side of a framed request/response exchange is unprotected**, 
while the read side is protected on both ends. On the client the missing protection is a cancellation guard, 
on the server it is a timeout. Either one alone leaves the boundary half-defended.
