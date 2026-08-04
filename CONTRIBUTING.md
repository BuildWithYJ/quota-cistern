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

Use `<type>/<short-description>`. `<type>` matches the commit rule below, and the description is lowercase kebab-case. You may prefix the related issue number.

```
feat/budget-hardlock
fix/session-race
docs/cli-exit-codes
feat/12-budget-hardlock   # issue #12
```

> The `cistern/*` prefix is reserved for the result branches the tool creates for each task. Do not use it for contribution branches.

### Commit and PR titles

Commit messages and PR titles follow [Conventional Commits](https://www.conventionalcommits.org/).
`<type>` is one of `feat` · `fix` · `docs` · `refactor` · `test` · `chore` (for example `feat: …`, `fix: …`).

## Development environment

Build and test steps will be added here once the stack is in place.

## Code of conduct

Taking part means you agree to the [Code of Conduct](./CODE_OF_CONDUCT.md).
