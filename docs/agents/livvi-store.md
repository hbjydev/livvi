# livvi-store

This document describes the `livvi-store` crate and how to work with Livvi's
persistent storage layer.

## What it is

`livvi-store` is the backend-agnostic persistence layer for Livvi. It defines
repository traits for the entities Livvi needs to remember across restarts,
starting with people and conversations. The concrete storage backend lives in a
separate module and implements those traits.

Currently the only backend is SQLite, but the trait design is intentionally
backend-agnostic: if you want PostgreSQL or another store later, you implement
the same traits without touching the callers.

## Workspace layout

```
livvi-store/
  src/
    lib.rs              # LivviStore trait and re-exports
    person.rs           # Person, PersonId, PersonStorage
    conversation.rs     # Conversation, ConversationId, ConversationStorage
    sqlite/mod.rs       # LivviSqliteStore implementation
  migrations/           # sqlx migrations for the SQLite backend
```

## Core traits

### PersonStorage

Handles canonical cross-transport identities. A `Person` is a human or agent that
Livvi can interact with. Each person can have multiple transport identities
(Discord, Bluesky, etc.) linked to them.

Key methods:

- `resolve_identity(transport_kind, transport_id)` — find a person by one of
their transport identities.
- `create_person(display_name, also_known_as, metadata)` — create a new person with no
  identities.
- `add_also_known_as(person_id, name)` — add an alternate display name.
- `ensure_identity(...)` — resolve or create a person + identity atomically.

### ConversationStorage

Handles conversation threads. A `Conversation` is identified by a transport
channel or room, and it tracks which `Person` records participate in it.

Key methods:

- `resolve_conversation(transport_kind, transport_id)` — find a conversation by
its transport identity.
- `create_conversation(...)` — create a new conversation record.
- `ensure_conversation(...)` — resolve or create a conversation.
- `add_participant(...)` / `get_participants(...)` — manage membership.

### LivviStore

`LivviStore` is the umbrella trait. It is automatically implemented for any type
that implements `PersonStorage + ConversationStorage + Send + Sync + 'static`:

```rust
pub trait LivviStore: PersonStorage + ConversationStorage + Send + Sync + 'static {}

impl<T> LivviStore for T where T: PersonStorage + ConversationStorage + Send + Sync + 'static {}
```

This lets you write functions that accept any backend without being generic over
five separate traits.

## Usage

`livvi-store` is available without the SQLite backend by default. Add the
`sqlite` feature when you want the real `LivviSqliteStore`:

```toml
[dependencies]
livvi-store = { path = "../livvi-store" }

# or, for the SQLite backend:
livvi-store = { path = "../livvi-store", features = ["sqlite"] }
```

This lets crates like `livvi-core` depend on the store traits and types without
pulling in `sqlx` and SQLite, while `livvi-daemon` can opt into the concrete
backend.

## Wiring into the agent loop

The storage layer is intentionally separate from the agent loop. The daemon
sits between the transport and the agent and resolves raw transport identities
to canonical `Person` and `Conversation` records.

Current flow in `livvi-daemon`:

```text
Discord message
  -> DiscordTransport sends Interrupt::ExternalEvent (raw transport IDs)
  -> daemon resolver task
       -> ensure_identity("discord", "<author_id>", ...)
       -> ensure_conversation("discord", "<channel_id>", ...)
       -> add_participant(conversation.id, person.id)
       -> sends Interrupt::ExternalEvent with person_id + conversation_id set
  -> Agent loop
       -> pushes an XML-formatted user Message with person_id into the in-memory Context
       (metadata is flattened into `<source>__author__<key>` and
       `<source>__conversation__<key>` attributes)
```

The `Person` and `Conversation` IDs travel on `Message` (via `person_id` and
`conversation_id`) so the transcript is annotated for future context
persistence and memory tools. Actual message history restoration is still out
of scope — only identity and conversation metadata are persisted right now.

See `livvi-daemon/src/main.rs` for the resolver implementation and
`livvi-discord/src/lib.rs` for how raw Discord events are emitted.

## SQLite backend

Use `LivviSqliteStore::connect(url)` to open a database and run migrations. For
tests, use `sqlite::memory:`.

```rust
use livvi_store::{LivviSqliteStore, PersonStorage, ConversationStorage};

let store = LivviSqliteStore::connect("sqlite:livvi.db").await?;

let person = store
    .ensure_identity("discord", "12345", Some("hayden".into()), json!({}))
    .await?;

let conversation = store
    .ensure_conversation("discord", "chan-1", Some("general".into()), json!({}))
    .await?;

store.add_participant(&conversation.id, &person.id).await?;
```

## Mock store for tests

`livvi-store` also provides `MockStore`, an in-memory implementation of the same
traits. Use it in unit tests or any context where you want storage semantics
without a real database:

```rust
use livvi_store::{MockStore, PersonStorage, ConversationStorage};

let store = MockStore::new();

let person = store
    .ensure_identity("discord", "12345", Some("hayden".into()), json!({}))
    .await?;

let conversation = store
    .ensure_conversation("discord", "chan-1", None, json!({}))
    .await?;
```

`MockStore` is `Send` + `Sync` and implements `LivviStore`, so it can be passed
to anything that accepts a generic store.

See also `livvi-store/examples/basic.rs` for a runnable example.

## Schema

Migrations live in `livvi-store/migrations/`. The initial schema creates four
tables:

- `persons` — canonical people.
- `person_identities` — transport identities linked to people.
- `conversations` — transport channels/rooms.
- `conversation_participants` — many-to-many link between conversations and
people.

Metadata columns are stored as JSON text. Stable identifiers and foreign keys
remain normalized columns so they can be indexed and queried.

### Adding new tables

When you add a new entity:

1. Define the model and a repository trait in `livvi-store/src/<entity>.rs`.
2. Add a migration in `livvi-store/migrations/`.
3. Implement the trait for `LivviSqliteStore` in
   `livvi-store/src/sqlite/mod.rs`.
4. Update the `LivviStore` blanket impl bound to include the new trait if it
   should be considered part of the core store surface.

## Design notes

- `livvi-store` does not store message history. That belongs to a context
  persistence layer built on top of these primitives.
- `Person` keeps a canonical `display_name` and an `also_known_as` list of
  alternate names. The daemon resolver adds a new display name to
  `also_known_as` when it differs from the canonical one.
- `Person` is the canonical identity. Transport crates emit raw transport IDs
  in `Interrupt::ExternalEvent`; the daemon resolves them to `PersonId` before
  events reach the agent loop.
- `Conversation` is intentionally transport-agnostic: only `transport_kind` and
  `transport_id` identify the source; everything else lives in `metadata`.

## Build/test

Use the standard Mise tasks:

```bash
mise run build
mise run test
mise run clippy
mise run fmt-check
```
