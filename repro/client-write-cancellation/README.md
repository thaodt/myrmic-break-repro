# Write-cancellation reproducer

This repro checks one narrow Signal Layer boundary: cancellation while a framed request is only partly written.

Run it from this directory:

```sh
cargo run
```

The fake stream accepts two bytes of the first frame and then blocks. A 20 ms timeout cancels 
`signal_layer_ipc::write_frame`. The repro then reuses the same stream for a second frame and verifies that the receiver 
can no longer recover the framing boundary.

Expected shape of the output:

```text
after cancellation: [.., ..]
next read after connection reuse: frame length ... exceeds the 64 KB cap
```

This proves the transport fact. The client-side defect is in `TapClient::send_recv`, which installs its 
connection teardown guard after `write_frame` completes, so a timeout during the write drops the write future before 
the guard exists. The fix (branch `challenge/client-write-cancellation`) arms the guard before both the write and the 
response read and is covered by the regression test 
`client_tests::cancelled_mid_write_send_recv_tears_down_connection`.

The lab depends on `signal-layer-ipc` by git, pinned to the exact master commit the finding was verified on 
(`rev = "841952a"` in `Cargo.toml`), so it builds anywhere without a local myrmic checkout. To build it against the 
fix instead, point the dependency at the fix branch as shown in the comment in `Cargo.toml`.
