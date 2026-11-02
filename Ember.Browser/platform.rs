//! Bridges browser-native promises to the framework's local future executor.
//! Neither IndexedDB nor fetch blocks the UI thread; persistence resolves only
//! after the IndexedDB transaction commits.

use ember_core::{BoxFuture, HttpClient, ReaderError, Store};
use ferroui_browser::interop::completion_helper::await_promise;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(raw_module = "./ember-platform.js")]
extern "C" {
    #[wasm_bindgen(js_name = fetchJson)]
    fn fetch_json(url: &str) -> JsValue;
    #[wasm_bindgen(js_name = databaseGet)]
    fn database_get(key: &str) -> JsValue;
    #[wasm_bindgen(js_name = databaseGetMany)]
    fn database_get_many(keys: js_sys::Array) -> JsValue;
    #[wasm_bindgen(js_name = databaseRemoveMany)]
    fn database_remove_many(keys: js_sys::Array) -> JsValue;
    #[wasm_bindgen(js_name = databaseRemovePrefix)]
    fn database_remove_prefix(prefix: &str) -> JsValue;
    #[wasm_bindgen(js_name = databaseCount)]
    fn database_count(prefix: &str) -> JsValue;
    #[wasm_bindgen(js_name = databasePut)]
    fn database_put(key: &str, value: &str) -> JsValue;
    #[wasm_bindgen(js_name = databaseRemove)]
    fn database_remove(key: &str) -> JsValue;
    #[wasm_bindgen(js_name = databaseSizes)]
    fn database_sizes(prefix: &str) -> JsValue;
    #[wasm_bindgen(js_name = databaseScan)]
    fn database_scan(prefix: &str) -> JsValue;
    #[wasm_bindgen(js_name = setLocation)]
    pub fn set_location(item: Option<String>, back: bool);
}

pub struct BrowserHttpClient;

impl HttpClient for BrowserHttpClient {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, ReaderError>> {
        Box::pin(async move {
            let value = await_promise(&fetch_json(url))
                .await
                .map_err(|error| ReaderError::Network(error.to_string()))?;
            value.as_string().ok_or_else(|| {
                ReaderError::Network("The browser returned an invalid response".to_owned())
            })
        })
    }
}

/// A versioned, origin-scoped database separate from the service worker cache.
pub struct IndexedDbStore;

impl Store for IndexedDbStore {
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>, ReaderError>> {
        Box::pin(async move {
            let value = await_promise(&database_get(key))
                .await
                .map_err(|error| ReaderError::Storage(error.to_string()))?;
            if value.is_null() || value.is_undefined() {
                Ok(None)
            } else {
                value
                    .as_string()
                    .map(Some)
                    .ok_or_else(|| ReaderError::Storage("The stored value is invalid".to_owned()))
            }
        })
    }

    fn put<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFuture<'a, Result<(), ReaderError>> {
        Box::pin(async move {
            await_promise(&database_put(key, value))
                .await
                .map(drop)
                .map_err(|error| ReaderError::Storage(error.to_string()))
        })
    }

    fn remove<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ReaderError>> {
        Box::pin(async move {
            await_promise(&database_remove(key))
                .await
                .map(drop)
                .map_err(|error| ReaderError::Storage(error.to_string()))
        })
    }

    fn get_many<'a>(
        &'a self,
        keys: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Option<String>>, ReaderError>> {
        Box::pin(async move {
            let value = await_promise(&database_get_many(
                keys.iter().map(|k| JsValue::from_str(k)).collect(),
            ))
            .await
            .map_err(|e| ReaderError::Storage(e.to_string()))?;
            let array = value.dyn_into::<js_sys::Array>().map_err(|_| {
                ReaderError::Storage("The browser returned an invalid record batch".into())
            })?;
            array
                .iter()
                .map(|value| {
                    if value.is_null() {
                        Ok(None)
                    } else {
                        value.as_string().map(Some).ok_or_else(|| {
                            ReaderError::Storage("The stored value is invalid".into())
                        })
                    }
                })
                .collect()
        })
    }

    fn remove_many<'a>(&'a self, keys: &'a [String]) -> BoxFuture<'a, Result<(), ReaderError>> {
        Box::pin(async move {
            await_promise(&database_remove_many(
                keys.iter().map(|k| JsValue::from_str(k)).collect(),
            ))
            .await
            .map(drop)
            .map_err(|e| ReaderError::Storage(e.to_string()))
        })
    }

    fn remove_prefix<'a>(&'a self, prefix: &'a str) -> BoxFuture<'a, Result<(), ReaderError>> {
        Box::pin(async move {
            await_promise(&database_remove_prefix(prefix))
                .await
                .map(drop)
                .map_err(|e| ReaderError::Storage(e.to_string()))
        })
    }

    fn count_prefix<'a>(&'a self, prefix: &'a str) -> BoxFuture<'a, Result<usize, ReaderError>> {
        Box::pin(async move {
            let count = await_promise(&database_count(prefix))
                .await
                .map_err(|e| ReaderError::Storage(e.to_string()))?;
            count.as_f64().map(|n| n as usize).ok_or_else(|| {
                ReaderError::Storage("The browser returned an invalid record count".into())
            })
        })
    }

    fn scan_sizes<'a>(
        &'a self,
        prefix: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, usize)>, ReaderError>> {
        Box::pin(async move {
            let value = await_promise(&database_sizes(prefix))
                .await
                .map_err(|e| ReaderError::Storage(e.to_string()))?;
            let json = value.as_string().ok_or_else(|| {
                ReaderError::Storage("The browser returned invalid record sizes".into())
            })?;
            serde_json::from_str(&json).map_err(|e| ReaderError::Storage(e.to_string()))
        })
    }

    fn scan_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, String)>, ReaderError>> {
        Box::pin(async move {
            let value = await_promise(&database_scan(prefix))
                .await
                .map_err(|error| ReaderError::Storage(error.to_string()))?;
            let json = value.as_string().ok_or_else(|| {
                ReaderError::Storage("The browser returned an invalid index".to_owned())
            })?;
            serde_json::from_str(&json).map_err(|error| ReaderError::Storage(error.to_string()))
        })
    }
}
