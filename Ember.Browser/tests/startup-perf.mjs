// Run against an extracted release site. Each sample gets a new Chromium process
// and profile, then reloads that same site after its offline shell has activated.
import { open, sleep } from "../../FerroUI/scripts/browser/harness.mjs";

const [site, count = "5"] = process.argv.slice(2);
if (!site || !Number.isSafeInteger(Number(count)) || Number(count) < 1) {
    throw new Error("Usage: node startup-perf.mjs SITE [SAMPLES]");
}
const initScript = `(() => {
    const original = globalThis.fetch.bind(globalThis);
    const ids = Array.from({length: 30}, (_, i) => i + 1);
    const requests = globalThis.readerRequests = [];
    globalThis.fetch = async (input, init) => {
        const url = new URL(input instanceof Request ? input.url : input, location.href);
        if (url.hostname !== "hacker-news.firebaseio.com") return original(input, init);
        requests.push(url.pathname);
        const id = Number(url.pathname.match(/item\\/(\\d+)/)?.[1]);
        const data = id ? {id, type: "story", by: "perf", title: id === 9001 ? "Benchmark discussion" : "Feed story " + id, kids: []} : ids;
        return new Response(JSON.stringify(data), {headers: {"Content-Type": "application/json"}});
    };
    function sample() {
        const wasm = globalThis.ember;
        if (wasm?.emberState && document.querySelector("canvas")) {
            const view = JSON.parse(wasm.emberState());
            if (view?.elements.some(e => e.name === "DetailTitle" && e.text === "Benchmark discussion")) {
                globalThis.detailReadyMs = performance.now();
                return;
            }
        }
        requestAnimationFrame(sample);
    }
    requestAnimationFrame(sample);
})();`;

async function read(page) {
    await page.waitFor("globalThis.detailReadyMs > 0");
    await sleep(300);
    return page.evaluate(`(() => {
        const wasm = performance.getEntriesByType("resource").filter(e => new URL(e.name).pathname.endsWith(".wasm"));
        if (!wasm.length) throw new Error("Missing Wasm resource timing");
        return {
            wasmStartMs: Math.round(wasm[0].startTime),
            wasmEndMs: Math.round(wasm[0].responseEnd),
            detailReadyMs: Math.round(detailReadyMs),
            wasmRequests: wasm.length,
            feedRequests: readerRequests.filter(p => p !== "/v0/item/9001.json").length
        };
    })()`);
}
const cold = [], warm = [];
for (let i = 0; i < Number(count); i++) {
    const page = await open(site, { width: 390, height: 844, query: "?item=9001", initScript, navigate: false });
    try {
        await page.send("Network.enable");
        await page.send("Network.emulateNetworkConditions", {
            offline: false, latency: 50, downloadThroughput: 12 * 1024 * 1024, uploadThroughput: 12 * 1024 * 1024,
        });
        await page.navigate();
        cold.push(await read(page));
        await page.waitFor("!!navigator.serviceWorker.controller", 60000);
        const origin = await page.evaluate("performance.timeOrigin");
        await page.send("Page.reload", {});
        await page.waitFor(`performance.timeOrigin !== ${origin}`);
        warm.push(await read(page));
    } finally { await page.close(); }
}
function medians(samples) {
    return Object.fromEntries(Object.keys(samples[0]).map(key => {
        const values = samples.map(s => s[key]).sort((a, b) => a - b);
        const mid = Math.floor(values.length / 2);
        return [key, values.length % 2 ? values[mid] : (values[mid - 1] + values[mid]) / 2];
    }));
}
console.log(JSON.stringify({cold, warm, coldMedian: medians(cold), warmMedian: medians(warm)}, null, 2));
