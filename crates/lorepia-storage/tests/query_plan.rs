mod common;

use lorepia_storage::{MessageId, MessageOrdinalCursor};
use rusqlite::{Connection, params};

use common::{create_chat, database, timestamp};

const ROOT_ID: &str = "ffffffffffffffffffffffffffffffff";
const ROW_COUNT: u64 = 100_000;

#[test]
fn hundred_thousand_row_pages_use_keyset_indexes_and_store_has_no_offset() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);

    let mut connection = Connection::open(&path).expect("open bulk writer");
    connection
        .execute_batch("PRAGMA foreign_keys = ON; PRAGMA wal_autocheckpoint = 0;")
        .expect("configure bulk writer");
    let transaction = connection.transaction().expect("bulk transaction");
    transaction
        .execute(
            "INSERT INTO messages(
                id, chat_id, parent_id, sibling_ord, depth, ordinal,
                role, status, text, created_at_ms, updated_at_ms, completed_at_ms
             ) VALUES (
                ?1, ?2, NULL, 1, 0, 1,
                'user', 'complete', 'root', 30, 30, 30
             )",
            params![ROOT_ID, chat.id.as_str()],
        )
        .expect("insert root");
    transaction
        .execute(
            "INSERT INTO active_path(chat_id, position, message_id) VALUES (?1, 0, ?2)",
            params![chat.id.as_str(), ROOT_ID],
        )
        .expect("select root");
    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO messages(
                    id, chat_id, parent_id, sibling_ord, depth, ordinal,
                    role, status, text, created_at_ms, updated_at_ms, completed_at_ms
                 ) VALUES (
                    ?1, ?2, ?3, ?4, 1, ?5,
                    'assistant', 'failed', '', ?6, ?6, NULL
                 )",
            )
            .expect("prepare child insert");
        for sequence in 1..=ROW_COUNT {
            let message_id = format!("{sequence:032x}");
            let sibling_ord = i64::try_from(sequence).expect("sibling ordinal");
            let ordinal = i64::try_from(sequence + 1).expect("message ordinal");
            let created_at_ms = i64::try_from(sequence + 30).expect("timestamp");
            statement
                .execute(params![
                    message_id,
                    chat.id.as_str(),
                    ROOT_ID,
                    sibling_ord,
                    ordinal,
                    created_at_ms
                ])
                .expect("insert child");
        }
    }
    transaction.commit().expect("commit bulk data");
    connection
        .execute_batch("ANALYZE")
        .expect("analyze indexes");

    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM messages WHERE chat_id = ?1 AND parent_id = ?2",
            params![chat.id.as_str(), ROOT_ID],
            |row| row.get(0),
        )
        .expect("count bulk children");
    assert_eq!(count, ROW_COUNT as i64);

    let branch_plan = query_plan(
        &connection,
        "EXPLAIN QUERY PLAN
         SELECT id, sibling_ord
         FROM messages
         WHERE chat_id = ?1 AND parent_id = ?2
           AND (sibling_ord > ?3 OR (sibling_ord = ?3 AND id > ?4))
         ORDER BY sibling_ord ASC, id ASC
         LIMIT ?5",
        params![chat.id.as_str(), ROOT_ID, 50_000_i64, "", 51_i64],
    );
    assert!(
        branch_plan.contains("messages_chat_parent_sibling"),
        "branch query plan: {branch_plan}"
    );
    assert!(!branch_plan.contains("SCAN messages"));

    let timeline_plan = query_plan(
        &connection,
        "EXPLAIN QUERY PLAN
         SELECT id, created_at_ms
         FROM messages
         WHERE chat_id = ?1
           AND (created_at_ms > ?2 OR (created_at_ms = ?2 AND id > ?3))
         ORDER BY created_at_ms ASC, id ASC
         LIMIT ?4",
        params![chat.id.as_str(), 50_000_i64, "", 51_i64],
    );
    assert!(
        timeline_plan.contains("messages_chat_created"),
        "timeline query plan: {timeline_plan}"
    );
    assert!(!timeline_plan.contains("SCAN messages"));

    let newest_history_plan = query_plan(
        &connection,
        "EXPLAIN QUERY PLAN
         SELECT m.id, m.ordinal
         FROM active_path AS p
         JOIN messages AS m ON m.chat_id = p.chat_id AND m.id = p.message_id
         WHERE p.chat_id = ?1 AND p.position < ?2
         ORDER BY p.position DESC
         LIMIT ?3",
        params![chat.id.as_str(), 50_000_i64, 201_i64],
    );
    assert!(
        newest_history_plan.contains("SEARCH p USING PRIMARY KEY"),
        "newest history query plan: {newest_history_plan}"
    );
    assert!(
        !newest_history_plan.contains("SCAN ") && !newest_history_plan.contains("USE TEMP B-TREE"),
        "newest history must keyset-seek: {newest_history_plan}"
    );

    let root = MessageId::parse(ROOT_ID).expect("root message ID");
    let first_page = store
        .load_branch_children(&chat.id, Some(&root), None, 50)
        .expect("bounded product page");
    assert_eq!(first_page.messages.len(), 50);
    assert!(first_page.next_cursor.is_some());
    let second_page = store
        .load_branch_children(&chat.id, Some(&root), first_page.next_cursor.as_ref(), 50)
        .expect("next bounded product page");
    assert_eq!(second_page.messages.len(), 50);
    assert!(second_page.messages[0].sibling_ord > first_page.messages[49].sibling_ord);

    let selected_child_id = format!("{ROW_COUNT:032x}");
    connection
        .execute(
            "INSERT INTO active_path(chat_id, position, message_id) VALUES (?1, 1, ?2)",
            params![chat.id.as_str(), selected_child_id],
        )
        .expect("select one child among many inactive siblings");

    let newest = store
        .load_recent_messages(&chat.id, None, 1)
        .expect("newest bounded history page");
    assert_eq!(newest.messages.len(), 1);
    assert_eq!(
        newest.messages.last().map(|message| message.ordinal),
        Some(100_001)
    );
    let older_cursor = newest.older_cursor.expect("older history cursor");
    assert_eq!(older_cursor.chat_id, chat.id);
    assert_eq!(older_cursor.ordinal, 100_001);

    // A later inactive sibling must not appear in the current branch window
    // and must not shift the older keyset page.
    connection
        .execute(
            "INSERT INTO messages(
                id, chat_id, parent_id, sibling_ord, depth, ordinal,
                role, status, text, created_at_ms, updated_at_ms, completed_at_ms
             ) VALUES (
                ?1, ?2, ?3, ?4, 1, ?5,
                'assistant', 'failed', '', ?6, ?6, NULL
             )",
            params![
                format!("{:032x}", ROW_COUNT + 1),
                chat.id.as_str(),
                ROOT_ID,
                i64::try_from(ROW_COUNT + 1).expect("new sibling ordinal"),
                i64::try_from(ROW_COUNT + 2).expect("new message ordinal"),
                i64::try_from(ROW_COUNT + 31).expect("new timestamp"),
            ],
        )
        .expect("insert inactive sibling after first page");

    let older = store
        .load_recent_messages(&chat.id, Some(&older_cursor), 1)
        .expect("older bounded history page");
    assert_eq!(older.messages.len(), 1);
    assert_eq!(older.messages[0].ordinal, 1);
    assert!(older.older_cursor.is_none());

    let zero_cursor = MessageOrdinalCursor {
        chat_id: chat.id.clone(),
        ordinal: 0,
    };
    assert!(
        store
            .load_recent_messages(&chat.id, Some(&zero_cursor), 50)
            .is_err(),
        "a forged zero cursor must fail closed"
    );
    let cross_chat_cursor = MessageOrdinalCursor::new(
        lorepia_storage::ChatId::parse("e".repeat(32)).expect("other chat ID"),
        100_001,
    )
    .expect("cursor");
    assert!(
        store
            .load_recent_messages(&chat.id, Some(&cross_chat_cursor), 50)
            .is_err(),
        "a cursor from another chat must fail closed"
    );

    let source = include_str!("../src/store.rs").to_ascii_uppercase();
    assert!(
        !source.contains(" OFFSET "),
        "storage product queries must use keyset pagination"
    );
}

fn query_plan<P>(connection: &Connection, sql: &str, parameters: P) -> String
where
    P: rusqlite::Params,
{
    let mut statement = connection.prepare(sql).expect("prepare query plan");
    let rows = statement
        .query_map(parameters, |row| row.get::<_, String>(3))
        .expect("read query plan");
    rows.map(|row| row.expect("query plan detail"))
        .collect::<Vec<_>>()
        .join(" | ")
}
