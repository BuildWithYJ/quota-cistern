# quota-cistern Constitution

This file is what Spec Kit reads before it writes a specification, a plan, or a
task list. It does not add rules. Everything binding on this project is already
written in `CONTRIBUTING.md` and in `docs/adr/`, and restating it here would
create a second copy to keep in step. What follows names those rules and says
which of them a generated draft is most likely to break.

## Core Principles

### I. The core decides, the adapters touch the world

`cisternd` is arranged as ports and adapters, recorded in
[ADR 0002](../../docs/adr/0002-core-internal-structure.md). A port is written in
the core's terms rather than a vendor's. `crates/cisternd/tests/architecture.rs`
enforces the direction of every reference, so a plan that puts a vendor's name,
a file path, or a process call inside `core/` fails the test suite rather than
review.

### II. A task states what an agent must not have to decide

`cistern task add` admits an instruction only once each of the six parts of a
specification is settled: goal, place, done when, on failure, why, and scope. A
plan this project produces is held to what the tool it ships demands of everyone
else. "Done when" is a command that fails now and passes when the work is
finished, not a description of a finished state.

### III. A comment records why, and a doc comment records what

`CONTRIBUTING.md` states this under Conventions. It is the rule generated prose
breaks most often: an agent restates what the line below already says. If a
comment would be true after the code is deleted, it is not the comment to write.

### IV. Every check a machine can make lives in one script

`scripts/check.sh` runs formatting, lints, tests, and the ASCII rule, and CI runs
that same script. A plan does not introduce a check somewhere else. Adding one
means adding a step to that script.

### V. A draft is not a document

A specification, a plan, and a task list are produced by an agent and stay under
`specs/`, which is a symlink into an ignored directory. `docs/` holds prose a
person wrote and a reviewer read. [ADR 0003](../../docs/adr/0003-draft-features-with-spec-kit.md)
records why the two are kept apart.

## Constraints a generated plan tends to miss

- Branch names and pull request titles are checked by a ruleset and by CI, and
  the permitted types are listed in `CONTRIBUTING.md`. `perf` is not among them.
  Spec Kit's feature directories are numbered; branches are not.
- `unwrap`, `expect`, `panic`, and `unsafe` are denied at the workspace level.
  Tests are exempt.
- Code files hold printable ASCII only. Markdown is exempt, and so is
  `.specify/`, which is vendored.
- A module's tests live at `<module>/tests.rs`, not at the foot of the file.

## Governance

`CONTRIBUTING.md` and the ADRs govern. Where this file and one of them disagree,
they are right and this file is stale; fix it here. Amending a rule means
amending it there, and adding a decision means writing an ADR from
`docs/adr/template.md`.

**Version**: 1.0.0 | **Ratified**: 2026-08-30 | **Last Amended**: 2026-08-30
