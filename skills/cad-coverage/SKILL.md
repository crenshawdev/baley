---
name: cad-coverage
description: "Read-only alias of /cad-audit: the phase-scoped requirement-to-evidence trace over the retained map and current verdicts; the test-generation arm is removed."
argument-hint: "<phase>"
allowed-tools:
  - mcp__cadence__cadence_query
---

Parse the phase as a positive JSON integer. Call cadence_query
`{"operation":"verification-audit","phase":13,"command":"cad-coverage"}` with the
selected integer. The answer is `verification-audit-1`: `sources` names each
input as it was read (REQUIREMENTS.md active declarations and trace rows,
ROADMAP.md declarations, the approved context, the native publications with
the requirements they claim, the coherent map with its superseded revisions,
and the current verification with its waivers and history); `traces` carries
one row per requirement seen anywhere, its origins, each edge as present or
missing, the phase's truth rows with item origins and current verdicts, every
break with its next action, and an outcome of met, waived, concerns, unmet,
pending or broken; `out_of_scope` lists rows assigned to other declared
phases; `report` is the rendered text.

Present `report`, then every break with its next action, then the out-of-scope
rows and limits. Structural coverage never certifies rejected or unseen
evidence, a historical judgment never counts as current, and a waived truth
is shown beside the met ones, never among them. A refused answer names the
input it could not use; report it and stop.

This alias keeps the old name for the read-only view only. It never
generates tests, never authors a gap plan and never edits status; a
requirement without failing-capable evidence appears as a broken or unmet
trace for the owner to act on through planning.

Association, for the example phase: a requirement assigned to phase 13 is joined to every current truth of phase 13 through the phase's typed map; no direct requirement-to-truth edge is authored or inferred. The only edges are requirement->phase (a trace
row), phase->roadmap (a declaration), phase->plan (a native publication
naming the requirement), plan->truths (the phase's approved truth set,
phase-scoped), truth->evidence (the current typed map) and evidence->verdict
(the current complete verification). Read-only: no status, map, UAT or store
record is written or repaired.
