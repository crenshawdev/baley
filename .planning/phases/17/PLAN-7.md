---
phase: 17
plan: 7
requirements: ["T7"]
files: ["crates/cadence/src/instruction_surfaces.rs","crates/cadence/src/wire_contract.rs","crates/cadence/src/lib.rs","crates/cadence/src/main.rs","crates/cadence/src/server.rs","crates/cadence/src/execution/render.rs","crates/cadence/src/review/wire.rs","crates/cadence/src/review/mod.rs","crates/cadence/src/review_service.rs","crates/cadence/src/capture/mod.rs","crates/cadence/src/capture_service.rs","crates/cadence/src/adoption/mod.rs","crates/cadence/src/adoption_service.rs","crates/cadence/tests/mcp.rs","crates/cadence/tests/help_reference.rs","crates/cadence/src/instruction_lint.rs","crates/cadence/src/config/schema.json","crates/cadence/src/help/table.rs","crates/cadence/src/execution/instructions.rs","crates/cadence/src/verification/instructions.rs","crates/cadence/src/context/instructions.rs","crates/cadence/src/plan/instructions.rs","crates/cadence/src/help/instructions.rs","crates/cadence/src/debug/instructions.rs","crates/cadence/src/spike/instructions.rs","crates/cadence/src/undo/instructions.rs","crates/cadence/src/landing/instructions.rs","crates/cadence/src/milestone/instructions.rs","crates/cadence/src/suggest/instructions.rs","crates/cadence/src/why/instructions.rs","crates/cadence/src/progress/instructions.rs","crates/cadence/src/capture/instructions.rs","crates/cadence/src/review/instructions.rs","crates/cadence/src/read/instructions.rs","crates/cadence/src/task/instructions.rs","skills/cad-help/SKILL.md","skills/cad-spike/SKILL.md","skills/cad-debug/SKILL.md","skills/cad-undo/SKILL.md","skills/cad-land/SKILL.md","skills/cad-milestone/SKILL.md","skills/cad-suggest/SKILL.md","skills/cad-why/SKILL.md","skills/cad-progress/SKILL.md","skills/cad-capture/SKILL.md","skills/cad-context/SKILL.md","skills/cad-plan/SKILL.md","skills/cad-executor-contract/SKILL.md","skills/cad-execute/SKILL.md","skills/cad-verifier-contract/SKILL.md","skills/cad-verify/SKILL.md","skills/cad-review/SKILL.md","skills/cad-decision-review/SKILL.md","skills/cad-minimalism-review/SKILL.md","skills/cad-plan-review/SKILL.md","skills/cad-audit/SKILL.md","skills/cad-coverage/SKILL.md","skills/cad-read-contract/SKILL.md","skills/cad-task/SKILL.md","cadence-core/bin/self-verify.mjs","cadence-core/bin/self-verify.test.mjs","cadence-core/bin/lib/config-reach.mjs","cadence-core/bin/lib/dispatch-phrasing.mjs","cadence-core/bin/lib/route-relay.mjs","cadence-core/bin/lib/merge-warnings.mjs","cadence-core/bin/lib/reference-routers.mjs","cadence-core/bin/lib/include-consumers.mjs","cadence-core/bin/lib/text-transport.mjs","cadence-core/bin/lib/bulk-output.mjs","cadence-core/bin/lib/scratch-path.mjs","cadence-core/bin/lib/capture-writers.mjs","cadence-core/bin/lib/hook-events.mjs","cadence-core/bin/dispatch-phrasing.test.mjs","cadence-core/bin/route-relay.test.mjs","cadence-core/bin/reference-routers.test.mjs","cadence-core/bin/include-consumers.test.mjs","cadence-core/bin/text-transport.test.mjs","cadence-core/bin/bulk-output.test.mjs","cadence-core/bin/scratch-path.test.mjs","cadence-core/bin/capture-writers.test.mjs","cadence-core/bin/hook-events.test.mjs","cadence-core/bin/lib/arg-contract.mjs","cadence-core/bin/arg-contract.test.mjs","cadence-core/bin/test.mjs","cadence-core/bin/helper-census.test.mjs","cadence-core/bin/lib/census-registry.mjs","cadence-core/bin/census-registry.test.mjs","cadence-core/bin/weight-budgets.json","cadence-core/bin/weight.mjs",".github/workflows/release.yml","CONTRIBUTING.md","METHOD.md","cadence-core/bin/why-record.test.mjs"]
directories: []
execution: {"schema":1,"suite":"cargo nextest run --workspace --no-fail-fast","tasks":[{"id":"P17-7-T1","verify":["cargo nextest run -p cadence --test help_reference help_lists_the_installed_skills_from_the_compiled_table","cargo nextest run -p cadence --test mcp skill_contract_matches_wire_patch_and_direct_tool_permissions"]},{"id":"P17-7-T2","verify":["cargo nextest run -p cadence --lib compiled_instructions_name_only_what_exists"]},{"id":"P17-7-T3","verify":["cargo nextest run -p cadence --lib compiled_instructions_name_only_what_exists","node --test --test-name-pattern='wrapped Files:|declaringTasks names' cadence-core/bin/why-record.test.mjs","node --test cadence-core/bin/census-registry.test.mjs","node --test cadence-core/bin/helper-census.test.mjs","node --test --test-name-pattern='every flag in every row|declarations the CONTEXT' cadence-core/bin/arg-contract.test.mjs"]}]}
---
## Goal

One native lint renders every compiled surface, rejects unresolved names with their surface, and replaces self-verify and its exclusive libraries.

## Must be true when done

- T7. When a compiled instruction names something that does not exist at HEAD (a config key, wire operation, skill, repository path, hook event), the maintainer sees cargo test fail naming the surface and the name, from one test that renders every surface through the binary's own renderers.

## Context

Last in sequence: task must already be rendered file 24. At HEAD the CLI directly calls library renderers, for example crates/cadence/src/main.rs:94 and :213; executor dispatch/contract/frontdoor are crates/cadence/src/execution/instructions.rs:313, :318 and :323. The production rendered-file registry is crates/cadence/src/execution/render.rs:144. The help table lives at crates/cadence/src/help/table.rs:16. The native schema is crates/cadence/src/config/schema.json (not the frozen cadence-core/config.schema.json). self-verify imports its twenty libraries at cadence-core/bin/self-verify.mjs:259. Contrary to the older context, .github/workflows/release.yml:64 is a real Node self-verify consumer; .github/workflows/test.yml:68-69 runs the Rust build and workspace nextest. The replacement is a --lib unit check, never a new self-verify CLI or a fabricated render fixture.

## Evidence map

```json
{
  "mode": "attached",
  "items": [
    {
      "kind": "check",
      "id": "check/compiled_instructions_name_only_what_exists",
      "spec": {
        "command": "cargo nextest run -p cadence --lib compiled_instructions_name_only_what_exists",
        "expected": {
          "kind": "property",
          "value": "The mutated fixture fails naming both its surface and model.phase17_invented; clean compiled surfaces resolve every extracted key/operation/skill/path/event, every native schema key has a real reader, and every rendered file is below its explicit byte ceiling. There are exactly 24 rendered files including cad-task. self-verify.mjs, self-verify.test.mjs and the eleven exclusive libraries (and their nine dedicated tests) do not exist. The expected token, count, pinned PreToolUse event and ceilings are handwritten."
        },
        "test": {
          "file": "crates/cadence/src/instruction_lint.rs",
          "function": "compiled_instructions_name_only_what_exists"
        },
        "setup": "In crates/cadence/src/instruction_lint.rs beside the library renderers, use the same instruction_surfaces dispatcher that main.rs's instruction arms invoke. Begin from all 24 production RENDERED_PROJECT_FILES including cad-task; render executor dispatch_text and command_policy for all four MANIFESTS plus no-manifest, then verification/context/plan source products, compiled help rows, and parse real hooks/hooks.json. Read native config/schema.json, the actual shared schema-derived wire registry, shipped skills and the current checkout, not fixture answers. Define the extraction grammar and literal ceilings in this test source.",
        "call": "Validate the real rendered corpus, then append the invented model.phase17_invented key to one named rendered fixture and invoke the identical validator. Require that deliberate mutation to return a named error and the clean corpus to return none. Resolve schema readers to production callsites, compare each rendered byte length to its named ceiling, assert installed/rendered identity, and assert the retired self-verify files and eleven exclusive libraries/tests are absent.",
        "boundary": "In-crate --lib test of the production render functions, not stdio. Only the deliberate invalid-name fixture is synthetic; registry and outputs are the same code paths the CLI uses.",
        "fakes": []
      },
      "reason": "This one check causes T7's trigger and inspects its stated outcome.",
      "associations": [
        {
          "truth_id": "T7",
          "truth_version": 1,
          "reason": "The trigger and outcome are exercised at the real boundary for T7."
        }
      ]
    },
    {
      "kind": "artifact",
      "id": "artifact/compiled-instruction-lint",
      "spec": {
        "locators": [
          "crates/cadence/src/instruction_lint.rs",
          "crates/cadence/src/instruction_surfaces.rs",
          "crates/cadence/src/wire_contract.rs"
        ],
        "substance": "One unit check states a candidate-name grammar and resolves the entire production-rendered corpus against native config, wire operations, skills, paths and hooks."
      },
      "reason": "One unit check states a candidate-name grammar and resolves the entire production-rendered corpus against native config, wire operations, skills, paths and hooks.",
      "associations": [
        {
          "truth_id": "T7",
          "truth_version": 1,
          "reason": "This artifact is required for T7's stated outcome."
        }
      ]
    },
    {
      "kind": "artifact",
      "id": "artifact/instruction-negative-fixture",
      "spec": {
        "locators": [
          "crates/cadence/src/instruction_lint.rs"
        ],
        "substance": "A real rendered surface augmented with model.phase17_invented proves the validator rejects an unknown candidate and names surface/key."
      },
      "reason": "A real rendered surface augmented with model.phase17_invented proves the validator rejects an unknown candidate and names surface/key.",
      "associations": [
        {
          "truth_id": "T7",
          "truth_version": 1,
          "reason": "This artifact is required for T7's stated outcome."
        }
      ]
    },
    {
      "kind": "artifact",
      "id": "artifact/instruction-byte-ceilings",
      "spec": {
        "locators": [
          "crates/cadence/src/instruction_lint.rs",
          "cadence-core/bin/weight-budgets.json"
        ],
        "substance": "Every one of the 24 rendered files has the explicit finite UTF-8 byte ceiling stated in task 2; no missing or derived-at-runtime ceiling can silently pass."
      },
      "reason": "Every one of the 24 rendered files has the explicit finite UTF-8 byte ceiling stated in task 2; no missing or derived-at-runtime ceiling can silently pass.",
      "associations": [
        {
          "truth_id": "T7",
          "truth_version": 1,
          "reason": "This artifact is required for T7's stated outcome."
        }
      ]
    },
    {
      "kind": "artifact",
      "id": "artifact/self-verify-retirement",
      "spec": {
        "locators": [
          "crates/cadence/src/instruction_lint.rs",
          ".github/workflows/release.yml",
          "CONTRIBUTING.md",
          "METHOD.md",
          "cadence-core/bin/lib/census-registry.mjs"
        ],
        "substance": "Native CI lint replaces self-verify, its test, eleven exclusive libraries and nine dedicated tests; the real release/doc/test-table consumers are removed and shared library/history consumers remain intact."
      },
      "reason": "Native CI lint replaces self-verify, its test, eleven exclusive libraries and nine dedicated tests; the real release/doc/test-table consumers are removed and shared library/history consumers remain intact.",
      "associations": [
        {
          "truth_id": "T7",
          "truth_version": 1,
          "reason": "This artifact is required for T7's stated outcome."
        }
      ]
    }
  ]
}
```

## Tasks

### Task 1: Expose the actual compiled render and wire registries to the library test

- **ID:** P17-7-T1
- **Files:** crates/cadence/src/instruction_surfaces.rs, crates/cadence/src/wire_contract.rs, crates/cadence/src/lib.rs, crates/cadence/src/main.rs, crates/cadence/src/server.rs, crates/cadence/src/execution/render.rs, crates/cadence/src/review/wire.rs, crates/cadence/src/review/mod.rs, crates/cadence/src/review_service.rs, crates/cadence/src/capture/mod.rs, crates/cadence/src/capture_service.rs, crates/cadence/src/adoption/mod.rs, crates/cadence/src/adoption_service.rs, crates/cadence/tests/mcp.rs, crates/cadence/tests/help_reference.rs
- **Action:** Add instruction_surfaces.rs as the single project-free renderer dispatch keyed by the existing RenderedProjectFile command arrays, including aliases/frontdoor flags and task-instructions; main.rs delegates its corresponding Command::*Instructions arms to those same functions. Keep execution dispatch_text available as an additional surface. Enumerate RENDERED_PROJECT_FILES rather than maintain a divergent list of 24 strings. Expose the actual schema-derived query/apply operation registry from wire_contract.rs to both server and library lint: relocate QueryArguments, ApplyArguments, group tags and schema walking; move only the binary-local review Query/Apply, capture Apply and adoption Apply declarations into library-facing modules and re-export them from their services. Use the existing library config_service Apply and existing domain model enums; do not invent a hand-maintained list disconnected from server routing. Preserve wire ordering and minimal schemas exactly, including plan 1 task operations and plan 3 why selector. Verify renderer parity through the existing real CLI test and schema/permission pins; no byte ceiling bump for an extraction.
- **Verify:**
  - cargo nextest run -p cadence --test help_reference help_lists_the_installed_skills_from_the_compiled_table
  - cargo nextest run -p cadence --test mcp skill_contract_matches_wire_patch_and_direct_tool_permissions

### Task 2: Deliver the one instruction lint red then green and delete its replaced surface

- **ID:** P17-7-T2
- **Files:** crates/cadence/src/instruction_lint.rs, crates/cadence/src/instruction_surfaces.rs, crates/cadence/src/wire_contract.rs, crates/cadence/src/config/schema.json, crates/cadence/src/help/table.rs, crates/cadence/src/execution/instructions.rs, crates/cadence/src/verification/instructions.rs, crates/cadence/src/context/instructions.rs, crates/cadence/src/plan/instructions.rs, crates/cadence/src/help/instructions.rs, crates/cadence/src/debug/instructions.rs, crates/cadence/src/spike/instructions.rs, crates/cadence/src/undo/instructions.rs, crates/cadence/src/landing/instructions.rs, crates/cadence/src/milestone/instructions.rs, crates/cadence/src/suggest/instructions.rs, crates/cadence/src/why/instructions.rs, crates/cadence/src/progress/instructions.rs, crates/cadence/src/capture/instructions.rs, crates/cadence/src/review/instructions.rs, crates/cadence/src/read/instructions.rs, crates/cadence/src/task/instructions.rs, skills/cad-help/SKILL.md, skills/cad-spike/SKILL.md, skills/cad-debug/SKILL.md, skills/cad-undo/SKILL.md, skills/cad-land/SKILL.md, skills/cad-milestone/SKILL.md, skills/cad-suggest/SKILL.md, skills/cad-why/SKILL.md, skills/cad-progress/SKILL.md, skills/cad-capture/SKILL.md, skills/cad-context/SKILL.md, skills/cad-plan/SKILL.md, skills/cad-executor-contract/SKILL.md, skills/cad-execute/SKILL.md, skills/cad-verifier-contract/SKILL.md, skills/cad-verify/SKILL.md, skills/cad-review/SKILL.md, skills/cad-decision-review/SKILL.md, skills/cad-minimalism-review/SKILL.md, skills/cad-plan-review/SKILL.md, skills/cad-audit/SKILL.md, skills/cad-coverage/SKILL.md, skills/cad-read-contract/SKILL.md, skills/cad-task/SKILL.md, cadence-core/bin/self-verify.mjs, cadence-core/bin/self-verify.test.mjs, cadence-core/bin/lib/config-reach.mjs, cadence-core/bin/lib/dispatch-phrasing.mjs, cadence-core/bin/lib/route-relay.mjs, cadence-core/bin/lib/merge-warnings.mjs, cadence-core/bin/lib/reference-routers.mjs, cadence-core/bin/lib/include-consumers.mjs, cadence-core/bin/lib/text-transport.mjs, cadence-core/bin/lib/bulk-output.mjs, cadence-core/bin/lib/scratch-path.mjs, cadence-core/bin/lib/capture-writers.mjs, cadence-core/bin/lib/hook-events.mjs, cadence-core/bin/dispatch-phrasing.test.mjs, cadence-core/bin/route-relay.test.mjs, cadence-core/bin/reference-routers.test.mjs, cadence-core/bin/include-consumers.test.mjs, cadence-core/bin/text-transport.test.mjs, cadence-core/bin/bulk-output.test.mjs, cadence-core/bin/scratch-path.test.mjs, cadence-core/bin/capture-writers.test.mjs, cadence-core/bin/hook-events.test.mjs, cadence-core/bin/lib/arg-contract.mjs, cadence-core/bin/arg-contract.test.mjs, cadence-core/bin/test.mjs, cadence-core/bin/helper-census.test.mjs, cadence-core/bin/lib/census-registry.mjs, cadence-core/bin/census-registry.test.mjs, cadence-core/bin/weight-budgets.json, cadence-core/bin/weight.mjs, .github/workflows/release.yml, CONTRIBUTING.md, METHOD.md
- **Action:** Add cfg(test) instruction_lint in the library, with exactly one acceptance test compiled_instructions_name_only_what_exists. Render every production surface from task 1, plus execution::instructions::command_policy for each of the four MANIFESTS and the no-manifest warning branch, and parse names with an explicit grammar: config tokens are backticked/JSON dotted identifiers and documented single-key config fields; capture candidate identifiers before checking membership so an unknown name cannot disappear. Operations are JSON operation string values, schema operation const values and backticked names in cadence_apply/query invocation syntax; skills are /cad-* tokens and skills/cad-*/SKILL.md paths, with user names resolved by the compiled help table and internal skills by their shipped frontmatter; paths are backtick repository-root paths and @ includes after expanding CLAUDE_PLUGIN_ROOT, stripping optional line suffixes. Only syntactically explicit placeholders/globs or described caller-project examples get a classified template treatment with a stated reason, never a catch-all unknown-name allowlist. Hook event keys come from parsed hooks/hooks.json and must equal the pinned set {PreToolUse}; validate its guard arm and 10-second timeout. Resolve names against native schema, the actual shared wire registry, compiled/shipped skill table, and the current checked-out HEAD tree. For every native schema key inspect a concrete production config read or an explicit dynamic family expansion naming its real reader symbol; a mention in schema/instructions/tests is not a reader. Check schema defaults against their types/enums and effort enums against the actual native supported-effort vocabulary; do not change effective config semantics to satisfy the lint. Add an invented-key fixture by appending a clearly parsed model.phase17_invented token to a real rendered surface; assert the validator's error is exactly that surface and token, then require no issues from unmodified renders. Register explicit UTF-8 ceilings in instruction_lint.rs: cad-help 1024; cad-spike 6144; cad-debug 15360; cad-undo 4096; cad-land 8192; cad-milestone 6144; cad-suggest 1536; cad-why 4096; cad-progress 1024; cad-capture 1536; cad-context 24576; cad-plan 57344; cad-executor-contract 28672; cad-execute 20480; cad-verifier-contract 18432; cad-verify 18432; cad-review 12288; cad-decision-review 12288; cad-minimalism-review 12288; cad-plan-review 12288; cad-audit 9216; cad-coverage 9216; cad-read-contract 6144; cad-task 32768. Keep budget entries consistent, retaining only intentional finite headroom. First run the sole check red, retain its real failure, then correct actual stale compiled names and regenerate their rendered artifacts within the leases.

HEAD import census across cadence-core/ and skills/, each module disposition: seam-io KEEP (planning/config/route/weight and other CLIs); surface-weight KEEP (weight, review-provider, prose-agreement); rung-agent KEEP (route, planning/core, config/route/prose tests); gate-agreement KEEP (route); config-reach DELETE (only self-verify; no separate dedicated test); global-only-keys KEEP (config-merge); dispatch-phrasing DELETE (only self-verify plus dispatch-phrasing.test); route-relay DELETE (only self-verify plus route-relay.test); merge-warnings DELETE (only self-verify and self-verify.test); frontmatter KEEP (resident-weight); deferred-reads KEEP (prose-agreement.test as well as its own tests); reference-routers DELETE (only self-verify and its dedicated/self-verify tests); include-consumers DELETE (same); refusal-hints KEEP (reason-census.test calls refusalSites); text-transport DELETE (only self-verify and dedicated test); bulk-output DELETE (same); scratch-path DELETE (same; scratch-readback only names it in commentary, retain scratch-readback); capture-writers DELETE (only self-verify and dedicated test); hook-events DELETE (same); arg-contract KEEP (multiple CLIs). Recheck imports, then delete self-verify.mjs/self-verify.test.mjs and exactly those eleven exclusive modules plus the nine dedicated .test.mjs files named in Files. Remove the self-verify arg-contract row and its PINNED fixture, recount only affected contract flag censuses, remove deleted test names from test.mjs, and retire census rows whose holders were deleted. Remove the real release.yml Node self-verify step, CONTRIBUTING command and explanatory paragraph, METHOD's self-verify path/claims and table row; point maintainers at this native check and the existing workspace CI consumer. Update weight.mjs and weight-budgets comments, helper-census references to deleted helpers and census-registry's obsolete asserting-site references. This deletion is part of the same green change that asserts the removed files are absent.
- **Verify:**
  - cargo nextest run -p cadence --lib compiled_instructions_name_only_what_exists

### Task 3: Verify surviving history consumers and census integrity

- **ID:** P17-7-T3
- **Files:** cadence-core/bin/why-record.test.mjs, cadence-core/bin/census-registry.test.mjs, cadence-core/bin/lib/census-registry.mjs, cadence-core/bin/helper-census.test.mjs, cadence-core/bin/arg-contract.test.mjs, cadence-core/bin/weight-budgets.json
- **Action:** Retain why-record.test.mjs's self-verify.mjs strings: they are paths in an archived plan fixture, not imports or live invocations. Run the wrapped-Files test and declaringTasks test explicitly, editing only if deletion exposed a real dependency. Register Rust lint/deadline/rendered-file pins with their actual holder/subject/test instead of pretending the retired JavaScript budget linter still runs. Keep unrelated frozen helper and census assertions. Confirm deleted modules have no surviving importer and every generated file remains byte-equal to the actual renderer; the unit check's key/path grammar must not inspect frozen historical fixture strings as present-tense product instructions.
- **Verify:**
  - cargo nextest run -p cadence --lib compiled_instructions_name_only_what_exists
  - node --test --test-name-pattern='wrapped Files:|declaringTasks names' cadence-core/bin/why-record.test.mjs
  - node --test cadence-core/bin/census-registry.test.mjs
  - node --test cadence-core/bin/helper-census.test.mjs
  - node --test --test-name-pattern='every flag in every row|declarations the CONTEXT' cadence-core/bin/arg-contract.test.mjs

## Notes

D-214 and D-216. T7 reaches actual renderers by a library instruction_surfaces registry used by main.rs's instruction-command arms and by the unit test; it enumerates all 24 RENDERED_PROJECT_FILES and separately renders executor dispatch/protocol and command_policy's four manifest variants and warning branch, verification/context/plan, help descriptions and hook manifest data. No second renderer or hand-copied instruction sample substitutes for the shipped surface. No operation names or tools/list bound change; pin stays 24. The only intentionally invalid fixture is a rendered surface augmented with config key model.phase17_invented; validation must report both fixture name and key.

Requirement dispositions (no REQUIREMENTS.md edit):
- CWT-02: carried by T7's named per-rendered-file byte ceilings.
- CWT-03: old agent/frontmatter recognizer retires; native direct-tool/skill permissions remain pinned by mcp.
- #44: repository path existence and compiled surface completeness carried by T7; old full-install directory census retires.
- #74: compiled skill-name resolution carried by T7; old agent preload mechanism retires.
- RNG-01: rung-file behavior recognizer retires; compiled renderer identity remains pinned.
- RNG-02: rung-file/effort recognizers retire; existing native routing semantics stay credited.
- ENF-02: schema key/default validity and reader reachability carried by T7; route-table recognizer retires with the table mechanism.
- TRN-01: shell argument text recognizer retires; typed wire values remain the native transport.
- TRN-02: scratch-file/bulk shell redirection recognizer retires; native bounded answers stay credited.
- HNT-02: hint-field recognizer retires under ruling P9; code/reason/rule/slot remains the refusal shape.
- HOK-02: hook names resolve against the pinned PreToolUse set in T7.
- CEN-01: surviving census-registry suite remains; remove retired rows with their holders and register the Rust rendered/caller/budget censuses, not a replacement whole-tree JavaScript lint.

Nine shared self-verify libraries and their surviving tests remain by the no-other-consumer rule; eleven exclusive libraries are removed, with the per-module census in task 2. D-215 heap, requirement retirement and issue closes are outside this plan set.
