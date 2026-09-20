//! Compiled debug role; the installed skill is only its rendering.
pub fn markdown() -> &'static str {
    r#"---
name: cad-debug
description: "Investigate symptoms with ranked hypotheses and durable evidence; resume from the binary debug record."
argument-hint: "[list | status <slug> | continue <slug> | --diagnose] [symptom]"
allowed-tools:
  - Task
  - mcp__cadence__cadence_apply
  - mcp__cadence__cadence_query
  - Write
  - Edit
  - Bash
  - AskUserQuestion
---

<role>
Use the scientific method to investigate the symptom. The binary owns the
debug record and renders its protected Markdown projection. Every investigation
write uses a debug operation. Source edits are explicit owner-approved fix
steps. A resolved record records the caller's reproduction outcome; it does
not complete a phase or verify settlement.
</role>

<route>
Parse $ARGUMENTS:
- list: call cadence_query {"operation":"debug-list"}, show the open records
  with slug and symptom, then stop.
- status <slug>: call debug-status with slug, show the returned record, then stop.
- continue <slug>: call debug-continue with slug and resume from its record
  alone. Keep the recorded symptom, hypothesis identities, states, rank reasons,
  observations and attempt count; do not ask the owner to repeat them.
- --diagnose: remove the flag and investigate the remaining symptom without
  applying a fix.
- Otherwise the remaining text is a new symptom.

Use cadence_query schema with tool apply or query and for set to the operation
name for its exact request shape. All mutations carry request.request_id, slug
and expected_version. Open uses expected_version 0; later steps copy the last
returned record.version. Slugs contain 1..80 lowercase letters, digits or
hyphens, start with a letter and do not end with a hyphen. Supply no document
path. For new mutations choose distinct request IDs; retry an uncertain delivery
with the identical request_id and inputs. Changed-input replay is refused.

The Markdown at .planning/debug/<slug>.md is an output, never continuation
input. Never write or edit it, search it for status, or reconstruct a record
from it. Read project code through Cadence search/read using issued references.
</route>

<method>
1. For a new session, capture the exact failing signal, reproduction and what
   success looks like. Ask only for missing information. Call debug-open with
   that symptom; it initializes status open and attempt_count 0.
2. Before the first hypothesis, consider the diagnostic patterns below.
   Record 2-5 plausible hypotheses through debug-hypothesis, each with a stable
   id, description, state and rank_reason. States are untested, testing,
   refuted and confirmed. Rank by likelihood and the cost of a discriminating
   check; prefer a cheap check that eliminates a whole class.
3. For the next hypothesis, predict what would be observed if it were true.
   Mark it testing with debug-hypothesis, retaining its identity and rank reason.
   Run the cheapest discriminating test, changing one variable at a time.
4. Call debug-observation immediately with the complete test and result and
   rules_in / rules_out hypothesis IDs (empty arrays when it decides neither).
   The binary records the observation and confirms/refutes those hypotheses in
   one transition. Distinguish a measured result from an inference.
5. A confirmed cause leads to the root cause report. Otherwise choose the next
   test from the evidence. When every hypothesis is refuted, form new hypotheses
   only from what the observations support. If no discriminating test remains,
   report the dead end and let the owner choose the next move.

Diagnostic patterns are a filter, not an assumed diagnosis:
- Stale or shadowed code: print the running path's filename/version and compare
  it to the code being inspected.
- Unexpected input: inspect raw boundary bytes, including whitespace, quotes
  and encoding.
- Swallowed errors: inspect catches, defaults and ignored return codes; expose
  the original failure once.
- Downstream overrides: inspect the effective value and its source at the
  consumer.
- Timing or initialization order: record sequence numbers around the suspect
  pair or serialize them once.
- Falsy valid values: try exactly zero, empty string and false.
- Boundary errors: try empty, one element and exactly the limit.
- Shared mutable state: isolate the case and vary the order of its neighbors.
- Environment differences: inspect cwd, interpreter and relevant environment
  values from inside the failing process without exposing secrets.
- Downstream symptoms: trace back to the first value already wrong.
- Recent changes: inspect the relevant history before expanding the theory.
A pattern without a matching signature and a cheap check does not earn the
top rank.
</method>

<resolve>
State the root cause with the confirming observation and the minimal proposed
fix. For diagnose-only, record the Root Cause Report using debug-observation,
leave the session open and stop without source edits or staging.

For a fix, obtain the owner's explicit approval for the proposed source edits
and staging. Apply that fix, then run git add with exactly its touched paths.
Re-run the original reproduction and retain the actual test and result.
Call debug-resolve with resolution and reproduction {test, result, passed}.
A successful reproduction marks resolved. A failed reproduction records a
failed attempt and keeps the session open. debug-attempt separately records
a failed attempt {description, result}; do not count the same failure twice.
Every fix attempt is a single deliberate step. Never loop fixes automatically.

At a reviewer boundary, follow cad-review-delivery with caller debug, preserve
the actual staged target, dispatch its host reviewer through Task, wait for the
durable return and satisfy its deferred obligations. Raw findings grant no
permission. Preserve any refusal and its exact next action.

@${CLAUDE_PLUGIN_ROOT}/skills/cad-review-delivery/SKILL.md
</resolve>

<recovery>
After /clear or a process restart, call debug-continue with the slug. Trust the
record even if its Markdown is absent or misleading. On a version refusal,
read debug-status and reconcile the new evidence before making a new request.
On an uncertain write, retry its identical request ID and inputs; never invent
a successful result. On a pending journal or unavailable record, report the
located refusal and preserve the stopped state for recovery.
</recovery>
"#
}
