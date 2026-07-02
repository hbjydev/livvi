# Agent Instructions

Guidance for AI agents (and humans) working in this repository.

## What this is

**Livvi** is an AI agent harness built to power _social_ agents, as opposed to
coding agents. It borrows primitives from coding agents (compaction, tools),
but is tuned to allow a much more conversational, interactive interface with
human users (as well as _other_ agents) than traditional coding agents allow
for.

**Source of truth for how Livvi behaves today is the code, not documentation.**
While effort should be made to keep documentation up-to-date, how the code
works (and should work) is driven by the code itself. Use documentation if you
need to figure out _why_ a decision was made as additional context, but don't
use it as a check for how the code is meant to work necessarily.

## Workspace layout

```
livvi-core/        The core logic & interfaces crate
livvi-core-macros/ Proc-macro helpers for livvi-core (e.g. ToolSchema derive)
livvi-discord/     The Discord transport implementation
livvi-openai/      The OpenAI LLM provider implementation
livvi-daemon/      The shipped binary, which runs the various components of Livvi
                   (provider, agent loop from livvi-core, etc)
```

## Locked technical decisions

| Concern   | Choice                                                               |
| --------- | -------------------------------------------------------------------- |
| Tools     | Implemented as traits, to allow ease of implementation & maintenance |
| Providers | Implemented as traits, to allow ease of implementation & maintenance |


## Build/test/verify

Tasks live in `.mise/config.toml`; `mise tasks` lists them, `mise run <task>`
runs them. Mise is mandatory and pins the Rust toolchain plus required
components.

```bash
mise run build
mise run test
mise run clippy
mise run fmt-check
```
