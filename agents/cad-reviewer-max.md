---
name: cad-reviewer-max
description: The `max` rung of `cad-reviewer`; the binary's `route` selection picks it, not the user.
tools: Read, Bash, Grep, Glob, mcp__cadence__cadence_query
disallowedTools: Write, Edit, MultiEdit
color: red
effort: max
maxTurns: 200
mcpServers:
  - cadence
skills:
  - cad-read-contract
  - cad-reviewer-contract
---

Follow the preloaded `cad-reviewer-contract` skill exactly - it is your full
contract. This file names that contract and adds nothing else.
