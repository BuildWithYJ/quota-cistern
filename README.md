# quota-cistern

[한국어](README.ko.md)

> Delegates coding work to Claude Code within a budget you declare, and keeps each result on its own branch to review and apply.

A focused day exhausts the session limit early. By the time direction and structure are settled, there is little quota left for turning any of it into code. The limit resets overnight, but those are not hours anyone can spend working, and what goes unused does not carry over.

Spending quota requires a person present. Invoking the agent, reading what came back, and deciding what comes next all take human hours, so quota is only consumed inside the hours a person is working. Implementation, once its direction is settled, needs nobody watching, and the constraint applies to it all the same.

quota-cistern removes that constraint. Register the work to delegate, declare how much may be spent before you step away, and the tool runs tasks within the remaining quota and stops at the figure you declared. Each result is preserved on a branch of its own by the time you are back.

You could build something yourself to run an agent unattended. Preparing it and watching it takes human hours again, and without control over how much it spends, a bad run is only discovered afterwards.

## Requirements

- macOS or Linux. The daemon listens on a Unix socket and reads the vendor's limit through a pseudo-terminal.
- git.
- Claude Code on your `PATH`, already logged in. 0.1.0 was verified against 2.1.227.

## Getting started

Installing gives you two commands, `cisternd` and `cistern`.

```console
$ cargo install --git https://github.com/BuildWithYJ/quota-cistern cistern cisternd
```

`cisternd` is the daemon that runs the tasks and holds the state. Leave it running while you work; it says nothing while it is idle.

```console
$ cisternd
```

After that, `cistern` works from any directory. `--version` says whether the two sides are talking.

```console
$ cistern --version
cistern 0.1.0
core    0.1.0
```

Every command goes through the daemon, so without one a command prints `the core is not running` and exits 5. Ctrl-C stops the daemon.

Run `cistern task add` from the repository you want the work done in. It walks up from the current directory to find one and refuses when there is none, and it records the path it found, so moving that repository afterwards leaves the task without one.

After upgrading, restart the daemon. A command and a core of different versions refuse each other, and everything but `cistern --version` exits 5 until both sides are the same build.

One session runs at a time, and it opens when you run `run`.

## Workflow

### Registering work

A task carries a title and an instruction, and tasks in the backlog are assigned when a session opens. With `--after`, a task takes its predecessor's result branch as its base and continues from there, and the base branch and the model can both be set per task. A budget declared as a percentage is measured against the vendor's five-hour limit.

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
  repo:   ~/work/api

$ cistern task add --title "update docs" --instruction "document the new API"
task:2 added to backlog
  title:  update docs
  branch: main (base)
  repo:   ~/work/api

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
| `config set`, `config get` | The vendor |
| `task add`, `task rm`, `task show`, `backlog` | Registering and reading tasks |
| `run`, `interrupt` | Declaring a budget, running, stopping |
| `session ls`, `session show` | Reading sessions |
| `trace`, `diff` | Progress and changes |
| `review ls`, `apply`, `discard` | Review and disposition |

Arguments, output, and exit codes are in the [CLI specification](docs/cli.md).

## What the agent may do

The agent runs with `--permission-mode bypassPermissions`. A work area is not a sandbox, so a task can read and write outside it, and discarding the result does not undo those changes.

## Known limitations

- A budget declared as a percentage is measured against the vendor's limit, which is read from its status line, so `run --usage 50%` and `interrupt` wait up to 90 seconds and the daemon answers nothing else while they do. A budget declared in tokens does not read it.
- That reading depends on how Claude Code presents its limit, so a change there stops percentage budgets from working until this catches up.
- Work areas stay under the data directory and result branches stay in the repository. Nothing removes either for you.

## Contributing

The project is in early development and proposals are welcome. [CONTRIBUTING](CONTRIBUTING.md) has the development setup and the conventions, [docs/](docs/) has the structure and the design decisions, and the [v0.1.0 milestone](https://github.com/BuildWithYJ/quota-cistern/milestone/1) shows what is being built now.

## License

MIT. See [LICENSE](LICENSE).
