//! The read contract's served text, pinned in source. It is the host interface's
//! short statement of what Baley serves by identity and what the host's own
//! tools read, so it lives beside the registry that serves it as
//! `bal-read-contract`.

/// The registry identity this text is served under.
pub const IDENTITY: &str = "bal-read-contract";

/// Pinned by hand: raise it by one whenever [`TEXT`] changes.
pub const VERSION: &str = "2";

/// Lowercase hex SHA-256 of [`TEXT`]'s exact bytes, computed outside the code
/// so a changed text without a new pin fails the registry's pin test.
pub const HASH: &str = "e20a4e61b36b0e0787a672c58558b04102a4357d428155ac3b618243b578ddd2";

/// What Baley serves by identity, how long answers are paged, and where source
/// is read.
pub const TEXT: &str = r#"Baley serves its help, its operation schemas and its instructions by identity, never by path. Ask through `baley_query`:

- `{"operation":"help"}` lists the commands, and `{"operation":"help","name":"<command name>"}` shows one.
- `{"operation":"schema","tool":"query","for":"<operation>"}` gives the argument schema of a `baley_query` operation. Use `"apply"` for `tool` to ask about `baley_apply`.
- `{"operation":"instruction","identity":"<identity>"}` gives the instruction with that identity, such as `bal-help`.
- `{"operation":"document","identity":{"kind":"capture","id":"<capture id>"}}` gives a capture's text, or its tombstone when its body was purged. It reads the current project.

An answer that fits in 24,576 bytes comes whole. A larger one comes in numbered parts: call again with `part` set to the value of `next` until `next` is null, and join the `body` values in order.

Read the project's source with the host's own file, search and shell tools: locate first, then read only the lines the work needs.

Never search for instructions or read them from disk. Ask for them by identity.
"#;
