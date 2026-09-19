---
name: cad-milestone
description: "Read the per-phase audits, select a local milestone close and request its committed prune."
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
3. Obtain the owner's explicit choice to close only, close and prune, or leave
   the selection open. To close, send only the returned `actions.close` typed payload
   unchanged to cadence_apply. Display the returned close record or exact refusal.
   Reuse the same request for a retry; changed inputs require a fresh read and
   owner choice. A ready close records readiness only; it changes no documents.
   Read milestone-read again with the same occurrence and selection. After the
   owner's choice to prune, send its returned `actions.prune` typed payload
   unchanged to cadence_apply. This is milestone-prune: it names the ready close
   id, expected generation, exact phase selection and request_id. The binary
   owns every removal, document replacement and single-parent commit.
   A deferred member stays unruled: no ruling operation
   exists here. A later clear scan cannot erase an earlier risk obligation.
4. Close-only stops at the local milestone close. Landing is a separate explicit
   action with its own owner choice and returned operations; a close never grants
   external authorization. Do not perform landing as part of this door. If the
   owner requests retuning, cadence_query `{"operation":"suggest"}` is advisory:
   its output grants no permission to apply a proposal or to land.
5. Show the prune's exact selected phase set, durable id, state, commit, single
   parent and next action. After interruption, retry the identical milestone-prune
   request; the binary recovers its frozen journal before answering. Never replace
   the request_id, recompute the selection or perform a removal or commit yourself.
   A committed result replays its receipt. A located interference refusal stops
   the action. Prune writes no ARCHIVE.md and grants no landing authorization.
</process>
