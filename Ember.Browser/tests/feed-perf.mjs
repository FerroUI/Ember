// Times the frame showing each accepted 30-story page after a real pointer release.
// Data-provider responses are deterministic. IndexedDB, Rust, XAML and layout are real.
import { open } from "../../FerroUI/scripts/browser/harness.mjs";

const [site, count = "5"] = process.argv.slice(2);
const runs = Number(count);
if (!site || !Number.isSafeInteger(runs) || runs < 1) {
    throw new Error("Usage: node feed-perf.mjs SITE [SAMPLES]");
}

const setup = `(() => {
    const original = globalThis.fetch.bind(globalThis);
    const ids = Array.from({ length: 300 }, (_, i) => i + 1001);
    globalThis.fetch = async (input, init) => {
        const url = new URL(input instanceof Request ? input.url : input, location.href);
        if (url.hostname !== "hacker-news.firebaseio.com") return original(input, init);
        const id = Number(url.pathname.match(/item\\/(\\d+)/)?.[1]);
        const body = id ? { id, type: "story", by: "reader", title: "Feed story " + id, url: "https://example.org/" + id } : ids;
        return new Response(JSON.stringify(body), { headers: { "Content-Type": "application/json" } });
    };
    globalThis.measurePage = count => {
        globalThis.pageLatency = new Promise((resolve, reject) => {
            const timeout = setTimeout(() => reject(new Error("Unread count did not reach " + count)), 30000);
            addEventListener("pointerup", () => {
                const start = performance.now();
                const sample = () => {
                    const view = JSON.parse(ember.emberState());
                    if (view.elements.some(e => e.text === "Unread · " + count)) {
                        clearTimeout(timeout);
                        return resolve(performance.now() - start);
                    }
                    requestAnimationFrame(sample);
                };
                requestAnimationFrame(sample);
            }, { once: true, capture: true });
        });
    };
})()`;

const samples = [];
for (let run = 0; run < runs; run++) {
    const page = await open(site, { width: 1280, height: 800, initScript: setup });
    try {
        await page.waitForView();
        await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.text === 'Unread · 30')");
        // Keep installation of the offline shell outside the measured interaction window.
        await page.waitFor("!!navigator.serviceWorker.controller", 60000);
        const times = [];
        for (let rows = 60; rows <= 300; rows += 30) {
            const button = await page.evaluate("JSON.parse(ember.emberState()).elements.find(e => e.name === 'LoadMoreButton' && e.hit)");
            if (!button) throw new Error("Load More missing before " + rows);
            const x = button.x + button.width / 2, y = button.y + button.height / 2;
            await page.mouseMove(x, y);
            await page.evaluate(`measurePage(${rows})`);
            await page.mouseDown(x, y);
            await page.mouseUp(x, y);
            times.push(await page.evaluate("pageLatency"));
        }
        samples.push(times);
    } finally {
        await page.close();
    }
}

function median(values) {
    values.sort((a, b) => a - b);
    const mid = Math.floor(values.length / 2);
    return values.length % 2 ? values[mid] : (values[mid - 1] + values[mid]) / 2;
}
const totals = samples.map(times => times.reduce((sum, value) => sum + value, 0));
console.log(JSON.stringify({ pagesMs: samples, totalMs: totals, medianTotalMs: median([...totals]) }, null, 2));
