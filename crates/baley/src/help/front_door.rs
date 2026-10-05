//! The help front door's served text, pinned in source. The registry in
//! `crate::instruction` serves it as `bal-help`. The help area owns the words,
//! so a change to help changes this file and its pin together.

/// The registry identity this text is served under.
pub const IDENTITY: &str = "bal-help";

/// Pinned by hand: raise it by one whenever [`TEXT`] changes.
pub const VERSION: &str = "1";

/// Lowercase hex SHA-256 of [`TEXT`]'s exact bytes, computed outside the code
/// so a changed text without a new pin fails the registry's pin test.
pub const HASH: &str = "6a546a62197f3070cdfa4725732e5f8e3c00d21addf3b77696459c0dcb469cad";

/// What a session is told when it follows the help command.
pub const TEXT: &str = r#"Baley help lists the commands Baley offers and says what each one does.

With no command name, call `baley_query` once with `{"operation":"help"}`. Present every cluster in the order given, and under it each command with its name and description, whether it is available, and the build that owns it. Say plainly that a command that is not available cannot be used until its owning build.

With a command name, call `baley_query` with `{"operation":"help","name":"<command name>"}`. One optional leading slash and one optional `bal-` prefix select the same command: debug, bal-debug and /bal-debug are one command. Present that single row. When no row matches, show the closest names in the order given. Do not invent a command, and do not treat a close name as an exact match.

When an answer carries a `next` that is not null, call again with the same arguments and `part` set to that value, until `next` is null. Join the `body` values in order.

Read nothing else, search nothing and write nothing. Help reads only what Baley has compiled.

Send `bal-help`, the identity of this instruction, as `instruction` on every `baley_apply` call you make while following it. Never send it on a `baley_query` call.
"#;
