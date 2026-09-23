# Store ownership and durability

The synchronous domain model owns versioned JSONL item revisions and decisions,
and a JSON state snapshot. The permanent semantic paths are `.planning/items.jsonl`,
`.planning/decisions.jsonl`, and `.planning/state.json`. Configuration retains its
repository and user-global layers. Snapshot integrity covers the two JSONL byte
strings and the snapshot's own serialized content with its integrity field empty.
Stable caller identities distinguish equal prose. Revisions start at one and must
advance by one. `Evidence` distinguishes missing, explicit null, and source text.
Only routing, gate, and refusal records enter the decisions log. Observed effort
normalization treats whitespace-only text as missing and preserves other host
spellings without comparing them to requested effort.

The `import` snapshot field is the import's own history and never changes after
completion. The layer mapping as it stands now lives in the `layers` field: the
current `active` paths, the digest of the global layer's bytes as the store last
wrote them (the writer refreshes it on every `global-config` participant), and
each accepted relocation with the generation that recorded it. A global layer
that canonicalizes to a new path at open, as when a home moves between a symlink
and a real directory, is accepted only when the bytes at the new place equal that
record, or the import's shared-global guard for a store that predates the record;
the mapping is then recorded at its own generation and the next open is exact.
A repo move, an absent record, or different bytes are refused as a changed layer
mapping, and the refusal names both places.

`Store::open` starts one dedicated blocking OS thread. A bounded Tokio request
queue feeds it; each request carries a separate oneshot reply. Tokio and file I/O
are outside the synchronous record algebra. Admitted work continues when a caller
cancels. A dropped reply is never evidence of success. Reads queue behind admitted
mutations, and startup recovery completes before a handle is returned.

`Policy::validate` is mandatory and has no default allow implementation. The
writer calls it at admission and after preparation, immediately before persisting
intent; recovery also validates policy before replay. PLAN-2 supplies a validator
that reloads both effective configuration layers and refuses a failed reload.
Store-integrity and policy failures return directly: they do not append a refusal
into a store they cannot safely mutate.

## Replacement and operation format

Every replacement exclusively creates a sibling temporary file, writes complete
bytes, calls `sync_all` on the temporary file, renames within that directory, calls
`sync_all` on the containing directory, and confirms installed bytes. Any failed
step prevents a success reply. File and ancestor device/inode identities, mode,
and complete bytes are compared at admission and after preparation. Symlinks and
unreadable existing files are refused. A known missing file is a conflict, not
initialization. Initialization requires all three stores to be absent.

All mutations use version-1 `.store-intent.json`, a temporary transaction artifact
containing the complete intended values and expected bytes/identities for every
participant, with its own digest. Participant names are the three semantic files
or `repo-config`/`global-config`. External names must be registered by the factory
with `Filesystem::with_participant`; journal text cannot choose arbitrary paths.
External config paths are absolute and their parents must already exist. Each
participant uses a sibling rename on its own filesystem; there is no claim that
renames across filesystems form an atomic operation.

The writer prepares disposable files and validates all participants before
installing intent. Intent itself uses the full file-sync/rename/directory-sync
protocol and must be confirmed before the first semantic replacement. Config
participants precede JSONL participants; the snapshot is last and holds operation
completion receipts. Intent removal and directory synchronization precede success.
The writer refuses further requests after a failed persistence operation.

Restart validates the entire intent and every participant before changing any
participant. An expected old target can advance. A target at the intended new
bytes is accepted only with the expected containing-directory identity, then both
its installed file and directory are synchronized again: interruption may have
happened after rename and before directory sync. Foreign bytes cause conflict
without partial replay. Every replay replacement follows the production durability
protocol. The intent is removed and its directory synchronized only after every
participant is complete. A second restart finds no intent and makes no update.
Disposable temporary files orphaned before an intent was installed are not semantic
records; they cannot authorize replay and are never adopted as a generation.

`Transaction::id` is a caller-supplied deterministic identity. Its logical content
digest excludes attempt-specific file identities, and completed identity/digest
pairs persist in `Snapshot::operations`, outside the user snapshot payload. Retrying
the same operation returns its confirmed view without duplicating records; reusing
an identity for different content is a conflict. Imports can use a frozen-source
set digest as the operation identity. Git and remote forge operations are outside
this local transaction protocol.

## Callable seams

PLAN-2 constructs `Filesystem`, registers config participant paths, supplies
`Policy`, and sends `Operation::Transact` with items, decisions, optional snapshot,
and `ExternalChange` entries carrying expected observations and complete new bytes.
`decisions::normalize` and `historical_observed_effort` share import/live evidence
rules. Missing historical receipts stay missing; worker observations without an
agent identity do not manufacture decisions.

PLAN-3 consumes `View::recall_items()` as its only structured item input. The
projection excludes every declined identity before exposing any revisions, then
returns the latest eligible revisions. `View::lookup_item` is the explicit evidence
and dedup lookup, not a recall source. Decision reads preserve append order.

## Limits

Filesystem validation is best effort at each check point. An uncooperative writer
can change a target after the last check and before replacement. Independent
session processes share this race. There is no OS compare-and-swap or cross-process
lock guarantee, and digests do not defend against a hostile rewrite of both data
and integrity metadata. Renames provide process-kill old-or-new atomicity; they do
not prove durability against power loss. AC8 requires syscall-order verification
of successful file and directory synchronization before acknowledgement.

## Shutdown cutoff and bound

The stdio ingress reserves capacity before decoding a tool request, then admits
the complete decoded request under the same lock that closes admission. Its
sequence number records transport order. An observed stdin EOF or received
SIGTERM closes admission atomically; a request that reaches admission after that
cutoff is refused. Bytes still buffered or partially decoded are not admitted.
Capacity is awaited: ingress retains at most 32 queued calls beside the active
call, and the resident and writer queues retain their 32-entry bounds.

The retained ingress worker owns each admitted call independently of the MCP
handler's reply future. It selects the earliest pending admission, runs handlers
in that order into the resident, and retains work when the caller cancels.
Caller cancellation is a structural contract here; live acceptance of it remains
unverified until phase 18.

`SERVER_DRAIN_BOUND` in `store::writer` is exactly
`Duration::from_secs(10)`, with no configuration key. EOF and SIGTERM record one
monotonic cutoff, and all handler draining, resident draining and writer joins
share its deadline. Successful shutdown explicitly closes the resident and
writer receivers, drains queued work, and joins the resident task and every
SessionFactory-owned writer thread, including writers opened before a later
initialization failure.

With work still open, `Drain::step` requests Wait at 9.999 seconds and returns a
typed DrainLimit at exactly 10 seconds or later. The stderr diagnostic names the
bound and open admitted request identity (admission sequence plus JSON-RPC id).
If only a writer join remains, it says so without inventing an open request.
An empty, completed drain requests normal join even when observed after the
bound. Shutdown neither acknowledges an unfinished write nor removes or adopts
its intent. Expiry exits without waiting indefinitely for a blocked syscall or
runtime blocking-pool teardown. Any open intent is left to the existing restart
contract above: validate the entire intent and all participants before replay.

## In-process evidence and its limits

The writer tests call production `Drain::step` with supplied admission state,
completion prefix, open-write identity and elapsed durations. The sole shutdown
check distinguishes Wait at 9.999 seconds from DrainLimit at 10 and 11 seconds.
Separate tests detect post-cutoff admission, selection out of admission order,
and needless waiting or a limit when nothing remains. Formatter tests detect a
wrong or omitted bound, a lost open-write identity, an invented identity during
join, or wording that claims shutdown acknowledged the open write.

These are constituent decisions, with no clock reads, signals, child processes,
store transaction or filesystem in the shutdown tests. They do not exercise the
assembled serve workflow. Actual EOF/SIGTERM delivery, caller cancellation,
elapsed exit timing, thread joins, journal acknowledgements after restart, real
crash survival and physical durability remain unverified until the phase 18
live acceptance gate. The former process-kill and strace test drivers are gone;
they are not runnable evidence. The file-sync, rename, directory-sync and
validated-recovery requirements above remain the durability contract.
