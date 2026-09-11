# Phase 13 hook-retirement record

## Disposable rehearsal — completed 2026-09-11

P13-CLOSE-HOOK is mandatory close work, not acceptance-map evidence. The
executor ran only disposable settings copies, using the exact reviewed
`.planning/phases/13/close/retire-rules-gate.py` procedure. No installed owner
settings or hook directory was read or written.

Command:
`cargo test -p cadence --test phase13_close phase13_rules_gate_retirement_rehearsal -- --exact`

Literal final result: exit 0; `test result: ok. 1 passed; 0 failed; 0 ignored;
0 measured; 0 filtered out; finished in 7.56s`.
The prior named pass also finished in 7.56s; the last run includes the shared
fixture's explicit Default implementation. No linker retry occurred.

Compiled prerequisite observations: the existing planner uses the compiled
`plan-instructions` front door and real stdio native context approval,
preview/publication and `plan-read`; it has no retained planner-prompt query.
Two actual executor dispatches and one retained verifier attempt were read
over real serve/stdio after real native admission, red/green receipts, exact
owner statements, signed fixture completions and suite/risk settlement.
The retained verify-next response replayed identically after shutdown and
store reopen, with unchanged persistence bytes. Exact identities follow.

The disposable settings fixture contained three matching registrations across
PreToolUse and Stop, plus a suffix near match, extra-flag near match, sibling
guard command, unchanged matcher/group/event order and reviewer-stop bridge.
The resulting settings matched the handwritten complete expected bytes.
Both recovery originals matched raw input bytes. Guard and bridge files
matched their handwritten original bytes.

| Case | Actual outcome |
| --- | --- |
| Exact removal | exit 0, retired, three registrations removed; hook absent |
| Same inspected recovery replay | exit 0, already-retired; both absences rechecked |
| Current absent binding, fresh recovery path | exit 0, already-absent; near matches preserved |
| Duplicate hooks key | exit 1, ambiguous duplicate JSON key; no mutation/recovery directory |
| Stale settings digest | exit 1, stale inspected preimage; originals unchanged |
| Stale hook digest | exit 1, stale inspected preimage; originals unchanged |
| Real permission denial after unlink | exit 2, unfinished; hook absent, settings original, both recovery originals intact |
| Partial rerun | exit 1, unfinished refusal |
| Explicit recovery | exit 0, recovered; both originals restored byte-for-byte |
| Fresh inspection/recovery directory after restoration | exit 0, retired; exact expected output and both absences confirmed |

The permission-denial case ran as uid 1000, changing only a disposable
directory's mode from 0700 to 0500 and back. It used a real failed write,
not an injected process result. Disposable roots were removed by their
TempDir owners after the regression; the identifiers below are historical
observations, not paths to installed settings.

## Actual command vectors and identities

Each JSON record below came directly from the named test's un-captured
stdout. Arguments are exact vectors supplied to python3 after `-B` and
the displayed script path. These are reproduction records, not instructions
to rerun against a removed temporary path.

```jsonl
CLOSE_PREREQUISITE {"planner":"plan-instructions plus real stdio native plan-read/plan-submit","planner_bytes_sha256":"f1a779b74b29dc6c5ad96b1d5e2528bc2dc1e2204f14e78ed00cbf7cdbdb811a","executor_dispatches":["a2960abab83e5a60152e0e67cb562a3e909871db62950c5eb7cc0b0bd690bfa1","56296b1ff37bf5e004c518fd2b977336cc409e28e09165fa36503f10b1a59bcd"],"verifier_attempt":"52a110ca74f07bd877beaab4fe2ff1f1c63a89eaaeee097b78b2e9e2f8847bb9","prompt_digest":"2908171868622ba96bdbfd6a5f173800458b831ab2a2c3b2b3f7f22182f8d115"}
CLOSE_REHEARSAL {"program":"python3","script":"/code/cadence/.planning/phases/13/close/retire-rules-gate.py","args":["retire","--settings-root","/tmp/.tmpsIo31q/success","--settings-sha256","b4463ed70fdb0707923b58e1f532f4ef1e83699b8363c157ae517075b9caf54f","--hook-sha256","c025096641631a2ecfffab64d7631fd4d9a7c98a63d5c5242eb9bb202a66fce2","--command","node /tmp/.tmpsIo31q/success/hooks/rules-gate.mjs","--recovery","/tmp/.tmpsIo31q/success-recovery"],"exit":0,"answer":{"status":"retired","binding":{"root":"/tmp/.tmpsIo31q/success","command":["node","/tmp/.tmpsIo31q/success/hooks/rules-gate.mjs"],"settings":"b4463ed70fdb0707923b58e1f532f4ef1e83699b8363c157ae517075b9caf54f","hook":"c025096641631a2ecfffab64d7631fd4d9a7c98a63d5c5242eb9bb202a66fce2"},"removed":3,"settings_after":"7016e620fee49b9390e70de2d4842bade3158b074a0987873522aa8b605e3241","recovery":"/tmp/.tmpsIo31q/success-recovery"}}
CLOSE_REHEARSAL {"program":"python3","script":"/code/cadence/.planning/phases/13/close/retire-rules-gate.py","args":["retire","--settings-root","/tmp/.tmpsIo31q/success","--settings-sha256","b4463ed70fdb0707923b58e1f532f4ef1e83699b8363c157ae517075b9caf54f","--hook-sha256","c025096641631a2ecfffab64d7631fd4d9a7c98a63d5c5242eb9bb202a66fce2","--command","node /tmp/.tmpsIo31q/success/hooks/rules-gate.mjs","--recovery","/tmp/.tmpsIo31q/success-recovery"],"exit":0,"answer":{"status":"already-retired","binding":{"root":"/tmp/.tmpsIo31q/success","command":["node","/tmp/.tmpsIo31q/success/hooks/rules-gate.mjs"],"settings":"b4463ed70fdb0707923b58e1f532f4ef1e83699b8363c157ae517075b9caf54f","hook":"c025096641631a2ecfffab64d7631fd4d9a7c98a63d5c5242eb9bb202a66fce2"},"removed":3,"settings_after":"7016e620fee49b9390e70de2d4842bade3158b074a0987873522aa8b605e3241"}}
CLOSE_REHEARSAL {"program":"python3","script":"/code/cadence/.planning/phases/13/close/retire-rules-gate.py","args":["retire","--settings-root","/tmp/.tmpsIo31q/success","--settings-sha256","7016e620fee49b9390e70de2d4842bade3158b074a0987873522aa8b605e3241","--hook-sha256","absent","--command","node /tmp/.tmpsIo31q/success/hooks/rules-gate.mjs","--recovery","/tmp/.tmpsIo31q/absent-recovery"],"exit":0,"answer":{"status":"already-absent","binding":{"root":"/tmp/.tmpsIo31q/success","command":["node","/tmp/.tmpsIo31q/success/hooks/rules-gate.mjs"],"settings":"7016e620fee49b9390e70de2d4842bade3158b074a0987873522aa8b605e3241","hook":"absent"},"settings_after":"7016e620fee49b9390e70de2d4842bade3158b074a0987873522aa8b605e3241"}}
CLOSE_REHEARSAL {"program":"python3","script":"/code/cadence/.planning/phases/13/close/retire-rules-gate.py","args":["retire","--settings-root","/tmp/.tmpsIo31q/ambiguous","--settings-sha256","22019cbcd699559d41f00765b9395e3e71d38171fcb736b4b463b10fbf44ec6c","--hook-sha256","c025096641631a2ecfffab64d7631fd4d9a7c98a63d5c5242eb9bb202a66fce2","--command","node /tmp/.tmpsIo31q/ambiguous/hooks/rules-gate.mjs","--recovery","/tmp/.tmpsIo31q/ambiguous-recovery"],"exit":1,"answer":{"status":"refused","reason":"ambiguous duplicate JSON key: hooks"}}
CLOSE_REHEARSAL {"program":"python3","script":"/code/cadence/.planning/phases/13/close/retire-rules-gate.py","args":["retire","--settings-root","/tmp/.tmpsIo31q/stale","--settings-sha256","0000000000000000000000000000000000000000000000000000000000000000","--hook-sha256","c025096641631a2ecfffab64d7631fd4d9a7c98a63d5c5242eb9bb202a66fce2","--command","node /tmp/.tmpsIo31q/stale/hooks/rules-gate.mjs","--recovery","/tmp/.tmpsIo31q/stale-recovery"],"exit":1,"answer":{"status":"refused","reason":"stale inspected preimage"}}
CLOSE_REHEARSAL {"program":"python3","script":"/code/cadence/.planning/phases/13/close/retire-rules-gate.py","args":["retire","--settings-root","/tmp/.tmpsIo31q/stale","--settings-sha256","c50c3bf18dd0ad9575b1fdbbfdf6d5200d278e1f2aa19d283bfb506614f336cc","--hook-sha256","0000000000000000000000000000000000000000000000000000000000000000","--command","node /tmp/.tmpsIo31q/stale/hooks/rules-gate.mjs","--recovery","/tmp/.tmpsIo31q/stale-recovery"],"exit":1,"answer":{"status":"refused","reason":"stale inspected preimage"}}
CLOSE_REHEARSAL {"program":"python3","script":"/code/cadence/.planning/phases/13/close/retire-rules-gate.py","args":["retire","--settings-root","/tmp/.tmpsIo31q/partial","--settings-sha256","af844ec764c424ebf52c5a5c74062a741f7e8ad4468458d1e1cdce67887fb78f","--hook-sha256","c025096641631a2ecfffab64d7631fd4d9a7c98a63d5c5242eb9bb202a66fce2","--command","node /tmp/.tmpsIo31q/partial/hooks/rules-gate.mjs","--recovery","/tmp/.tmpsIo31q/partial-recovery"],"exit":2,"answer":{"status":"unfinished","reason":"[Errno 13] Permission denied: '/tmp/.tmpsIo31q/partial/settings.json.phase13-close-new'","recovery":"/tmp/.tmpsIo31q/partial-recovery"}}
CLOSE_REHEARSAL {"program":"python3","script":"/code/cadence/.planning/phases/13/close/retire-rules-gate.py","args":["retire","--settings-root","/tmp/.tmpsIo31q/partial","--settings-sha256","af844ec764c424ebf52c5a5c74062a741f7e8ad4468458d1e1cdce67887fb78f","--hook-sha256","c025096641631a2ecfffab64d7631fd4d9a7c98a63d5c5242eb9bb202a66fce2","--command","node /tmp/.tmpsIo31q/partial/hooks/rules-gate.mjs","--recovery","/tmp/.tmpsIo31q/partial-recovery"],"exit":1,"answer":{"status":"refused","reason":"unfinished or recovered attempt; recover, then inspect with a fresh recovery directory"}}
CLOSE_REHEARSAL {"program":"python3","script":"/code/cadence/.planning/phases/13/close/retire-rules-gate.py","args":["recover","--settings-root","/tmp/.tmpsIo31q/partial","--settings-sha256","af844ec764c424ebf52c5a5c74062a741f7e8ad4468458d1e1cdce67887fb78f","--hook-sha256","c025096641631a2ecfffab64d7631fd4d9a7c98a63d5c5242eb9bb202a66fce2","--command","node /tmp/.tmpsIo31q/partial/hooks/rules-gate.mjs","--recovery","/tmp/.tmpsIo31q/partial-recovery"],"exit":0,"answer":{"status":"recovered","binding":{"root":"/tmp/.tmpsIo31q/partial","command":["node","/tmp/.tmpsIo31q/partial/hooks/rules-gate.mjs"],"settings":"af844ec764c424ebf52c5a5c74062a741f7e8ad4468458d1e1cdce67887fb78f","hook":"c025096641631a2ecfffab64d7631fd4d9a7c98a63d5c5242eb9bb202a66fce2"}}}
CLOSE_REHEARSAL {"program":"python3","script":"/code/cadence/.planning/phases/13/close/retire-rules-gate.py","args":["retire","--settings-root","/tmp/.tmpsIo31q/partial","--settings-sha256","af844ec764c424ebf52c5a5c74062a741f7e8ad4468458d1e1cdce67887fb78f","--hook-sha256","c025096641631a2ecfffab64d7631fd4d9a7c98a63d5c5242eb9bb202a66fce2","--command","node /tmp/.tmpsIo31q/partial/hooks/rules-gate.mjs","--recovery","/tmp/.tmpsIo31q/reinspected-recovery"],"exit":0,"answer":{"status":"retired","binding":{"root":"/tmp/.tmpsIo31q/partial","command":["node","/tmp/.tmpsIo31q/partial/hooks/rules-gate.mjs"],"settings":"af844ec764c424ebf52c5a5c74062a741f7e8ad4468458d1e1cdce67887fb78f","hook":"c025096641631a2ecfffab64d7631fd4d9a7c98a63d5c5242eb9bb202a66fce2"},"removed":3,"settings_after":"71fe5b5f249cc03172973a38bb2bb347267014d92352755009cacea2cc9da1ed","recovery":"/tmp/.tmpsIo31q/reinspected-recovery"}}
```

## Installed result — PENDING PLAN-4 close

No installed-state inspection or removal has been performed by PLAN-1.
The orchestrator must inspect the installed binary and compiled front doors,
the explicitly selected actual settings root, exact hook and settings
preimages, registrations and unrelated guard bytes; review the concrete
invocation in `../close/rules-gate.md`; run the same close-only script; and
record actual hook absence, matching registration absence and preserved
unrelated bytes here. An interruption or partial result remains unfinished.
The verifier inspects those actual records and does not author this report.

Installed root, preimage identities, command, removal outcome, both absence
checks and preserved unrelated bytes: **pending; no success claimed**.
