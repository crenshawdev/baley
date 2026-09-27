//! Compiled query-only help front door.
pub fn markdown() -> String {
    super::table::render_description(
        "bal-help",
        r#"---
name: bal-help
description: ""
argument-hint: "[command name]"
allowed-tools:
  - mcp__baley__baley_query
---

Call `mcp__baley__baley_query` once with `{"operation":"help"}` when no
name is supplied. Present every returned cluster in order, with each command's
name and compiled description.

With a command name, call `{"operation":"help","name":"<command name>"}`.
One optional leading slash and one optional bal- prefix are accepted: debug,
bal-debug and /bal-debug select the same command. Present the single row.
If no row matches, show the returned closest names in their supplied order;
do not invent a command or treat the suggestions as an exact match.

Help reads only the compiled command table. Read nothing else: no project
files, command reference, search, or state. Help writes nothing.
"#,
    )
    .expect("compiled help front matter")
}
