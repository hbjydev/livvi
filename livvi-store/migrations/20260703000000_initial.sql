CREATE TABLE persons (
    id TEXT PRIMARY KEY,
    display_name TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL
);

CREATE TABLE person_identities (
    person_id TEXT NOT NULL REFERENCES persons(id),
    transport_kind TEXT NOT NULL,
    transport_id TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}',
    linked_at DATETIME NOT NULL,
    PRIMARY KEY (transport_kind, transport_id)
);

CREATE TABLE conversations (
    id TEXT PRIMARY KEY,
    transport_kind TEXT NOT NULL,
    transport_id TEXT NOT NULL,
    title TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at DATETIME NOT NULL,
    last_active_at DATETIME NOT NULL,
    UNIQUE(transport_kind, transport_id)
);

CREATE TABLE conversation_participants (
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    person_id TEXT NOT NULL REFERENCES persons(id),
    joined_at DATETIME NOT NULL,
    PRIMARY KEY (conversation_id, person_id)
);

CREATE INDEX idx_conversations_transport ON conversations(transport_kind, transport_id);
CREATE INDEX idx_conversation_participants_person ON conversation_participants(person_id);
