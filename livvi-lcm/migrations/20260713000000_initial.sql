CREATE TABLE IF NOT EXISTS lcm_messages (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    role TEXT NOT NULL,
    content TEXT,
    person_id TEXT,
    tool_calls_json TEXT,
    tool_call_id TEXT,
    thinking_content TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_lcm_messages_conversation_sequence
    ON lcm_messages (conversation_id, sequence);

CREATE TABLE IF NOT EXISTS lcm_summaries (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL,
    depth INTEGER NOT NULL,
    content TEXT NOT NULL,
    source_ids_json TEXT NOT NULL,
    parent_id TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_lcm_summaries_conversation_depth
    ON lcm_summaries (conversation_id, depth);
