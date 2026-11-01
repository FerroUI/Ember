# Measuring Ember

A release interaction includes input routing, repository work, the retained control tree and layout. For a code change, measure the interaction first, then use the narrower workloads below to identify the cost. Compare the same fixture, renderer, viewport, CPU allocation and Cargo profile on both revisions. Run timing workloads sequentially.

## Work removed since `ec9eb5e`

| Workload | Before | After | What the measurement covers |
| --- | ---: | ---: | --- |
| Nine 30-story appends, ending at 300 stories | 1,629 collection notifications | 9 | Events from the real bound list, including adds and removals |
| Hide one of 300 visible stories | 300 notifications | 1 | Other rows keep their identities |
| Read and independently verify 1,001 cached items | 2,002 transactions | 65 | Chromium IndexedDB, ordered reads, 1 KiB payloads |
| Inspect the sizes of those cached records | 1,046,938 bytes | 23,916 bytes | UTF-8 JSON returned across the browser storage bridge |
| Persist 10,000 history entries with 100,000 seen replies | 3,560,332 bytes | 900,220 bytes | Actual stored JSON, verified by reading it back |
| Open a phone permalink to an item outside a 30-story feed | 31 unrelated HN requests | 0 before the feed is shown | One feed request plus 30 item requests in the baseline |
| Replace only the offline shell while the installed Wasm digest is unchanged | Module included in the new shell install | No module fetch or copy | Every module cache is keyed by its build digest and registration scope |

On the comparison host, the median of 25 optimized repository saves to an in-memory store fell from 4.77 to 1.17 ms. In release-mode in-memory SQLite, collection and verification of the 1,001-item fixture fell from 38.68 to 10.51 ms. Checking an under-limit cache fell from 3.219 to 0.425 ms. The SQLite figures are medians of four alternating runs, each containing nine samples. Physical disk, device thermals and provider latency can change end-to-end results.

The native control-tree workload with 4,096 rich replies measured four successive Find projections at 1,003.6 ms before and 31.1 ms after, using the unoptimized test profile and seven samples. The search text cache belongs to the current immutable discussion and retains at most 4 MiB of normalized body text. Queries still search bodies beyond that budget. Fresh data gets a fresh index. On the x86-64 test host, the virtualized row metadata occupied 40 bytes per row compared with 280 bytes for the previous owned row, before counting any separately allocated strings. These figures describe the row representation and do not measure process RSS.

## Reproduce

Set `CHROME` to a Chromium executable and `SITE` to the path of an extracted release browser site. Put the same benchmark driver on the measurement machine for both releases. The startup, feed and discussion drivers exercise the compiled reader with deterministic provider data. They create temporary browser profiles and print individual samples.

```sh
node Ember.Browser/tests/startup-perf.mjs "$SITE" 5
node Ember.Browser/tests/feed-perf.mjs "$SITE" 5
node Ember.Browser/tests/discussion-perf.mjs "$SITE"
node Ember.Browser/tests/storage-perf.mjs
node Ember.Browser/tests/shell-update.mjs "$SITE"
```

Startup records the first frame in which the view exposes the expected discussion title and the Wasm resource timing. It uses 390 × 844, 50 ms of emulated latency and 12 MiB/s throughput. The warm sample reloads after the worker has activated. Feed measures pointer release to the frame exposing the updated unread count. Discussion measures explicit Find commits to the frame exposing the expected match count. The storage script isolates IndexedDB and JSON from rendering.

```sh
cargo run --release --locked -p ember-core --example state_bench
cargo test --release --locked -p ember-core --lib sqlite_offline_workload -- --ignored --nocapture
cargo test --locked -p ember --lib measure_large_thread_projection_and_find -- --ignored --nocapture
cargo test --locked -p ember-content --lib measure_content_parse_and_extract -- --ignored --nocapture
```

Normal tests assert ordering, content, focus, durable state, cache integrity and collection change counts. Timing workloads are opt-in so shared-runner variation cannot fail CI. The [performance PR](https://github.com/FerroUI/Ember/pull/2) records release measurements and build checks.
