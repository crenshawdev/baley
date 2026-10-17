# Security policy

## Reporting a vulnerability

Report security problems privately through GitHub: on this repository, open the **Security** tab and choose **Report a vulnerability**. Do not open a public issue.

Include what you found, how to reproduce it, and what an attacker could do with it. You will get an answer, and credit in the fix if you want it.

## Scope

Baley is under active redesign and has no released version. Reports about the current `main` branch are in scope. Of most interest:

- anything that lets an agent bypass the owner's approval, the recorded evidence, or the branch policy the hook enforces;
- secrets reaching logs, records or the material sent to review providers;
- tampering with records that goes undetected.

## What runs on every pull request

Two workflows run on every pull request.

- `test.yml` holds the required checks: `cargo-test`, `clippy`, `cargo-deny` and `fmt`. A pull request cannot merge into `main` until all four pass.
- `fast.yml` runs the fast checks from [sentinel](https://github.com/jcrenshaw-dev/sentinel), in two jobs. Neither is a required check, so neither blocks a merge, but a failed job shows on the pull request.
  - `mechanical` runs sentinel's `check.sh`. `cargo audit`, `cargo deny`, `cargo clippy` and `semgrep` are gates, and the job fails when one of them fails. `cargo geiger`, a build of the fuzz targets in `crates/baley-core/fuzz` and `crates/baley/fuzz`, and Miri on the unit tests (the code has `unsafe` blocks) are reported only.
  - `security-review` has Claude review the pull request's diff against [THREAT-MODEL.md](THREAT-MODEL.md) with read-only tools. This repository is public, so the review is encrypted to the maintainer's key, and the pull request comment shows only the number of findings by severity. The job fails only when the review did not run or its output is malformed. Findings do not fail it. Pull requests opened by Dependabot skip it, because GitHub gives them no secrets.

## The full audit

The full audit runs on the maintainer's machine through sentinel's `/snt:audit`, never in CI, so findings stay private until they are fixed. Every step reads THREAT-MODEL.md as the definition of what matters.

1. **Mechanical checks:** the same `check.sh`.
2. **Context map:** the Trail of Bits `audit-context-building` skill maps the code. Every later reviewer reads the map first.
3. **Claude Security scan** of the whole repository.
4. **Lenses:** eight review prompts, one concern each, run over a read-only copy of the repository.

   | Lens | Concern |
   |------|---------|
   | crypto-and-state | Anything hashed, signed or encrypted, identifiers, randomness, and secrets stored on disk |
   | data-exposure | Secrets, personal data or paths leaking into logs, output, files, the network, error messages or MCP responses |
   | hook-execution | Anything the tool runs or lets Claude Code run: how commands are built, how arguments are escaped, what it assumes about `PATH` |
   | path-handling | Traversal, symlinks, races between check and use, canonicalization, writes outside the data folder, temporary files |
   | permissions | File modes, umask, world-readable state, lock files and sockets |
   | resource-limits | Unbounded reads, memory growth, regular expression blowup, runaway processes, filling the disk, missing timeouts |
   | supply-chain | Dependencies, build scripts, vendored code, submodules, downloads and update mechanisms |
   | untrusted-input | How settings, command-line arguments, environment variables and MCP input are parsed, bounded and decoded, and whether they can inject into a command, SQL or a prompt |

   By default each lens runs three times on Claude and once on OpenAI's Codex, 32 runs in all. The Claude Security findings join as one more run of each lens. A merge groups each lens's reports that cite the same lines and counts how many runs reported each one.
5. **Adjudication:** a fresh reviewer judges the findings in batches of ten. A finding is confirmed only by a failing test that enters through a way in that THREAT-MODEL.md names. The rest are marked unproven, or dropped with a reason.
6. **Refutation:** another fresh reviewer sees only the confirmed and dropped rulings and tries to prove each one wrong. It reruns every confirmed test and checks every drop's reason against the code. Every ruling that does not hold goes to the maintainer.
7. **Variant analysis:** for each confirmed finding, the Trail of Bits `variant-analysis` skill looks for the same pattern elsewhere, and new hits go through steps 5 and 6.
8. **Findings log:** one row per step.

## Every confirmed finding becomes a regression test

The failing test that confirms a finding is kept. Until the fix lands, it sits in a git-ignored `tests/security-unfixed/` folder and is skipped by default, so an open vulnerability is never published. The fix moves it to `tests/security/`, where it runs with the rest of the suite and keeps the bug from coming back.
