mod common;

use lorepia_storage::{
    EvictRenderCache, MAX_RENDERED_HTML_BYTES, PutRenderCache, RenderCacheCas, RendererVersion,
    StorageError,
};

use common::{begin_turn, create_chat, database, timestamp};

#[test]
fn render_cache_is_bounded_cas_guarded_touchable_and_evictable() {
    let (_directory, path) = database();
    let store = lorepia_storage::Store::open_at(&path, timestamp(10)).expect("open store");
    let chat = create_chat(&store, 20);
    let started = begin_turn(&store, &chat, "hello", 30);
    let version_one = RendererVersion::new(1).expect("renderer version");
    let version_two = RendererVersion::new(2).expect("renderer version");

    let inserted = store
        .put_render_cache(PutRenderCache {
            message_id: started.user_message_id.clone(),
            renderer_version: version_one,
            html: "<p>hello</p>".to_owned(),
            expected: None,
            at_ms: timestamp(40),
        })
        .expect("insert render cache");
    let duplicate = store
        .put_render_cache(PutRenderCache {
            message_id: started.user_message_id.clone(),
            renderer_version: version_one,
            html: "duplicate".to_owned(),
            expected: None,
            at_ms: timestamp(41),
        })
        .expect_err("insert-only CAS rejects existing entry");
    assert!(matches!(
        duplicate,
        StorageError::Conflict {
            entity: "render cache"
        }
    ));
    assert!(
        store
            .get_render_cache(&started.user_message_id, version_two, timestamp(45))
            .expect("wrong renderer is a cache miss")
            .is_none()
    );
    let touched = store
        .get_render_cache(&started.user_message_id, version_one, timestamp(50))
        .expect("touch cache")
        .expect("cache hit");
    assert_eq!(touched.last_used_at_ms, timestamp(50));

    let stale = store
        .put_render_cache(PutRenderCache {
            message_id: started.user_message_id.clone(),
            renderer_version: version_two,
            html: "new".to_owned(),
            expected: Some(RenderCacheCas {
                renderer_version: inserted.renderer_version,
                last_used_at_ms: inserted.last_used_at_ms,
            }),
            at_ms: timestamp(60),
        })
        .expect_err("stale touch token rejects overwrite");
    assert!(matches!(
        stale,
        StorageError::Conflict {
            entity: "render cache revision"
        }
    ));
    let updated = store
        .put_render_cache(PutRenderCache {
            message_id: started.user_message_id.clone(),
            renderer_version: version_two,
            html: "<p>new</p>".to_owned(),
            expected: Some(RenderCacheCas {
                renderer_version: touched.renderer_version,
                last_used_at_ms: touched.last_used_at_ms,
            }),
            at_ms: timestamp(60),
        })
        .expect("CAS update");
    assert_eq!(updated.renderer_version, version_two);

    let oversized = store
        .put_render_cache(PutRenderCache {
            message_id: started.user_message_id.clone(),
            renderer_version: version_two,
            html: "x".repeat(MAX_RENDERED_HTML_BYTES + 1),
            expected: Some(RenderCacheCas {
                renderer_version: updated.renderer_version,
                last_used_at_ms: updated.last_used_at_ms,
            }),
            at_ms: timestamp(61),
        })
        .expect_err("reject oversized HTML");
    assert!(matches!(
        oversized,
        StorageError::InvalidInput {
            field: "rendered HTML",
            ..
        }
    ));

    let eviction = store
        .evict_render_cache(EvictRenderCache {
            older_than_ms: timestamp(61),
            limit: 10,
        })
        .expect("evict LRU cache");
    assert_eq!(eviction.evicted_entries, 1);
    assert_eq!(eviction.evicted_html_bytes, "<p>new</p>".len() as u64);
    assert!(
        store
            .get_render_cache(&started.user_message_id, version_two, timestamp(70))
            .expect("cache miss after eviction")
            .is_none()
    );
}
