// The browser-specific boundary. HN data is rendered by Rust/FerroUI, never by innerHTML.
const DATABASE = "ferroui-ember";
const DATABASE_VERSION = 1;
const RECORDS = "records";
const MAX_RESPONSE_BYTES = 8 * 1024 * 1024;
let databasePromise;

export function validateApiUrl(value) {
    const url = new URL(value);
    const hn = url.hostname === "hacker-news.firebaseio.com" && url.pathname.startsWith("/v0/");
    const search = url.hostname === "hn.algolia.com" && url.pathname.startsWith("/api/v1/");
    if (url.protocol !== "https:" || url.port || url.username || url.password || !(hn || search)) {
        throw new Error("Ember accepts requests only to its documented HN data providers");
    }
    return url.href;
}

async function boundedText(response) {
    const length = Number(response.headers.get("content-length"));
    if (Number.isFinite(length) && length > MAX_RESPONSE_BYTES) throw new Error("The response exceeds the reader's size limit");
    if (!response.body?.getReader) {
        const text = await response.text();
        if (new TextEncoder().encode(text).length > MAX_RESPONSE_BYTES) throw new Error("The response exceeds the reader's size limit");
        return text;
    }
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    const chunks = [];
    let bytes = 0;
    try {
        while (true) {
            const { value, done } = await reader.read();
            if (done) break;
            bytes += value.byteLength;
            if (bytes > MAX_RESPONSE_BYTES) {
                await reader.cancel("Response too large");
                throw new Error("The response exceeds the reader's size limit");
            }
            chunks.push(decoder.decode(value, { stream: true }));
        }
        chunks.push(decoder.decode());
        return chunks.join("");
    } finally {
        reader.releaseLock();
    }
}

export async function fetchJson(address) {
    const url = validateApiUrl(address);
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 20000);
    try {
        const response = await fetch(url, {
            method: "GET", mode: "cors", credentials: "omit", redirect: "error",
            cache: "no-cache", headers: { Accept: "application/json" }, signal: controller.signal,
        });
        if (!response.ok) throw new Error(`The data provider returned HTTP ${response.status}`);
        return await boundedText(response);
    } finally {
        clearTimeout(timer);
    }
}

function openDatabase() {
    if (databasePromise) return databasePromise;
    const pending = new Promise((resolve, reject) => {
        if (!globalThis.indexedDB) {
            reject(new Error("IndexedDB is unavailable. This browser cannot persist the reader's data."));
            return;
        }
        const request = indexedDB.open(DATABASE, DATABASE_VERSION);
        let settled = false;
        const timer = setTimeout(() => {
            settled = true;
            reject(new Error("The database is blocked by another tab. Close older Ember tabs and retry."));
        }, 15000);
        request.onupgradeneeded = () => {
            if (!request.result.objectStoreNames.contains(RECORDS)) request.result.createObjectStore(RECORDS);
        };
        request.onerror = () => {
            clearTimeout(timer);
            if (!settled) reject(request.error ?? new Error("Unable to open the reader database"));
            settled = true;
        };
        request.onsuccess = () => {
            clearTimeout(timer);
            const database = request.result;
            if (settled) {
                database.close();
                return;
            }
            settled = true;
            database.onversionchange = () => {
                database.close();
                databasePromise = undefined;
            };
            database.onclose = () => { databasePromise = undefined; };
            resolve(database);
        };
    });
    databasePromise = pending.catch(error => {
        databasePromise = undefined;
        throw error;
    });
    return databasePromise;
}

async function transaction(mode, work) {
    const database = await openDatabase();
    return new Promise((resolve, reject) => {
        const tx = database.transaction(RECORDS, mode);
        let answer = null;
        // Request success is not a durable write. Resolve only when the transaction commits.
        tx.oncomplete = () => resolve(answer);
        tx.onabort = () => reject(tx.error ?? new Error("The database transaction was aborted"));
        tx.onerror = () => { /* IndexedDB aborts the transaction after an unhandled request error. */ };
        try {
            work(tx.objectStore(RECORDS), value => { answer = value; });
        } catch (error) {
            tx.abort();
            reject(error);
        }
    });
}

export function databaseGet(key) {
    return transaction("readonly", (store, answer) => {
        const request = store.get(key);
        request.onsuccess = () => answer(request.result ?? null);
    });
}

export function databasePut(key, value) {
    if (typeof key !== "string" || typeof value !== "string") return Promise.reject(new Error("Database keys and values must be strings"));
    return transaction("readwrite", store => { store.put(value, key); });
}

export function databaseRemove(key) {
    return transaction("readwrite", store => { store.delete(key); });
}

export function databaseScan(prefix) {
    return transaction("readonly", (store, answer) => {
        const records = [];
        // Core keys are ASCII namespaces (item/, state/, feed/, ...). Iterating
        // from lowerBound also handles arbitrary Unicode values without a sentinel.
        const request = store.openCursor(IDBKeyRange.lowerBound(prefix));
        request.onsuccess = () => {
            const cursor = request.result;
            if (!cursor || typeof cursor.key !== "string" || !cursor.key.startsWith(prefix)) {
                answer(JSON.stringify(records));
                return;
            }
            if (typeof cursor.value !== "string") {
                request.transaction?.abort();
                return;
            }
            records.push([cursor.key, cursor.value]);
            cursor.continue();
        };
    });
}

export async function storageEstimate() {
    const estimate = await globalThis.navigator?.storage?.estimate?.();
    return { usage: estimate?.usage ?? null, quota: estimate?.quota ?? null };
}

// Keep the startup HN link and history writes in sync with the reader's u64 item IDs.
function itemId(value) {
    if (typeof value !== "string" || !/^\d+$/.test(value)) return null;
    const digits = value.replace(/^0+/, "");
    if (!digits || digits.length > 20 || (digits.length === 20 && digits > "18446744073709551615")) return null;
    return digits;
}

export function locationItem(href) {
    for (const part of href.split(/[?&#]/)) {
        const match = /^(?:item=|id=|\/?item\/)(\d+)$/.exec(part);
        const id = match && itemId(match[1]);
        if (id) return id;
    }
    return null;
}

/** Keep story links shareable and make browser Back/Forward follow the reader. */
export function setLocation(item) {
    // wasm-bindgen represents Option<String>::None as undefined.
    item = item ?? null;
    if (item !== null && itemId(item) !== item) return;
    const url = new URL(globalThis.location.href);
    if (locationItem(url.href) === item) return;
    // A legacy id= or fragment route would otherwise reopen its story on reload.
    for (const [name, value] of [...url.searchParams]) {
        const namedRoute = (name === "id" || name === "item") && itemId(value);
        const pathRoute = value === "" && locationItem(name);
        if (namedRoute || pathRoute) url.searchParams.delete(name, value);
    }
    if (url.hash) {
        const parts = url.hash.slice(1).split("&").filter(part => !locationItem(part));
        url.hash = parts.join("&");
    }
    if (item !== null) url.searchParams.set("item", item);
    globalThis.history.pushState({ emberItem: item }, "", url.href);
}
