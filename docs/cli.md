# quota-cistern 0.1.0 — CLI specification

[한국어](cli.ko.md)

## 1. Global conventions

### Common flags

These apply to every command.

| Flag | Value | Default | Description |
| --- | --- | --- | --- |
| `-h`, `--help` | — | — | Prints usage to stdout and exits (code 0) |

`cistern --version` prints the version. Running `cistern` with no subcommand, or with invalid arguments, prints usage to stderr and exits with code 2.

### The core

Every command reaches the core, which owns what is stored. A command that finds no core running starts one and carries on, and the core it started keeps running afterwards. `--version` is the exception: it reports whether the two sides match, so it starts none.

The core is looked for beside the command line and then on the `PATH`. A command that cannot start one, or that starts one which stops before answering, says so and exits with code 5.

What the core writes goes to `$XDG_STATE_HOME/cistern/daemon.log`, or `~/.local/state/cistern/daemon.log` when that variable is unset. A core started by hand writes to the terminal instead.

### Exit codes

| Code | Meaning | Description |
| --- | --- | --- |
| 0 | Success | — |
| 1 | General failure | The operation was refused |
| 2 | Usage error | Bad argument or flag |
| 3 | Not found | No such session or task id |
| 4 | State conflict | Operation not possible in the current state |
| 5 | Core error | No core could be started, the one that was started stopped before answering, its version does not match the surface's, or it failed while handling the request |

### Output

Output is text. The output table in each command section names the fields that command answers with, and [the IPC document](ipc.md) records how they reach a surface. A value that is absent is shown in parentheses, such as `(none)`, whether it stands as a labelled field or as the whole output.

### Identifiers

- Session: `session:<n>` (for example `session:1`)
- Task: `task:<n>` (for example `task:1`)
- Branch: `cistern/<taskid>`, created by the core for each task

`<n>` is a monotonically increasing integer. Tasks and sessions have independent sequences, and numbers are never reused. The prefix may be omitted in command arguments.

### States

Task states:

| State | Meaning |
| --- | --- |
| `Pending` | In the backlog, not yet assigned |
| `Running` | Assigned to a session and executing |
| `Completed` | Finished. Result kept on the branch |
| `Interrupted` | Ended by budget hardlock or by the user. Partial work kept on the branch |
| `Error` | Failed during execution. Partial work, if any, kept on the branch |

Terminal states (`Completed`, `Interrupted`, `Error`) all leave a branch and enter the review queue. The disposition is recorded in `disposition`, separately from the task state.

Task `reason`: `budget hardlock` · `vendor limit` · `task ceiling` · `interrupted` · the execution failure.

`task ceiling` means the task consumed its own ceiling and stopped; the session continues. The user does not set this ceiling.

Session states:

| State | Meaning |
| --- | --- |
| `running` | The unattended loop is executing |
| `stopped` | Ended |

`stopped_reason`: `budget hardlock` · `vendor limit` · `observation unreadable` · `interrupted` · `all done` (every assigned task ended) · `blocked` · `error`.

`budget hardlock` means the declared budget was spent or the declared time ran out, `vendor limit` means the vendor blocked execution at its own limit, and `observation unreadable` means usage could no longer be read. `blocked` means tasks were left and every one of them waited on a task that did not complete; `retry` is what puts one of those back.

The tool never deletes or moves result branches. Pushing, merging, and cleanup are the user's own work.

### List output

List commands (`backlog`, `session ls`, `review ls`) succeed with code 0 even when empty, printing nothing.

## 2. Commands

Commands follow the loop and fall into four groups — tasks and backlog, sessions and execution, observation, review and disposition — with configuration standing apart, before any run.

### 2.1 Tasks and backlog

A task records the repository it was added from. The core takes it from the directory the command was run in, walking up to the repository root, and refuses the command when there is no repository there.

The backlog is stored at `$XDG_DATA_HOME/cistern/backlog.json`, or `~/.local/share/cistern/backlog.json` when that variable is unset.

#### `cistern task add`

Adds a task to the backlog as `Pending`. It is not assigned to a session directly; which session picks it up is decided by the core when a session opens.

```
cistern task add --title <T> --instruction <I> [--branch <B>] [--after <task>] [--model <M>]
```

**Arguments**

| Name | Required | Form | Description |
| --- | --- | --- | --- |
| `--title <T>` | yes | string | Task title |
| `--instruction <I>` | yes | string | Instruction for the agent. `-` reads from stdin |
| `--branch <B>` | no | branch name | Branch the task starts from. Defaults to `main`, or to the predecessor's result branch when `--after` is given |
| `--after <task>` | no | id | Predecessor task. This task is not assigned until that one completes |
| `--model <M>` | no | model name | Model for this task. Falls back to the session's `--model` |

The two may be given together. The task then waits for its predecessor and starts from the branch that was named.

With `--after`, the task is not eligible for assignment until the predecessor reaches `Completed`; if the predecessor ends in any other terminal state, the task stays `Pending`.

**Output**

| Field | Type | Description |
| --- | --- | --- |
| `id` | string | Task identifier |
| `title` | string | Task title |
| `base_branch` | string | Base branch |
| `after` | string | Predecessor task, or null |
| `model` | string | Model given for this task, or null |
| `repository` | string | Repository the task was added from |
| `state` | enum | `Pending` on creation |

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 2 | Argument error (missing `--title`) |
| 3 | The task named by `--after` does not exist |
| 4 | The command was not run inside a repository |
| 5 | Core error |

**Example**

```console
$ cistern task add --title "refactor X" --instruction "tidy up src/utils"
task:1 added to backlog
  title:  refactor X
  branch: main (base)
  repo:   ~/work/api
```

#### `cistern task rm`

Removes a task from the backlog. Only `Pending` tasks that have not been assigned are eligible; finished tasks leave the review queue through `discard`.

A task that was waiting for the removed one waits for what that one was waiting for, and waits for nothing when it was first in the chain. Its base branch follows, since the branch the removed task would have produced is never made.

```
cistern task rm <task>
```

**Output**

| Field | Type | Description |
| --- | --- | --- |
| `id` | string | Removed task |
| `title` | string | Task title |

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 3 | No such task |
| 4 | The task is not `Pending` |
| 5 | Core error |

**Example**

```console
$ cistern task rm 3
task:3 removed from backlog
```

#### `cistern backlog`

Lists `Pending` tasks that have not been assigned yet.

```
cistern backlog
```

**Output** — array of items

| Field | Type | Description |
| --- | --- | --- |
| `id` | string | Task identifier |
| `title` | string | Task title |
| `base_branch` | string | Base branch |

One item per line.

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 5 | Core error |

**Example**

```console
$ cistern backlog
○ task:1  refactor X         base main
○ task:2  add integration    base main
○ task:3  update README      base main
```

#### `cistern task show`

Prints the detail of one task.

```
cistern task show <task>
```

**Output**

| Field | Type | Description |
| --- | --- | --- |
| `id` | string | Task identifier |
| `session` | string | Session it was assigned to, or null |
| `state` | enum | Task state |
| `title` | string | Task title |
| `base_branch` | string | Base branch |
| `after` | string | Predecessor task, or null |
| `model` | string | Model the task ran on |
| `repository` | string | Repository the task was added from. Shown with the home directory as `~` |
| `branch` | string | Result branch, or null |
| `reason` | string | Reason it ended, or null |
| `worktree` | string | Path of the work area, or null once it has been cleaned up |
| `disposition` | enum | `applied` · `discarded` · null while undisposed |

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 3 | No such task |
| 5 | Core error |

**Example**

```console
$ cistern task show 2
task:2  Interrupted
  session:     session:1
  title:       add tests
  base:        main
  after:       (none)
  repo:        ~/work/api
  branch:      cistern/2
  reason:      budget hardlock
  worktree:    ~/.local/share/cistern/worktrees/2
  disposition: (none)
```

### 2.2 Sessions and execution

Sessions are stored at `$XDG_DATA_HOME/cistern/sessions.json`, or `~/.local/share/cistern/sessions.json` when `XDG_DATA_HOME` is unset.

Every run of every task is appended to `$XDG_DATA_HOME/cistern/runs.jsonl`, one line each, and nothing rewrites a line already there. A run of a session declared as a share also records how far the vendor's limit was spent when it started and when it stopped, which is what that run cost in the unit the share was declared in. Both are readings the session had already taken, so writing them down asks the vendor nothing further. A task runs more than once when the vendor turns it away, and the backlog keeps only the most recent, which is what `task show` reports. What a budget is worked out from is read from the ledger instead, so a second run does not displace the first. Nothing removes the file for you.

A session is held to the time it declared whether or not anything ends. Nothing else asks about a session until one of its tasks ends, and a session with one long run going would otherwise pass its deadline unnoticed.

A task runs in a checkout of its own, made with `git worktree` under `$XDG_DATA_HOME/cistern/worktrees`.

#### `cistern run`

Declares a budget and starts the session's unattended loop. It is non-blocking and returns at once.

```
cistern run --usage <N> --time <T> [--model <M>]
```

**Arguments**

| Name | Required | Form | Example | Description |
| --- | --- | --- | --- | --- |
| `--usage <N>` | yes | percentage or token count | `50%` · `2M` | With `%`, a share of the vendor's five-hour limit; without it, a token count |
| `--time <T>` | yes | duration | `8h` · `2h30m` | Time limit |
| `--model <M>` | no | model name | `opus` · `sonnet` | Default for tasks that name no model. Falls back to the vendor default |

On start the core assigns some of the backlog and runs those tasks in parallel. Assignment is dynamic: each time a task ends, the core decides from what that task actually consumed whether one more fits in the remaining budget, and tasks that are not assigned stay `Pending` in the backlog. A task whose predecessor has not reached `Completed` is not eligible.

Tasks within a session run in parallel, but only one session runs at a time. A second `run` while a session is running is refused.

`%` is a share of the vendor's five-hour limit, a whole number from 1 to 100; a token count is a whole number and takes the suffixes `K` (=1,000) and `M` (=1,000,000).

What is reported as consumed is approximate.

The session stops automatically at whichever runs out first, usage or time. Consumption is reported in the unit that was declared: `%` for a percentage declaration, tokens for a token one.

**Output**

| Field | Type | Description |
| --- | --- | --- |
| `session` | string | The session created and started |
| `state` | enum | `running` |
| `assigned` | int | Tasks assigned at start. Assignment is dynamic, so this grows afterwards |
| `budget` | object | The declared budget (usage, time) |

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Started |
| 1 | No task available to assign |
| 2 | Malformed argument (for example `--time 8x`) |
| 4 | Another session is running |
| 5 | Core error |

**Example**

```console
$ cistern run --usage 50% --time 8h
session:1 running (2 tasks assigned to start)
  budget:  usage 50% · time 8h
  observe: cistern trace <task> --follow
  stop:    cistern interrupt
```

#### `cistern interrupt`

Stops the running session. Only one session runs at a time, so no target is given; tasks still running end as `Interrupted`.

```
cistern interrupt
```

**Output**

| Field | Type | Description |
| --- | --- | --- |
| `session` | string | The session that was stopped |
| `state` | enum | `stopped` |
| `interrupted_tasks` | array | Ids of tasks that ended as `Interrupted` |
| `consumed` | object | Measured consumption (usage, time) |

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 4 | No session is running |
| 5 | Core error |

**Example**

```console
$ cistern interrupt
session:1 interrupted
  task:2 → Interrupted
  consumed 38% · time 2h05m
```

#### `cistern session ls`

Lists sessions, newest first.

```
cistern session ls [--page <N>] [--limit <M>]
```

**Arguments**

| Name | Required | Form | Description |
| --- | --- | --- | --- |
| `--page <N>` | no | integer ≥1 | Page number. Defaults to 1 |
| `--limit <M>` | no | integer | Items per page. Defaults to 20 |

**Output** — array of items

| Field | Type | Description |
| --- | --- | --- |
| `id` | string | Session identifier |
| `state` | enum | Session state |
| `consumed` | string | Consumption |
| `task_count` | int | Number of tasks in the session |
| `updated_at` | string | Last update |

One session per line, newest first.

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 2 | Argument error (for example `--page 0`) |
| 5 | Core error |

**Example**

```console
$ cistern session ls
session:3  running    usage 12%   2 tasks   just now
session:1  stopped    usage 50%   3 tasks   3h ago
```

#### `cistern session show`

Prints the detail of one session, including its task list.

```
cistern session show <session>
```

**Output**

| Field | Type | Description |
| --- | --- | --- |
| `budget` | object | The declared budget (usage, time) |
| `consumed` | object | Measured consumption (usage, time) |
| `stopped_reason` | enum | Why it stopped, or null while running |
| `resets_at` | string | When the vendor limit resets. Present only after `vendor limit` |
| `updated_at` | string | Last update |
| `tasks` | array | Tasks in the session, each with id, state, title, branch, reason |

`stopped_reason` is shown in parentheses on the first line; while running the line shows `running` and no reason.

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 3 | No such session |
| 5 | Core error |

**Example**

```console
$ cistern session show 1
session:1  stopped (budget hardlock)
  budget:   usage 50% · time 8h
  consumed: usage 50% · time 3h12m
  tasks:
    ✓  task:1  Completed    refactor X     → cistern/1
    ⚠  task:2  Interrupted  add tests      → cistern/2  (budget hardlock)
    ✕  task:4  Error        update docs    → cistern/4  (process died)
```

### 2.3 Observation

#### `cistern trace`

Reads a task's trace. While the task runs this returns the trace so far; after it ends, the stored trace.

```
cistern trace <task> [--follow] [--since <cursor>]
```

**Arguments**

| Name | Required | Form | Description |
| --- | --- | --- | --- |
| `<task>` | yes | id | Target task |
| `--follow` | no | flag | Keeps printing new trace while the task runs, and ends by itself when the task ends |
| `--since <cursor>` | no | cursor | Prints only what follows that point |

The trace is the agent's own output, which the core stores append-only. This command returns the events so far and the next cursor.

**Output**

| Field | Type | Description |
| --- | --- | --- |
| `events` | array | Trace events in time order |
| `cursor` | string | Where to resume |
| `done` | bool | True once the task is terminal and the trace can no longer grow |

One event per line, the time it happened followed by what it said.

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 3 | No such task |
| 5 | Core error |

**Example**

```console
$ cistern trace 1
[11:19:36] I'll start implementing app/scoring.py based on the specification.
[11:19:37] Read SPEC.md
[11:20:13] Read app/scoring.py
[11:20:13] failed: File does not exist
[11:20:20] Write app/scoring.py
[11:20:23] Bash python3 -m pytest tests/test_scoring.py -v
[11:20:36] All done! I've implemented app/scoring.py with two functions.
```

#### `cistern diff`

Prints what the task changed on its branch.

```
cistern diff <task> [--stat]
```

**Arguments**

| Name | Required | Form | Description |
| --- | --- | --- | --- |
| `<task>` | yes | id | Target task |
| `--stat` | no | flag | Per-file summary only, files changed with insertions and deletions |

**Output**

| Field | Type | Description |
| --- | --- | --- |
| `base` | string | Base branch |
| `branch` | string | Result branch |
| `files` | array | Per file: `path`, `added`, `removed` |
| `patch` | string | Unified diff |

A standard unified diff, or the per-file summary with `--stat`. With no changes it prints `(no changes)`.

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 1 | The branch holds no change, or it cannot be read |
| 3 | No such task |
| 5 | Core error |

**Example**

```console
$ cistern diff 1 --stat
 src/utils/index.ts   | 12 +++---
 src/utils/graph.ts   | 40 ++++++++++----
 2 files changed, 34 insertions(+), 18 deletions(-)
```

### 2.4 Review and disposition

`review ls` lists what is waiting to be disposed of, and `apply`, `discard`, and `retry` dispose of it. None of them changes a branch.

#### `cistern retry`

Puts a task that ended back in the backlog, so the next session may take it again.

```
cistern retry <task>
```

For a task cut off at its ceiling, or one that failed. The branch its last run left stays, and the run that starts next starts from it. A task waiting again is out of the review queue and back in the backlog, which is what lets the tasks waiting on it run.

**Output**

| Field | Type | Description |
| --- | --- | --- |
| `task` | string | The task now waiting |
| `branch` | string | The branch its last run left, which stays |
| `attempts` | string | How many times it has been assigned so far |

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 3 | No such task |
| 4 | The task has not ended |
| 5 | Core error |

#### `cistern review ls`

Lists tasks waiting for disposition across all sessions. `Completed`, `Interrupted`, and `Error` appear together.

```
cistern review ls
```

**Output** — array of items

| Field | Type | Description |
| --- | --- | --- |
| `id` | string | Task identifier |
| `title` | string | Task title |
| `session` | string | Session it came from |
| `branch` | string | Result branch |
| `state` | enum | Terminal state |
| `commit_count` | int | Commits on the result branch |
| `base_ahead` | int | Commits the base branch has gained since the task diverged |

One task per line.

A disposed task leaves the queue. `base_ahead` is computed on every query. A task whose branch cannot be read stays in the queue, with both counts absent.

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 5 | Core error |

**Example**

```console
$ cistern review ls
✓  task:5  verify webhook signature   session:1  → cistern/5  Completed    3 commits
⚠  task:6  test report generation     session:2  → cistern/6  Interrupted  1 commit · base +2
```

#### `cistern apply`

Applies the result branch's changes to the working tree of the repository the task was added from. It does not commit, and it does not move or delete any branch.

```
cistern apply <task>
```

The range applied runs from where the base branch and the result branch diverged up to the result branch, the same range `diff` uses.

The command is refused if the working tree has uncommitted changes, and if applying would conflict, nothing is applied. It reads from the branch, so it still works after the worktree has been cleaned up.

**Output**

| Field | Type | Description |
| --- | --- | --- |
| `task` | string | The task disposed of |
| `branch` | string | The result branch that was read |
| `files` | array | Per applied file: `path`, `added`, `removed` |

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 1 | Nothing to apply, or it is in the working tree already, or applying would conflict |
| 3 | No such task |
| 4 | The task has not ended, or the working tree has uncommitted changes |
| 5 | Core error |

**Example**

```console
$ cistern apply 5
task:5 applied to working tree
  src/webhook/verify.ts   +64 -3
  src/webhook/index.ts     +8 -1
  (nothing committed · review and commit in your own environment)
```

#### `cistern discard`

Removes a task from the review queue. It changes neither the branch, the worktree, nor the task state.

```
cistern discard <task>
```

The result branch stays, so a disposed task can still be read with `task show` and applied later.

**Output**

| Field | Type | Description |
| --- | --- | --- |
| `task` | string | The task disposed of |
| `branch` | string | The result branch, which stays |

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 3 | No such task |
| 4 | The task has not ended |
| 5 | Core error |

**Example**

```console
$ cistern discard 6
task:6 discarded
  branch cistern/6 is kept
```

### 2.5 Configuration

#### `cistern config`

Sets the vendor.

```
cistern config set <key> <value>
cistern config get [<key>]
```

**Keys**

| Key | Value | Description |
| --- | --- | --- |
| `vendor` | a name a definition exists for | The agent to run. `claude` ships with the daemon, and a file at `$XDG_CONFIG_HOME/cistern/vendors/<name>.toml` adds a name or lays over one that ships, `claude` included. A file laid over another replaces an array whole rather than adding to it, so an override that changes `args` has to keep whatever `answer.reader` reads |

Configuration is stored at `$XDG_CONFIG_HOME/cistern/config.toml`, or `~/.config/cistern/config.toml` when that variable is unset.

**Output**

`set` prints the key and value it applied; `get` prints the current configuration, or a single value when given a key.

**Exit codes**

| Code | Condition |
| --- | --- |
| 0 | Success |
| 2 | Unknown key or value |
| 5 | Core error |

**Example**

```console
$ cistern config set vendor claude
vendor = claude

$ cistern config get
vendor: claude
```
