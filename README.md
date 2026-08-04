# quota-cistern

[한국어](README.ko.md)

> Budget-based workload scheduler for coding agents.\
> Runs delegable coding work on isolated agents, unattended, while quota is left over.

**Status:** early development (`v0.1.0` in progress). No usable release yet.

---

## Why quota-cistern

> For the developer who maxes out the session limit every day and still ends the week at half the weekly quota.

When you do agentic coding on a subscription plan, two things compete for the same limit: the hours you spend setting direction and standards, and the hours an agent spends executing. Focused work drains the limit, and whatever is left while you sleep or step away goes unused.

quota-cistern runs delegable work during that leftover quota, so a limited plan is spent rather than wasted. The point is not a task queue that consumes the limit in a fixed order. It is treating usage itself as a schedulable resource.

---

## Documentation

- [CLI specification](docs/cli.md) — commands, flags, exit codes, JSON output
- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security policy](SECURITY.md)
- [License](LICENSE)
