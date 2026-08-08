---
status: proposed
date: 2026-08-04
---

# 0001. Run the core as a separate process

## Context

`docs/cli.md` names the core throughout but never says where it runs. Two conditions in the specification decide that.

Something has to outlive the CLI process. `run` returns at once while the loop keeps going, and `session show` reads that state from a separate process.

One party, and only one, may assign work, accumulate usage, and write state. The specification allows one session at a time, compares accumulated usage against the budget, and keeps state in a file. None of the three holds if two parties decide them.

## Decision

The core runs as a separate process and surfaces send it requests. However many surfaces are running, there is one core process, and only that process assigns work, accumulates usage, and writes state.

We give up the convenience of debugging a single process.

## Consequences

- Good, because state and usage are counted by one party no matter how many surfaces are running.
- Good, because the core can be run without a surface, so domain behaviour is exercised without the CLI.
- Good, because adding or replacing a surface does not touch the core.
- Neutral, because what the surface and the core exchange, and how the core is started and stopped, follow from this and are settled separately.
- Neutral, because the repository gets a binary per program and a crate for the types they share. `docs/architecture.md` carries the layout.
- Bad, because two processes are running, so debugging spans both.
- Bad, because an inter-process protocol and a daemon lifetime convention are added to 0.1.0.

## Alternatives considered

### Keep the core as a library that surfaces link against

Rejected. It meets neither condition: the process ends with the surface, so the loop stops, and every surface gets its own core, so each opens its own session and writes its own state.

