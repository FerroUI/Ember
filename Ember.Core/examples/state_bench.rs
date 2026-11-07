//! cargo run --release --locked -p ember-core --example state_bench
use ember_core::{BoxFuture, HttpClient, MemoryStore, ReaderError, ReaderState, Repository, Store};
use futures_executor::block_on;
use std::{hint::black_box, rc::Rc, time::Instant};

struct Offline;
impl HttpClient for Offline {
    fn get<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<String, ReaderError>> {
        unreachable!("reader state uses only local storage")
    }
}

fn main() {
    let mut state = ReaderState::default();
    for id in 1..=10_000 {
        state.mark_read(id, 1_700_000_000 + id);
        if id % 20 == 0 {
            state.toggle_saved(id, 1_700_000_000 + id);
        }
        if id % 10 == 0 {
            state
                .items
                .get_mut(&id)
                .unwrap()
                .seen_comments
                .extend(1..=100);
        }
    }
    let store = Rc::new(MemoryStore::default());
    let repository = Repository::new(Rc::new(Offline), store.clone());
    block_on(async {
        repository.save_state(&state).await.unwrap();
        let mut times = Vec::new();
        for _ in 0..25 {
            let start = Instant::now();
            repository.save_state(black_box(&state)).await.unwrap();
            times.push(start.elapsed());
        }
        times.sort();
        let bytes = store.get("reader:state").await.unwrap().unwrap().len();
        assert_eq!(repository.load_state().await.unwrap(), state);
        println!("10000 stories, 100000 seen replies: {bytes} stored bytes, {:.2} ms median repository save (25 samples)", times[12].as_secs_f64() * 1000.0);
    });
}
