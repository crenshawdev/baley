# Read-layer planner measurement

The phase-close measurement is a read-only `document` identity for one actual
Claude Code planner round. Its caller supplies only the phase, Claude session
UUID, and the actual first and last turn UUIDs. Cadence derives Claude's
encoded project directory from the server's startup-bound project; it never
accepts or returns a host-record path, transcript bytes, or caller-calculated
metric.

The resolver follows the exact main-session parent chain between the two turn
boundaries and correlates the named planner worker through Claude's retained
agent metadata. It rejects absent, ambiguous, incomplete, cross-project, or
changing records. The source digest covers the exact immutable JSONL snapshots
used by the measurement.

Each unique actual tool-use id is classified once. Cadence query operations
that expose source or process content, built-in Read/Grep/Glob calls, and shell
or other tool calls that read project content all count as reads. A direct
Read without a bounded range, or whose requested range covers the served file,
counts as a whole-file read. An opaque shell or reader call whose extent cannot
be proved increments `unclassified_reads`; build and test process activity is
not mistaken for a planner-requested read. Missing tool results or usage make
the record incomplete rather than producing zero.

For tokens, the final complete usage row for each actual `(session id,
message.id)` contributes exactly four raw components: `input_tokens`,
`cache_creation_input_tokens`, `cache_read_input_tokens`, and `output_tokens`.
Streaming details, cache-duration subtotals, output-token details, and
iterations are not added again. Conflicting final rows are ambiguous.

The bounded report names its host/session/turn/worker boundaries and source
digest, the three read counters, all four token components and their total. It
also reports the owner-approved Cadence 3.7 planner median of 183000, the signed
difference and the exact ratio. The historical raw samples and aggregation
procedure were not supplied, so the comparison is numerical; a like-for-like
savings claim remains contingent on confirmation of that procedure.
