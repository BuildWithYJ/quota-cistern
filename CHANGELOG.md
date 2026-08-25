# Changelog

Notable changes to this project are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.1.1] - 2026-08-24

What a session decides with, and what a person can see of it afterwards.

### Added

- A ledger of every run there has ever been, one line appended per run and never rewritten, at `$XDG_DATA_HOME/cistern/runs.jsonl`. It holds the model, the turns, what the vendor priced the run at, what the session set aside for it, and what the vendor's limit read either side of it. What a budget is worked out from is read from here rather than from the backlog, which keeps only a task's most recent run.
- What a run of a model is expected to take, worked out from that ledger and set aside before the run starts, so that what a session has handed out cannot sum past what it declared.
- A clock that holds a session to the time it declared whether or not anything ends, since a decision is otherwise only reached when a task ends.
- `retry` and `resume`, which put a task that ended back in the backlog. `retry` starts the work over; `resume` carries on the conversation its last run was in.
- `tidy`, which takes away the work areas of tasks already disposed of. The branch is kept whatever happens to the work area.
- `config set timing`, `pacing`, and `locking`, which are how a person chooses what a session does about a run the clock will not let finish, one the budget will not outlast, and ones still going once the budget is spent.
- `blocked` and `nothing fits` as reasons a session stops, told apart from `all done` and from the budget being spent.
- A command that starts a core when it finds none, so there is nothing to start by hand, and one that says when the core it reached predates the core program on disk, which a version string cannot show.
- Built binaries attached to a release, so installing needs no toolchain.
- When a task's run started and when it stopped.

### Changed

- Each connection is answered on a thread of its own. A command asking the vendor for its limit no longer holds every other command for as long as that takes.
- The vendor's own words moved out of the daemon and into a definition it reads. A file at `$XDG_CONFIG_HOME/cistern/vendors/<name>.toml` is laid over the one that ships, so `config set vendor` takes any name a definition exists for.
- A run's size in the unit a share is declared in is worked out from what the vendor priced it at rather than from the tokens it counted, since what a token costs differs between models by several times over.

### Fixed

- A share is measured across a window that starts over, rather than reading the fall as a session having spent nothing.
- What is running is counted under the same hold that starts more, so two sessions cannot each start against a count taken before the other.
- `run --model` reaches the run. It was stored and reported and never used, so a task that named no model went to the vendor with none and ran whatever the vendor defaults to.
- A task's instruction reaches the vendor apart from the condition its run is gated on, rather than pasted into it, where a long instruction ran past the length the vendor takes.
- An answer is read off the line that says it is the answer. A run that wrote anything after it was read as having answered that instead.
- A run whose answer counted nothing is recorded as one that could not be read, rather than as one that finished having spent nothing.

## [0.1.0] - 2026-08-11

First release. One vendor, Claude Code, on a single machine.

### Added

- `cisternd`, a daemon holding the core, and `cistern`, a command line that sends it requests over a local socket. Every request carries the version of the side that sent it, and a core of another version refuses it.
- A configuration kept in a file, with `vendor` as its one key.
- A backlog, with `task add`, `task rm`, `task show`, and `backlog`. A task carries the repository it was added from and a base branch, and a predecessor and a model can be given per task.
- Unattended execution. `run` declares a budget as a usage and a time and answers as soon as the session is open. Each task runs in a work area and on a branch of its own, and how many run at once is decided from what the tasks before them consumed.
- Consumption read from the vendor and accumulated per session, so a budget declared as a share of the five-hour limit and one declared in tokens both stop the session at the figure declared.
- Session reading with `session ls` and `session show`, and `interrupt` to stop the session that is running.
- The agent's output, kept for each task. `trace` serves it back while the task runs and after it has ended.
- Review and disposition with `review ls`, `diff`, `apply`, and `discard`. Nothing is committed, merged, or pushed.

[Unreleased]: https://github.com/BuildWithYJ/quota-cistern/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/BuildWithYJ/quota-cistern/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/BuildWithYJ/quota-cistern/releases/tag/v0.1.0
