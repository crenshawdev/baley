---
name: cad-plan-review
description: "Alias of /cad-review plan: review a phase's native plan slices with its locked context, or one plan document."
argument-hint: "<phase|plan-path>"
allowed-tools:
  - mcp__cadence__cadence_query
  - mcp__cadence__cadence_apply
  - Task
---

<objective>
Deliver exactly the review the owner selected. The binary resolves the target,
retains its exact bytes, compiles the kind-specific intent into the dispatch
and records every delivery event; this front door only relays. Nothing here
applies a finding: no file is changed, deleted, staged or committed.
</objective>

<process>
1. Select. Split `$ARGUMENTS` on whitespace and call cadence_query
   `{"operation":"review-select","command":"cad-plan-review","arguments":[<tokens>]}`.
   This alias selects the `plan` kind; the argument is a phase number or a plan document path.
   A refused answer names what is missing, ambiguous or unresolvable in its
   reason: report that request and stop. Never widen a target to its parent
   directory, the whole phase or the tree, and never substitute a paragraph of
   your own for the resolved document. Retain `result.kind`, `result.target`,
   `result.material`, `result.intent` and `result.admission`.
2. Admit. Call cadence_apply `{"operation":"review-admit","request":<result.admission, unchanged>}`.
   Only an answer with an admitted fire proceeds; a replayed answer names the
   review already admitted for this selection.
3. Deliver. Call cadence_query `{"operation":"review-next","fire":<fire>}` and
   follow only the saved dispatch: invoke Task with `dispatch.agent` and exactly
   `dispatch.prompt`, passing `dispatch.model` only when present. Forward the
   actual launch and return events with review-observation and the unchanged
   raw return with review-return under the issued identity, wait for the
   durable acknowledgment, then poll review-next again until delivery is
   usable-complete or complete-with-failure. Provider work stays with the
   resident binary; missing or malformed output is failure, never an empty
   clean result.
4. Present. Show the reviewer's findings unedited - file, line, severity, claim
   and failure scenario - beside the retained target and the selected kind. A
   clean pass names the target that was read; it is never a bare "no findings".
   The owner decides what to change and does it; this command edits nothing.
</process>

<intent>
The binary compiles the intent for the selected kind into the local dispatch
and the provider payload alike:

- decision: Refute the selected decision: argue against it from the retained decision text and its retained inline context, name the claim each objection rests on, and apply no amendment.
- minimalism: Rank code that should not exist, as a deletion list ordered by severity: reinvented standard library or dependency, an abstraction with one implementation, unused flexibility and configuration nobody sets. Propose deletions; apply nothing.
- plan: Work backward from the phase goal and its locked decisions: for each task ask which truth it serves, whether the retained plan can deliver it as written, and whether any step contradicts a locked or durable decision. Return findings; edit nothing.

A decision review takes the exact decision line and the whole document as its
inline context. A minimalism review retains the named file, the frozen
directory membership or the native phase range and dispatches the one base
reviewer with no provider and no gate. A plan review by phase retains every
native plan slice with the approved locked context and uses the ordinary
manual-plan trigger with its configured gate; by path it retains that one
document.
</intent>
