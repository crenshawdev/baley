---
phase: 17
plan: 4
requirements: ["T4"]
files: ["crates/cadence/src/main.rs","crates/cadence/src/review_ingress.rs","crates/cadence/src/server.rs","crates/cadence/src/recall/mod.rs","crates/cadence/src/import/mod.rs","crates/cadence/src/store/writer.rs","crates/cadence/Cargo.toml","Cargo.lock","crates/cadence/tests/serve_shutdown.rs","crates/cadence/tests/support/serve_process.rs","crates/cadence/tests/support/fsync_stall.c","crates/cadence/tests/support/serve.rs","docs/architecture/store.md","crates/cadence/tests/store.rs","crates/cadence/src/store/crash_tests.rs"]
directories: []
execution: {"schema":1,"suite":"cargo nextest run --workspace --no-fail-fast","tasks":[{"id":"P17-4-T1","verify":["cargo nextest run -p cadence --test serve_shutdown serve_drains_admitted_writes_on_transport_end"]},{"id":"P17-4-T2","verify":["cargo nextest run -p cadence --test serve_shutdown serve_drains_admitted_writes_on_transport_end","cargo nextest run -p cadence --test store confirmation_holds_own_reply_and_cancellation_preserves_admitted_work","cargo nextest run -p cadence --test store concurrently_queued_callers_receive_distinct_outcomes","cargo nextest run -p cadence --lib store::crash_tests::acknowledged_operations_survive_normal_process_restart"]}]}
---
## Goal

Stop admission, drain accepted work and join the store writer within a named ten-second shutdown bound.

## Must be true when done

- T4. When the transport ends while admitted writes are queued, the restarting owner sees every admitted write acknowledged in the journal within the drain bound, nothing admitted after the close, and a write still open at the bound left to journal recovery.

## Context

At HEAD crates/cadence/src/main.rs:298 returns from service.waiting on transport end. crates/cadence/src/store/writer.rs:242 creates channel(32), :244 discards the spawned thread handle, and :257 loops until every sender drops; :304 awaits queue capacity. The resident has another queue and owns the SessionFactory at crates/cadence/src/recall/mod.rs:509. InputTransport at crates/cadence/src/review_ingress.rs:586 owns ingress and reports lost answers on EOF. Waiting only for an empty store queue would miss accepted requests still in ingress or the resident queue. Caller cancellation is a separate existing guarantee in crates/cadence/tests/store.rs:92.

## Evidence map

```json
{
  "mode": "attached",
  "items": [
    {
      "kind": "check",
      "id": "check/serve_drains_admitted_writes_on_transport_end",
      "spec": {
        "command": "cargo nextest run -p cadence --test serve_shutdown serve_drains_admitted_writes_on_transport_end",
        "expected": {
          "kind": "property",
          "value": "Normal EOF/SIGTERM exit within 10 seconds plus a fixed 2-second scheduling margin; exactly ten complete acknowledged capture records survive restart in admission order and no eleventh is recorded. Draining does not depend on reading answers. The stalled process reports the 10-second limit and exits between 10 and 12 seconds after cutoff; its pending intent remains. Restart validates the valid intent before recovery and refuses the corrupted copy rather than adopting it blindly. Caller cancellation retains admitted work."
        },
        "test": {
          "file": "crates/cadence/tests/serve_shutdown.rs",
          "function": "serve_drains_admitted_writes_on_transport_end"
        },
        "setup": "Use the initialized real transport shape at crates/cadence/tests/support/serve.rs:21 and split send/receive example at crates/cadence/tests/support/read_fixtures.rs:95 in a new support/serve_process.rs. Use a small real initialized planning store in a fixture git repo. Ten distinct valid capture note applies carry handwritten request ids drain-01..drain-10 and distinct texts; capture's typed operation is crates/cadence/src/capture_service.rs:19. Keep the stdout pipe open and unread for success answers so its bounded output fits. Prepare separate EOF, SIGTERM and actual-fsync-stall processes.",
        "call": "Write ten complete apply frames, immediately close stdin, and time the real process exit; attempt an eleventh frame after close and require no corresponding journal item. For SIGTERM, request standard progress-token admission notifications, wait for all ten actual admissions without consuming success answers as evidence, then send SIGTERM and try a late frame. Restart through real serve and inspect the journal with reopened at crates/cadence/tests/support/serve.rs:224. In the stalled copy, wait for the external fsync barrier after intent durability, close transport, and observe deadline exit; remove the stall and restart to exercise normal journal recovery, plus a copy with deliberately invalid intent material.",
        "boundary": "Actual serve lifetime, ingress and real journal; only fixture inputs/clock/repository are controlled, plus the explicitly approved external fsync stall.",
        "fakes": [
          "An external fixture blocks one actual fsync to hold a real transaction open; no production test flag, fake store, or fake transport."
        ]
      },
      "reason": "This one check causes T4's trigger and inspects its stated outcome.",
      "associations": [
        {
          "truth_id": "T4",
          "truth_version": 1,
          "reason": "The trigger and outcome are exercised at the real boundary for T4."
        }
      ]
    },
    {
      "kind": "artifact",
      "id": "artifact/serve-drain",
      "spec": {
        "locators": [
          "crates/cadence/src/main.rs",
          "crates/cadence/src/review_ingress.rs",
          "crates/cadence/src/recall/mod.rs",
          "crates/cadence/src/import/mod.rs",
          "crates/cadence/src/store/writer.rs"
        ],
        "substance": "An admission cutoff, ordered drain and owned thread join share SERVER_DRAIN_BOUND=10s; timeout preserves the open intent for validated recovery."
      },
      "reason": "An admission cutoff, ordered drain and owned thread join share SERVER_DRAIN_BOUND=10s; timeout preserves the open intent for validated recovery.",
      "associations": [
        {
          "truth_id": "T4",
          "truth_version": 1,
          "reason": "This artifact is required for T4's stated outcome."
        }
      ]
    },
    {
      "kind": "artifact",
      "id": "artifact/serve-sigterm",
      "spec": {
        "locators": [
          "crates/cadence/src/main.rs",
          "crates/cadence/Cargo.toml"
        ],
        "substance": "SIGTERM enters the same bounded shutdown path using tokio signal support; it does not bypass the drain."
      },
      "reason": "SIGTERM enters the same bounded shutdown path using tokio signal support; it does not bypass the drain.",
      "associations": [
        {
          "truth_id": "T4",
          "truth_version": 1,
          "reason": "This artifact is required for T4's stated outcome."
        }
      ]
    }
  ]
}
```

## Tasks

### Task 1: Implement real admission shutdown and the one end-to-end drain check

- **ID:** P17-4-T1
- **Files:** crates/cadence/src/main.rs, crates/cadence/src/review_ingress.rs, crates/cadence/src/server.rs, crates/cadence/src/recall/mod.rs, crates/cadence/src/import/mod.rs, crates/cadence/src/store/writer.rs, crates/cadence/Cargo.toml, Cargo.lock, crates/cadence/tests/serve_shutdown.rs, crates/cadence/tests/support/serve_process.rs, crates/cadence/tests/support/fsync_stall.c, crates/cadence/tests/support/serve.rs
- **Action:** Deliver serve_drains_admitted_writes_on_transport_end red then green. Introduce an owned shutdown coordinator and SERVER_DRAIN_BOUND=10s. Admit complete decoded requests in transport order before releasing them to handlers; retain their work independently of caller future cancellation. EOF stops reads after prior complete frames; SIGTERM atomically closes admission. Publish optional admission progress for a caller's MCP progressToken only after the request owns its admission sequence, so tests can observe the actual boundary. Reject/drop frames after the cutoff, not already admitted requests. Drain admitted handlers into the resident queue in admission order, await resident work and all SessionFactory-owned writers, close their receivers explicitly, and retain/join each cadence-store JoinHandle. Do not rely on dropping every clone. Keep awaited 32-entry backpressure and cancellation behavior. Run one total deadline across ingress, resident and writer drain; at expiry emit the named drain-limit disposition on stderr and exit without deleting/adopting an open intent. Use tokio signal support for SIGTERM. Add a real-process driver that exposes send-without-recv, EOF, signal, bounded wait, stderr, and child reaping. The external LD_PRELOAD fixture compiled from tests/support/fsync_stall.c blocks an actual target fsync at an explicitly signaled filesystem barrier and delegates all other calls; it must never replace the store transaction or its recovery.
- **Verify:**
  - cargo nextest run -p cadence --test serve_shutdown serve_drains_admitted_writes_on_transport_end

### Task 2: Document the cutoff and retain cancellation/recovery guarantees

- **ID:** P17-4-T2
- **Files:** docs/architecture/store.md, crates/cadence/tests/store.rs, crates/cadence/src/store/crash_tests.rs, crates/cadence/tests/support/serve_process.rs
- **Action:** Document the precise admission cutoff, ten-second shared drain deadline, normal thread join and timeout leaving journal recovery in charge. Keep confirmation_holds_own_reply_and_cancellation_preserves_admitted_work, concurrently_queued_callers_receive_distinct_outcomes, and acknowledged_operations_survive_normal_process_restart behavior unchanged; adapt handles to the owned shutdown API only as needed. On timeout restart the real binary first; use recovered only for the deliberate still-open intent case and reopened after successful recovery. A blocked fsync target must not be simulated by the in-process with_probe helper, since that would never test serve exit.
- **Verify:**
  - cargo nextest run -p cadence --test serve_shutdown serve_drains_admitted_writes_on_transport_end
  - cargo nextest run -p cadence --test store confirmation_holds_own_reply_and_cancellation_preserves_admitted_work
  - cargo nextest run -p cadence --test store concurrently_queued_callers_receive_distinct_outcomes
  - cargo nextest run -p cadence --lib store::crash_tests::acknowledged_operations_survive_normal_process_restart

## Notes

D-211, after plan 3 so refused applies use the same drain. Add tokio's signal feature (currently absent), update the lockfile only if resolution needs it, and handle SIGTERM with the runtime signal stream. SERVER_DRAIN_BOUND is exactly Duration::from_secs(10), no config key. The bounded timeout exits even if fsync is blocked; it cannot synchronously join an uninterruptible syscall after the deadline. Normal shutdown joins successfully; timeout leaves the existing intent for recovery and terminates the process without waiting for blocking-pool teardown. The fsync stall is the approved external fixture, not a production switch. A standard progress-token admission notification in the SIGTERM fixture distinguishes actually admitted work from bytes merely written to a pipe; no new tool operation is introduced.
