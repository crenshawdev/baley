# 0040: Judge and remember a commit at its target checkout

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-06 |
| Deciders | John Crenshaw |
| Design document | [0010: Guard](../design/0010-guard.md) |
| Supersedes | [0036](0036-per-user-guard-records.md), in part: the checkout used to key remembered denials |
| Superseded by | |

## Context and problem

The guard gets policy from the session project at `CLAUDE_PROJECT_DIR`. A shell command can commit in another checkout through git's `-C` option. Reading the branch at the hook's working directory can then pass a commit to a protected branch or deny one on an unprotected branch ([issue 212](https://github.com/crenshawdev/baley/issues/212)). Some commands change directory or git environment before the commit, so their target cannot be established by the guard's bounded scan.

[ADR 0036](0036-per-user-guard-records.md) keys remembered denials by the session project's canonical root, the canonical checkout root containing the hook's working directory, and the host. Once the branch comes from the commit's target, that checkout component must name the same target. Otherwise torn settings could apply a denial remembered for another checkout.

## Decision drivers

- Judge the branch at the checkout the commit lands in when the command establishes it.
- Ask when the command does not establish one target checkout.
- Keep policy with the session project, regardless of the commit's target.
- Keep remembered denials isolated by target checkout and preserve recorded answers on redelivery.
- Keep the existing launch count, budget, storage port and event fields.

## Considered options

1. Resolve the commit's target, read its branch and key remembered denials by its checkout root
2. Keep reading and remembering the hook cwd's checkout
3. Ask for every redirected commit

## Decision

Chosen option: **1**, because it applies the session project's branch rules where the commit lands and separates remembered denials by that same checkout.

The scanner gives a commit `Cwd`, an ordered `Directory` list of git `-C` operands, or `Unestablished`. Starting at the hook cwd, each operand is joined to the previous directory; an absolute operand replaces it. Path components are preserved for filesystem resolution. `--work-tree` and `-c` do not redirect the checkout. `--git-dir` in either form, an earlier segment led by `cd`, `pushd`, `popd` or `export`, an empty or expanding `-C` operand, or commits naming different targets make it unestablished. An expanding operand starts with `~` or contains `*`, `?` or `[`. Only the first word of a segment counts as a directory or environment change. Push still takes precedence and always asks in a bound project.

In a bound project an unestablished target asks with one fixed reason, with no branch or settings read. It is recorded like every other ask, and an unrecordable ask becomes a deny under GRD-R9. With no project bound, commands still pass with nothing recorded.

The branch lookup and the bounded `.git/HEAD` fallback use the resolved commit directory, with the same single branch launch and budget grant. Policy continues to come only from the session project. A walk from the resolved directory supplies the canonical target checkout root for remembered denials. A failed target walk makes the decision unrecordable; it never reuses the hook cwd's root. This replaces only ADR 0036's choice of checkout root. Its per-user records, replay binding and bounded access remain in force.

`guard.answered.target` holds a redirected commit's resolved directory as well as a path tool's resolved target. The hook cwd stays in `cwd`. A commit at cwd and an unestablished target keep `target` null. Both guard events remain version 1: no field shape or reader changes, and old records remain readable. The policy view already keys on a target checkout root and needs no version change. Existing recorded answers still replay under GRD-R10, without rejudging the command.

## Consequences

### Positive

- A commit redirected to `main` is judged on `main`, even when the hook cwd is on a task branch.
- The session project's policy remains authoritative across checkouts.
- Remembered denials cannot cross from the cwd checkout into a different commit target.
- Recorded redirected answers identify the directory whose branch was read.
- No new git launch, dependency, storage port change or event migration is needed.

### Negative

- Ambiguous targets ask even when a shell could resolve them, and an unrecordable ask denies.
- Different operand lists are conservatively treated as different targets even if they might reach the same directory.
- An old denial recorded for a redirected command was keyed by cwd. It stays under that old key, since the old event lacks enough information to move it reliably.
- Branch reads remain observations before execution; a later branch change is not prevented by this hook.

### Follow-up

- Build 3 T12 owns observing these answers and redelivery on the live host, as described in design 0010. The in-process tests establish scanner, decision and record behavior, not the assembled host workflow.

## Options in detail

### Resolve the target and use its checkout root (chosen)

The scan retains the information needed for a literal `-C` chain. The hook can read the correct branch without another git launch, and the existing checkout walk provides the remembered-denial key. An explicit unknown target makes ambiguity visible to the owner.

### Keep reading and remembering the hook cwd's checkout

This keeps the old code but preserves both errors: a redirected protected commit can pass, and a redirected unprotected commit can be denied. The recorded branch describes the wrong checkout.

### Ask for every redirected commit

This avoids using the cwd branch but ignores branch rules even when a literal directory is known. A protected target under `refuse` would become an ask instead of the required denial.
