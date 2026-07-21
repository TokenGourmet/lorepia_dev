DROP INDEX request_state_one_running_per_chat;
DROP INDEX request_state_chat_started;

ALTER TABLE schema_meta RENAME TO schema_meta_v1;
CREATE TABLE schema_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version = 2),
    migrated_at_ms INTEGER NOT NULL CHECK (migrated_at_ms BETWEEN 0 AND 9007199254740991)
) STRICT;
DROP TABLE schema_meta_v1;

ALTER TABLE request_state RENAME TO request_state_v1;
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
    provider_id, model_id, 'legacy-v1', id, status,
    last_seq, last_seq, NULL,
    provider_response_id, input_tokens, output_tokens, cached_input_tokens, reasoning_tokens,
    failure_code, started_at_ms, updated_at_ms, finished_at_ms
FROM request_state_v1;

DROP TABLE request_state_v1;

CREATE UNIQUE INDEX request_state_one_running_per_chat
ON request_state(chat_id)
WHERE status = 'running';

CREATE INDEX request_state_chat_started
ON request_state(chat_id, started_at_ms DESC, id DESC);

CREATE INDEX request_state_owner_running
ON request_state(owner_label, status, started_at_ms, id);

PRAGMA user_version = 2;
