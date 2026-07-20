CREATE TABLE schema_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    migrated_at_ms INTEGER NOT NULL CHECK (migrated_at_ms BETWEEN 0 AND 9007199254740991)
) STRICT;

CREATE TABLE chats (
    row_id INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    character_id TEXT NOT NULL,
    title TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
    updated_at_ms INTEGER NOT NULL CHECK (
        updated_at_ms BETWEEN created_at_ms AND 9007199254740991
    ),
    CHECK (length(id) = 32 AND id NOT GLOB '*[^0-9a-f]*'),
    CHECK (length(CAST(character_id AS BLOB)) BETWEEN 1 AND 128),
    CHECK (length(CAST(title AS BLOB)) <= 1024)
) STRICT;

CREATE TABLE messages (
    row_id INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    chat_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 1 AND 9007199254740991),
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    status TEXT NOT NULL CHECK (status IN ('complete', 'partial', 'failed')),
    text TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
    updated_at_ms INTEGER NOT NULL CHECK (
        updated_at_ms BETWEEN created_at_ms AND 9007199254740991
    ),
    UNIQUE (chat_id, ordinal),
    UNIQUE (chat_id, id),
    FOREIGN KEY (chat_id) REFERENCES chats(id) ON DELETE CASCADE,
    CHECK (length(id) = 32 AND id NOT GLOB '*[^0-9a-f]*'),
    CHECK (length(CAST(text AS BLOB)) <= 1048576),
    CHECK ((role = 'user' AND status = 'complete') OR role = 'assistant')
) STRICT;

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
    status TEXT NOT NULL CHECK (
        status IN ('running', 'completed', 'cancelled', 'failed', 'interrupted')
    ),
    last_seq INTEGER NOT NULL CHECK (last_seq BETWEEN 0 AND 9007199254740991),
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
    CHECK (length(CAST(model_id AS BLOB)) BETWEEN 1 AND 256),
    CHECK (provider_response_id IS NULL OR length(CAST(provider_response_id AS BLOB)) BETWEEN 1 AND 256),
    CHECK (
        (status = 'running' AND finished_at_ms IS NULL AND failure_code IS NULL) OR
        (status IN ('completed', 'cancelled') AND finished_at_ms IS NOT NULL AND failure_code IS NULL) OR
        (status = 'failed' AND finished_at_ms IS NOT NULL AND failure_code IS NOT NULL AND failure_code != 'APP_RESTARTED') OR
        (status = 'interrupted' AND finished_at_ms IS NOT NULL AND failure_code = 'APP_RESTARTED')
    )
) STRICT;

CREATE UNIQUE INDEX request_state_one_running_per_chat
ON request_state(chat_id)
WHERE status = 'running';

CREATE INDEX request_state_chat_started
ON request_state(chat_id, started_at_ms DESC, id DESC);

CREATE TABLE settings (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    selected_provider_id TEXT NOT NULL CHECK (
        selected_provider_id IN ('openai', 'anthropic', 'deepseek', 'ollama_cloud', 'gemini', 'vertex_ai')
    ),
    openai_model_id TEXT NOT NULL,
    anthropic_model_id TEXT NOT NULL,
    deepseek_model_id TEXT NOT NULL,
    ollama_cloud_model_id TEXT NOT NULL,
    gemini_model_id TEXT NOT NULL,
    theme TEXT NOT NULL CHECK (theme IN ('system', 'light', 'dark')),
    default_mode TEXT NOT NULL CHECK (default_mode IN ('chat', 'story')),
    revision INTEGER NOT NULL CHECK (revision BETWEEN 0 AND 9007199254740991),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
    CHECK (length(CAST(openai_model_id AS BLOB)) <= 256),
    CHECK (length(CAST(anthropic_model_id AS BLOB)) <= 256),
    CHECK (length(CAST(deepseek_model_id AS BLOB)) <= 256),
    CHECK (length(CAST(ollama_cloud_model_id AS BLOB)) <= 256),
    CHECK (length(CAST(gemini_model_id AS BLOB)) <= 256)
) STRICT;

INSERT INTO settings(
    singleton, selected_provider_id,
    openai_model_id, anthropic_model_id, deepseek_model_id, ollama_cloud_model_id, gemini_model_id,
    theme, default_mode, revision, updated_at_ms
) VALUES (1, 'openai', '', '', '', '', '', 'system', 'chat', 0, 0);

CREATE VIRTUAL TABLE messages_fts USING fts5(
    text,
    content = 'messages',
    content_rowid = 'row_id',
    tokenize = 'trigram'
);

CREATE TRIGGER messages_fts_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, text) VALUES (new.row_id, new.text);
END;

CREATE TRIGGER messages_fts_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, text)
    VALUES ('delete', old.row_id, old.text);
END;

CREATE TRIGGER messages_fts_au AFTER UPDATE OF text ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, text)
    VALUES ('delete', old.row_id, old.text);
    INSERT INTO messages_fts(rowid, text) VALUES (new.row_id, new.text);
END;

PRAGMA user_version = 1;
