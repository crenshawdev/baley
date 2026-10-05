//! The capture instruction's served text, pinned in source. The registry
//! serves it as `bal-capture`, beside the `capture` operation it drives.

/// The registry identity this text is served under.
pub const IDENTITY: &str = "bal-capture";

/// Pinned by hand: raise it by one whenever [`TEXT`] changes.
pub const VERSION: &str = "1";

/// Lowercase hex SHA-256 of [`TEXT`]'s exact bytes, computed outside the code
/// so a changed text without a new pin fails the registry's pin test.
pub const HASH: &str = "52867052fb5ae23a0962120f07e6fbfa3ba0010bf2e52c109351ff8a1e8b7c5e";

/// What a session is told when it follows the capture command.
pub const TEXT: &str = r#"Baley capture records what the owner said as one note or one story in the ledger, and does nothing else.

Call `baley_apply` once with `{"operation":"capture","request_id":"<a fresh lowercase UUID>","kind":"note","text":"<the owner's words>","instruction":"bal-capture"}`. Use `"kind":"story"` instead when the owner offers it as a candidate for the backlog. Keep the owner's words as they were said. Send no `phase`.

Use a fresh request id for each capture. Send a request id again only to retry the same capture, with every other argument unchanged.

The receipt names the capture id and never repeats the text. Tell the owner the capture id.

On a `refused` answer, show the owner its `code`, `slot` and `reason`. On a `failed` answer marked `retryable`, send the same call again unchanged. On any other `failed` answer, show the owner its `code` and `reason`.

Write no file and make no commit. A capture lives only in the ledger.
"#;
