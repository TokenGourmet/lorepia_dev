DROP TRIGGER messages_fts_ai;
DROP TRIGGER messages_fts_ad;
DROP TRIGGER messages_fts_au;
DROP TABLE messages_fts;

DROP INDEX request_state_one_running_per_chat;
DROP INDEX request_state_chat_started;
DROP INDEX request_state_owner_running;

ALTER TABLE schema_meta RENAME TO schema_meta_v2;
CREATE TABLE schema_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version = 3),
    migrated_at_ms INTEGER NOT NULL CHECK (migrated_at_ms BETWEEN 0 AND 9007199254740991)
) STRICT;
DROP TABLE schema_meta_v2;

ALTER TABLE request_state RENAME TO request_state_v2;
ALTER TABLE messages RENAME TO messages_v2;

CREATE TABLE messages (
    row_id INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    chat_id TEXT NOT NULL,
    parent_id TEXT,
    sibling_ord INTEGER NOT NULL CHECK (sibling_ord BETWEEN 1 AND 9007199254740991),
    depth INTEGER NOT NULL CHECK (depth BETWEEN 0 AND 9007199254740991),
    ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 1 AND 9007199254740991),
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    status TEXT NOT NULL CHECK (status IN ('complete', 'partial', 'failed')),
    text TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
    updated_at_ms INTEGER NOT NULL CHECK (
        updated_at_ms BETWEEN created_at_ms AND 9007199254740991
    ),
    completed_at_ms INTEGER CHECK (
        completed_at_ms IS NULL OR completed_at_ms BETWEEN created_at_ms AND 9007199254740991
    ),
    UNIQUE (chat_id, ordinal),
    UNIQUE (chat_id, id),
    FOREIGN KEY (chat_id) REFERENCES chats(id) ON DELETE CASCADE,
    FOREIGN KEY (chat_id, parent_id) REFERENCES messages(chat_id, id) ON DELETE CASCADE,
    CHECK (length(id) = 32 AND id NOT GLOB '*[^0-9a-f]*'),
    CHECK (parent_id IS NULL OR (length(parent_id) = 32 AND parent_id NOT GLOB '*[^0-9a-f]*')),
    CHECK (length(CAST(text AS BLOB)) <= 1048576),
    CHECK ((role = 'user' AND status = 'complete') OR role = 'assistant'),
    CHECK (
        (status = 'complete' AND completed_at_ms IS NOT NULL) OR
        (status IN ('partial', 'failed') AND completed_at_ms IS NULL)
    ),
    CHECK (
        (parent_id IS NULL AND depth = 0) OR
        (parent_id IS NOT NULL AND depth >= 1)
    )
) STRICT;

INSERT INTO messages(
    row_id, id, chat_id, parent_id, sibling_ord, depth, ordinal,
    role, status, text, created_at_ms, updated_at_ms, completed_at_ms
)
SELECT
    current.row_id,
    current.id,
    current.chat_id,
    previous.id,
    1,
    current.ordinal - 1,
    current.ordinal,
    current.role,
    current.status,
    current.text,
    current.created_at_ms,
    current.updated_at_ms,
    CASE WHEN current.status = 'complete' THEN current.updated_at_ms ELSE NULL END
FROM messages_v2 AS current
LEFT JOIN messages_v2 AS previous
  ON previous.chat_id = current.chat_id
 AND previous.ordinal = current.ordinal - 1;

CREATE TABLE request_state (
    row_id INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    chat_id TEXT NOT NULL,
    user_message_id TEXT NOT NULL,
    assistant_message_id TEXT NOT NULL,
    provider_id TEXT NOT NULL CHECK (
        provider_id IN ('openai', 'anthropic', 'deepseek', 'ollama_cloud', 'gemini', 'vertex_ai')
    ),
    model_id TEXT NOT NULL,
    owner_label TEXT NOT NULL,
    stream_generation TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (
        status IN ('running', 'completed', 'cancelled', 'failed', 'interrupted')
    ),
    last_delivered_seq INTEGER NOT NULL CHECK (
        last_delivered_seq BETWEEN 0 AND 9007199254740991
    ),
    last_durable_seq INTEGER NOT NULL CHECK (
        last_durable_seq BETWEEN 0 AND 9007199254740991
    ),
    last_acked_seq INTEGER CHECK (
        last_acked_seq IS NULL OR last_acked_seq BETWEEN 1 AND 9007199254740991
    ),
    provider_response_id TEXT,
    input_tokens INTEGER CHECK (input_tokens IS NULL OR input_tokens BETWEEN 0 AND 9007199254740991),
    output_tokens INTEGER CHECK (output_tokens IS NULL OR output_tokens BETWEEN 0 AND 9007199254740991),
    cached_input_tokens INTEGER CHECK (
        cached_input_tokens IS NULL OR cached_input_tokens BETWEEN 0 AND 9007199254740991
    ),
    reasoning_tokens INTEGER CHECK (
        reasoning_tokens IS NULL OR reasoning_tokens BETWEEN 0 AND 9007199254740991
    ),
    failure_code TEXT CHECK (failure_code IS NULL OR failure_code IN (
        'NETWORK_UNAVAILABLE',
        'AUTHENTICATION_FAILED',
        'RATE_LIMITED',
        'PROVIDER_REJECTED',
        'TIMEOUT',
        'PROTOCOL_VIOLATION',
        'RESPONSE_TOO_LARGE',
        'INTERNAL',
        'APP_RESTARTED'
    )),
    started_at_ms INTEGER NOT NULL CHECK (started_at_ms BETWEEN 0 AND 9007199254740991),
    updated_at_ms INTEGER NOT NULL CHECK (
        updated_at_ms BETWEEN started_at_ms AND 9007199254740991
    ),
    finished_at_ms INTEGER CHECK (
        finished_at_ms IS NULL OR finished_at_ms BETWEEN started_at_ms AND 9007199254740991
    ),
    FOREIGN KEY (chat_id) REFERENCES chats(id) ON DELETE CASCADE,
    FOREIGN KEY (chat_id, user_message_id) REFERENCES messages(chat_id, id) ON DELETE CASCADE,
    FOREIGN KEY (chat_id, assistant_message_id) REFERENCES messages(chat_id, id) ON DELETE CASCADE,
    CHECK (length(id) = 32 AND id NOT GLOB '*[^0-9a-f]*'),
    CHECK (
        length(CAST(owner_label AS BLOB)) BETWEEN 1 AND 128 AND
        owner_label NOT GLOB '*[^A-Za-z0-9_:/-]*'
    ),
    CHECK (
        length(stream_generation) = 32 AND stream_generation NOT GLOB '*[^0-9a-f]*'
    ),
    CHECK (length(CAST(model_id AS BLOB)) BETWEEN 1 AND 256),
    CHECK (provider_response_id IS NULL OR length(CAST(provider_response_id AS BLOB)) BETWEEN 1 AND 256),
    CHECK (
        (last_acked_seq IS NULL OR last_acked_seq <= last_durable_seq) AND
        last_durable_seq <= last_delivered_seq
    ),
    CHECK (
        (status = 'running' AND finished_at_ms IS NULL AND failure_code IS NULL) OR
        (status IN ('completed', 'cancelled') AND finished_at_ms IS NOT NULL AND failure_code IS NULL) OR
        (status = 'failed' AND finished_at_ms IS NOT NULL AND failure_code IS NOT NULL AND failure_code != 'APP_RESTARTED') OR
        (status = 'interrupted' AND finished_at_ms IS NOT NULL AND failure_code = 'APP_RESTARTED')
    )
) STRICT;

INSERT INTO request_state(
    row_id, id, chat_id, user_message_id, assistant_message_id,
    provider_id, model_id, owner_label, stream_generation, status,
    last_delivered_seq, last_durable_seq, last_acked_seq,
    provider_response_id, input_tokens, output_tokens, cached_input_tokens, reasoning_tokens,
    failure_code, started_at_ms, updated_at_ms, finished_at_ms
)
SELECT
    row_id, id, chat_id, user_message_id, assistant_message_id,
    provider_id, model_id, owner_label, stream_generation, status,
    last_delivered_seq, last_durable_seq, last_acked_seq,
    provider_response_id, input_tokens, output_tokens, cached_input_tokens, reasoning_tokens,
    failure_code, started_at_ms, updated_at_ms, finished_at_ms
FROM request_state_v2;

DROP TABLE request_state_v2;
DROP TABLE messages_v2;

CREATE UNIQUE INDEX request_state_one_running_per_chat
ON request_state(chat_id)
WHERE status = 'running';

CREATE INDEX request_state_chat_started
ON request_state(chat_id, started_at_ms DESC, id DESC);

CREATE INDEX request_state_owner_running
ON request_state(owner_label, status, started_at_ms, id);

CREATE UNIQUE INDEX messages_unique_child_sibling
ON messages(chat_id, parent_id, sibling_ord)
WHERE parent_id IS NOT NULL;

CREATE UNIQUE INDEX messages_unique_root_sibling
ON messages(chat_id, sibling_ord)
WHERE parent_id IS NULL;

CREATE INDEX messages_chat_parent_sibling
ON messages(chat_id, parent_id, sibling_ord, id);

CREATE INDEX messages_chat_created
ON messages(chat_id, created_at_ms, id);

CREATE TRIGGER messages_parent_bi
BEFORE INSERT ON messages
WHEN new.parent_id IS NOT NULL
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM messages AS parent
        WHERE parent.chat_id = new.chat_id
          AND parent.id = new.parent_id
          AND parent.depth + 1 = new.depth
    ) THEN RAISE(ABORT, 'invalid message parent') END;
END;

CREATE TRIGGER messages_parent_bu
BEFORE UPDATE OF chat_id, parent_id, depth ON messages
WHEN new.parent_id IS NOT NULL
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM messages AS parent
        WHERE parent.chat_id = new.chat_id
          AND parent.id = new.parent_id
          AND parent.depth + 1 = new.depth
    ) THEN RAISE(ABORT, 'invalid message parent') END;
END;

CREATE TABLE active_path (
    chat_id TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position BETWEEN 0 AND 9007199254740991),
    message_id TEXT NOT NULL,
    PRIMARY KEY (chat_id, position),
    UNIQUE (chat_id, message_id),
    FOREIGN KEY (chat_id) REFERENCES chats(id) ON DELETE CASCADE,
    FOREIGN KEY (chat_id, message_id) REFERENCES messages(chat_id, id) ON DELETE CASCADE
) WITHOUT ROWID, STRICT;

INSERT INTO active_path(chat_id, position, message_id)
SELECT chat_id, depth, id
FROM messages
ORDER BY chat_id, depth;

CREATE TABLE message_render_cache (
    message_id TEXT PRIMARY KEY,
    renderer_ver INTEGER NOT NULL CHECK (renderer_ver BETWEEN 1 AND 9007199254740991),
    html TEXT NOT NULL CHECK (length(CAST(html AS BLOB)) <= 2097152),
    last_used_at_ms INTEGER NOT NULL CHECK (last_used_at_ms BETWEEN 0 AND 9007199254740991),
    FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE
) WITHOUT ROWID, STRICT;

CREATE INDEX message_render_cache_lru
ON message_render_cache(last_used_at_ms, message_id);

CREATE VIRTUAL TABLE messages_fts USING fts5(
    text,
    content = 'messages',
    content_rowid = 'row_id',
    tokenize = 'trigram'
);

INSERT INTO messages_fts(rowid, text)
SELECT row_id, text FROM messages WHERE status = 'complete';

CREATE TRIGGER messages_fts_ai
AFTER INSERT ON messages
WHEN new.status = 'complete'
BEGIN
    INSERT INTO messages_fts(rowid, text) VALUES (new.row_id, new.text);
END;

CREATE TRIGGER messages_fts_ad
AFTER DELETE ON messages
WHEN old.status = 'complete'
BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, text)
    VALUES ('delete', old.row_id, old.text);
END;

CREATE TRIGGER messages_fts_au_delete
AFTER UPDATE OF text, status ON messages
WHEN old.status = 'complete'
BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, text)
    VALUES ('delete', old.row_id, old.text);
END;

CREATE TRIGGER messages_fts_au_insert
AFTER UPDATE OF text, status ON messages
WHEN new.status = 'complete'
BEGIN
    INSERT INTO messages_fts(rowid, text) VALUES (new.row_id, new.text);
END;

PRAGMA user_version = 3;
