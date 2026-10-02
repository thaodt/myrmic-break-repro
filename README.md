# Myrmic Build & Break Challenge - Break Track Submission

**Challenge ID:** `MYR26-UFBQ7LWQ`
**Track:** Break
**Target:** [peeriot/myrmic](https://github.com/peeriot/myrmic) @ `841952a` (master, 2026-10-02)

## Summary

This submission is the result of a focused audit of the Signal Layer IPC boundary - the unix-socket protocol between the 
Myrmic runtime and the Linux Signal Layer pipeline - read from **both** sides, plus a cross-check of the cell-isolation 
guarantees documented in the security chapter against the shipped defaults.

Three independently reproducible findings, each with its own report:

| # | Finding | Severity | Status |
|---|---------|----------|--------|
| A | [Client: cancellation mid-`write_frame` desynchronises the shared connection](findings/A-client-write-cancellation.md) | Medium (robustness) | Repro + failing test + fix (PR) |
| B | [Server: response write has no timeout → permanent connection-slot wedge](findings/B-server-write-timeout.md) | Medium-High (DoS) | Repro + failing test + fix (PR) |
| C | [Cell isolation defaults contradict the documented guarantees](findings/C-isolation-defaults.md) | Medium | Report + repro steps |

A and B are the same class of defect on opposite sides of one protocol: the **write side of a framed request/response 
exchange is not protected against cancellation/stall**, while the read side is carefully protected on both. The client 
fix upstream landed for the read side (B2, `TeardownGuard`) but left the write side open; the server never got a 
write-side bound at all.

## Recordings

- **Finding A** - [mid-write cancellation desyncs the signal-layer client: failing test, fix and transport-level proof](https://asciinema.org/a/1HAqKhQ9q658hg8g)
- **Finding B** - [the signal-layer server has no response write timeout: failing test, fix and the wedge against the real server](https://asciinema.org/a/G2fgf8KUxDWm0MxD)

## Reproducing

Each finding under `repro/` is a standalone crate (outside the Myrmic workspace) that depends on 
`signal-layer-ipc` **by git**, pinned to the exact upstream master commit the findings were verified on 
(`rev = "841952a"`) - no local myrmic checkout needed. To observe the corrected behavior instead, point the dependency 
at the fix branch on the fork, as shown in the comment in each repro's `Cargo.toml`. See the README in each finding dir.

Fix branches (intended as upstream PRs):

- `challenge/client-write-cancellation`
- `challenge/server-write-timeout`
