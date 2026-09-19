//! The milestone front door is compiled alongside its typed operations.
pub fn markdown() -> &'static str {
    r#"---
name: cad-milestone
description: "Read the per-phase audits and request an explicitly selected local milestone close."
argument-hint: "<phase ids> <display label>"
allowed-tools:
  - mcp__cadence__cadence_query
  - mcp__cadence__cadence_apply
---

<process>
1. Select the owner's explicit nonempty set of positive integer phase ids,
   sorted without duplicates, a display label and a stable occurrence name.
   Ask for missing selection inputs. A milestone label is display text, never
   cad-audit's phase argument; never parse a label into phases. Call cadence_query
   `{"operation":"milestone-read","occurrence":"<occurrence>","selection":{"phases":[15,16],"label":"<display label>"}}`.
2. Show the returned close identity, generation (record version), immutable
   selection and label. Present every existing audit outcome with its integer
   phase identity, and every unsettled record by kind, phase and exact identity.
   milestone-read invokes verification-audit for each selected phase. Preserve
   those outcomes; invent no audit verdict or bypass. A refusal stops the action.
3. Obtain the owner's explicit choice to close only or leave the selection open.
   If close only is chosen, send only the returned `actions.close` typed payload
   unchanged to cadence_apply. Display the returned close record or exact refusal.
   Reuse the same request for a retry; changed inputs require a fresh read and
   owner choice. A ready close records readiness only in this version; pruning
   is not yet available. A deferred member stays unruled: no ruling operation
   exists here. A later clear scan cannot erase an earlier risk obligation.
4. Close-only stops at the local milestone close. Landing is a separate explicit
   action with its own owner choice and returned operations; a close never grants
   external authorization. Do not perform landing as part of this door. If the
   owner requests retuning, cadence_query `{"operation":"suggest"}` is advisory:
   its output grants no permission to apply a proposal or to land.
</process>
"#
}
