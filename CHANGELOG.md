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

## [0.2.0] - 2026-08-26

A task is not taken on until it says enough to be run unattended.

### Added

- An admission gate at `task add`. A run that nobody is watching cannot stop to ask, so anything the instruction leaves out is something the agent settles on its own, and every unattended accident is one of those. What is measured is how many of them are left: a task is registered when none are.
- Six parts a task is read as -- what the work is, where it is, when it is done, what to do when it cannot get there, what goes wrong today, and how far to reach. A person writes a line as before; a model reads what they were looking at when they wrote it and fills in what it can work out, and each part carries what it was drawn from.
- What the repository says about the parts that name it. A place is what `git ls-files` says it is: a file reaches one, a directory reaches what it holds, a name nothing tracks reaches nothing. A way to tell the work is done is run once and has to fail -- one that passes already says either the work is done or that it does not tell that it is not, and one nothing can run was invented.
- One screen showing the whole spec with what each inference was drawn from, and a question for each part nobody settled with answers to choose between. What a run does when it cannot get there is asked of a person and never of a model: the common unattended accident is an agent that could not pass a check and edited the check.
- The text the author wrote, kept beside the spec that was registered, since what registers is not what they typed.
- `--force`, which registers the instruction as written and asks nothing.
- A person is answered in the language they wrote in.

### Changed

- `task add` may refuse a command that registered a task before, and answers with `outcome`: `registered`, or `unconfirmed` when nothing was registered and something is still to be settled. Exit code 1 is now "nothing was registered", where before it was "the instruction was turned back".
- `task show` answers with `instruction` and `original`. Both are printed under their labels rather than beside them, because an instruction read from standard input runs to more than a column holds.
- A value in `params` crosses the socket in the form its argument table gives it -- a flag as a boolean, everything else as a string -- where `docs/ipc.md` had said every value crosses as a string. A flag of any other type is refused with code 2 rather than read as absent.
- A session assigns as many tasks at once as the budget allows rather than four, which bound before any budget worth declaring did.

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

[Unreleased]: https://github.com/BuildWithYJ/quota-cistern/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/BuildWithYJ/quota-cistern/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/BuildWithYJ/quota-cistern/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/BuildWithYJ/quota-cistern/releases/tag/v0.1.0
