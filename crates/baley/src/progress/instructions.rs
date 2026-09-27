pub fn markdown() -> &'static str {
    static MARKDOWN: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        crate::help::table::render_description("bal-progress", r#"---
name: bal-progress
description: <compiled>
allowed-tools:
  - mcp__baley__baley_query
---

Call `mcp__baley__baley_query` once with `{"operation":"progress"}`.
Print the returned `text` unchanged. If the query is refused, show its exact
code and located reason. The binary derives the answer; do not reconstruct it.
"#).expect("compiled skill front matter")
    });
    &MARKDOWN
}
