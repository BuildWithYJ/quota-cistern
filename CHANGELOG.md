# Changelog

Notable changes to this project are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- An admission gate at `task add`. An instruction that points at no place to work and gives no way to tell the work is done is turned back before the backlog is read, and the refusal names what it did not find. An unattended run cannot stop to ask, so an instruction carrying too little spends its budget on a guess. Both signals are read by rule rather than by a model, so the answer is the same every time.
- `--force`, which registers a task as written, for an author who knows better or means to fill it in after. The refusal says so, since the flag is spelled on the command line and nowhere else.
- Filling a loose instruction in from the repository rather than turning it back: from the file the author is in the middle of, from a file the repository holds by a word the instruction used, or from what a model proposed when no rule could settle it.
- The text the author wrote, kept beside an instruction that was filled in. It is kept only when the run is given something other than what was typed, so its presence is what says a fill happened.

### Changed

- `task add` refuses with exit code 1 when it turns an instruction back. A command that registered a task before may now be refused; `--force` registers it as written.
- `task show` answers with `instruction` and `original`. Both are printed under their labels rather than beside them, because an instruction read from standard input runs to more than a column holds.
- A value in `params` crosses the socket in the form its argument table gives it -- a flag as a boolean, everything else as a string -- where `docs/ipc.md` had said every value crosses as a string. A value of any other type is refused with code 2 rather than read as absent.

### Deprecated

### Removed

### Fixed

### Security

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

[Unreleased]: https://github.com/BuildWithYJ/quota-cistern/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/BuildWithYJ/quota-cistern/releases/tag/v0.1.0
