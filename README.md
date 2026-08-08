# quota-cistern

[한국어](README.ko.md)

> Runs delegated coding work within a budget you declare, and keeps each result on its own branch to review and apply. Early development, with no usable release yet.

A focused day exhausts the session limit early. By the time direction and structure are settled, there is little quota left for turning any of it into code. The limit resets overnight, but those are not hours anyone can spend working, and what goes unused does not carry over.

Spending quota requires a person present. Invoking the agent, reading what came back, and deciding what comes next all take human hours, so quota is only consumed inside the hours a person is working. Implementation, once its direction is settled, needs nobody watching, and the constraint applies to it all the same.

quota-cistern removes that constraint. Register the work to delegate, declare how much may be spent before you step away, and the tool runs tasks within the remaining quota and stops at the figure you declared. Each result is preserved on a branch of its own by the time you are back.

You could build something yourself to run an agent unattended. Preparing it and watching it takes human hours again, and without control over how much it spends, a bad run is only discovered afterwards.

## Workflow

### Registering work

A task carries a title and an instruction, and tasks in the backlog are assigned when a session opens. With `--after`, a task takes its predecessor's result branch as its base and continues from there, and the base branch and the model can both be set per task. Declaring a budget as a percentage requires a plan, which is set once with `config set plan`.

### Unattended execution

Declare usage and time, and the session ends at whichever runs out first. Reaching that point does not only stop new assignments, it interrupts the tasks still running. The backlog is not started all at once either: each time a task ends, the tool reads what that task actually consumed and decides whether one more fits, because consumption varies per task and cannot be known before the task runs.

Each task runs in a work area and on a branch of its own, so tasks running in parallel cannot affect each other's results. Interrupting a session with `interrupt` still preserves the work done up to that point on the branch.

### Reviewing results

Every task that ends is up for review, whether it completed, was interrupted, or failed, and the reason the session stopped is recorded alongside it. The agent's output is kept per task and readable both while the task runs and after it ends, and the changes can be read as a diff or summarised per file with `--stat`.

`apply` puts the changes into your working tree without committing them, and `discard` takes a task off the list without deleting its branch, so it can still be applied later. The tool does not modify a result branch after creating it, and it neither merges nor pushes, so what becomes of that branch is yours to decide.

### Example

```console
$ cistern task add --title "refactor utils" --instruction "tidy up src/utils"
task:1 added to backlog
  title:  refactor utils
  branch: main (base)

$ cistern task add --title "update docs" --instruction "document the new API"
task:2 added to backlog
  title:  update docs
  branch: main (base)

$ cistern run --usage 50% --time 8h
session:1 running (2 tasks assigned to start)
  budget:  usage 50% · time 8h
  observe: cistern trace <task> --follow
  stop:    cistern interrupt

# after the session ends

$ cistern review ls
✓  task:1  refactor utils  session:1  → cistern/1  Completed    3 commits
⚠  task:2  update docs     session:1  → cistern/2  Interrupted  1 commit

$ cistern apply 1
task:1 applied to working tree
  (nothing committed · review and commit in your own environment)
```

## Commands

| Command | What it does |
| --- | --- |
| `config set`, `config get` | Vendor and plan |
| `task add`, `task rm`, `task show`, `backlog` | Registering and reading tasks |
| `run`, `interrupt` | Declaring a budget, running, stopping |
| `session ls`, `session show` | Reading sessions |
| `trace`, `diff` | Progress and changes |
| `review ls`, `apply`, `discard` | Review and disposition |

Every command takes `-o json` for machine-readable output. Arguments, output, and exit codes are in the [CLI specification](docs/cli.md).

## Getting started

There is no release yet. Building from source is covered in [CONTRIBUTING](CONTRIBUTING.md).

0.1.0 covers one vendor, `claude`, on a single machine. One session runs at a time, and it opens when you run `run`.

## Contributing

The project is in early development and proposals are welcome. [CONTRIBUTING](CONTRIBUTING.md) has the development setup and the conventions, [docs/](docs/) has the structure and the design decisions, and the [v0.1.0 milestone](https://github.com/BuildWithYJ/quota-cistern/milestone/1) shows what is being built now.

## License

MIT. See [LICENSE](LICENSE).
