---
status: proposed
date: 2026-08-30
---

# 0003. Draft features with Spec Kit, publish only what a person wrote

## Context

Work here reaches code through a chain nobody wrote down: an issue states a want, discussion settles a direction, and a branch turns it into commits. What the feature had to satisfy is recoverable only by reading the pull request afterwards. `docs/adr/` records decisions once they are made, and `docs/` describes what exists, but neither holds the step between a want and an implementation.

[Spec Kit](https://github.com/github/spec-kit) covers that step. Its `specify` CLI installs skills that walk a feature from description to specification, plan, and tasks, and it supports Claude Code, which is the agent this project already delegates to.

Four things about this repository decide how it can be installed.

`scripts/check.sh` requires that every `*.sh`, `*.yml`, and `*.yaml` file in the working tree hold printable ASCII only, and it builds that list with `find`, not from git. Spec Kit 1.0.1 installs six bash scripts and a workflow file under `.specify/`, and four of them — `common.sh`, `check-prerequisites.sh`, `create-new-feature.sh`, and `workflows/speckit/workflow.yml` — carry non-ASCII punctuation across seventeen lines.

`CONTRIBUTING.md` names the branch types a ruleset accepts and reserves `cistern/*` for the branches the tool itself creates. Spec Kit's optional git extension creates a branch per feature, named `NNN-slug`.

`create-new-feature.sh` writes each feature to `$REPO_ROOT/specs`. A specification, a plan, and a task list produced by an agent are drafts. Everything under `docs/` was written by a person and reviewed as prose.

`.claude/` holds no tracked file today, only per-checkout state. Installing the Claude Code integration puts ten skills there, which every clone needs and no clone should have to install by hand.

## Decision

Spec Kit is installed with the Claude Code integration and no extensions, so `.specify/` and `.claude/skills/speckit-*/` are tracked and the workflow is available to anyone who clones the repository. Without the git extension, creating a feature only writes a directory; branches stay hand-made under the rule `CONTRIBUTING.md` already states. Because `.claude/` becomes a tracked directory, the two per-checkout files that already sat in it are ignored by name, so that a later `git add` cannot sweep a machine's own settings into a commit.

`scripts/check.sh` excludes `.specify/` from the ASCII sweep. Those files are vendored and `specify` rewrites them on upgrade, so the rule about what we write does not reach them.

`specs/` is a symlink into `.private/docs/`, and git ignores it. A specification, plan, and task list are drafts and stay unpublished. What survives review reaches `docs/` or `docs/adr/` as prose a person wrote.

`.specify/memory/constitution.md` is filled in, and it is the one file inside `.specify/` we maintain. It names the rules in `CONTRIBUTING.md` and the ADRs rather than restating them, so that there is nothing in it to fall out of step, and it lists the constraints a generated plan is most likely to break.

We give up having the reasoning behind a feature visible in the repository at the moment it is written.

## Consequences

- Good, because a feature's requirements are settled and written down before an agent is asked to implement it, which is what `cistern task add` already demands of a task.
- Good, because the adoption is visible in the repository as configuration rather than as generated prose.
- Good, because the ASCII exclusion holds across versions: it names a directory rather than the lines that offend today.
- Neutral, because the numbered `NNN-slug` directories are a numbering convention inside `.private/docs/specs/` only, and no branch takes that name.
- Bad, because `.specify/memory/constitution.md` is ours inside a directory that is otherwise vendored, so an upgrade has to be read for what it did to that one file.
- Bad, because a draft and the document it becomes are kept in two places, so a published document can fall behind the specification it came from.
- Bad, because a reader outside the project cannot see the specification a change was built against.

## Alternatives considered

### Track `specs/` in this repository

Rejected. It puts generated drafts beside documents written and reviewed line by line, and lowers what a reader can assume about anything under a documentation path. Adoption is already evident from `.specify/` and this record.

### Install the git extension and set `branch_prefix` to a permitted type

Rejected. `feat/{number}-{slug}` would satisfy the ruleset, but `CONTRIBUTING.md` tells contributors to keep numbers out of branch names, and a second system creating branches sits next to the `cistern/*` branches the tool creates. Nothing is gained that `git checkout -b` does not already do.

### Patch the vendored scripts to ASCII instead of excluding them

Rejected. `specify` rewrites those files on every upgrade, so the edit would have to be reapplied each time, and a missed upgrade fails CI rather than the check it was meant to serve.

### Leave the constitution as the template ships it

Rejected. The template is placeholders, and an agent reading it learns nothing, so every generated plan would have to be corrected on the same four points. Writing it once is the cheaper side of that trade, and keeping it to references rather than restatements is what keeps it from going stale.

## References

- `scripts/check.sh`, the `ascii_only` step and its `sources` list.
- `CONTRIBUTING.md`, branch naming and the conventions the constitution refers to.
- `.specify/memory/constitution.md`, which points at those conventions rather than restating them.
- `.specify/scripts/bash/create-new-feature.sh`, where `SPECS_DIR` is set.
