use ember_core::Repository;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
ferroui_base::ferro_thread_local! {
    static FACTORY: RefCell<Option<Rc<dyn Fn() -> Rc<Repository>>>> = RefCell::new(None);
    static DATABASE_PATH: RefCell<Option<PathBuf>> = RefCell::new(None);
    static INITIAL_LOCATION: RefCell<Option<String>> = RefCell::new(None);
}
/// Install before application startup; browser hosts supply Fetch and IndexedDB.
pub fn set_repository_factory(factory: Rc<dyn Fn() -> Rc<Repository>>) {
    FACTORY.with(|v| *v.borrow_mut() = Some(factory));
}
/// Mobile hosts select the OS-owned application data directory.
pub fn set_database_path(path: PathBuf) {
    DATABASE_PATH.with(|v| *v.borrow_mut() = Some(path));
}
pub fn set_initial_location(location: String) {
    INITIAL_LOCATION.with(|v| *v.borrow_mut() = Some(location));
}
pub(crate) fn take_initial_location() -> Option<String> {
    INITIAL_LOCATION.with(|v| v.borrow_mut().take())
}
pub(crate) fn repository() -> Result<Rc<Repository>, String> {
    if let Some(factory) = FACTORY.with(|v| v.borrow().clone()) {
        return Ok(factory());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use ember_core::native::{NativeHttpClient, SqliteStore};
        let path = DATABASE_PATH
            .with(|v| v.borrow().clone())
            .unwrap_or_else(default_database_path);
        let store = SqliteStore::open(path).map_err(|e| e.to_string())?;
        Ok(Rc::new(Repository::new(
            Rc::new(NativeHttpClient::new()),
            Rc::new(store),
        )))
    }
    #[cfg(target_arch = "wasm32")]
    Err("The browser storage service was not installed by the host.".into())
}
#[cfg(not(target_arch = "wasm32"))]
fn default_database_path() -> PathBuf {
    if let Some(path) = std::env::var_os("EMBER_DATABASE") {
        return PathBuf::from(path);
    }
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let root =
        std::env::var_os("HOME").map(|v| PathBuf::from(v).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".local/share")));
    root.unwrap_or_else(|| PathBuf::from("."))
        .join("Ember")
        .join("reader.sqlite3")
}
pub(crate) fn now() -> u64 {
    use ferroui_base::utilities::DateTime;
    DateTime::utc_now()
        .ticks()
        .saturating_sub(DateTime::UNIX_EPOCH.ticks())
        .max(0) as u64
        / 10_000_000
}
