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

// IndexedDB orders string keys by UTF-16 code units. A true exclusive end
// includes suffixes containing U+FFFF, without inspecting unrelated keys.
function prefixRange(prefix) {
    for (let i = prefix.length - 1; i >= 0; --i) {
        const unit = prefix.charCodeAt(i);
        if (unit !== 0xffff) {
            return IDBKeyRange.bound(prefix, prefix.slice(0, i) + String.fromCharCode(unit + 1), false, true);
        }
    }
    // Binary keys sort immediately after all string keys.
    try {
        return IDBKeyRange.bound(prefix, new Uint8Array(0), false, true);
    } catch (error) {
        // Some implementations reject zero-length binary keys. For those,
        // a single zero byte is the smallest admissible binary key.
        if (error.name !== "DataError") throw error;
        return IDBKeyRange.bound(prefix, new Uint8Array([0]), false, true);
    }
}

export function databaseGetMany(keys) {
    return transaction("readonly", (store, answer) => {
        const values = Array(keys.length).fill(null);
        answer(values);
        keys.forEach((key, index) => {
            const request = store.get(key);
            request.onsuccess = () => { values[index] = request.result ?? null; };
        });
    });
}

export function databaseRemoveMany(keys) {
    return transaction("readwrite", store => {
        for (const key of keys) store.delete(key);
    });
}

export function databaseRemovePrefix(prefix) {
    return transaction("readwrite", store => { store.delete(prefixRange(prefix)); });
}

export function databaseCount(prefix) {
    return transaction("readonly", (store, answer) => {
        const request = store.count(prefixRange(prefix));
        request.onsuccess = () => answer(request.result);
    });
}

// A cursor posts a separate success callback for every record. Pair bounded
// getAllKeys/getAll requests on the same read snapshot; enqueue each next
// page before leaving the callback so the transaction stays active.
function scanRecords(prefix, project) {
    return transaction("readonly", (store, answer) => {
        const records = [];
        const range = prefixRange(prefix);
        const pageSize = 64;
        function readPage(page) {
            const keys = store.getAllKeys(page, pageSize);
            const values = store.getAll(page, pageSize);
            values.onsuccess = () => {
                const pageKeys = keys.result;
                const pageValues = values.result;
                for (let i = 0; i < pageKeys.length; i++) {
                    const value = pageValues[i];
                    if (typeof value !== "string") {
                        store.transaction.abort();
                        return;
                    }
                    records.push([pageKeys[i], project(value)]);
                }
                if (pageKeys.length === pageSize) {
                    readPage(IDBKeyRange.bound(pageKeys[pageSize - 1], range.upper, true, true));
                } else {
                    answer(JSON.stringify(records));
                }
            };
        }
        readPage(range);
    });
}

export function databaseScan(prefix) {
    return scanRecords(prefix, value => value);
}

export function databaseSizes(prefix) {
    const encoder = new TextEncoder();
    return scanRecords(prefix, value => /[^\x00-\x7f]/.test(value) ? encoder.encode(value).byteLength : value.length);
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

function discussionAddress(href, item) {
    const url = new URL(href);
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
    return url.href;
}

// Browser history survives a reload, while reader visits belong to one running
// document. Stamp entries lazily so a previous document's keys cannot alias ours.
const historySession = Array.from(crypto.getRandomValues(new Uint32Array(4))).join("-");
let nextEntry = 1;
let lastAddress;
let lastEntry = 0;
let lastPosition;
let previousAddress;
let reportEntry;
let pendingBack = null;
const queuedChanges = [];

function currentEntry() {
    const history = globalThis.history, href = globalThis.location.href;
    const state = history.state;
    if (state?.emberSession === historySession && state.emberHref === href && state.emberEntry > 0) {
        return state.emberEntry;
    }
    const id = nextEntry++;
    history.replaceState({ ...state, emberSession: historySession, emberEntry: id, emberHref: href,
        emberPreviousHref: state?.emberHref === href ? state.emberPreviousHref : null,
        emberPreviousEntry: 0 }, "", href);
    return id;
}

function rememberAddress() {
    lastAddress = globalThis.location.href;
    lastEntry = currentEntry();
    const state = globalThis.history.state;
    lastPosition = globalThis.navigation?.currentEntry?.index ?? state.emberPosition;
    previousAddress = state.emberPreviousHref;
}

function ownedPredecessor() {
    const href = globalThis.location.href, state = globalThis.history.state;
    if (state?.emberHref !== href || typeof state.emberPreviousHref !== "string") return null;
    try {
        const previous = new URL(state.emberPreviousHref), current = new URL(href);
        if (previous.origin === current.origin && previous.pathname === current.pathname) {
            return { href: previous.href, entry: state.emberPreviousEntry || 0 };
        }
    } catch {
        // An old or foreign history.state can have an unusable predecessor.
    }
    return null;
}

// A tab switch can discard the native route stack. Only traverse a predecessor
// written by this reader; a direct permalink must not send the user off-site.
export function requestBrowserBack() {
    if (!reportEntry) return false;
    if (pendingBack) return pendingBack.navigate;
    const previous = ownedPredecessor();
    if (!previous) return false;
    pendingBack = { ...previous, navigate: true };
    globalThis.history.back();
    return true;
}

// popstate and hashchange can both describe the same traversal. When Ember
// initiated Back, its restored view can be a profile or an unfinished form.
export function observeLocation(navigate, committed = () => {}) {
    reportEntry = committed;
    rememberAddress();
    committed(lastEntry);
    const changed = () => {
        const href = globalThis.location.href;
        const entry = currentEntry();
        if (href === lastAddress && entry === lastEntry) return;
        const from = lastPosition, previous = previousAddress;
        rememberAddress();
        const backward = Number.isInteger(from) && Number.isInteger(lastPosition)
            ? lastPosition < from : href === previous;
        const pending = pendingBack;
        const restored = pending?.href === href && (!pending.entry || pending.entry === entry);
        pendingBack = null;
        if (!restored || (pending.navigate && !queuedChanges.length)) {
            queuedChanges.length = 0;
            navigate(href, entry, backward || restored);
            return;
        }
        committed(entry);
        while (queuedChanges.length && !pendingBack) {
            const [item, back] = queuedChanges.shift();
            setLocation(item, back);
        }
    };
    globalThis.addEventListener("popstate", changed);
    globalThis.addEventListener("hashchange", changed);
}

/** Keep reader Back on the browser's existing stack, including its Forward entries. */
export function setLocation(item, back = false) {
    item = item ?? null;
    if (item !== null && itemId(item) !== item) return;
    if (pendingBack) {
        queuedChanges.push([item, back]);
        reportEntry?.(0);
        return;
    }
    const href = globalThis.location.href;
    if (locationItem(href) === item) {
        if (reportEntry) reportEntry(currentEntry());
        return;
    }
    const history = globalThis.history;
    const address = discussionAddress(href, item);
    const entry = history.state;
    const previous = back && reportEntry ? ownedPredecessor() : null;
    if (previous && locationItem(previous.href) === item) {
        pendingBack = { ...previous, navigate: false };
        reportEntry?.(pendingBack.entry);
        history.back();
        return;
    }
    const key = nextEntry++;
    const position = entry?.emberPosition ?? 0;
    const stamp = { emberSession: historySession, emberEntry: key, emberHref: address,
        emberItem: item, emberPosition: back ? position : position + 1 };
    if (back) {
        // A direct permalink or a legacy entry has no known Ember predecessor.
        // Keep the preceding site's entry intact when returning to the feed.
        history.replaceState({ ...entry, ...stamp, emberPreviousHref: null, emberPreviousEntry: 0 }, "", address);
    } else {
        const previousEntry = reportEntry ? currentEntry() : 0;
        history.pushState({ ...stamp, emberPreviousHref: href, emberPreviousEntry: previousEntry }, "", address);
    }
    if (reportEntry) {
        rememberAddress();
        reportEntry(key);
    }
}

// This face is fetched only when the reader needs a glyph from its CJK fallback.
// The packager uses the same immutable identity. No text enters an asset request.
export const cjkFontAsset = Object.freeze({
    path: "optional-fonts/NotoSansCJKsc-Regular.2c76254f6fc379fd.otf",
    size: 16437364,
    sha256: "2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b",
});

async function readVerifiedFont(response, asset) {
    if (response.status !== 200 || !response.body) throw new Error("The optional reader font is unavailable.");
    const bytes = new Uint8Array(asset.size);
    const reader = response.body.getReader();
    let offset = 0;
    try {
        for (;;) {
            const {done, value} = await reader.read();
            if (done) break;
            if (value.byteLength > bytes.length - offset) {
                await reader.cancel();
                throw new Error("The optional reader font exceeds its expected size.");
            }
            bytes.set(value, offset);
            offset += value.byteLength;
        }
    } finally {
        reader.releaseLock();
    }
    if (offset !== bytes.length || bytes[0] !== 0x4f || bytes[1] !== 0x54 || bytes[2] !== 0x54 || bytes[3] !== 0x4f) {
        throw new Error("The optional reader font is incomplete or invalid.");
    }
    const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
    const digest = Array.from(hash, byte => byte.toString(16).padStart(2, "0")).join("");
    if (digest !== asset.sha256) throw new Error("The optional reader font failed its integrity check.");
    return bytes;
}

export function createFontLoader(asset, moduleUrl) {
    const root = new URL("./", moduleUrl);
    const url = new URL(asset.path, root).href;
    const name = "ember-fonts:" + root.href;
    let pending;
    async function read() {
        let cache;
        try {
            cache = await globalThis.caches?.open(name);
            const cached = await cache?.match(url);
            if (cached) {
                try {
                    return await readVerifiedFont(cached, asset);
                } catch {
                    await cache.delete(url);
                }
            }
        } catch {
            // CacheStorage may be blocked in private browsing. Rendering can still proceed.
        }
        const response = await fetch(url, {
            mode: "same-origin", credentials: "omit", redirect: "error", referrerPolicy: "no-referrer", cache: "no-store",
        });
        const bytes = await readVerifiedFont(response, asset);
        if (cache) {
            try {
                await cache.put(url, new Response(bytes, {headers: {"Content-Type": "font/otf"}}));
                for (const key of await cache.keys()) {
                    if (key.url !== url) await cache.delete(key);
                }
            } catch {
                // Quota failures must not prevent use of the verified font in this session.
            }
        }
        return bytes;
    }
    return () => {
        // Coalesce concurrent glyph misses, then release the JS buffer after the native copy.
        // A failed attempt can be retried after connectivity or storage access changes.
        pending ??= read().finally(() => { pending = undefined; });
        return pending;
    };
}

export const loadCjkFallbackFont = createFontLoader(cjkFontAsset, import.meta.url);
