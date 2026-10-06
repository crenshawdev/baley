# 0015: Repository upkeep and the build gate

| | |
|---|---|
| Status | Accepted |
| Design issue | [#47](https://github.com/crenshawdev/baley/issues/47) |
| Requirement prefix | UPK |
| Applies | [0002: System design](0002-system-design.md) |
| Related | ADRs: [0009](../adr/0009-served-instructions.md), [0027](../adr/0027-vendor-folders-and-plain-keys.md), [0033](../adr/0033-host-security-bar.md), [0034](../adr/0034-one-server-per-session.md) · C4 view: not applicable (no runtime component) |

The current design of this area, and nothing else. Edit it in place when the design changes; git holds the history. It describes the design only, never the work still to do.

## 1. Purpose and scope

Baley uses one public name, describes its built state accurately, and requires a reproducible set of checks before changes reach `main`. This area defines those names, the treatment of project history, repository metadata and labels, the formatting gate, and the evidence used to assess the test suite's cost.

The host interface in [0012](0012-host-interface.md) owns installation and protocol behavior. How Baley is delivered, who writes the host files and how an existing registration is replaced wait on the owner's delivery decision under [HST-R17](0012-host-interface.md#3-requirements), carried by [#24](https://github.com/crenshawdev/baley/issues/24) and the [roadmap's held Build 3 tasks T14 to T17](../roadmap.md#build-3-hosts). The evidence ledger in [0001](0001-evidence-ledger.md) owns storage replacement and compatibility. The release gate belongs to [#14](https://github.com/crenshawdev/baley/issues/14), and the architecture overview to [#44](https://github.com/crenshawdev/baley/issues/44).

Cadence is Baley's predecessor project, a separately maintained tool that can still be registered in the same host.

This design changes no runtime architecture, dependency, port or adapter. It produces no new ADR and changes no accepted ADR body.

## 2. Terms

This area uses the existing [glossary](../../CONTEXT.md) terms without changing their meanings.

| Term | Meaning here |
|---|---|
| Host | Claude Code, the program whose session connects to Baley over MCP ([0012](0012-host-interface.md#2-terms)) |
| Server | The Baley process one host session starts over stdio ([0012](0012-host-interface.md#2-terms)) |
| Stub | A file a host needs on disk to list or launch something, rendered by Baley from its tables and pointing at Baley ([0012](0012-host-interface.md#2-terms)) |
| Build | A slice of Baley's own development in the [roadmap](../roadmap.md), separate from a managed project's phase |
| Requirement | A numbered design rule with a stable identifier, reason and status |

## 3. Requirements

| Id | Rule | Why | Depends on | Status |
|---|---|---|---|---|
| UPK-R1 | The executable and MCP server are `baley`; the tools are `baley_version`, `baley_query` and `baley_apply`; instruction and agent identities use `bal-`. | Host configuration, advertised tools and instructions must agree. | HST-R5, HST-R12 | Active |
| UPK-R2 | Installation never replaces a separately owned Cadence registration by name alone; registration replacement waits on the owner's delivery decision under [HST-R17](0012-host-interface.md#3-requirements), carried by [#24](https://github.com/crenshawdev/baley/issues/24) and the [held Build 3 tasks T14 to T17](../roadmap.md#build-3-hosts). | An occupied name does not establish ownership. | HST-R16, HST-R17 | Active |
| UPK-R3 | Existing Baley spellings in inherited configuration, records and renderers remain consistent until their consumers are removed; upkeep does not rewrite stored bytes. | Record markers and hash inputs are contracts, not display text. | EVD-R19 | Active |
| UPK-R4 | The historical mentions listed in section 6 remain, without becoming aliases; baseline performance material describes the replaced store without its project name. | Product history is separate from a supported interface. | | Active |
| UPK-R5 | `main` requires `cargo-test`, `clippy`, `cargo-deny` and `fmt`, with strict up-to-date checks and the expected check producer preserved; `fmt` becomes required only after its job exists and passes. | The gate includes formatting without requiring a result no workflow can produce. | | Active |
| UPK-R6 | Formatting uses the exact toolchain in `rust-toolchain.toml`, explicitly installs that toolchain's `rustfmt`, and runs `cargo fmt --all --check`. | A minimal toolchain installation does not supply every optional component. | UPK-R5 | Active |
| UPK-R7 | Repository metadata and README status describe the built product and supported host, using section 9's values. | Public entry points must describe the same project. | ADR 0033 | Active |
| UPK-R8 | New work uses `design`, `bug`, `question`, `dependencies` and `rust` as applicable; `enhancement` keeps its assignments on 53 closed issues and is not used for new work; only the seven unused labels in section 9 are removed. | Removing a used label erases classification. | | Active |
| UPK-R9 | The bug form requires one of section 9's three impact choices, independently of project priority. | Only the reporter knows what the problem costs them. | | Active |
| UPK-R10 | Suite cost is measured at the implementation commit without a competing build, with six build jobs and six test threads, full per-file executed counts, the twenty slowest cases plus ties, and an assessment of every represented file. | Elapsed time and source counts do not establish test value. | UPK-R11 | Active |
| UPK-R11 | The repository build default is six jobs, nextest commands explicitly set six build jobs and six test threads, and clippy explicitly uses `-j 6`. | Compilation and test execution have separate concurrency limits. | | Active |
| UPK-Q1 | Every old-name match is classified, and no active Baley interface uses an old name. | A zero-match rule would erase intentional history. | UPK-R1, UPK-R4 | Active |
| UPK-Q2 | The format gate rejects a formatting defect and accepts its correction without modifying source, and the applied branch rules require its exact context. | A local formatter or an optional job alone does not enforce the gate. | UPK-R5, UPK-R6 | Active |
| UPK-Q3 | The suite report reconciles discovered, executed, passed, failed, ignored and filtered cases with its retained output and source mapping; no unit test asserts on timing. | Measurement must be reproducible without making tests depend on machine speed. | UPK-R10 | Active |

## 4. Roles and actors

| Actor | Receives | Returns | Model and effort from |
|---|---|---|---|
| Owner | Check results and repository settings | Applied branch rules and repository metadata | Not applicable |
| Contributor | Pinned commands and requirements | Code, matching documentation and verification evidence | Not applicable |
| GitHub Actions | Pull request or main push | Named check results | Not applicable |
| GitHub branch rules | Check results and pull request state | Whether the change may merge | Not applicable |
| Claude Code | Its Baley registration and stubs | Session startup and MCP calls | Not applicable |
| Test assessor | Full run output and the production units exercised | Counts, timings and keep, fold or delete assessments | Not applicable |

## 5. Commands and operations

### Check the repository

**Inputs:** the workspace and the toolchain pinned in [rust-toolchain.toml](../../rust-toolchain.toml).

```text
cargo nextest run --locked --workspace --no-fail-fast --build-jobs 6 --test-threads 6
cargo clippy --workspace --all-targets --locked -j 6 -- -D warnings
cargo fmt --all --check
cargo deny --locked check
```

**Outputs:** the four check results. A nonzero exit fails its check. Formatting reports differences without rewriting files. The existing dependency check that prevents `baley-core` from reaching SQLite remains part of clippy's job ([test.yml:91-104](../../.github/workflows/test.yml#L91)).

The build default is `[build] jobs = 6` in `.cargo/config.toml`. Nextest's default profile has `test-threads = 6`. Explicit command limits keep the intended compilation and execution limits visible even when configuration differs. Cargo documents the build setting in its [configuration reference](https://doc.rust-lang.org/cargo/reference/config.html#buildjobs), and nextest distinguishes build jobs from simultaneous tests in [Running tests](https://nexte.st/docs/running/).

The `fmt` job belongs in [test.yml](../../.github/workflows/test.yml). It uses the workflow's pinned checkout action and toolchain-channel extraction, installs that exact channel with the minimal profile, selects it, explicitly adds its `rustfmt` component, and runs the format command above. It needs no compilation or dependency cache. The job runs on the existing pull-request and main-push events, without a path filter, conditional skip or allowed failure. The [rustfmt documentation](https://github.com/rust-lang/rustfmt/blob/main/README.md) describes component installation and check-only operation.

### Inspect the applied gate

**Input:** `gh api repos/crenshawdev/baley/rules/branches/main`.

**Output:** the applied contexts, strict-check setting and expected integration for each check. A workflow job's presence does not establish that it is required. The required context is exactly `fmt`, from the same expected producer as the existing checks. Other branch rules remain intact. GitHub documents the expected source in [ruleset checks](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets) and pending results from skipped workflows in [required-check troubleshooting](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks).

### Inventory names

**Inputs:** tracked file contents, searched case-insensitively for the predecessor's name and separately for its instruction prefix. Searches cover active source, fixtures and documentation. An absent search root is reported as absent, never as a successful zero-match scan.

**Output:** occurrences, matching lines, files and a classification for every match. The inventory passes when only the permitted historical explanations and separately owned project references remain. A registration's name alone is not evidence that Baley owns it.

### Measure the suite

**Inputs:** a clean checkout at the implementation commit, no competing build, the pinned compiler and nextest version, and the nextest command above with `--status-level pass` to retain passing-case timings.

**Outputs:** uncondensed stdout and stderr, exit code, compilation time, suite wall time, full per-file executed counts, and the twenty longest individual test executions including all ties at the cutoff. Each case has its binary identity, full test name, source file and line, result and duration. The report records platform, target, tool versions, profile, target directory, warm or cold build state, and relevant environment overrides.

Discovery can be recorded with `cargo nextest list --locked --workspace --build-jobs 6 --message-format json`, as described in [nextest's machine-readable lists](https://nexte.st/docs/machine-readable/list/). A listing supplements the run; it supplies neither executed counts nor timing evidence. The source mapping accounts for macro expansion and a source file compiled into multiple test binaries. Ignored and filtered cases are separate from executed cases, and retries are identified rather than silently counted as distinct tests.

**Refusals:** a competing build prevents a valid cost measurement. A failed run remains a failed run in the report. Missing cases or timing lines prevent a complete count or ranking. Static test-attribute counts do not supply the executed counts or timing evidence required by UPK-R10 and UPK-Q3.

## 6. Records

### Historical names

The retained historical passages are:

| Passage | Current location | Treatment |
|---|---|---|
| Product boundary | [PRD:142](prd/baley.md#L142) | Keep the distinction between the predecessor's behavior and Baley's requirements |
| No record import | [PRD:162](prd/baley.md#L162) | Keep the explicit exclusion |
| Heritage | [PRD:166](prd/baley.md#L166) | Keep the explanation of the working loop's origin |
| Story design context | [ADR 0017:14](../adr/0017-stories-and-sprints.md#L14) | Keep the accepted decision's body |
| Review design context | [ADR 0019:14](../adr/0019-reviews-adjudicated-and-ruled.md#L14) | Keep the accepted decision's body |
| README lineage | [README:45](../../README.md#L45) | Keep the name, repository link and attribution before the `baley-start` tag |

Outside this document, a case-insensitive tracked-content search for `cadence` finds eight occurrences on six lines in four files: the five passages in `docs` above and three occurrences on the README lineage line. `crates` and `.github` contain zero matches; `scripts` is absent. The old `cad-` instruction prefix has zero matches outside this document. This document's explanatory mentions and registration boundary are also permitted, without establishing aliases. No active identifier is exempted as history.

The unnamed baseline is retained in [0001:35](0001-evidence-ledger.md#L35), its [non-goals](0001-evidence-ledger.md#L57), [performance account](0001-evidence-ledger.md#L1318) and [compatibility rule](0001-evidence-ledger.md#L1353). The profile is [baseline-store.json](../../spikes/evidence-ledger-bench/profile/baseline-store.json), identified by the benchmark [README:30](../../spikes/evidence-ledger-bench/README.md#L30) and [loader:558](../../spikes/evidence-ledger-bench/src/main.rs#L558). Those figures describe the replaced store, not the current test suite.

### Canonical names and consumers

The names below cover the public and stored surfaces of [#53](https://github.com/crenshawdev/baley/issues/53). Retained engine definitions are distinguished from the session server that runs in production.

| Surface | Names and current source |
|---|---|
| Package and executable | `baley`, in [Cargo.toml:2](../../crates/baley/Cargo.toml#L2) and [main.rs:17](../../crates/baley/src/main.rs#L17); Rust paths use `baley::` |
| MCP server and tools | `baley`, `baley_version`, `baley_query`, `baley_apply`, in [mcp/tools.rs:58-63](../../crates/baley/src/mcp/tools.rs#L58) and [83-120](../../crates/baley/src/mcp/tools.rs#L83) |
| Session startup and hook command | `baley serve`, in [main.rs:186-199](../../crates/baley/src/main.rs#L186); `baley guard`, in [hooks.json:9](../../hooks/hooks.json#L9) |
| Inherited environment controls | `BALEY_GLOBAL_CONFIG`: two reads, in [milestone/release.rs:410](../../crates/baley/src/milestone/release.rs#L410) and [milestone/prune.rs:223](../../crates/baley/src/milestone/prune.rs#L223). Debug-only controls: `BALEY_PRUNE_STOP` in [prune.rs:55](../../crates/baley/src/milestone/prune.rs#L55) and `BALEY_LANDING_EXIT_AFTER_EFFECT` in [landing_service.rs:360](../../crates/baley/src/landing_service.rs#L360) |
| Inherited configuration paths | `baley/providers.env`, in [review/provider/credentials.rs:66](../../crates/baley/src/review/provider/credentials.rs#L66) |
| Record markers and hash domain | `baley.rail.receipt.v1`, [rail/receipts.rs:400](../../crates/baley/src/rail/receipts.rs#L400); `baley.rail.observation.v1`, [rail/risk.rs:97](../../crates/baley/src/rail/risk.rs#L97); `baley.native_evidence.v1`, [evidence/persistence.rs:12](../../crates/baley/src/evidence/persistence.rs#L12); `baley.pause.risk-surface.v1`, [pause/risk.rs:14](../../crates/baley/src/pause/risk.rs#L14); `baley.lifecycle`, [derivation/memo.rs:4](../../crates/baley/src/derivation/memo.rs#L4) |
| Serialized execution names | `baley-query` and `baley-apply`, in [store/writer.rs:2853-2854](../../crates/baley/src/store/writer.rs#L2853); `BALEY-PLAN-BODY`, in [execution/render.rs:583](../../crates/baley/src/execution/render.rs#L583). The hyphenated spellings are stored values, separate from MCP tool names |
| Temporary paths | `.baley-release-`, [milestone/release.rs:541](../../crates/baley/src/milestone/release.rs#L541); `.baley-prune-index-` and `.baley-undo-index-`, [rail/commit.rs:318](../../crates/baley/src/rail/commit.rs#L318) and [380](../../crates/baley/src/rail/commit.rs#L380); `baley-risk-index-`, [pause/git.rs:260](../../crates/baley/src/pause/git.rs#L260); `baley-task-`, [task/mod.rs:32](../../crates/baley/src/task/mod.rs#L32) |
| Instructions and agent identities | The compiled instruction registry uses `bal-` at [instruction/mod.rs:65-210](../../crates/baley/src/instruction/mod.rs#L65). The inherited role table contains six roles and thirty role/rung entries at [config/roles.rs:7-60](../../crates/baley/src/config/roles.rs#L7); that table does not prove stubs are installed |

The file-valued `BALEY_GLOBAL_CONFIG` is not an alias for the directory-valued `BALEY_HOME`, read at [folders.rs:47](../../crates/baley/src/folders.rs#L47). Supported settings and key locations are the vendor folders of [ADR 0027](../adr/0027-vendor-folders-and-plain-keys.md) and [0003](0003-configuration-and-routing.md). An inherited spelling does not make its format part of that interface. The parked services remain compiled for their tests and are removed by Build 9 ([inherited.rs:1-4](../../crates/baley/src/inherited.rs#L1)); their bytes need no second rename.

### Suite-cost report

The measurement belongs in the pull request for [#54](https://github.com/crenshawdev/baley/issues/54), not in a runtime ledger record.

| Field | Meaning |
|---|---|
| Commit and tree | Exact source measured |
| Command, tool versions, platform, target and profile | Reproduction inputs |
| Build jobs, test threads, target directory and build state | Conditions affecting elapsed time |
| Compilation time, suite wall time and exit code | Separate build and execution observations |
| Binary, full test name, source file and line | Identity and location of each case |
| Result, duration and retries | What the runner observed |
| Per-file executed count and sum of case durations | Composition and execution work; the sum is not suite wall time |
| Owned behavior and defect detected | What the production unit proves and which wrong behavior the case catches |
| Seam and assessment | Plain values or one allowed external seam; keep, fold into a unit test, or delete, with reasons |

## 7. States

Not applicable to application state: repository upkeep introduces no runtime state machine or ledger record. GitHub produces and evaluates check results. Installation state belongs to [0012](0012-host-interface.md).

## 8. Workflows

### Format-check activation

The workflow job and the branch requirement are separate parts of the gate. The activation order is:

1. The pull request adding `fmt` runs under the existing required checks and produces the new job's result. A formatting defect on a disposable verification branch fails that job, and its correction passes.
2. Once the job exists on `main` and has passed, its emitted context and producer are verified. The owner adds `fmt` to the applied ruleset, preserving strict checks and every existing rule.
3. Reading the applied branch rules confirms all four required contexts and their expected producer. A documentation-only change still produces `fmt`. The README names four required checks only in this state.

The isolated formatting revision is already recorded at [.git-blame-ignore-revs:2](../../.git-blame-ignore-revs#L2). The gate requires no new bulk formatting commit.

### Host registration

Claude Code is the only supported host ([ADR 0033](../adr/0033-host-security-bar.md)). It starts `baley serve` once per session, as specified by [0012, HST-R16](0012-host-interface.md#3-requirements). The owner does not start the server. This command is the session server.

The canonical server and tool names remain those in UPK-R1. An installation preserves a separately owned registration rather than inferring ownership from its name. How Baley is delivered, who writes the host files and how an existing Baley registration is replaced wait on the owner's delivery decision under [0012, HST-R17](0012-host-interface.md#3-requirements), carried by [#24](https://github.com/crenshawdev/baley/issues/24) and the [held Build 3 tasks T14 to T17](../roadmap.md#build-3-hosts). This area specifies neither a replacement algorithm nor a second set of tool aliases.

### Test assessment

Every file represented in the twenty slowest cases plus ties receives a keep, fold or delete assessment. A case is kept when it checks a distinct production decision against an independently derived expected result. A case that drives a workflow is folded into tests of its constituent decisions. A duplicate or invalid case is deleted only with an account of the valid coverage retained or supplied. Neither count nor duration alone justifies deletion.

Tests execute production logic in-process, with plain inputs or a minimal Rust substitute at one filesystem, process or clock seam. Gathering external observations is separate from judging them. Tests launch no programs and use no live clock. A fresh temporary directory may be the filesystem seam. Store tests use real SQLite in such a directory, never a fake store. Operation-order tests do not prove crash survival or physical durability. A new test is demonstrated to fail with the behavior it guards broken, then that behavior is restored. No unit test asserts on timing.

UPK-R10 and UPK-Q3 require the measurement specified in section 5, full per-file accounting and an assessment of every file represented in the twenty slowest cases plus ties. Build 9's application performance budgets remain separate from this suite-cost evidence.

## 9. Settings

### Public metadata

| Setting | Value or rule | Set where |
|---|---|---|
| Description | Keeps AI coding agents accountable to the person who answers for their work. | GitHub repository metadata |
| Topics | `rust`, `mcp`, `ai-agents`, `claude-code`, `sqlite`, `event-sourcing` | GitHub repository metadata |
| Wiki | Disabled | GitHub repository settings |
| Product status | Designed and being built, not ready to use; no release is implied by accepting a design | [README:5](../../README.md#L5) |
| Host scope | Claude Code | [README:23](../../README.md#L23), ADR 0033 |
| Contributions | Issues welcome; pull requests by invitation | [README:37](../../README.md#L37) |
| CI description | Names the checks actually required on `main` | [README:33](../../README.md#L33) |

### Labels

The all-state GitHub issue collection, including pull requests, gives these assignments on 2026-10-06. Open and closed counts below exclude pull requests, which have their own column. Six labels remain.

| Label | Open issues | Closed issues | Pull requests | Use |
|---|---:|---:|---:|---|
| `design` | 1 | 12 | 10 | Active |
| `bug` | 7 | 2 | 0 | Active |
| `question` | 0 | 0 | 0 | Active; the [question form:3](../../.github/ISSUE_TEMPLATE/question.yml#L3) applies it |
| `dependencies` | 0 | 0 | 6 | Active |
| `rust` | 1 | 0 | 6 | Active |
| `enhancement` | 0 | 53 | 0 | Keep these assignments; not used for new work |
| `accessibility`, `documentation`, `duplicate`, `good first issue`, `help wanted`, `invalid`, `wontfix` | 0 each | 0 each | 0 each | Removed; none was assigned |

The 53 `enhancement` assignments are closed issues 73 through 126, excluding 93. Removal covers seven labels and relabels no issue. UPK-R8 requires a fresh inventory of assignments before deletion, including closed issues and pull requests.

### Bug impact

The bug form has a required dropdown with id `impact`, label `Impact`, no preselected answer, and exactly these choices:

| Choice | Meaning |
|---|---|
| Stops me working | The reporter cannot continue |
| Wrong result, I can work around it | The reporter can continue with a workaround |
| Minor or cosmetic | The reporter identifies a smaller impact |

The existing required `what`, `repro` and `version` fields remain. Impact does not assign project priority. The form is verified by a valid GitHub preview in which omission prevents submission and each choice is carried into the report.

## 10. Instructions served

Not applicable as a new instruction surface: repository upkeep serves no model instruction. Existing served instructions carry the canonical names. `bal-help`, `bal-read-contract` and `bal-capture` are registered in [instruction/mod.rs:164-210](../../crates/baley/src/instruction/mod.rs#L164); the capture text calls `baley_apply` at [instruction/capture.rs:17](../../crates/baley/src/instruction/capture.rs#L17). Stub rendering and installation belong to [0012, HST-R12 and HST-R17](0012-host-interface.md#3-requirements).

## 11. Build status

### Requirements

| Requirement | Status | Where |
|---|---|---|
| UPK-R1 | Built | Package, executable and session server: [Cargo.toml:2](../../crates/baley/Cargo.toml#L2), [main.rs:17](../../crates/baley/src/main.rs#L17), [mcp/tools.rs:58-120](../../crates/baley/src/mcp/tools.rs#L58). Instruction registry: [instruction/mod.rs:65-210](../../crates/baley/src/instruction/mod.rs#L65). Installed stubs are not established by these definitions |
| UPK-R2 | Not built | The CLI has no installer ([main.rs:23-94](../../crates/baley/src/main.rs#L23)); registration replacement under [0012, HST-R17](0012-host-interface.md#3-requirements) waits on the owner's delivery decision, carried by [#24](https://github.com/crenshawdev/baley/issues/24) and the [held Build 3 tasks T14 to T17](../roadmap.md#build-3-hosts) |
| UPK-R3 | Built | The definitions in section 6 remain; services are parked at [inherited.rs:1-4](../../crates/baley/src/inherited.rs#L1) |
| UPK-R4 | Built | The six retained passages and unnamed benchmark material are cited in section 6 |
| UPK-R5 | Built | [test.yml:121-140](../../.github/workflows/test.yml#L121) runs `fmt` beside the three other jobs, and the applied rules require all four from one producer with strict checks, as the note below records |
| UPK-R6 | Built | [test.yml:127-140](../../.github/workflows/test.yml#L127) reads the exact channel from [rust-toolchain.toml:11](../../rust-toolchain.toml#L11), installs and selects it, explicitly adds its `rustfmt` component and runs `cargo fmt --all --check` |
| UPK-R7 | Built | README describes the [status:5](../../README.md#L5), [host:23](../../README.md#L23), [four required checks:33](../../README.md#L33) and [contribution policy:37](../../README.md#L37). GitHub's description, six topics and disabled wiki match section 9, as read on 2026-10-06 |
| UPK-R8 | Built | Section 9's six labels remain; the seven no issue or pull request used are removed, and `enhancement` keeps its 53 closed issues without being used for new work |
| UPK-R9 | Built | [bug.yml:12-21](../../.github/ISSUE_TEMPLATE/bug.yml#L12) requires one of section 9's three impact choices with no preselected answer; the required `what`, `repro` and `version` fields remain at [5-11](../../.github/ISSUE_TEMPLATE/bug.yml#L5) and [22-35](../../.github/ISSUE_TEMPLATE/bug.yml#L22) |
| UPK-R10, UPK-Q3 | Not built | Static counts below supply no executed counts or durations; [#54](https://github.com/crenshawdev/baley/issues/54) owns the measurement and assessment |
| UPK-R11 | Built | [.cargo/config.toml:3](../../.cargo/config.toml#L3) sets six build jobs; [.config/nextest.toml:3](../../.config/nextest.toml#L3) keeps six test threads; nextest at [test.yml:60](../../.github/workflows/test.yml#L60) explicitly sets `--build-jobs 6 --test-threads 6`, and clippy at [test.yml:89](../../.github/workflows/test.yml#L89) sets `-j 6` |
| UPK-Q1 | Built | Section 6 classifies every retained match; no active source uses an old name |
| UPK-Q2 | Built | The check-only command at [test.yml:140](../../.github/workflows/test.yml#L140) failed on a disposable misformatted commit and passed on its correction ([#210](https://github.com/crenshawdev/baley/pull/210), closed unmerged), a documentation-only pull request produces it, and the applied rules require its exact context |

The applied `main` rules, read on 2026-10-06, come from ruleset `23995653`. They require `cargo-test`, `clippy`, `cargo-deny` and `fmt`, each from integration `15368`, with `strict_required_status_checks_policy: true` and `do_not_enforce_on_create: false`. The endpoint also reports deletion and non-fast-forward protection, required signatures, and a pull-request rule with resolved review threads, dismissed stale approvals, zero required approving reviews, and merge or squash. Its bypass list is empty.

The workflow pins nextest 0.9.144 at [test.yml:55](../../.github/workflows/test.yml#L55) and cargo-deny 0.20.2 at [test.yml:115](../../.github/workflows/test.yml#L115).

### Static test counts

These counts include only Rust source lines containing actual `#[test]` or `#[tokio::test]` attributes, with or without arguments, under `crates`. Each matching line counts once, including attributes in macro definitions; mentions in strings or comments are excluded. They are not discovered or executed test counts.

| Crate | Test-attribute lines | Files containing those lines |
|---|---:|---:|
| `baley` | 2,263 | 177 |
| `baley-core` | 430 | 14 |
| `baley-store` | 105 | 11 |
| `baley-store-sqlite` | 305 | 12 |
| `baley-bench` | 5 | 2 |
| Total | 3,108 | 216 |

The count does not expand macros or resolve binary membership. For example, [ledger/tests.rs:16-22](../../crates/baley/src/ledger/tests.rs#L16) defines an attribute inside a macro, and the store's [conformance macro:249-352](../../crates/baley-store/src/conformance/mod.rs#L249) generates adapter cases. The test count and timing figures in older issues are not measurements of this tree. No compilation, formatter, test, lint or dependency-check execution is claimed by this document's static evidence.

## 12. Open questions

| Question | Decided by |
|---|---|
| How does installation replace an existing Baley registration while preserving separately owned entries? | The owner's delivery decision under [0012, HST-R17](0012-host-interface.md#3-requirements), carried by [#24](https://github.com/crenshawdev/baley/issues/24) and the [roadmap's held Build 3 tasks T14 to T17](../roadmap.md#build-3-hosts) |
