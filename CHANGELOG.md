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
