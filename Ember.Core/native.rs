//! Native adapters. Network I/O and SQLite operations run on workers, never
//! while polling the returned futures on the UI thread.
use crate::{BoxFuture, HttpClient, ReaderError, Store};
use futures_channel::oneshot;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    io::Read,
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const HTTP_WORKERS: usize = 8;
const HTTP_QUEUE_CAPACITY: usize = 128;
type HttpResult = Result<String, ReaderError>;
type HttpHandler = Arc<dyn Fn(&str) -> HttpResult + Send + Sync>;

struct HttpJob {
    url: String,
    result: oneshot::Sender<HttpResult>,
}

#[derive(Clone)]
pub struct NativeHttpClient {
    jobs: mpsc::SyncSender<HttpJob>,
    startup_error: Option<String>,
}
impl Default for NativeHttpClient {
    fn default() -> Self {
        Self::new()
    }
}
impl NativeHttpClient {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(8))
            .timeout_read(Duration::from_secs(20))
            .timeout(Duration::from_secs(30))
            .user_agent("Ember-HN/0.1 (FerroUI; public read API)")
            // As the browser client: no redirects, HTTPS only.
            .redirects(0)
            .https_only(true)
            .build();
        Self::with_handler(Arc::new(move |url| {
            let response = agent
                .get(url)
                .set("Accept", "application/json")
                .call()
                .map_err(|e| ReaderError::Network(e.to_string()))?;
            let mut bytes = Vec::new();
            response
                .into_reader()
                .take((MAX_BODY_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|e| ReaderError::Network(e.to_string()))?;
            if bytes.len() > MAX_BODY_BYTES {
                return Err(ReaderError::InvalidData("Response exceeds 8 MiB".into()));
            }
            String::from_utf8(bytes).map_err(|e| ReaderError::InvalidData(e.to_string()))
        }))
    }

    fn with_handler(handler: HttpHandler) -> Self {
        let (jobs, receiver) = mpsc::sync_channel::<HttpJob>(HTTP_QUEUE_CAPACITY);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut startup_error = None;
        for index in 0..HTTP_WORKERS {
            let receiver = receiver.clone();
            let handler = handler.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("ember-http-{index}"))
                .spawn(move || {
                    loop {
                        // Only receiving holds the lock; HTTP work runs concurrently.
                        let job = match receiver.lock() {
                            Ok(receiver) => receiver.recv(),
                            Err(_) => break,
                        };
                        let Ok(job) = job else { break };
                        if job.result.is_canceled() {
                            continue;
                        }
                        let _ = job.result.send(handler(&job.url));
                    }
                });
            if let Err(error) = spawned {
                if index == 0 {
                    startup_error = Some(format!("Could not start HTTP workers: {error}"));
                }
                break;
            }
        }
        Self {
            jobs,
            startup_error,
        }
    }
}
impl HttpClient for NativeHttpClient {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, ReaderError>> {
        let jobs = self.jobs.clone();
        let startup_error = self.startup_error.clone();
        let url = url.to_owned();
        Box::pin(async move {
            if let Some(error) = startup_error {
                return Err(ReaderError::Network(error));
            }
            let (result, receiver) = oneshot::channel();
            // Never block a UI dispatcher while the pool is saturated. Dropping
            // an unpolled future queues nothing; a canceled queued job is skipped.
            jobs.try_send(HttpJob { url, result }).map_err(|error| {
                ReaderError::Network(match error {
                    mpsc::TrySendError::Full(_) => {
                        "HTTP request queue is full; retry shortly".into()
                    }
                    mpsc::TrySendError::Disconnected(_) => "HTTP workers stopped".into(),
                })
            })?;
            receiver
                .await
                .map_err(|_| ReaderError::Network("HTTP worker stopped".into()))?
        })
    }
}

type DbJob = Box<dyn FnOnce(&mut Connection) + Send>;

/// One worker owns the connection. A write resolves only after SQLite commits.
#[derive(Clone)]
pub struct SqliteStore {
    jobs: mpsc::Sender<DbJob>,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ReaderError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(storage_error)?;
        }
        Self::from_connection(Connection::open(path).map_err(storage_error)?)
    }
    pub fn in_memory() -> Result<Self, ReaderError> {
        Self::from_connection(Connection::open_in_memory().map_err(storage_error)?)
    }
    fn from_connection(mut connection: Connection) -> Result<Self, ReaderError> {
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(storage_error)?;
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(storage_error)?;
        if version > 1 {
            return Err(ReaderError::Storage(
                "Database was created by a newer Ember version".into(),
            ));
        }
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
            BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS ember_records (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL) WITHOUT ROWID;
            PRAGMA user_version=1; COMMIT;").map_err(storage_error)?;
        let (jobs, receiver) = mpsc::channel::<DbJob>();
        std::thread::Builder::new()
            .name("ember-database".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    job(&mut connection);
                }
            })
            .map_err(storage_error)?;
        Ok(Self { jobs })
    }
    fn execute<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection) -> Result<T, ReaderError> + Send + 'static,
    ) -> BoxFuture<'static, Result<T, ReaderError>> {
        let (sender, receiver) = oneshot::channel();
        let queued = self.jobs.send(Box::new(move |connection| {
            let _ = sender.send(work(connection));
        }));
        Box::pin(async move {
            queued.map_err(|_| ReaderError::Storage("Database worker stopped".into()))?;
            receiver
                .await
                .map_err(|_| ReaderError::Storage("Database operation was interrupted".into()))?
        })
    }
}

impl Store for SqliteStore {
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>, ReaderError>> {
        let key = key.to_owned();
        self.execute(move |db| {
            db.query_row(
                "SELECT value FROM ember_records WHERE key=?1",
                [key],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage_error)
        })
    }
    fn put<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFuture<'a, Result<(), ReaderError>> {
        let key = key.to_owned();
        let value = value.to_owned();
        self.execute(move |db| {
            db.execute("INSERT INTO ember_records(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key,value]).map_err(storage_error)?; Ok(())
        })
    }
    fn remove<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ReaderError>> {
        let key = key.to_owned();
        self.execute(move |db| {
            db.execute("DELETE FROM ember_records WHERE key=?1", [key])
                .map_err(storage_error)?;
            Ok(())
        })
    }
    fn scan_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, String)>, ReaderError>> {
        let prefix = prefix.to_owned();
        self.execute(move |db| {
            // SQLite LIKE is ASCII case-insensitive by default. Record prefixes
            // are exact and case-sensitive, including usernames and literal %/_ .
            let mut statement = db.prepare("SELECT key,value FROM ember_records WHERE key>=?1 AND substr(key,1,length(?1))=?1 COLLATE BINARY ORDER BY key")
                .map_err(storage_error)?;
            let rows = statement.query_map([prefix], |row| Ok((row.get(0)?, row.get(1)?))).map_err(storage_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
        })
    }
}

fn storage_error(error: impl std::fmt::Display) -> ReaderError {
    ReaderError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_executor::block_on;
    use std::sync::{
        Condvar,
        atomic::{AtomicUsize, Ordering},
    };
    use std::task::{Context, Poll};

    #[test]
    fn native_pool_bounds_running_work_and_skips_canceled_queued_requests() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let (started, starts) = mpsc::channel();
        let (completed, completions) = mpsc::channel();
        let client = NativeHttpClient::with_handler({
            let gate = gate.clone();
            let active = active.clone();
            let peak = peak.clone();
            let total = total.clone();
            Arc::new(move |url| {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                total.fetch_add(1, Ordering::SeqCst);
                started.send(url.to_owned()).unwrap();
                let (released, timed) = gate
                    .1
                    .wait_timeout_while(
                        gate.0.lock().unwrap(),
                        Duration::from_secs(5),
                        |released| !*released,
                    )
                    .unwrap();
                drop(released);
                active.fetch_sub(1, Ordering::SeqCst);
                completed.send(()).unwrap();
                if timed.timed_out() {
                    Err(ReaderError::Network("Test worker release timed out".into()))
                } else {
                    Ok(url.to_owned())
                }
            })
        });
        let waker = futures_util::task::noop_waker();
        let mut context = Context::from_waker(&waker);
        let urls: Vec<_> = (0..HTTP_WORKERS)
            .map(|id| format!("running-{id}"))
            .collect();
        let mut running: Vec<_> = urls.iter().map(|url| client.get(url)).collect();
        for future in &mut running {
            assert!(future.as_mut().poll(&mut context).is_pending());
        }
        for _ in 0..HTTP_WORKERS {
            starts.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert_eq!(active.load(Ordering::SeqCst), HTTP_WORKERS);
        drop(running);
        drop(client.get("never-polled"));

        let mut survivor = client.get("survivor");
        assert!(survivor.as_mut().poll(&mut context).is_pending());
        for _ in 1..HTTP_QUEUE_CAPACITY {
            let mut canceled = client.get("canceled");
            assert!(canceled.as_mut().poll(&mut context).is_pending());
            drop(canceled);
        }
        let mut overflow = client.get("overflow");
        assert!(matches!(
            overflow.as_mut().poll(&mut context),
            Poll::Ready(Err(_))
        ));
        assert_eq!(total.load(Ordering::SeqCst), HTTP_WORKERS);

        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        assert_eq!(block_on(survivor).unwrap(), "survivor");
        for _ in 0..=HTTP_WORKERS {
            completions.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert_eq!(total.load(Ordering::SeqCst), HTTP_WORKERS + 1);
        assert_eq!(peak.load(Ordering::SeqCst), HTTP_WORKERS);
        assert_eq!(starts.try_iter().collect::<Vec<_>>(), ["survivor"]);
    }
}
