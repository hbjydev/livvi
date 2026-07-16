CREATE TABLE tool_permissions (
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    tool_name TEXT NOT NULL,
    allowed BOOLEAN NOT NULL,
    updated_at DATETIME NOT NULL,
    PRIMARY KEY (conversation_id, tool_name)
);

CREATE INDEX idx_tool_permissions_conversation ON tool_permissions(conversation_id);
