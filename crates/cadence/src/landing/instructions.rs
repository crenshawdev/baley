//! The landing front door delegates permission and effects to the binary.
pub fn markdown() -> &'static str {
    r#"---
name: cad-land
description: "Read a landing and request one explicitly authorized external step."
argument-hint: "<landing id>"
allowed-tools:
  - mcp__cadence__cadence_query
  - mcp__cadence__cadence_apply
---

<process>
1. Require the identified landing id, then call cadence_query
   `{"operation":"land-read","landing":"<landing id>"}`. Show its exact id,
   generation, frozen source branch and commit, remote name and URL, and base
   branch and commit. Present the actual branch, ahead, dirty and remote state,
   the read-only tracker report, every deferred member and all retained step
   intents, authorizations and receipts. Report unavailable observations as such.
   Show `done`, including each receipt's observed remote ref/object or exact PR
   identity/state and reconciliation provenance, and the single `next_step`.
   If `resume` is present, send that typed land-resume payload unchanged to
   cadence_apply. It reads the actual remote before any unfinished step, records
   an existing effect once, and runs an absent effect only under the retained
   exact authorization. It grants no new permission. Show its receipt or exact
   discrepancy, then land-read again. A failed or ambiguous read stops without
   a success receipt or a repeated mutation. Reuse the same request on transport
   interruption; after resolving a retained discrepancy, read a fresh `resume`.
2. Show the proposed exact step: push, open, merge or tag-push. Obtain its request
   schema through cadence_query `{"operation":"schema","tool":"apply","for":"land-authorize"}`.
   Show every input before asking: source and destination refs; for open, configured
   forge provider, repository, host and the complete proposed title/body; for merge,
   that same forge and the recorded PR identity; for tag-push, the exact tag and
   object id. Missing inputs require an owner answer. Configuration grants no
   permission. A different landing, version, step or changed head needs a fresh choice.
3. Only after the owner's explicit choice, record land-authorize through
   cadence_apply with a fresh request_id, the landing id and expected_generation,
   exact source/base/remote copied from land-read, that one step's inputs, and
   the actual owner's name and authorization time. Ask for missing attribution;
   never invent it. Declining stops without writing an authorization or running a step.
4. Send the returned `action` typed payload unchanged to cadence_apply. It invokes
   land-publish, land-open, land-merge or land-tag-push. Print the exact refusal or
   durable receipt, including landing, step and authorization identity, and read
   land-read again. A refusal stops. An uncertain intent requires reconciliation;
   return to step 1 for land-resume. Never invent success, replace a request_id
   to retry an effect, or retry blind. Repeated resume keeps the same step receipt.
   A reconciled MERGED state is not the owner's merge confirmation: show
   `confirm-merge` as the next step and obtain that record before any cleanup.
5. The binary owns every external subprocess. Do no raw push, PR creation or merge,
   shell branching, tracker mutation, FILED write or local branch reap. This door
   grants no checkout, pull, local tag or cleanup permission. Each later external
   step requires its own explicit owner choice and authorization record.
</process>
"#
}
