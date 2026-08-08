# Architecture

quota-cistern runs as two programs. `cisternd` holds the core and keeps running; `cistern` is a surface that sends it requests. The core is arranged as ports and adapters.

[ADR 0001](adr/0001-core-surface-separation.md) records why the core is a separate process, and [ADR 0002](adr/0002-core-internal-structure.md) records why the inside is arranged this way. This document lists what follows from them, and grows as the code does.

## The core

Everything the core needs from outside is declared in `port` as a trait. A vendor name, a file path, a git invocation, and a clock reading do not appear in the core.

`domain` holds the entities and the rules over them. `service` drives them.

`domain` is a private module. The types that appear in port signatures are public, and `core` converts between the two, so nothing outside `core` names an entity.

## Adapters

An adapter sits beside `core` in `cisternd` and reaches it only through a port. It gets a crate of its own when its dependency has to be isolated, and not before.

An adapter is where a vendor's field names, a file format, and a git invocation belong. None of them cross back into the core.

## Crates

```
crates/
  cistern/            surface   the command line
  cistern-contract/   shared    the request and response types a surface sends
  cisternd/           daemon    core, adapters, composition root

cisternd/src/
  core/               domain, port, service
  adapter/            port implementations
  main.rs             composition root
```

A crate exists to isolate a dependency. A split made for any other reason is a module boundary instead.

The core is a module, not a crate. A private module already keeps adapters out of it, and a crate would only add a check that a dependency has not reached the core — which `std` defeats, since the process, the filesystem, and the clock are there.

## Surfaces

A surface reaches the core over the types in `cistern-contract`. It does not read or write core state itself.
