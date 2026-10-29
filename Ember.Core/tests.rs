use super::*;
use futures_executor::block_on;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

#[derive(Default)]
struct FakeHttp {
    responses: RefCell<BTreeMap<String, Result<String, ReaderError>>>,
    calls: Cell<usize>,
    active: Cell<usize>,
    peak: Cell<usize>,
}
impl FakeHttp {
    fn item(&self, id: u64, json: &str) {
        self.responses
            .borrow_mut()
            .insert(format!("{HN_API}/item/{id}.json"), Ok(json.into()));
    }
    fn response(&self, url: &str, json: &str) {
        self.responses
            .borrow_mut()
            .insert(url.into(), Ok(json.into()));
    }
}
impl HttpClient for FakeHttp {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, ReaderError>> {
        Box::pin(async move {
            self.calls.set(self.calls.get() + 1);
            self.active.set(self.active.get() + 1);
            let _active = ActiveRequest(&self.active);
            self.peak.set(self.peak.get().max(self.active.get()));
            let mut yielded = false;
            futures_util::future::poll_fn(|cx| {
                if yielded {
                    std::task::Poll::Ready(())
                } else {
                    yielded = true;
                    cx.waker().wake_by_ref();
                    std::task::Poll::Pending
                }
            })
            .await;
            self.responses
                .borrow()
                .get(url)
                .cloned()
                .unwrap_or_else(|| Err(ReaderError::Network("offline".into())))
        })
    }
}

struct ActiveRequest<'a>(&'a Cell<usize>);
impl Drop for ActiveRequest<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

#[test]
fn concurrency_is_bounded_across_simultaneous_repository_operations() {
    block_on(async {
        let (repo, http, _) = repository();
        let repo = repo.with_policy(CachePolicy {
            concurrency: 2,
            ..CachePolicy::default()
        });
        for id in 1..=6 {
            http.item(id, &format!(r#"{{"id":{id}}}"#));
        }
        let (a, b) = futures_util::join!(
            repo.load_items(&[1, 2, 3], 1, FetchMode::Refresh),
            repo.load_items(&[4, 5, 6], 1, FetchMode::Refresh)
        );
        assert_eq!(a.items.len() + b.items.len(), 6);
        assert!(http.peak.get() <= 2);
    });
}

#[test]
fn canceled_shared_requests_release_their_future_and_permit() {
    let (repo, http, _) = repository();
    let repo = repo.with_policy(CachePolicy {
        concurrency: 1,
        ..CachePolicy::default()
    });
    http.item(1, r#"{"id":1}"#);
    let mut first = Box::pin(repo.load_item(1, 1, FetchMode::Refresh));
    let mut context = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(std::future::Future::poll(first.as_mut(), &mut context).is_pending());
    assert_eq!(http.active.get(), 1);
    drop(first);
    assert_eq!(http.active.get(), 0);
    assert!(block_on(repo.load_item(1, 1, FetchMode::Refresh)).is_ok());
    assert_eq!(http.calls.get(), 2);
}

fn repository() -> (Repository, Rc<FakeHttp>, Rc<MemoryStore>) {
    let http = Rc::new(FakeHttp::default());
    let store = Rc::new(MemoryStore::default());
    (Repository::new(http.clone(), store.clone()), http, store)
}
fn item(id: u64, kids: &[u64]) -> HnItem {
    HnItem {
        id,
        kids: kids.to_vec(),
        ..HnItem::default()
    }
}

#[test]
fn tolerant_api_models_handle_deleted_and_unknown_types() {
    let item: HnItem = serde_json::from_str(r#"{"id":9,"deleted":true,"future":42}"#).unwrap();
    assert!(item.deleted);
    assert!(item.by.is_none());
    assert!(item.kids.is_empty());
    let item: HnItem = serde_json::from_str(r#"{"id":10,"type":"futuretype"}"#).unwrap();
    assert_eq!(item.kind, Some(ItemKind::Other));
}

#[test]
fn cache_ttl_refresh_fallback_and_offline_are_distinct() {
    block_on(async {
        let (repo, http, _) = repository();
        http.item(1, r#"{"id":1,"title":"cached"}"#);
        assert_eq!(
            repo.load_item(1, 100, FetchMode::CacheFirst)
                .await
                .unwrap()
                .source,
            DataSource::Network
        );
        assert_eq!(
            repo.load_item(1, 150, FetchMode::CacheFirst)
                .await
                .unwrap()
                .source,
            DataSource::Cache
        );
        assert_eq!(http.calls.get(), 1);
        http.responses.borrow_mut().clear();
        let offline = repo.load_item(1, 500, FetchMode::Offline).await.unwrap();
        assert!(offline.stale);
        assert_eq!(http.calls.get(), 1);
        let fallback = repo.load_item(1, 500, FetchMode::Refresh).await.unwrap();
        assert_eq!(fallback.source, DataSource::StaleCache);
        assert!(fallback.warning.is_some());
        assert_eq!(fallback.value.unwrap().title.as_deref(), Some("cached"));
        assert!(repo.load_item(2, 500, FetchMode::Offline).await.is_err());
    });
}

#[test]
fn null_item_is_a_negative_cache_entry_not_transport_failure() {
    block_on(async {
        let (repo, http, _) = repository();
        http.item(1, "null");
        assert!(
            repo.load_item(1, 100, FetchMode::CacheFirst)
                .await
                .unwrap()
                .value
                .is_none()
        );
        assert!(
            repo.load_item(1, 101, FetchMode::Offline)
                .await
                .unwrap()
                .value
                .is_none()
        );
        assert_eq!(http.calls.get(), 1);
    });
}

#[test]
fn request_deduplication_and_order_preservation_are_bounded() {
    block_on(async {
        let (repo, http, _) = repository();
        let repo = repo.with_policy(CachePolicy {
            concurrency: 2,
            ..CachePolicy::default()
        });
        for id in [1, 2, 3, 4] {
            http.item(id, &format!(r#"{{"id":{id}}}"#));
        }
        let batch = repo
            .load_items(&[4, 2, 4, 1, 3], 1, FetchMode::CacheFirst)
            .await;
        assert_eq!(
            batch.items.iter().map(|i| i.id).collect::<Vec<_>>(),
            [4, 2, 4, 1, 3]
        );
        assert!(http.peak.get() <= 2);
        assert_eq!(http.calls.get(), 4);
    });
}

#[test]
fn concurrent_identical_refreshes_share_transport() {
    block_on(async {
        let (repo, http, _) = repository();
        http.item(1, r#"{"id":1}"#);
        let (a, b) = futures_util::join!(
            repo.load_item(1, 1, FetchMode::Refresh),
            repo.load_item(1, 1, FetchMode::Refresh)
        );
        assert!(a.is_ok() && b.is_ok());
        assert_eq!(http.calls.get(), 1);
    });
}

#[test]
fn feed_pagination_retains_server_rank_and_reports_missing_items() {
    block_on(async {
        let (repo, http, _) = repository();
        http.response(&format!("{HN_API}/topstories.json"), "[4,2,1,3]");
        http.item(2, "null");
        http.item(1, r#"{"id":1}"#);
        let page = repo
            .load_feed(FeedKind::Top, 1, 2, 1, FetchMode::Refresh)
            .await
            .unwrap();
        assert_eq!(page.total, 4);
        assert_eq!(page.next_offset, Some(3));
        assert_eq!(page.missing_ids, [2]);
        assert_eq!(page.items[0].id, 1);
    });
}

#[test]
fn thread_budget_and_cycles_report_partial_state() {
    block_on(async {
        let (repo, http, _) = repository();
        http.item(1, r#"{"id":1,"kids":[2,3]}"#);
        http.item(2, r#"{"id":2,"kids":[1,4]}"#);
        http.item(3, r#"{"id":3}"#);
        http.item(4, r#"{"id":4}"#);
        let partial = repo
            .load_thread(1, 2, 1, FetchMode::CacheFirst)
            .await
            .unwrap();
        assert!(!partial.complete);
        assert_eq!(partial.items.len(), 2);
        assert_eq!(partial.pending_ids, [3, 4]);
        let complete = repo
            .load_thread(1, 20, 1, FetchMode::CacheFirst)
            .await
            .unwrap();
        assert!(complete.complete);
        assert_eq!(complete.items.len(), 4);
        assert_eq!(
            flatten_thread(&complete, &ThreadOptions::default())
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>(),
            [2, 4, 3]
        );
    });
}

#[test]
fn collapse_and_search_keep_ancestor_context_without_mutating_state() {
    let mut snapshot = ThreadSnapshot {
        root: item(1, &[2, 3]),
        ..ThreadSnapshot::default()
    };
    snapshot.items.insert(2, item(2, &[4]));
    snapshot.items.insert(3, item(3, &[]));
    snapshot.items.insert(
        4,
        HnItem {
            id: 4,
            text: Some("A rare needle".into()),
            ..HnItem::default()
        },
    );
    let mut options = ThreadOptions::default();
    options.collapsed.insert(2);
    assert_eq!(flatten_thread(&snapshot, &options).len(), 2);
    options.query = "needle".into();
    let rows = flatten_thread(&snapshot, &options);
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [2, 4]);
    assert!(!rows[0].matches_query);
    assert!(rows[1].matches_query);
    assert!(options.collapsed.contains(&2));
}

#[test]
fn custom_thread_matching_keeps_context_and_respects_visibility_guards() {
    let mut snapshot = ThreadSnapshot {
        root: item(1, &[2, 3, 5]),
        ..ThreadSnapshot::default()
    };
    snapshot.items.insert(2, item(2, &[4]));
    snapshot.items.insert(
        4,
        HnItem {
            id: 4,
            text: Some("<p>Don&#x27;t <b>lose</b> this text.</p>".into()),
            ..HnItem::default()
        },
    );
    snapshot.items.insert(
        3,
        HnItem {
            deleted: true,
            ..item(3, &[])
        },
    );
    snapshot.items.insert(
        5,
        HnItem {
            dead: true,
            ..item(5, &[])
        },
    );
    let mut options = ThreadOptions {
        query: "  DON'T LOSE  ".into(),
        ..ThreadOptions::default()
    };
    options.collapsed.insert(2);
    let mut searched = Vec::new();
    let rows = flatten_thread_with_match(&snapshot, &options, |item, query| {
        searched.push(item.id);
        assert_eq!(query, "don't lose");
        item.id == 4 && "don't lose this text.".contains(query)
    });
    assert_eq!(searched, [2, 4]);
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), [2, 4]);
    assert!(!rows[0].matches_query);
    assert!(rows[1].matches_query);
    assert_eq!(rows[1].item.as_ref().unwrap().text, snapshot.items[&4].text);
    assert!(options.collapsed.contains(&2));
    options.show_dead = true;
    searched.clear();
    flatten_thread_with_match(&snapshot, &options, |item, _| {
        searched.push(item.id);
        true
    });
    assert_eq!(searched, [2, 4, 5]);
}

#[test]
fn unsafe_navigation_urls_are_rejected() {
    for unsafe_url in [
        "javascript:alert(1)",
        "data:text/html,hi",
        "file:///etc/passwd",
        "https://user:secret@example.com",
        "https://example.com\n@evil.test",
        "//example.com",
    ] {
        assert!(safe_web_url(unsafe_url).is_none(), "{unsafe_url}");
    }
    assert!(safe_web_url("https://example.com/a?b=c#d").is_some());
    assert!(safe_web_url("http://example.com").is_some());
    assert_eq!(item_url(42), "https://news.ycombinator.com/item?id=42");
}

#[test]
fn search_queries_encode_data_and_validate_filters() {
    let query = SearchQuery {
        query: "Rust & C# + λ".into(),
        author: Some("Case_Sensitive".into()),
        min_points: Some(100),
        since: Some(1000),
        ..SearchQuery::default()
    };
    let url = url::Url::parse(&query.url().unwrap()).unwrap();
    let pairs: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(pairs["query"], "Rust & C# + λ");
    assert_eq!(pairs["tags"], "story,author_Case_Sensitive");
    assert!(pairs["numericFilters"].contains("points>=100"));
    assert!(
        SearchQuery {
            author: Some("x,(comment)".into()),
            ..query.clone()
        }
        .url()
        .is_err()
    );
    assert!(
        SearchQuery {
            since: Some(4),
            until: Some(3),
            ..query
        }
        .url()
        .is_err()
    );
}

#[test]
fn search_maps_comments_and_story_context_without_html_highlight_injection() {
    block_on(async {
        let (repo, http, _) = repository();
        let query = SearchQuery {
            kind: SearchKind::Comments,
            ..SearchQuery::default()
        };
        http.response(&query.url().unwrap(), r#"{"hits":[{"objectID":"42","author":"user","comment_text":"<p>Hello</p>","story_id":5,"story_title":"Original","_tags":["comment"],"_highlightResult":{"comment_text":{"value":"<script>no</script>"}}}],"nbPages":2,"nbHits":31,"page":0}"#);
        let page = repo.search(&query, 1, FetchMode::CacheFirst).await.unwrap();
        assert_eq!(page.story_ids[&42], 5);
        assert_eq!(page.items[0].text.as_deref(), Some("<p>Hello</p>"));
        assert_eq!(page.items[0].kind, Some(ItemKind::Comment));
        assert_eq!(page.pages, 2);
    });
}

#[test]
fn local_state_round_trips_and_normalizes_preferences() {
    let mut state = ReaderState::default();
    state.toggle_saved(4, 5);
    state.mark_read(4, 6);
    state.toggle_hidden(9);
    state.preferences.font_size = 500.0;
    state.preferences.accent = "bad".into();
    state.remember_search(" rust ");
    state.remember_search("c#");
    state.remember_search("rust");
    let copy = ReaderState::import_json(&state.export_json().unwrap()).unwrap();
    assert!(copy.is_saved(4));
    assert!(copy.is_read(4));
    assert_eq!(copy.saved_ids(), [4]);
    assert_eq!(copy.history_ids(), [4]);
    assert_eq!(copy.preferences.font_size, 32.0);
    assert_eq!(copy.preferences.accent, "#FF6B35");
    assert_eq!(copy.recent_searches, ["rust", "c#"]);
    assert!(ReaderState::import_json(r#"{"version":999}"#).is_err());
}

#[test]
fn mutes_respect_domain_boundaries_and_username_case() {
    let mut mutes = MuteRules::default();
    mutes.domains.insert("example.com".into());
    mutes.users.insert("Alice".into());
    let story = HnItem {
        id: 1,
        url: Some("https://sub.example.com/story".into()),
        ..HnItem::default()
    };
    assert!(mutes.matches(&story));
    assert!(!mutes.matches(&HnItem {
        url: Some("https://notexample.com/story".into()),
        by: Some("alice".into()),
        ..story
    }));
}

#[test]
fn cache_eviction_preserves_saved_and_downloaded_payloads_and_state() {
    block_on(async {
        let (repo, http, _) = repository();
        for id in 1..=4 {
            http.item(id, &format!(r#"{{"id":{id}}}"#));
        }
        repo.load_items(&[1, 2, 3, 4], 1, FetchMode::Refresh).await;
        let mut state = ReaderState::default();
        state.toggle_saved(1, 1);
        state.downloaded_threads.insert(2, vec![3]);
        state.preferences.cache_max_items = 1;
        repo.save_state(&state).await.unwrap();
        assert_eq!(repo.prune_cache(&state).await.unwrap(), 1);
        assert_eq!(repo.cache_stats().await.unwrap().items, 3);
        repo.clear_cache().await.unwrap();
        assert!(repo.load_state().await.unwrap().is_saved(1));
    });
}

#[test]
fn metadata_cache_is_bounded_by_age_without_evicting_feeds_or_local_state() {
    block_on(async {
        let (repo, http, store) = repository();
        let mut state = ReaderState::default();
        state.toggle_saved(700, 1);
        repo.save_state(&state).await.unwrap();
        http.item(700, r#"{"id":700}"#);
        repo.load_item(700, 0, FetchMode::Refresh).await.unwrap();
        for feed in FeedKind::ALL {
            store
                .put(
                    &format!("cache:feed:{}", feed.endpoint()),
                    r#"{"fetched_at":0,"value":[700]}"#,
                )
                .await
                .unwrap();
        }
        let mut keys = Vec::new();
        for age in 0..(METADATA_CACHE_LIMIT + 4) {
            let kind = if age % 2 == 0 { "user" } else { "search" };
            let key = format!("cache:{kind}:record-{:03}", 1000 - age);
            let json = serde_json::json!({"fetched_at":age, "value":null}).to_string();
            store.put(&key, &json).await.unwrap();
            keys.push(key);
        }
        assert_eq!(repo.prune_cache(&state).await.unwrap(), 4);
        for (index, key) in keys.iter().enumerate() {
            assert_eq!(store.get(key).await.unwrap().is_some(), index >= 4);
        }
        assert_eq!(
            repo.cache_stats().await.unwrap().records,
            METADATA_CACHE_LIMIT + 7
        );
        for feed in FeedKind::ALL {
            assert!(
                repo.load_feed(feed, 0, 1, 1, FetchMode::Offline)
                    .await
                    .is_ok()
            );
        }
        assert!(repo.load_state().await.unwrap().is_saved(700));

        // A standalone profile load enforces the same budget without requiring
        // a later feed refresh or a UI-triggered pruning pass.
        http.response(&format!("{HN_API}/user/recent.json"), r#"{"id":"recent"}"#);
        repo.load_user("recent", 1000, FetchMode::Refresh)
            .await
            .unwrap();
        assert!(store.get(&keys[4]).await.unwrap().is_none());
        assert!(store.get("cache:user:recent").await.unwrap().is_some());
        assert_eq!(
            repo.cache_stats().await.unwrap().records,
            METADATA_CACHE_LIMIT + 7
        );
    });
}

#[test]
fn mismatched_item_identity_is_rejected_and_removed() {
    block_on(async {
        let (repo, http, store) = repository();
        http.item(1, r#"{"id":99}"#);
        assert!(repo.load_item(1, 1, FetchMode::Refresh).await.is_err());
        assert!(store.get("cache:item:1").await.unwrap().is_none());
    });
}

struct UnwritableStore(MemoryStore);
impl Store for UnwritableStore {
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>, ReaderError>> {
        self.0.get(key)
    }
    fn put<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<(), ReaderError>> {
        Box::pin(async { Err(ReaderError::Storage("quota exceeded".into())) })
    }
    fn remove<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ReaderError>> {
        self.0.remove(key)
    }
    fn scan_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, String)>, ReaderError>> {
        self.0.scan_prefix(prefix)
    }
}

#[test]
fn cache_write_failure_allows_reading_but_never_claims_offline_success() {
    block_on(async {
        let http = Rc::new(FakeHttp::default());
        http.item(1, r#"{"id":1,"kids":[2]}"#);
        http.item(2, r#"{"id":2}"#);
        let repo = Repository::new(http, Rc::new(UnwritableStore(MemoryStore::default())));
        let online = repo
            .load_thread(1, 10, 1, FetchMode::Refresh)
            .await
            .unwrap();
        assert!(online.complete);
        assert!(online.warning.is_some());
        assert_eq!(online.items.len(), 2);
        let download = repo.download_thread(1, 10, 1).await.unwrap();
        assert!(!download.complete);
        assert_eq!(download.missing_ids, [1, 2]);
        assert!(repo.save_state(&ReaderState::default()).await.is_err());
    });
}

#[test]
fn completed_download_can_reopen_fully_offline() {
    block_on(async {
        let (repo, http, _) = repository();
        http.item(1, r#"{"id":1,"kids":[2]}"#);
        http.item(2, r#"{"id":2}"#);
        let download = repo.download_thread(1, 10, 1).await.unwrap();
        assert!(download.complete);
        http.responses.borrow_mut().clear();
        let offline = repo
            .load_thread(1, 10, 1000, FetchMode::Offline)
            .await
            .unwrap();
        assert!(offline.complete);
        assert!(offline.stale);
        assert_eq!(offline.items.len(), 2);
        assert_eq!(http.calls.get(), 2);
    });
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn sqlite_store_persists_across_connections_and_scans_literal_prefixes() {
    block_on(async {
        let path = std::env::temp_dir().join(format!(
            "ember-core-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let store = native::SqliteStore::open(&path).unwrap();
            store.put("user:a_%", "saved").await.unwrap();
            store.put("user:abc", "other").await.unwrap();
            store.put("user:A_%", "case-sensitive").await.unwrap();
            assert_eq!(
                store.scan_prefix("user:a_").await.unwrap(),
                [("user:a_%".into(), "saved".into())]
            );
        }
        {
            let reopened = native::SqliteStore::open(&path).unwrap();
            assert_eq!(
                reopened.get("user:a_%").await.unwrap().as_deref(),
                Some("saved")
            );
            reopened.remove("user:a_%").await.unwrap();
            assert!(reopened.get("user:a_%").await.unwrap().is_none());
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    });
}
