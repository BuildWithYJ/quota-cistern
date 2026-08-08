# Architecture

quota-cistern runs as two programs. `cisternd` holds the core and keeps running; `cistern` is a surface that sends it requests. The core is arranged as ports and adapters.

[ADR 0001](adr/0001-core-surface-separation.md) records why the core is a separate process, and [ADR 0002](adr/0002-core-internal-structure.md) records why the inside is arranged this way. This document lists what follows from them, and grows as the code does.

## Which way a reference may point

References point inward. An adapter may name the core; no file in the core names an adapter or anything in `platform`. `domain` names nothing outside itself, `port` included.

An inbound adapter names the inbound ports and nothing else.

`cisternd/tests/architecture.rs` checks these clauses. A reference that breaks one fails `cargo test`.

## The core

`port` declares both edges. `port::inbound` is what the core offers, one trait per command group with the values those commands answer with. `port::outbound` is what the core needs from outside. A vendor name, a file path, a git invocation, and a clock reading do not appear in either.

`domain` holds the entities and the rules over them. `service` drives them: one service per command group, holding the outbound ports its own commands use and implementing the inbound trait those commands are declared as.

`domain` is a private module, so nothing outside `core` names an entity. It is given values it can already take, never the text a store kept them as. Reading what a store hands back is `service`'s work, and so is writing it out again.

## Adapters

An adapter has a port on one side and something outside on the other. Code that touches no port is not an adapter, however technical it is. It gets a crate of its own when its dependency has to be isolated, and not before.

An inbound adapter turns an envelope into a use case call and the answer back into an envelope. Which exit code a refusal becomes is decided here; the core never names one.

An outbound adapter is where a vendor's field names, a file format, and a git invocation belong. None of them cross back into the core.

## Platform

`platform` holds what `cisternd` needs in order to run as a program and touches no port: the accept loop, and the signal handler that gives up the socket.

## Crates

```
crates/
  cistern/            surface   the command line
  cistern-contract/   shared    the envelope, the address, and one exchange
  cisternd/           daemon    core, adapters, platform, composition root

cisternd/src/
  core/domain/        entities and the rules over them
  core/port/inbound/  what the core offers
  core/port/outbound/ what the core needs from outside
  core/service/       one per command group
  adapter/inbound/    envelope to use case
  adapter/outbound/   port to file or git
  platform/           what touches no port
  main.rs             composition root
```

A crate exists to isolate a dependency. A split made for any other reason is a module boundary instead.

The core is a module, not a crate. A private module already keeps adapters out of it, and a crate would only add a check that a dependency has not reached the core — which `std` defeats, since the process, the filesystem, and the clock are there.

## Names

One concept keeps one word wherever it appears, and the role is added to it. Configuration is `Configuration` in the domain, `ConfigurationUseCase` and `ConfigurationStore` at the two edges, and `FileConfiguration` in the adapter. A name that has to be reached through an alias is the wrong name.

## Surfaces

A surface reaches the core over the types in `cistern-contract`. It does not read or write core state itself.
