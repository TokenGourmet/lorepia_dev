#![allow(dead_code)]

use std::path::PathBuf;

use lorepia_storage::{
    BeginTurn, CharacterId, Chat, CreateChat, ModelId, ProviderId, ProviderSelection,
    ResponseCheckpoint, StartedTurn, Store, TimestampMillis,
};
use tempfile::TempDir;

pub fn database() -> (TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("lorepia.sqlite3");
    (directory, path)
}

pub fn timestamp(value: i64) -> TimestampMillis {
    TimestampMillis::new(value).expect("valid timestamp")
}

pub fn selection() -> ProviderSelection {
    ProviderSelection {
        provider_id: ProviderId::OpenAi,
        model_id: ModelId::parse("gpt-test").expect("valid model ID"),
    }
}

pub fn create_chat(store: &Store, at_ms: i64) -> Chat {
    store
        .create_chat(CreateChat {
            character_id: CharacterId::parse("character-seraphine").expect("character ID"),
            title: "세라핀과의 대화".to_owned(),
            at_ms: timestamp(at_ms),
        })
        .expect("create chat")
}

pub fn begin_turn(store: &Store, chat: &Chat, text: &str, at_ms: i64) -> StartedTurn {
    store
        .begin_turn(BeginTurn {
            chat_id: chat.id.clone(),
            selection: selection(),
            user_text: text.to_owned(),
            started_at_ms: timestamp(at_ms),
        })
        .expect("begin turn")
}

pub fn checkpoint(
    started: &StartedTurn,
    expected_last_seq: u64,
    through_seq: u64,
    text: &str,
    at_ms: i64,
) -> ResponseCheckpoint {
    ResponseCheckpoint {
        request_state_id: started.request_state_id.clone(),
        expected_last_seq,
        through_seq,
        appended_text: text.to_owned(),
        provider_response_id: None,
        usage: None,
        at_ms: timestamp(at_ms),
    }
}
