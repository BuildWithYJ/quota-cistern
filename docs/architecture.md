# Architecture

quota-cistern runs as two programs. `cisternd` holds the core and keeps running; `cistern` is a surface that sends it requests. The core is arranged as ports and adapters.

[ADR 0001](adr/0001-core-surface-separation.md) records why the core is a separate process, and [ADR 0002](adr/0002-core-internal-structure.md) records why the inside is arranged this way. This document lists what follows from them, and grows as the code does.

## Which way a reference may point

References point inward. An adapter may name the core; no file in the core names an adapter or anything in `platform`. `domain` names nothing outside itself, `port` included.

An inbound adapter names the inbound ports and nothing else.

`cisternd/tests/architecture.rs` checks these clauses. A reference that breaks one fails `cargo test`.

## The core

`port` declares both edges. `port::inbound` is what the core offers, one trait per command group with the values those commands answer with. A command group is the commands that are about the same thing: a task, a session, a result, the configuration. Sharing an identifier is not being about the same thing, which is why reading what a run wrote sits beside the session rather than beside the task it names. `port::outbound` is what the core needs from outside. A vendor name, a file path, a git invocation, and a clock reading do not appear in either.

One outside, one place. An outside that holds several conversations is a directory named after that outside, and one that holds a single conversation is a file. The vendor answers two questions, running a task and how much of its allowance is left, so `port::outbound::vendor` holds both. The repository a task was added from answers three, so `port::outbound::repository` holds those.

`domain` holds the entities and the rules over them. `service` drives them: one service per command group, holding the outbound ports its own commands use and implementing the inbound trait those commands are declared as.

One thing in `service` answers no command. Whether a session carries on and with what is a single decision, and the commands over sessions and the workers that carry tasks on both arrive at it; `service::supervision` is that decision, held by both rather than owned by either. A rule about spending is added there.

`domain` is a private module, so nothing outside `core` names an entity. It is given values it can already take, never the text a store kept them as. Reading what a store hands back is `service`'s work, and so is writing it out again.

## Adapters

An adapter has a port on one side and something outside on the other. Code that touches no port is not an adapter, however technical it is. It gets a crate of its own when its dependency has to be isolated, and not before.

An inbound adapter turns an envelope into a use case call and the answer back into an envelope. Which exit code a refusal becomes is decided here; the core never names one.

An outbound adapter is where a file format and a git invocation belong. A vendor's own words do not belong in code at all: they sit in a definition the daemon reads, and `cisternd/tests/architecture.rs` fails on a vendor's field name written in the daemon's own code. Tests are the exception, since a vendor's answer has to be written out somewhere for the code that reads one to be tested against it.

Outbound adapters are grouped by the means rather than by the outside: `program`, `git`, `file`, and the clock. Whatever a means needs in order to work — a stand-in used in tests, the part every file store shares — sits in that directory too.

## The vendor

The means for a vendor is an external program, and which program it is comes from a definition rather than from code. A definition names the program, its arguments, the goal that leads the prompt, the two ceilings a run is cut off at, the words the vendor uses when a run hits one, and where each figure sits in the answer. A second vendor is a file.

`program/claude.toml` is the definition this build ships with. It travels in the binary and nothing is written to disk, so an upgrade has nothing of a user's to overwrite. A file at `$XDG_CONFIG_HOME/cistern/vendors/<name>.toml` is laid over it, holding only what differs, so a definition we improve reaches someone who changed one line of it. A name nothing ships has nothing to lay over and has to be written out. A name with no definition either way stops the daemon starting rather than failing on the first task.

What stays in code is the part that does not change with the vendor: starting the child, ending its process group, reading its pipes, and following a path into its answer. A path is names joined by dots, and one name may be `*`, which stands for every key of the object at that point and adds up the numbers under it. That is the only rule beyond following names, and it is there so that a vendor reporting per model what the core counts once needs no code.

Two things a definition cannot add on its own. The shape an answer arrives in and the way an allowance is asked for are both code, and a definition picks between the ones written by name. A shape nobody has written yet is a change to Rust.

The format of a definition is what a user writes against. Changing it breaks the files they placed.

A vendor's words reach one more place. What a line of a run's output amounts to is read by the trace store, which is a file adapter and has no business naming a vendor's module. The composition root carries the names across, so the store follows names it was handed rather than names it knows.

The two edges are named differently on purpose. A port says who is on the other side, because that is what the core is talking to. An adapter says how, because that is what changes when the same conversation is held another way.

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
  adapter/outbound/   port to a program, a file, or git, one directory per means
  platform/           what touches no port
  main.rs             composition root
```

A crate exists to isolate a dependency. A split made for any other reason is a module boundary instead.

The core is a module, not a crate. A private module already keeps adapters out of it, and a crate would only add a check that a dependency has not reached the core — which `std` defeats, since the process, the filesystem, and the clock are there.

## Names

One concept keeps one word wherever it appears, and the role is added to it. Configuration is `Configuration` in the domain, `ConfigurationUseCase` and `ConfigurationStore` at the two edges, and `FileConfiguration` in the adapter. A name that has to be reached through an alias is the wrong name.

## Surfaces

A surface reaches the core over the types in `cistern-contract`. It does not read or write core state itself.
