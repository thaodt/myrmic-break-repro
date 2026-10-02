# Finding C - Cell isolation defaults contradict the documented guarantees

**Component:** `swarm/sorg-execution` (wasmtime cell host), `swarm/sorg-common` config
**Kind:** robustness / isolation-gap between documentation and shipped defaults
**Severity:** Medium - one misbehaving cell can hang or crash the runtime process, taking down every other cell on the node. 
(Developer Preview scope: cells are first-party code deployed by the node operator, so this is a self-inflicted-wound 
bound rather than a multi-tenant boundary. It still decides whether one bug kills one cell or the whole node.)

## Summary

The security chapter (`doc/chapters/11_security.md`) states:

> On OS-based Nodes the runtime meters guest instruction execution with Wasmtime fuel, so the computation a Cell performs 
> is bounded by the fuel its runner grants it.

The mechanism exists, but the shipped default grants effectively unlimited fuel and no memory limit is configured at all:

- `swarm/sorg-common/src/configs/exec.rs`: `DEFAULT_FUEL: u64 = u64::MAX` - a cell in `loop {}` never runs out of fuel.
- No `StoreLimits` / memory cap anywhere in the workspace - guest  `memory.grow` is bounded only by the host's RAM + swap. 
A growing cell OOM-kills the **runtime process**, i.e. every cell on the node, not just itself.
- `swarm/db-client/src/application.rs` (~line 177): host-side `defer` buffering of cell-initiated DB work is unbounded, 
giving a second, host-side amplification path for memory growth driven by a cell.

So the documentation's "bounded by the fuel its runner grants it" is true in the same way "speed is bounded by the limiter" 
is true of a car shipped with the limiter set to infinity.

## Reproduction steps

1. `myrmic new spin` and make the cell's `init` (or a `cmd`) execute `loop {}`, build and deploy on a one-node swarm. 
Observe: the cell's executor thread is pegged indefinitely, nothing reaps it. With `runner_fuel` set to a small value in 
the exec config, the same cell is interrupted - proving the mechanism works and only the default is wrong.
2. `myrmic new grow` and have a `cmd` grow the linear memory directly with `core::arch::wasm32::memory_grow`, 
touching one byte per new page so the host commits real memory. Observe: host RSS climbs without bound - nothing stops 
the growth short of the OOM killer terminating the runtime process, which kills all cells on the node. 
Note that going through the SDK's allocator alone would NOT demonstrate this: `define_alloc_heap!` sizes the heap from 
a static array baked into the module's initial linear memory, so `Vec` growth alone never calls `memory.grow` - the repro 
must grow memory with the raw intrinsic (any cell can, the SDK allocator is a convenience, not a boundary).

(These are provided as steps, the fuel half was verified by code inspection of the config default and the wasmtime store 
setup, the memory half by the absence of any store limits. A recorded terminal run is attached if time permitted.)

## Suggested direction

I'm thinking the right method to achieve this. Please be patient!

## Related observations

- Host panics reachable from cell input: unchecked guest pointer/length indexing in `host_functions/mod.rs` (`as_slice`/`as_slice_mut`)
and an `unreachable!` on a guest-supplied log level in `host_functions/logging.rs`. A cell can crash its own task and
drive a supervision restart loop.
- `spawn_cell` has no quota, rate or depth limit (`host_functions/cell/spawning.rs`): a cell can spawn cells
recursively until the node (or swarm) is exhausted.
- An `.expect()` on payloads above 2 GiB in the command/event path (`cell_task/commands.rs`, `events.rs`) - cross-cell panic,
impractical to trigger on 32-bit wasm but unguarded.

These are listed for completeness, the defaults issue above is the finding proper.
