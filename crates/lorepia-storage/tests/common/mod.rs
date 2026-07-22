#![allow(dead_code)]

use std::path::PathBuf;

use lorepia_storage::{
    BeginTurn, CharacterId, Chat, CreateChat, DeliveryCheckpoint, ModelId, ProviderId,
    ProviderSelection, ResponseCheckpoint, StartedTurn, Store, StreamGeneration, StreamOwnerLabel,
    TimestampMillis,
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
            owner_label: StreamOwnerLabel::parse("main").expect("owner label"),
            stream_generation: StreamGeneration::new(),
            user_text: text.to_owned(),
            started_at_ms: timestamp(at_ms),
        })
        .expect("begin turn")
}

pub fn checkpoint(
    started: &StartedTurn,
    expected_last_durable_seq: u64,
    through_seq: u64,
    text: &str,
    at_ms: i64,
) -> ResponseCheckpoint {
    ResponseCheckpoint {
        request_state_id: started.request_state_id.clone(),
        owner_label: started.owner_label.clone(),
        stream_generation: started.stream_generation.clone(),
        expected_last_durable_seq,
        through_seq,
        appended_text: text.to_owned(),
        provider_response_id: None,
        usage: None,
        at_ms: timestamp(at_ms),
    }
}

pub fn deliver_through(
    store: &Store,
    started: &StartedTurn,
    from_seq: u64,
    through_seq: u64,
    at_ms: i64,
) {
    for expected in from_seq..through_seq {
        store
            .record_response_delivery(DeliveryCheckpoint {
                request_state_id: started.request_state_id.clone(),
                owner_label: started.owner_label.clone(),
                stream_generation: started.stream_generation.clone(),
                expected_last_delivered_seq: expected,
                through_seq: expected + 1,
                at_ms: timestamp(at_ms),
            })
            .expect("record contiguous delivery");
    }
}
