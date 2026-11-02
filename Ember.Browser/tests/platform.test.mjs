import assert from "node:assert/strict";
import { after, test } from "node:test";
import "fake-indexeddb/auto";
import { databaseGet, databasePut, databaseRemove, databaseScan, fetchJson, locationItem, observeLocation, setLocation, validateApiUrl } from "../wwwroot/ember-platform.js";

const originalFetch = globalThis.fetch;
after(() => { globalThis.fetch = originalFetch; });

test("requests are confined to documented provider HTTPS endpoints", () => {
    assert.equal(validateApiUrl("https://hacker-news.firebaseio.com/v0/item/8863.json"), "https://hacker-news.firebaseio.com/v0/item/8863.json");
    assert.equal(validateApiUrl("https://hn.algolia.com/api/v1/search?query=Rust%20%26%20UI"), "https://hn.algolia.com/api/v1/search?query=Rust%20%26%20UI");
    for (const url of [
        "http://hacker-news.firebaseio.com/v0/item/1.json", "https://hn.algolia.com.evil.example/api/v1/search",
        "https://hn.algolia.com/api/v1/../../private", "https://password@hn.algolia.com/api/v1/search",
        "javascript:alert(1)", "https://hn.algolia.com:8443/api/v1/search", "https://example.org/article",
    ]) assert.throws(() => validateApiUrl(url));
});

test("fetch strips credentials, preserves JSON null and reports status failures", async () => {
    globalThis.fetch = async (_url, options) => {
        assert.equal(options.credentials, "omit");
        assert.equal(options.redirect, "error");
        assert.equal(options.method, "GET");
        return new Response("null", { status: 200 });
    };
    assert.equal(await fetchJson("https://hacker-news.firebaseio.com/v0/item/42.json"), "null");
    globalThis.fetch = async () => new Response("unavailable", { status: 503 });
    await assert.rejects(fetchJson("https://hacker-news.firebaseio.com/v0/topstories.json"), /HTTP 503/);
});

test("declared and streamed oversized responses are rejected", async () => {
    globalThis.fetch = async () => new Response("{}", { headers: { "content-length": String(9 * 1024 * 1024) } });
    await assert.rejects(fetchJson("https://hacker-news.firebaseio.com/v0/topstories.json"), /size limit/);
    globalThis.fetch = async () => new Response(new ReadableStream({
        start(controller) {
            controller.enqueue(new Uint8Array(8 * 1024 * 1024));
            controller.enqueue(new Uint8Array(1));
            controller.close();
        },
    }));
    await assert.rejects(fetchJson("https://hacker-news.firebaseio.com/v0/topstories.json"), /size limit/);
});

test("IndexedDB commits writes and scans exact namespaces in key order", async () => {
    await databasePut("test/item/2", JSON.stringify({ title: "Two" }));
    await databasePut("test/item/1", JSON.stringify({ title: "One" }));
    await databasePut("test/item-adjacent/3", "unrelated");
    assert.equal(await databaseGet("test/item/2"), '{"title":"Two"}');
    assert.equal(await databaseGet("test/missing"), null);
    assert.deepEqual(JSON.parse(await databaseScan("test/item/")), [
        ["test/item/1", '{"title":"One"}'], ["test/item/2", '{"title":"Two"}'],
    ]);
    await databaseRemove("test/item/1");
    assert.equal(await databaseGet("test/item/1"), null);
    assert.equal(JSON.parse(await databaseScan("test/item/")).length, 1);
});

test("independent concurrent writes do not lose local state", async () => {
    await Promise.all(Array.from({ length: 40 }, (_, index) => databasePut(`test/saved/${index.toString().padStart(2, "0")}`, String(index))));
    const saved = JSON.parse(await databaseScan("test/saved/"));
    assert.equal(saved.length, 40);
    assert.deepEqual(saved.map(([, value]) => Number(value)), Array.from({ length: 40 }, (_, index) => index));
});

test("invalid writes reject without reporting a saved state", async () => {
    await assert.rejects(databasePut("test/invalid", { nested: true }), /must be strings/);
    assert.equal(await databaseGet("test/invalid"), null);
});

test("a transaction abort rejects a write even after its request succeeded", async () => {
    const put = IDBObjectStore.prototype.put;
    IDBObjectStore.prototype.put = function (...args) {
        const request = put.apply(this, args);
        request.addEventListener("success", () => this.transaction.abort());
        return request;
    };
    try {
        await assert.rejects(databasePut("test/aborted", "must not persist"), /aborted/);
    } finally {
        IDBObjectStore.prototype.put = put;
    }
    assert.equal(await databaseGet("test/aborted"), null);
});

test("reader navigation retains rendering options and does not duplicate browser history", () => {
    const location = globalThis.location;
    const history = globalThis.history;
    const locations = [];
    globalThis.location = { href: "https://example.test/ember/?Renderer=Vello" };
    globalThis.history = { pushState: (_state, _title, address) => { locations.push(address); globalThis.location.href = address; } };
    try {
        setLocation("8863");
        setLocation("8863");
        assert.equal(locations.length, 1);
        assert.equal(new URL(locations[0]).searchParams.get("Renderer"), "Vello");
        assert.equal(new URL(locations[0]).searchParams.get("item"), "8863");
        setLocation("javascript:alert(1)");
        assert.equal(locations.length, 1);
        setLocation(null);
        setLocation(undefined);
        assert.equal(locations.length, 2);
        assert.equal(new URL(locations[1]).searchParams.get("item"), null);
        for (const hash of ["#item=42", "#item/42", "#/item/42"]) {
            globalThis.location.href = "https://example.test/ember/?Renderer=Vello" + hash;
            const count = locations.length;
            setLocation("42");
            assert.equal(locations.length, count, "Opening an existing hash item must not add history");
            setLocation(null);
            assert.equal(new URL(locations.at(-1)).hash, "");
            assert.equal(new URL(locations.at(-1)).searchParams.get("Renderer"), "Vello");
        }
    } finally {
        if (location === undefined) delete globalThis.location; else globalThis.location = location;
        if (history === undefined) delete globalThis.history; else globalThis.history = history;
    }
});

test("legacy discussion URLs can return to the feed and keep unrelated URL state", () => {
    const original = { location: globalThis.location, history: globalThis.history };
    const base = "https://example.test/Ember/?Renderer=Vello";
    globalThis.history = { pushState: (_state, _title, href) => { globalThis.location.href = href; } };
    try {
        for (const route of ["&id=00042", "&item/42", "#id=42", "#/item/42", "#item=42&debug", "&id=42&item=7#item/9"]) {
            globalThis.location = { href: base + route };
            assert.equal(locationItem(globalThis.location.href), "42");
            setLocation("42");
            assert.equal(globalThis.location.href, base + route);
            setLocation("64");
            assert.equal(locationItem(globalThis.location.href), "64");
            setLocation(null);
            assert.equal(globalThis.location.href, base + (route.includes("debug") ? "#debug" : ""));
        }
        assert.equal(locationItem(base + "&id=18446744073709551615"), "18446744073709551615");
        assert.equal(locationItem(base + "&id=18446744073709551616"), null);
        assert.equal(locationItem(base + "&id=0"), null);
    } finally {
        for (const [key, value] of Object.entries(original)) {
            if (value === undefined) delete globalThis[key]; else globalThis[key] = value;
        }
    }
});

test("reader Back reuses owned entries, queues rapid actions and replaces an initial permalink", t => {
    const start = "https://example.test/read/?theme=dark#id=5&debug";
    const entries = [{ href: start, state: { renderer: "native" } }];
    const events = new EventTarget();
    let position = 0;
    let moving = false;
    const location = { href: start };
    const history = {
        get state() { return entries[position].state; },
        pushState(state, _, href) {
            entries.splice(++position, Infinity, { state, href });
            location.href = href;
        },
        replaceState(state, _, href) {
            entries[position] = { state, href };
            location.href = href;
        },
        back() {
            assert.ok(position > 0 && !moving, "Only one owned traversal may be pending");
            moving = true;
        },
    };
    const traverse = delta => {
        moving = false;
        position += delta;
        location.href = entries[position].href;
        events.dispatchEvent(new Event("popstate"));
        events.dispatchEvent(new Event("hashchange"));
    };
    const original = new Map(["location", "history", "addEventListener"].map(key => [key, globalThis[key]]));
    Object.assign(globalThis, { location, history, addEventListener: events.addEventListener.bind(events) });
    t.after(() => {
        for (const [key, value] of original) {
            if (value === undefined) delete globalThis[key]; else globalThis[key] = value;
        }
    });
    const visits = [];
    observeLocation(href => visits.push(locationItem(href)));
    setLocation(null, true);
    assert.equal(entries.length, 1);
    assert.equal(location.href, "https://example.test/read/?theme=dark#debug");
    assert.equal(history.state.renderer, "native");
    setLocation("10");
    setLocation("20");
    setLocation("10", true);
    setLocation(null, true);
    setLocation("30");
    assert.equal(entries.length, 3);
    traverse(-1);
    assert.ok(moving);
    traverse(-1);
    assert.equal(locationItem(location.href), "30");
    assert.equal(entries.length, 2, "Only opening a new item should truncate Forward");
    assert.deepEqual(visits, [], "Reader-initiated traversals must preserve restored views");
    traverse(-1);
    traverse(1);
    assert.deepEqual(visits, [null, "30"]);
});

test("ordered batch reads use one transaction and range cleanup preserves neighboring keys", async () => {
    const { databaseCount, databaseGetMany, databaseRemoveMany, databaseRemovePrefix } = await import("../wwwroot/ember-platform.js");
    const prefix = "batch/%_/\uffff";
    const keys = [prefix + "3", prefix + "😀", prefix + "\uffffZ"];
    for (const key of [...keys, "batch/%_/X"]) await databasePut(key, key);
    const original = IDBDatabase.prototype.transaction;
    let transactions = 0;
    IDBDatabase.prototype.transaction = function (...args) {
        transactions++;
        return original.apply(this, args);
    };
    try {
        assert.deepEqual(await databaseGetMany([keys[2], prefix + "missing", keys[0], keys[2]]),
            [keys[2], null, keys[0], keys[2]]);
        assert.equal(transactions, 1);
        assert.equal(await databaseCount(prefix), 3);
        await databaseRemoveMany([keys[0], keys[1]]);
        assert.equal(transactions, 3);
        assert.equal(await databaseCount(prefix), 1);
        await databaseRemovePrefix(prefix);
        assert.equal(await databaseCount(prefix), 0);
        assert.equal(await databaseGet("batch/%_/X"), "batch/%_/X");
    } finally {
        IDBDatabase.prototype.transaction = original;
    }
});

test("paged size scans return ordered UTF-8 lengths without returning cached documents", async () => {
    const { databaseCount, databaseRemoveMany, databaseRemovePrefix, databaseSizes } = await import("../wwwroot/ember-platform.js");
    const prefix = "\uffff";
    const entries = Array.from({ length: 139 }, (_, i) => [prefix + i.toString().padStart(3, "0"), "Ł😀" + i]);
    await Promise.all(entries.map(([key, value]) => databasePut(key, value)));
    assert.equal(await databaseCount(prefix), entries.length);
    assert.deepEqual(JSON.parse(await databaseSizes(prefix)), entries.map(([key, value]) => [key, Buffer.byteLength(value)]));
    assert.deepEqual(JSON.parse(await databaseScan(prefix)), entries);
    // An invalid key after the first queued delete aborts the entire transaction.
    await assert.rejects(databaseRemoveMany([entries[0][0], null]));
    assert.equal(await databaseCount(prefix), entries.length);
    await databaseRemovePrefix(prefix);
    assert.equal(await databaseCount(prefix), 0);
});
