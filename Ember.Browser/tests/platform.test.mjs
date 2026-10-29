import assert from "node:assert/strict";
import { after, test } from "node:test";
import "fake-indexeddb/auto";
import { databaseGet, databasePut, databaseRemove, databaseScan, fetchJson, setLocation, validateApiUrl } from "../wwwroot/ember-platform.js";

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
