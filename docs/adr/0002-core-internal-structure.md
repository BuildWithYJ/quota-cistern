---
status: proposed
date: 2026-08-04
---

# 0002. Core internal structure

## Context

0001 put the core in its own process but did not settle how it is arranged inside.

0.1.0 is the first version, and little about it is settled. The specification will grow, and the model underneath it — sessions, budgets, a review queue — has not been proven against real use yet.

We do not know which parts will move, so we keep a seam between the code that decides and the code that touches the world. Either side can then be replaced without the other. The ones we expect to move first are how the agent is run, how usage is read, and where state is kept; a vendor changing its output or its quota terms is only the most visible case.

## Decision

We use ports and adapters. The core declares what it needs from outside as traits, and the adapters implement them.

A port is written in the core's terms, not the vendor's: `Agent::run(task) -> Observations`, not `run_stream_json`.

Layers are modules. `cisternd` holds `core`, which holds `domain`, `port`, and `service`, with the adapters beside it.

```
cisternd/src/
  core/       domain, port, service
  adapter/    port implementations
  main.rs     composition root
```

`domain` is a private module of `core`. The types in port signatures are public and `core` converts between them and the entities, so nothing outside `core` names an entity.

The core is not a crate of its own. A private module already keeps the adapters out of it, and a crate boundary would add only a check that no dependency reached the core — which `std` defeats, since the process, the filesystem, and the clock are there. We give up that check.

## Consequences

- Good, because budget and assignment rules run without a vendor process, git, or a repository, so they can be tested.
- Good, because a change in what the vendor prints stops at the adapter.
- Good, because how state is stored and how usage is measured can be replaced without touching the core.
- Neutral, because the clause list lives in `docs/architecture.md` and grows as the code does.
- Bad, because nothing stops core code from reaching a dependency of `cisternd`, which review has to catch.
- Bad, because conversion code appears between entities and port types.

## Alternatives considered

### Traits shaped by whatever they wrap

Rejected. Writing a trait from code that already works gives `ClaudeRunner::run_stream_json`, because that is what the vendor prints. When the output changes the trait changes, and the core changes with it. The trait alone does not keep the change out; writing it in the core's terms does.

### Traditional layering

Rejected. It puts data access at the bottom, but the outside here is not only storage: it is the vendor process, git, the clock, and usage measurement. Layers do not express several kinds of outside.
