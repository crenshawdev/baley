# 0010: Define the projector and event schema traits in the port

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-25 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | [0005](0005-storage-port.md), in part: where projectors live |
| Superseded by | |

## Context and problem

ADR 0005 puts storage behind a port. `baley-core` holds views, projectors and domain rules, `baley-store` holds the event, the chain and the port traits, and `baley-store-sqlite` implements the port. It says that projectors "live in the core and are handed to the adapter". This record supersedes only that placement. The rest of ADR 0005 stands: the port and adapter split, the crate boundary that keeps rusqlite out of the core, the conformance suite, and the rule that no business rule lives in an adapter.

As built, the placement cannot hold in full, for two reasons. First, both sides of the port have to name the projector's interface. The adapter calls `spec`, `handles`, `keys` and `apply` for every appended event and every replayed one, and the core implements them. The same is true of the event schema: the adapter asks it which event types and versions this binary reads, and for the current version and upcast payload a projector sees. The adapter cannot depend on `baley-core`, since the core depends on the port, so a trait both sides share has to live in the port crate.

Second, the store records some events itself. `command.completed` carries a command's outcome, and the `request` view built from it is what answers a retried request (EVD-R6). The adapter looks a request up in that view before any decision runs, replays its outcome, and rebuilds the view like any other. The event's shape, the view's spec and its projector are part of the store's own contract with every adapter, not a domain rule the core decides. If that projector lived in the core, the store could not answer a retry, or rebuild its own view, without a core it must not depend on.

## Decision drivers

- The crate graph stays one way: the core depends on the port, the adapter depends on the port, and neither the port nor the core depends on the engine.
- No business rule lives in an adapter.
- Every adapter gets the store-owned behaviour, request replay and its view, from one place, and the conformance suite can test it without the core.
- The core keeps everything that is a domain decision.

## Considered options

1. Traits and every projector in `baley-core`, as ADR 0005 reads
2. Traits in `baley-store`; domain projectors and the event schema registry in `baley-core`; the `request` projector in `baley-store`
3. Traits and every projector, domain ones included, in `baley-store`

## Decision

Chosen option: **2**. The `Projector` and `EventSchema` traits are defined in `baley-store`, the port. Domain projectors and the event type registry, with its upcasters, implement them in `baley-core` and are handed to the adapter when the store opens. The store's own `request` projector, with the `command.completed` event and the `request` view spec it reads, lives in `baley-store`, because the store owns command replay. The adapter registers it beside the core's projectors and runs them all the same way.

## Consequences

### Positive

- The crate graph is unchanged and still enforced: `baley-core` cannot reach rusqlite, and the adapter never depends on the core.
- No business rule lives in the adapter. It runs projectors and a schema it is handed, and the one projector it always registers comes from the port.
- Request replay and the `request` view behave identically in every adapter, and the conformance suite covers them without a core.
- The core still owns every domain view, every domain projector, the event types and their upcasters, and every rule that decides what may be recorded.

### Negative

- The port crate carries one projector of its own, so "projectors live in the core" is no longer true without the exception for store-owned events.
- A change to the store's own event or view is a change to the port, reviewed as one.

### Follow-up

- None. The code already follows this record; design 0001 points its storage port section here.

## Options in detail

### Option 1: traits and every projector in the core

Matches ADR 0005's wording. The adapter would need the traits to call projectors and the `request` projector to answer retries, and both would sit in a crate it must not depend on. It fails the crate graph driver, or forces the adapter to depend on the core and with it every domain rule.

### Option 2: traits in the port, domain projectors in the core, the request projector in the port

Both sides name one trait from the crate they already share. Domain decisions stay in the core. The store's own events and view come with the port every adapter implements. Meets every driver; the cost is one stated exception to where projectors live.

### Option 3: every projector in the port

Keeps all projectors in one place, but moves domain rules into the port, where every adapter and the conformance suite would carry them. It fails the driver that the core keeps every domain decision, and it grows the port with each new view.
