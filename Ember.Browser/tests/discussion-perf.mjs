// Measures a real Find commit in a released Wasm reader. The timer starts at Enter
// keydown and stops on the first animation frame exposing the expected result count.
// Run with CHROME=/path/to/chrome node Ember.Browser/tests/discussion-perf.mjs SITE [SAMPLES]
import { open } from "../../FerroUI/scripts/browser/harness.mjs";

const [site, repetitions = "5"] = process.argv.slice(2);
const samples = Number(repetitions);
if (!site || !Number.isSafeInteger(samples) || samples < 1) {
    throw new Error("Specify an extracted release site and a positive sample count");
}

const initScript = `(() => {
    const root = 1, count = 4096, title = "Thread projection workload";
    const frontier = Array.from({length: count / 8}, (_, i) => 2 + i * 8);
    const original = fetch.bind(globalThis);
    globalThis.fetch = async (input, options) => {
        const url = new URL(input instanceof Request ? input.url : input, location.href);
        if (url.hostname !== "hacker-news.firebaseio.com") return original(input, options);
        const id = Number(url.pathname.match(/item\\/(\\d+)/)?.[1]);
        let data = [root];
        if (id === root) {
            data = { id, type: "story", by: "editor", title, kids: frontier, descendants: count };
        } else if (id) {
            const branch = (id - 2) % 8 === 0;
            const marker = id % 128 === 0 ? "Rare <i>needle</i>" : "notes";
            data = { id, type: "comment", by: "reader" + id % 73,
                parent: branch ? root : id - (id - 2) % 8,
                kids: branch ? Array.from({length: 7}, (_, i) => id + 1 + i) : [],
                text: "<p>Entry " + id + ": <b>layout</b> &amp; paint. " + marker + ".</p><p>Second paragraph with <code>UTF-8</code> details.</p>" };
        }
        return new Response(JSON.stringify(data), { headers: { "Content-Type": "application/json" } });
    };
})();`;

const page = await open(site, { width: 1280, height: 844, query: "?item=1", initScript });
const elements = "JSON.parse(ember.emberState()).elements";
async function click(name) {
    const selector = `${elements}.find(e => e.name === ${JSON.stringify(name)} && e.hit)`;
    await page.waitFor(selector);
    const rect = await page.evaluate(selector);
    await page.click(rect.x + rect.width / 2, rect.y + rect.height / 2);
}

async function measure(query, count) {
    await page.press("a", ["ctrl"]);
    await page.type(query);
    const expected = count ? `1 of ${count} matching comments` : "No matches in loaded comments";
    const label = `${elements}.find(e => e.name === 'FindCount')?.text`;
    await page.evaluate(`(() => {
        if (${label} === ${JSON.stringify(expected)}) throw new Error("The debounce already applied this query");
        globalThis.findLatency = null;
        addEventListener("keydown", () => {
            const start = performance.now();
            function observe() {
                if (${label} === ${JSON.stringify(expected)}) {
                    globalThis.findLatency = performance.now() - start;
                } else {
                    requestAnimationFrame(observe);
                }
            }
            requestAnimationFrame(observe);
        }, { capture: true, once: true });
    })()`);
    await page.press("Enter");
    await page.waitFor("globalThis.findLatency !== null", 10000);
    return Number((await page.evaluate("findLatency")).toFixed(2));
}

try {
    await page.waitForView();
    await page.waitFor(`${elements}.some(e => e.name === "DetailTitle" && e.text === "Thread projection workload")`);
    await click("DownloadButton");
    await page.waitFor(`${elements}.some(e => e.text === "Discussion downloaded. It is available offline in Saved stories.")`);
    const loaded = await page.evaluate(`import("./ember-platform.js").then(async m =>
        JSON.parse(await m.databaseGet("reader:state")).downloaded_threads["1"].length)`);
    if (loaded !== 4097) throw new Error(`Expected 4,097 verified items, got ${loaded}`);
    await click("FindButton");
    const firstFindMs = await measure("rare needle", 32);
    const series = [];
    for (let i = 0; i < samples; i++) {
        const times = [];
        for (const [query, matches] of [["absent", 0], ["rare", 32], ["layout", 4096], ["rare needle", 32]]) {
            times.push(await measure(query, matches));
        }
        series.push(times);
    }
    const totals = series.map(row => row.reduce((a, b) => a + b, 0)).sort((a, b) => a - b);
    const middle = Math.floor(totals.length / 2);
    const median = totals.length % 2 ? totals[middle] : (totals[middle - 1] + totals[middle]) / 2;
    console.log(JSON.stringify({ replies: loaded - 1, firstFindMs, queries: ["absent", "rare", "layout", "rare needle"],
        samplesMs: series, medianFourFindsMs: Number(median.toFixed(2)) }));
    if (page.errors.length) throw new Error(page.errors.join("\n"));
} finally {
    await page.close();
}
