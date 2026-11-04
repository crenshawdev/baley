# 0030: Put refinement and planning decisions to the owner in dependency-ordered question rounds

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-28 |
| Deciders | John Crenshaw |
| Design document | [0005: Context, plans and acceptance](../design/0005-context-plans-and-acceptance.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

Refinement writes a story's truths with the owner, and planning writes the plans that deliver them ([design 0005](../design/0005-context-plans-and-acceptance.md)). Both depend on decisions only the owner can make: what a story leaves unsaid, and choices a plan cannot take on its own.

The earlier design of 0005 dispatched the analyzer once. It returned the assumptions it found together with draft truths; the host session adjudicated both; the owner accepted assumptions and approved truths; the accepted assumptions were stored on `story.refined`. The inherited engine goes further the same way: the phase context carries `assumptions` as strings the session writes (`crates/baley/src/context/model.rs:44`), `/bal-context` has the session hold the interview itself (`crates/baley/src/context/instructions.rs:43-44`), and the compiled test derivation rules tell the planner to flag ambiguity for the owner in prose (`crates/baley/src/plan/instructions.rs:124`).

An assumptions list mixes facts the code and the records already settle with decisions only the owner can make. Its truths are drafted before the owner has answered, so a correction means redrafting. It gives no order, although some decisions only make sense once another is made. It keeps no record of a recommendation the owner turned down, or why. A plan has no place for a decision at all, so decisions end up in `notes` or task prose, where nothing holds the approval back until they are made. [ADR 0017](0017-stories-and-sprints.md) decides that truths are written with the owner at refinement; it does not decide how, and this record does not change it.

## Decision drivers

- The owner is asked only for decisions; a fact the code or the records hold is found by the model, never asked.
- Every question arrives with the asker's recommended answer, so the owner can decide quickly.
- A question that depends on another is asked after it is answered.
- Truths and plans are written on decisions already made.
- Every answer, including a rejected recommendation, and every deferral is on the record in the owner's words, and never edited.
- Baley works out the order; the model and the host session never choose it, and the session relays questions and answers unchanged.
- Nothing is approved while a decision is open, unless the owner defers it on the record with a reason.

## Considered options

1. One pass: the analyzer returns assumptions and draft truths together, and the owner accepts or corrects the list
2. All questions at once, unordered, answered together before drafting
3. Typed questions with a recommended answer and dependencies, put to the owner in rounds Baley works out, with truths and plans written on the answers
4. A free interview: the host session talks the story through with the owner and writes down the result

## Decision

Chosen option: **3**. A question is typed: an id, the one decision, a recommended answer and the ids of the questions it depends on, and Baley refuses a set with a blank or colliding id, a dependency outside the set or on itself, or a cycle (PLN-R23). The analyzer is dispatched twice: first it finds the facts itself and returns questions only for the decisions left to the owner, never truths; once no question of the set is open, Baley issues a second work order carrying every answer and deferral, and the analyzer drafts truths from them (PLN-R6). A plan carries its own `questions` field, and the planner writes the plan on the recommended answers (PLN-R11, PLN-R27).

Baley works out each round from the dependencies: the open questions whose dependencies are all answered (PLN-R24). The host session puts every question of the round to the owner with its recommended answer, shows what waits and on what, and relays the answers and deferrals unchanged. Each answer is recorded as `question.answered`, marked as taking or rejecting the recommendation; each deferral as `question.deferred` with the owner's reason; neither is ever edited (PLN-R25). `story truths approve` and `plan approve` are refused with `question-open` while a question of the set is open (PLN-R26). A plan whose set closed with a rejected recommendation is refused with `answer-not-applied`, and Baley issues the planner a revision work order carrying the draft and every answer (PLN-R27). This replaces the one-pass assumptions list.

## Consequences

### Positive

- The owner decides only what is theirs to decide, in an order that makes each question answerable when it is asked.
- Truths and plans are drafted on the owner's answers, not on guesses the owner corrects afterwards.
- The record keeps every decision, the recommendation it accepted or turned down, and every deferral with its reason, for the next reader and the next planner.
- A plan can no longer be approved with a decision buried in prose, or with a recommendation the owner rejected still written into it.

### Negative

- Refinement costs two analyzer dispatches instead of one, and a rejected recommendation on a plan costs a planner revision and another check.
- More round trips between the owner and the host session, one per round.
- New records, a `questions` view and thirteen refusal codes to build and test.
- A question that waits on a deferred one can never enter a round; the owner has to defer it too, which is deliberate but adds a step.
- The host session's instructions must hold it to relaying; a session that answers, merges or rewords questions breaks the design, and only its instructions stop it.

### Follow-up

- Build 4 ([#25](https://github.com/crenshawdev/baley/issues/25)) builds question sets, rounds, answers and deferrals, the `questions` view, the refusals, the analyzer's two work orders and the plan's `questions` field, as PLN-R6, PLN-R11 and PLN-R23 to PLN-R27 of design 0005. The seams are the refinement and plan submit operations of 0005; the inherited `assumptions` field (`crates/baley/src/context/model.rs:44`) and the prose ambiguity rule in the compiled derivation text (`crates/baley/src/plan/instructions.rs:124`) are removed there.
- Build 7 ([#28](https://github.com/crenshawdev/baley/issues/28)) adds the next-action and progress rules that name the current round and the waiting questions (design 0013, NXT-R3, NXT-R6).
- How each host puts a round's questions to the owner and returns the answers is decided in [design 0012](../design/0012-host-interface.md), an open question of 0005.

## Options in detail

### One pass with an assumptions list

Cheapest: one dispatch and one review by the owner. It puts facts and decisions in one list, drafts truths before any decision is made, gives no order, and records only the assumptions accepted, not the recommendations turned down. Plans have no place for decisions at all.

### All questions at once

Keeps questions apart from truths and drafts only after the answers, but asks the owner questions whose meaning depends on answers not yet given, and leaves the order to whoever presents them.

### Typed questions in rounds (chosen)

Every question carries its recommendation and its dependencies, Baley fixes the order from them, and truths and plans are written on the answers. The cost is more dispatches, more round trips and more records.

### A free interview in the host session

Natural for the owner, but the session decides what to ask and in what order, the record holds only what the session wrote down, and nothing stops the session from answering in the owner's place.
