# Contributing

quota-cistern is in early development. Proposals and discussion are welcome at any time.

[한국어](CONTRIBUTING.ko.md)

## Discussion and proposals

For bugs, ideas, and questions, open an [issue](../../issues).\
When something needs to be discussed, post in [Discussions](../../discussions).\
All code work follows discussion. If you would like to contribute, please pick up a task once it has been assigned to you.

## Pull requests

1. Fork the repository and branch from `main`.
2. Keep each change small and to a single logical unit.
3. Open a PR and link the related issue.

### Branch naming

Use `<type>/<short-description>`. `<type>` matches the commit rule below, and the description is lowercase kebab-case. Do not put the issue number in the branch name; the pull request links the issue.

```
feat/budget-hardlock
fix/session-race
docs/cli-exit-codes
```

A ruleset rejects a branch pushed to this repository whose name does not start with one of those types. A branch in your own fork is not covered, so name it by the same rule.

> The `cistern/*` prefix is reserved for the result branches the tool creates for each task. Do not use it for contribution branches.

### Commit and PR titles

A pull request title follows [Conventional Commits](https://www.conventionalcommits.org/).
`<type>` is one of `feat` · `fix` · `docs` · `refactor` · `test` · `chore` (for example `feat: …`, `fix: …`). CI checks the title.

Pull requests are squashed on merge, so the title becomes the commit message on `main`. The commit messages on your branch are not checked; write them for whoever reads the branch.

## Development environment

The project is written in Rust, edition 2024. `rust-toolchain.toml` pins the toolchain, so `rustup` installs the right version and components on first build.

```console
$ cargo build
$ cargo test
$ ./scripts/check.sh
```

`scripts/check.sh` runs formatting, lints, tests, and a check that code files contain only ASCII. CI runs the same script, so a local run and a CI run cover the same ground. Every step runs even after one fails, so a single run reports all of them.

### Conventions

- Formatting follows rustfmt defaults. `cargo fmt` before you push.
- Lints run at clippy's default level with warnings treated as errors.
- API design follows the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/).
- A doc comment states what an item does. An ordinary comment records why the code has the shape it has, which is what the next person needs in order to change it — in shell and workflow files as much as in Rust. A placeholder value says that it is one, and what would settle it.
- `unwrap`, `expect`, `panic`, and `unsafe` are denied at the workspace level. Tests are exempt: they may panic to signal failure.
- A module's tests go in a file of their own, declared as `#[cfg(test)] mod tests;` and written at `<module>/tests.rs`. They stay a child module, so they still reach what the module keeps private, and the code being tested stays readable on its own. Older modules still carry their tests at the foot of the file; they move as they are worked on rather than all at once.
- Comments, documentation, and commit messages are written in English.
- Code files hold tab, newline, and printable ASCII only. Markdown is exempt, since prose uses characters outside that set and the `*.ko.md` files are Korean translations.

`check.sh` does not enforce all of these. The API guidelines and the substance of a doc comment are read in review.

## Code of conduct

Taking part means you agree to the [Code of Conduct](./CODE_OF_CONDUCT.md).
