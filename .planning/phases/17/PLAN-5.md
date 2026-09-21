---
phase: 17
plan: 5
requirements: ["T5"]
files: ["crates/cadence/src/acquisition.rs","crates/cadence/src/lib.rs","crates/cadence/src/read/source.rs","crates/cadence/src/read/slice.rs","crates/cadence/src/read/search.rs","crates/cadence/src/read/list.rs","crates/cadence/src/read/document.rs","crates/cadence/src/read/measurement.rs","crates/cadence/src/read/outline/mod.rs","crates/cadence/src/store/filesystem.rs","crates/cadence/src/store/cache.rs","crates/cadence/src/store/mod.rs","crates/cadence/src/recall/documents.rs","crates/cadence/src/recall/history.rs","crates/cadence/src/recall/mod.rs","crates/cadence/src/read_service.rs","crates/cadence/tests/acquisition_bounds.rs","crates/cadence/tests/support/serve_process.rs","crates/cadence/tests/support/read_fixtures.rs","crates/cadence/tests/support/support_records.rs","docs/architecture/store.md"]
directories: []
execution: {"schema":1,"suite":"cargo nextest run --workspace --no-fail-fast","tasks":[{"id":"P17-5-T1","verify":["cargo nextest run -p cadence --test acquisition_bounds acquisition_bounds_name_the_crossing"]},{"id":"P17-5-T2","verify":["cargo nextest run -p cadence --test acquisition_bounds acquisition_bounds_name_the_crossing"]}]}
---
## Goal

Refuse acquisition above the source/store limits before loading content and name every crossing to its caller.

## Must be true when done

- T5. When a file the resident acquires exceeds its acquisition bound, the caller sees the crossing named (file, size, bound) instead of the content, and the resident never loads the whole file.

## Context

HEAD reads metadata then unbounded fs::read at crates/cadence/src/read/source.rs:12; search silently skips acquisition errors at crates/cadence/src/read/search.rs:298 and list filters them at crates/cadence/src/read/list.rs:30. Outline threshold is 24*1024 at crates/cadence/src/read/outline/mod.rs:43. Store filesystem read_to_end is crates/cadence/src/store/filesystem.rs:505, but the independent read-only cache also reads state/items/decisions at crates/cadence/src/store/cache.rs:87. Recall acquires document text at crates/cadence/src/recall/documents.rs:105 and walks metadata at :144; crates/cadence/src/recall/history.rs:203 reads ARCHIVE.md separately. Config's named-bound pattern is crates/cadence/src/config/floor.rs:16. These read paths, not answer truncation, are the change.

## Evidence map

```json
{
  "mode": "attached",
  "items": [
    {
      "kind": "check",
      "id": "check/acquisition_bounds_name_the_crossing",
      "spec": {
        "command": "cargo nextest run -p cadence --test acquisition_bounds acquisition_bounds_name_the_crossing",
        "expected": {
          "kind": "property",
          "value": "The source crossing names its path, size 16777217 and bound 16777216, has incomplete=true and no content. Search and recall skip the over-bound file and name that same triple. Exactly 16777216 is allowed and read returns an outline. State acquisition refuses open naming state.json, 1073741825 and 1073741824 before content load. For each metadata-rejected file the process reads fewer bytes and grows its resident high-water by less than that file's size, with no whole-file content in an answer. All expected sizes/dispositions are literals; comparisons cover actual sparse files."
        },
        "test": {
          "file": "crates/cadence/tests/acquisition_bounds.rs",
          "function": "acquisition_bounds_name_the_crossing"
        },
        "setup": "Use the real Client/file-reference machinery at crates/cadence/tests/support/read_fixtures.rs:37 and support_records::recall_fixture at crates/cadence/tests/support/support_records.rs:162. In a small real fixture repo, list a small valid Rust source to obtain its actual resident-issued file_reference, then File::set_len it to 16777217 bytes (sparse) before read. A separate fixture contains exactly 16777216 bytes of valid Rust plus whitespace, so an issued reference can reach a real outline. Create a sparse .planning/state.json of 1073741825 bytes in an independent otherwise-valid initialized store. Add an over-bound eligible RECORD.md to the recall fixture. Observe child /proc/<pid>/io rchar and VmHWM before/after each over-bound request; these are observations of the real process, not faked read/allocator seams.",
        "call": "Use cadence_query read with the issued reference, search over the source, read the exactly-at-bound reference, recall the eligible corpus, and a real store-opening query/apply in the oversized-state process. Repeat through cold cache and a restarted process so writer-only protection cannot pass. For crossing cases compare OS read-count deltas and high-water RSS before/after to the handwritten bound, and require a live answered request; do not turn an OOM or silent skipped file into a passing test.",
        "boundary": "Real serve and actual sparse files, real read/recall/store acquisition and OS observations. No synthetic source::content error or fake allocation counter.",
        "fakes": []
      },
      "reason": "This one check causes T5's trigger and inspects its stated outcome.",
      "associations": [
        {
          "truth_id": "T5",
          "truth_version": 1,
          "reason": "The trigger and outcome are exercised at the real boundary for T5."
        }
      ]
    },
    {
      "kind": "artifact",
      "id": "artifact/acquisition-constants",
      "spec": {
        "locators": [
          "crates/cadence/src/acquisition.rs"
        ],
        "substance": "MAX_SOURCE_BYTES=16777216 and MAX_STORE_BYTES=1073741824 are enforced on metadata before every relevant acquisition, with bounded reads and revalidation for races."
      },
      "reason": "MAX_SOURCE_BYTES=16777216 and MAX_STORE_BYTES=1073741824 are enforced on metadata before every relevant acquisition, with bounded reads and revalidation for races.",
      "associations": [
        {
          "truth_id": "T5",
          "truth_version": 1,
          "reason": "This artifact is required for T5's stated outcome."
        }
      ]
    },
    {
      "kind": "artifact",
      "id": "artifact/source-crossing-answers",
      "spec": {
        "locators": [
          "crates/cadence/src/read/source.rs",
          "crates/cadence/src/read/slice.rs",
          "crates/cadence/src/read/search.rs",
          "crates/cadence/src/read/list.rs",
          "crates/cadence/src/recall/documents.rs",
          "crates/cadence/src/recall/history.rs"
        ],
        "substance": "The actual {file,size,bound} crossing appears in incomplete read answers and skipped-file search/list/recall notes instead of file content."
      },
      "reason": "The actual {file,size,bound} crossing appears in incomplete read answers and skipped-file search/list/recall notes instead of file content.",
      "associations": [
        {
          "truth_id": "T5",
          "truth_version": 1,
          "reason": "This artifact is required for T5's stated outcome."
        }
      ]
    },
    {
      "kind": "artifact",
      "id": "artifact/store-crossing-refusal",
      "spec": {
        "locators": [
          "crates/cadence/src/store/filesystem.rs",
          "crates/cadence/src/store/cache.rs",
          "crates/cadence/src/store/mod.rs"
        ],
        "substance": "Both writer/open/recovery and independent cache acquisition reject oversized store files before reading/parsing, preserving file, size and bound in the refusal."
      },
      "reason": "Both writer/open/recovery and independent cache acquisition reject oversized store files before reading/parsing, preserving file, size and bound in the refusal.",
      "associations": [
        {
          "truth_id": "T5",
          "truth_version": 1,
          "reason": "This artifact is required for T5's stated outcome."
        }
      ]
    }
  ]
}
```

## Tasks

### Task 1: Bound every read-layer, recall and store acquisition and deliver its check

- **ID:** P17-5-T1
- **Files:** crates/cadence/src/acquisition.rs, crates/cadence/src/lib.rs, crates/cadence/src/read/source.rs, crates/cadence/src/read/slice.rs, crates/cadence/src/read/search.rs, crates/cadence/src/read/list.rs, crates/cadence/src/read/document.rs, crates/cadence/src/read/measurement.rs, crates/cadence/src/read/outline/mod.rs, crates/cadence/src/store/filesystem.rs, crates/cadence/src/store/cache.rs, crates/cadence/src/store/mod.rs, crates/cadence/src/recall/documents.rs, crates/cadence/src/recall/history.rs, crates/cadence/src/recall/mod.rs, crates/cadence/src/read_service.rs, crates/cadence/tests/acquisition_bounds.rs, crates/cadence/tests/support/serve_process.rs, crates/cadence/tests/support/read_fixtures.rs, crates/cadence/tests/support/support_records.rs
- **Action:** Create acquisition.rs with the two named u64 constants and a typed crossing {file,size,bound}. Check path metadata and opened-handle metadata before allocating; preserve confinement and regular-file checks. Read at most the bound, revalidate identity/size after the read to refuse a growing/changing file, and never reserve capacity from an unchecked metadata length. For already over-bound metadata acquire no content. Route source::content, slice/current/acquire, list/search, document's direct ROADMAP read and measurement's session/meta/recheck reads through the source helper. Issued-reference bound crossing must take precedence over stale content revision and return incomplete with the crossing note, no body; unissued capabilities still refuse normally. Search/list retain explicit skipped-file notes and incomplete=true rather than silently losing the file. Apply the same helper to recall Files::text, the metadata walk, ARCHIVE residue, and historical blob acquisition (size preflight plus capped streaming, preserving ReadGit's real production adapter). Route filesystem's store/recovery reads and cache's independent state/items/decisions reads through MAX_STORE_BYTES, before parse/hash/load; typed store-open errors preserve the crossing in the serve answer. Add the sole acquisition_bounds_name_the_crossing test red then green against actual sparse files and real stdio, observing process read counts/peak RSS rather than replacing the allocator or reader.
- **Verify:**
  - cargo nextest run -p cadence --test acquisition_bounds acquisition_bounds_name_the_crossing

### Task 2: State bounds at their acquisition boundary and audit bypasses

- **ID:** P17-5-T2
- **Files:** docs/architecture/store.md, crates/cadence/src/acquisition.rs, crates/cadence/src/read/source.rs, crates/cadence/src/store/filesystem.rs, crates/cadence/src/store/cache.rs, crates/cadence/src/recall/documents.rs, crates/cadence/src/recall/history.rs, crates/cadence/src/read/document.rs, crates/cadence/src/read/measurement.rs, crates/cadence/tests/acquisition_bounds.rs
- **Action:** Document the constants, crossing notes and store-open refusal, explicitly distinguishing them from answer bounds and heap work. Review every fs::read/read_to_string/read_to_end and File::open in the leased acquisition modules, including fast-cache hits after a file changes, direct document reads, recall historical objects and recovery input; leave no bypass of its chosen source/store helper. Keep the exactly-at-bound source case on the real outline path, with no new arbitrary parser deadline or cap change. Strengthen cases inside the same single acceptance function if an alternate real acquisition route escaped the first pass.
- **Verify:**
  - cargo nextest run -p cadence --test acquisition_bounds acquisition_bounds_name_the_crossing

## Notes

D-212 and D-216: MAX_SOURCE_BYTES=16*1024*1024 (16777216), MAX_STORE_BYTES=1024*1024*1024 (1073741824), checked before allocation/read and unchanged here. The store cap is a corrupt-input guard, not a heap claim; GH-264 remains outside this plan. Cover state.json, items.jsonl, decisions.jsonl and recovery-intent store input in both writer and cold-cache routes. Source acquisition covers read-layer and recall documents including ARCHIVE and host measurement documents; do not broaden the two constants into a config migration. Existing config limits remain as they are. No wire names, rendered pin or tools/list ceiling change.
