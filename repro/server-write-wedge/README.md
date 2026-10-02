# Server write-wedge reproducer

This repro checks one Signal Layer boundary: a client that keeps the protocol's read side healthy but never reads responses.

Run it from this directory (about 40 seconds):

```sh
cargo run
```

The repro starts the real `serve()` server on a real unix socket, with one tap whose retained payload is 32 KB. 
The client handshakes, pipelines 48 cheap requests and then never reads a response. The server keeps reading requests -
so the 30 s `REQUEST_TIMEOUT_SECS` read timeout never fires - while its response write parks forever once the socket buffers fill.
(48 requests is plenty: the server parks after a handful of 32 KB responses. Pipelining many more would eventually 
stall the client too - every queued request costs ~1-2 KB of kernel socket-buffer memory.)

Against `master`, expected output ends with:

```text
probe write succeeded - the connection is still open
BUG PRESENT: the response write has no timeout, so the server holds this connection (and one of its 64 slots) forever, 64 such clients deny the whole signal layer
```

Against the fix branch `challenge/server-write-timeout`, the probe write fails instead - the server reaps the 
wedged connection when the write timeout expires. The lab depends on `signal-layer-ipc` by git, pinned to the master 
commit the finding was verified on (`rev = "841952a"` in `Cargo.toml`), so it builds anywhere without a local myrmic 
checkout; to build it against the fix, point the dependency at the fix branch as shown in the comment in `Cargo.toml`.

This repro demonstrates the single-connection wedge. The full denial of service is the same wedge times `MAX_CONNECTIONS` (64): 
every slot held permanently, no self-healing, the whole Signal Layer unreachable until the pipeline process is restarted. 
64 parallel clients are straightforward but slow for a demo; the deterministic in-memory variant runs in milliseconds as
the regression test `server_tests::wedged_response_write_closes_connection_within_timeout` in the fix branch.
