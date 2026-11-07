// Exercise font shaping and focus on the packaged reader before and after the
// first optional download, then repeat it from a worker-controlled offline page.
import path from "node:path";
import { open, assert } from "../../FerroUI/scripts/browser/harness.mjs";

export async function verifyCjkFallback(site, captures) {
    const title = "漢字とかな · 中文 · 한글";
    const input = "界語かな한글";
    const fixture = `(() => {
        const original = fetch.bind(globalThis);
        const title = ${JSON.stringify(title)};
        const requests = globalThis.fontRequests = [];
        const gate = new Promise(resolve => { globalThis.allowFont = resolve; });
        globalThis.fetch = async (source, options) => {
            const url = new URL(source instanceof Request ? source.url : source, location.href);
            if (url.pathname.includes("/optional-fonts/") && url.pathname.endsWith(".otf")) {
                requests.push(url.href);
                if (!sessionStorage.offlineFontTest) {
                    if (requests.length === 1) return new Response(null, { status: 503 });
                    await gate;
                }
            }
            if (url.hostname === "hacker-news.firebaseio.com") {
                if (sessionStorage.offlineFontTest) throw new TypeError("Offline");
                const id = Number(/item\\/(\\d+)/.exec(url.pathname)?.[1]);
                return Response.json(id ? { id, type: "story", by: "reader", time: 1793577600,
                    title: id === 9001 ? title : "An English headline", kids: [] } : [1]);
            }
            return original(source, options);
        };
    })();`;
    const page = await open(site, { width: 1280, height: 800, initScript: fixture });
    const element = name => `JSON.parse(ember.emberState(${JSON.stringify(name)})).elements.find(e => e.name === ${JSON.stringify(name)})`;
    const shaped = name => `(() => {
        const runs = (${element(name)})?.glyphRuns ?? [];
        const glyphs = runs.filter(run => run.family?.includes("Noto Sans CJK")).flatMap(run => run.glyphs);
        return glyphs.length >= 4 && glyphs.every(id => id !== 0) && new Set(glyphs).size >= 4;
    })()`;
    try {
        await page.waitForView();
        await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.text === 'An English headline')");
        await page.waitFor("!!navigator.serviceWorker.controller", 60000);
        const manifest = await page.evaluate('fetch("./offline-manifest.json").then(r => r.json())');
        assert(!manifest.files.some(f => f.startsWith("optional-fonts/")), "The offline shell eagerly includes the optional font");
        assert(await page.evaluate("fontRequests.length") === 0, "A Latin feed requested the CJK face");
        await page.evaluate('location.hash = "item=9001"');
        await page.waitFor(`(${element("DetailTitle")})?.text === ${JSON.stringify(title)}`);
        await page.waitFor("fontRequests.length === 1", 20000);
        const box = await page.evaluate(element("SearchBox"));
        await page.click(box.x + box.width / 2, box.y + box.height / 2);
        await page.type(input);
        await page.press("ArrowLeft");
        await page.press("ArrowLeft", ["shift"]);
        const before = await page.evaluate(element("SearchBox"));
        assert(before.text === input && before.focused && before.selectionStart !== before.selectionEnd,
            "The editor must contain a focused, non-empty selection before the font arrives");
        assert(!await page.evaluate(shaped("SearchBox")), "An unavailable face should not report successful shaping");
        await page.evaluate("dispatchEvent(new Event('online'))");
        await page.waitFor("fontRequests.length === 2", 12000);
        await page.evaluate("dispatchEvent(new Event('online')); allowFont()");

        await page.waitFor(shaped("DetailTitle"), 30000);
        await page.waitFor(shaped("SearchBox"), 30000);
        const after = await page.evaluate(element("SearchBox"));
        assert(after.text === before.text && after.focused && after.caretIndex === before.caretIndex &&
            after.selectionStart === before.selectionStart && after.selectionEnd === before.selectionEnd,
            "Installing the fallback disturbed the focused editor");
        assert(await page.evaluate("fontRequests.length") === 2, "Multiple text surfaces or online events duplicated the retry");
        await page.screenshot(path.join(captures, "desktop-multilingual-reader.png"));

        await page.evaluate("sessionStorage.offlineFontTest = '1'");
        await page.send("Network.enable");
        await page.send("Network.emulateNetworkConditions", { offline: true, latency: 0, downloadThroughput: 0, uploadThroughput: 0 });
        const origin = await page.evaluate("performance.timeOrigin");
        await page.send("Page.reload", {});
        await page.waitFor(`performance.timeOrigin !== ${origin} && !!globalThis.ember?.emberState`);
        await page.waitFor(shaped("DetailTitle"), 30000);
        assert(await page.evaluate("fontRequests.length") === 0, "Offline shaping tried to download an already verified face");
        assert(!page.errors.length, page.errors.join("\n"));
    } finally { await page.close(); }
}
