# Memini Memory Integration Plan

## Summary

Integrate [Memini](https://github.com/eleboucher/memini) as Livvi's persistent memory backend. The integration is built around a backend-agnostic `MemoryProvider` trait in `livvi-core`, with Memini implemented as the first provider in a new `livvi-memini` crate. The model can explicitly recall and remember via tools, and the agent automatically captures turns as episodic memory and injects a conversation-start briefing.

This plan has been updated to account for the merged `feature/lossless-context` PR (`livvi-lcm` crate, per-conversation LRU contexts, async `Compactor`, `Context::conversation_id`, and message IDs).

## Decisions

| Decision | Choice | Rationale |
| --- | --- | --- |
| Namespace scoping | **Hierarchical** | Conversation-scoped memory is primary; person and global/agent memories cascade in via Memini's `scope=full`. Fits a social agent. |
| Integration style | **Tools + automatic** | Model-driven `memory_recall`/`memory_remember` tools plus automatic turn capture and briefing. |
| Trait scope | **Full** | `remember`, `recall`, `briefing`, `get`, `list`, `forget`, `update` — matches Memini's surface and gives the trait a useful, complete shape. |

## 1. Core abstraction: `MemoryProvider` in `livvi-core`

New module: `livvi-core/src/memory/mod.rs`, re-exported from `livvi-core/src/lib.rs`.

### Types

- `MemoryContext` — namespace, optional home namespace, optional `PersonId`/`ConversationId` for provenance metadata. Includes a helper `from_tool_context(base, &context::Context)` that derives the conversation and person from the current conversation context.
- `RememberRequest`, `RecallRequest`, `BriefingRequest`, `UpdateRequest`, `ListRequest` — request structs mirroring Memini's request fields but backend-agnostic. Date-time fields are strings to avoid schema/serde complexity.
- `Memory`, `ScoredMemory`, `Briefing`, `BriefingItem`, `BriefingChild` — response structs.
- `Tier` (`working`, `episodic`, `semantic`, `procedural`), `Level` (`explicit`, `deduced`), `Scope` (`project`, `full`, `everywhere`) — small enums with `Display`, `FromStr`, and `JsonSchema`.

### Trait

```rust
#[async_trait]
pub trait MemoryProvider: Send + Sync + 'static {
    async fn remember(&self, ctx: MemoryContext, request: RememberRequest) -> Result<Memory>;
    async fn recall(&self, ctx: MemoryContext, request: RecallRequest) -> Result<Vec<ScoredMemory>>;
    async fn briefing(&self, ctx: MemoryContext, request: BriefingRequest) -> Result<Briefing>;
    async fn get(&self, ctx: MemoryContext, id: &str) -> Result<Option<Memory>>;
    async fn list(&self, ctx: MemoryContext, request: ListRequest) -> Result<Vec<Memory>>;
    async fn forget(&self, ctx: MemoryContext, id: &str) -> Result<()>;
    async fn update(&self, ctx: MemoryContext, request: UpdateRequest) -> Result<Memory>;

    fn clone_dyn(&self) -> Box<dyn MemoryProvider>;
}
```

`update` is mapped to Memini's upsert behaviour (a `remember` call with an `id`).

`MemoryProvider` is also implemented for `Box<dyn MemoryProvider>` so that a boxed trait object can be passed to `AgentBuilder::with_memory_provider`.

## 2. Memini implementation: `livvi-memini` crate

New workspace member. Add `livvi-memini` to the root `Cargo.toml` workspace members and as a dependency of `livvi-daemon`.

### Files

- `livvi-memini/Cargo.toml` — depends on `livvi-core`, `reqwest` (with `json`), `serde`, `serde_json`, `anyhow`, `async-trait`, `tokio`, `schemars`, `time`, `tracing`.
- `src/lib.rs` — re-exports.
- `src/client.rs` — `MeminiClient` wrapping `reqwest::Client`. Handles:
  - `MEMINI_BASE_URL` / `MEMINI_API_KEY` (or constructor args).
  - `Authorization: Bearer <key>`.
  - `X-Memini-Namespace` and `X-Memini-Home` headers per request.
  - REST endpoints: `POST /v1/memories`, `POST /v1/search`, `GET /v1/namespaces/briefing`, `GET /v1/memories`, `GET /v1/memories/{id}`, `DELETE /v1/memories/{id}`.
- `src/memory.rs` — `MeminiMemoryProvider` implements `MemoryProvider` using `MeminiClient`. `update` is a `remember` call with the request's `id`.
- `src/tools/mod.rs` — `#[tool]` functions:
  - `memory_recall`
  - `memory_remember`
  - `memory_briefing`
  - `memory_get`
  - `memory_list`
  - `memory_update`
  - `memory_forget`

Tool inputs are derived with `schemars::JsonSchema`. Tools extract the provider via `State<'_, dyn MemoryProvider>` so the agent state only needs to implement `AsRef<dyn MemoryProvider>`. They also extract `Context<'_>` from the tool call to build a `MemoryContext`.

## 3. Agent integration in `livvi-core`

Modify `livvi-core/src/agent/mod.rs`, `livvi-core/src/agent/turns.rs`, and `livvi-core/src/agent/interrupts.rs`.

- Add `memory_provider: Option<Box<dyn MemoryProvider>>` and `memory_namespace: String` to `AgentBuilder` and `Agent`.
- Add `with_memory_provider(impl MemoryProvider)` and `with_memory_namespace(impl Into<String>)` to `AgentBuilder`.
- Update `handle_interrupt` and `run_turn` to carry the base memory namespace.
- If a memory provider is configured:
  - On the first turn of a conversation (`context.turns` is empty before the user message is pushed), call `briefing` with `scope: Scope::Full` and inject the pinned/facts/procedures into the context as additional system messages.
  - After each turn completes, serialize the user message and final assistant response and call `remember` with `tier: Tier::Episodic`, tags including `livvi_turn`, and metadata capturing `person_id`, `conversation_id`, and `source: livvi_turn_capture`.

Automatic turn capture should be careful not to include the memory tools themselves in a circular way; capture the final assistant/user exchange before the memory tool call results are finalised, and tag the captured memory with `source: livvi_turn_capture` so it can be excluded from recall if needed.

## 4. Daemon wiring in `livvi-daemon`

Modify `livvi-daemon/src/main.rs` and `livvi-daemon/Cargo.toml`.

- Read env vars:
  - `LIVVI_MEMINI_BASE_URL`
  - `LIVVI_MEMINI_API_KEY`
  - `LIVVI_MEMINI_NAMESPACE` (default: `livvi`)
- If `LIVVI_MEMINI_BASE_URL` is set, build `MeminiMemoryProvider`.
- Define an `AppState` struct:

  ```rust
  pub struct AppState {
      pub discord: DiscordState,
      pub memory: Arc<dyn MemoryProvider>,
  }

  impl AsRef<DiscordState> for AppState { ... }
  impl AsRef<dyn MemoryProvider> for AppState { ... }
  ```

- Change the agent state from `Arc<DiscordState>` to `AppState`.
- Add memory tools to the toolbox when the provider is configured.
- Pass a `clone_dyn()` of the provider to `AgentBuilder::with_memory_provider` and set the memory namespace.

## 5. Namespace mapping

Derived from the resolved `PersonId` and `ConversationId` for every memory operation.

| Scope | Memini namespace | Header |
| --- | --- | --- |
| Conversation (primary) | `livvi/conversations/<conversation-id>` | `X-Memini-Namespace` |
| Person (home) | `livvi/persons/<person-id>` | `X-Memini-Home` |
| Agent/global | `livvi` | ancestor of conversation namespace |

Recall/briefing use `Scope::Full`, so:
- The current conversation namespace is searched with all tiers (episodic, working, semantic, procedural).
- Ancestors (`livvi/conversations`, `livvi`) and the home namespace (`livvi/persons/<person-id>`) are searched with durable tiers only (`semantic`, `procedural`).

`memory_remember` defaults to `visibility: project` (write to the conversation namespace). For durable facts about the person, the model can use `visibility: personal`, which writes to the home namespace. For durable facts about the whole agent, the model can use an ancestor visibility like `livvi`.

## 6. Testing and verification

- Unit tests in `livvi-memini` for request serialization and response parsing using `wiremock` or a small local HTTP server.
- Update/add tests in `livvi-core` for the `AgentBuilder` memory provider plumbing and first-turn briefing behaviour.
- Run `mise run build`, `mise run test`, `mise run clippy`, `mise run fmt-check`.
- Smoke test against the live Memini instance to validate the cascade:
  1. Write a `semantic` memory to `livvi/persons/<person-id>`.
  2. Write an `episodic` memory to `livvi/conversations/<conversation-id>`.
  3. Recall from `livvi/conversations/<conversation-id>` with `X-Memini-Home: livvi/persons/<person-id>` and `scope=full`.
  4. Expect: the conversation episodic memory is returned, and the person semantic memory is returned via the home leg. The person episodic memory is not returned because home is durable-only.

## 7. Open questions / blockers

- Need the Memini base URL and API key for the smoke test. Implementation can proceed without it, but live validation requires credentials.
- Confirm the running Memini instance has `MEMINI_CASCADE=true` (the default) so the ancestor/home cascade is active.
