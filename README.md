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

## Usage

A task carries a title and an instruction for the agent.

```console
$ cistern task add --title "refactor utils" --instruction "tidy up src/utils"
task:1 added to backlog
```

A run declares how much of the quota it may spend and for how long. The command returns at once and the session keeps going.

```console
$ cistern run --usage 50% --time 8h
session:1 running (2 tasks assigned to start)
  budget:  usage 50% · time 8h
```

Whatever a task ends up as, its result is kept on a branch of its own.

```console
$ cistern review ls
✓  task:1  refactor utils  session:1  → cistern/1  Completed    3 commits
⚠  task:3  update docs     session:1  → cistern/3  Interrupted  1 commit

$ cistern apply 1
$ cistern discard 3
```

`apply` brings a task's changes into your working tree, and `discard` drops it from the list. Neither one commits, pushes, merges, or deletes anything.

Every command, flag, and exit code is in the [CLI specification](docs/cli.md).

---

## Contributing

The project is in early development and proposals are welcome. [CONTRIBUTING.md](CONTRIBUTING.md) has the development setup, the conventions, and how the code is arranged.
