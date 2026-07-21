mod common;

use lorepia_storage::{
    AppendBranchMessage, MessageId, MessageRole, SelectActivePath, StorageError,
};
use rusqlite::{Connection, params};

use common::{create_chat, database, timestamp};

#[test]
fn branch_append_selection_and_keyset_pages_are_cas_guarded() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);

    let root = store
        .append_branch_message(AppendBranchMessage {
            chat_id: chat.id.clone(),
            parent_id: None,
            expected_active_leaf_id: None,
            role: MessageRole::User,
            text: "root".to_owned(),
            at_ms: timestamp(30),
        })
        .expect("append root branch");
    assert_eq!(root.message.parent_id, None);
    assert_eq!(root.message.sibling_ord, 1);
    assert_eq!(root.message.depth, 0);
    assert_eq!(root.message.completed_at_ms, Some(timestamp(30)));

    let first_child = store
        .append_branch_message(AppendBranchMessage {
            chat_id: chat.id.clone(),
            parent_id: Some(root.message.id.clone()),
            expected_active_leaf_id: Some(root.message.id.clone()),
            role: MessageRole::Assistant,
            text: "first child".to_owned(),
            at_ms: timestamp(40),
        })
        .expect("append first child");
    let second_child = store
        .append_branch_message(AppendBranchMessage {
            chat_id: chat.id.clone(),
            parent_id: Some(root.message.id.clone()),
            expected_active_leaf_id: Some(first_child.message.id.clone()),
            role: MessageRole::Assistant,
            text: "second child".to_owned(),
            at_ms: timestamp(50),
        })
        .expect("append sibling reroll");
    assert_eq!(first_child.message.sibling_ord, 1);
    assert_eq!(second_child.message.sibling_ord, 2);
    assert_eq!(second_child.message.depth, 1);

    let stale = store
        .append_branch_message(AppendBranchMessage {
            chat_id: chat.id.clone(),
            parent_id: Some(root.message.id.clone()),
            expected_active_leaf_id: Some(first_child.message.id.clone()),
            role: MessageRole::Assistant,
            text: "stale".to_owned(),
            at_ms: timestamp(60),
        })
        .expect_err("stale active path CAS");
    assert!(matches!(
        stale,
        StorageError::Conflict {
            entity: "active path"
        }
    ));

    let page_one = store
        .load_branch_children(&chat.id, Some(&root.message.id), None, 1)
        .expect("first child page");
    assert_eq!(page_one.messages.len(), 1);
    assert_eq!(page_one.messages[0].id, first_child.message.id);
    let page_two = store
        .load_branch_children(
            &chat.id,
            Some(&root.message.id),
            page_one.next_cursor.as_ref(),
            1,
        )
        .expect("second child page");
    assert_eq!(page_two.messages.len(), 1);
    assert_eq!(page_two.messages[0].id, second_child.message.id);
    assert!(page_two.next_cursor.is_none());

    let recent_second = store
        .load_recent_messages(&chat.id, None, 10)
        .expect("current active branch history");
    assert_eq!(
        recent_second
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect::<Vec<_>>(),
        vec![root.message.id.clone(), second_child.message.id.clone()],
        "inactive sibling must not enter the visible history"
    );
    let stale_branch_cursor =
        lorepia_storage::MessageOrdinalCursor::new(chat.id.clone(), second_child.message.ordinal)
            .expect("second branch cursor");

    let selection = store
        .select_active_path(SelectActivePath {
            chat_id: chat.id.clone(),
            expected_leaf_id: Some(second_child.message.id.clone()),
            leaf_message_id: first_child.message.id.clone(),
            at_ms: timestamp(60),
        })
        .expect("select earlier sibling");
    assert_eq!(selection.path_length, 2);
    let recent_first = store
        .load_recent_messages(&chat.id, None, 1)
        .expect("newest current-branch page");
    assert_eq!(recent_first.messages[0].id, first_child.message.id);
    let older_cursor = recent_first.older_cursor.expect("older path cursor");
    let recent_root = store
        .load_recent_messages(&chat.id, Some(&older_cursor), 1)
        .expect("older current-branch page");
    assert_eq!(recent_root.messages[0].id, root.message.id);
    assert!(recent_root.older_cursor.is_none());
    assert!(
        store
            .load_recent_messages(&chat.id, Some(&stale_branch_cursor), 1)
            .is_err(),
        "a cursor from a no-longer-active sibling must fail closed"
    );
    let path_one = store
        .load_active_path(&chat.id, None, 1)
        .expect("first active path page");
    assert_eq!(path_one.entries[0].message.id, root.message.id);
    let path_two = store
        .load_active_path(&chat.id, path_one.next_position, 1)
        .expect("second active path page");
    assert_eq!(path_two.entries[0].message.id, first_child.message.id);
    assert!(path_two.next_position.is_none());

    let timeline_one = store
        .load_message_timeline(&chat.id, None, 2)
        .expect("first timeline page");
    assert_eq!(timeline_one.messages.len(), 2);
    let timeline_two = store
        .load_message_timeline(&chat.id, timeline_one.next_cursor.as_ref(), 2)
        .expect("second timeline page");
    assert_eq!(timeline_two.messages.len(), 1);
    assert_eq!(timeline_two.messages[0].id, second_child.message.id);
}

#[test]
fn streaming_text_enters_fts_only_when_the_assistant_message_completes() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let started = common::begin_turn(&store, &chat, "question", 30);
    common::deliver_through(&store, &started, 0, 2, 35);
    store
        .checkpoint_response(common::checkpoint(&started, 0, 1, "unindexedpartial", 40))
        .expect("partial checkpoint");
    assert!(
        store
            .search_messages(&chat.id, "unindexedpartial", 10)
            .expect("search while partial")
            .is_empty()
    );

    store
        .complete_turn(common::checkpoint(&started, 1, 2, "terminal", 50))
        .expect("complete assistant message");
    let hits = store
        .search_messages(&chat.id, "unindexedpartialterminal", 10)
        .expect("search completed canonical text");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message.id, started.assistant_message_id);
    assert_eq!(hits[0].message.completed_at_ms, Some(timestamp(50)));
}

#[test]
fn deep_branch_switch_materializes_a_set_based_divergent_suffix() {
    const DEPTH: u64 = 2_048;

    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let mut connection = Connection::open(&path).expect("open seed connection");
    let transaction = connection.transaction().expect("seed transaction");
    {
        let mut insert = transaction
            .prepare(
                "INSERT INTO messages(
                    id, chat_id, parent_id, sibling_ord, depth, ordinal,
                    role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'assistant', 'complete', '', 30, 30, 30)",
            )
            .expect("prepare deep branch seed");
        for depth in 0..DEPTH {
            let id = format!("{value:032x}", value = depth + 1);
            let parent = (depth > 0).then(|| format!("{value:032x}", value = depth));
            insert
                .execute(params![
                    id,
                    chat.id.as_str(),
                    parent,
                    1_i64,
                    i64::try_from(depth).unwrap(),
                    i64::try_from(depth + 1).unwrap(),
                ])
                .expect("insert deep branch message");
        }
        let sibling_id = format!("{value:032x}", value = DEPTH + 1);
        let sibling_parent = format!("{value:032x}", value = DEPTH - 1);
        insert
            .execute(params![
                sibling_id,
                chat.id.as_str(),
                sibling_parent,
                2_i64,
                i64::try_from(DEPTH - 1).unwrap(),
                i64::try_from(DEPTH + 1).unwrap(),
            ])
            .expect("insert deep sibling");
    }
    transaction.commit().expect("commit deep branch seed");
    drop(connection);

    let first_leaf = MessageId::parse(format!("{DEPTH:032x}")).expect("first leaf");
    let first = store
        .select_active_path(SelectActivePath {
            chat_id: chat.id.clone(),
            expected_leaf_id: None,
            leaf_message_id: first_leaf.clone(),
            at_ms: timestamp(40),
        })
        .expect("materialize deep path");
    assert_eq!(first.path_length, DEPTH);

    let sibling_leaf =
        MessageId::parse(format!("{value:032x}", value = DEPTH + 1)).expect("sibling leaf");
    let sibling = store
        .select_active_path(SelectActivePath {
            chat_id: chat.id.clone(),
            expected_leaf_id: Some(first_leaf),
            leaf_message_id: sibling_leaf.clone(),
            at_ms: timestamp(50),
        })
        .expect("switch only the divergent suffix");
    assert_eq!(sibling.path_length, DEPTH);
    assert_eq!(
        store
            .load_recent_messages(&chat.id, None, 1)
            .expect("load selected leaf")
            .messages[0]
            .id,
        sibling_leaf
    );
}

#[test]
fn recent_history_page_is_bounded_by_utf8_bytes_before_ipc() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let large = "x".repeat(lorepia_storage::MAX_MESSAGE_BYTES);
    let root = store
        .append_branch_message(AppendBranchMessage {
            chat_id: chat.id.clone(),
            parent_id: None,
            expected_active_leaf_id: None,
            role: MessageRole::Assistant,
            text: large.clone(),
            at_ms: timestamp(30),
        })
        .expect("append large root");
    let child = store
        .append_branch_message(AppendBranchMessage {
            chat_id: chat.id.clone(),
            parent_id: Some(root.message.id.clone()),
            expected_active_leaf_id: Some(root.message.id.clone()),
            role: MessageRole::Assistant,
            text: large,
            at_ms: timestamp(40),
        })
        .expect("append large child");

    let newest = store
        .load_recent_messages(&chat.id, None, 200)
        .expect("byte bounded newest page");
    assert_eq!(newest.messages.len(), 1);
    assert_eq!(newest.messages[0].id, child.message.id);
    let cursor = newest.older_cursor.expect("older data remains");
    assert_eq!(cursor.ordinal, child.message.ordinal);

    let older = store
        .load_recent_messages(&chat.id, Some(&cursor), 200)
        .expect("older byte bounded page");
    assert_eq!(older.messages.len(), 1);
    assert_eq!(older.messages[0].id, root.message.id);
    assert!(older.older_cursor.is_none());
}
