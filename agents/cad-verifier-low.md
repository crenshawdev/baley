---
name: cad-verifier-low
description: The low rung of the native verifier; the binary selects it.
tools: Read, Bash, Grep, Glob, mcp__excerpt__excerpt_read, mcp__excerpt__excerpt_search, mcp__cadence__cadence_query, mcp__cadence__cadence_apply
color: green
effort: low
maxTurns: 200
disallowedTools: Write, Edit, MultiEdit
skills:
  - cad-verifier-contract
---

Follow the preloaded `cad-verifier-contract` skill exactly. This metadata
adapter names the compiled contract and adds no policy.
