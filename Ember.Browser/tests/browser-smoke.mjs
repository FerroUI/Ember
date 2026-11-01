// Exercises the compiled FerroUI canvas with actual pointer/keyboard input.
// Usage: CHROME=/path/to/chrome node Ember.Browser/tests/browser-smoke.mjs [site]
// Only the HTTP boundary is deterministic; rendering, navigation, DB and controls are real.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { open, run, assert, sleep, colours } from "../../FerroUI/scripts/browser/harness.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
// Build output follows CARGO_TARGET_DIR, as scripts/build-browser.sh does.
const targetDir = process.env.CARGO_TARGET_DIR ? path.resolve(process.env.CARGO_TARGET_DIR) : path.join(root, "target");
const site = process.argv[2] ?? path.join(targetDir, "browser/ember-browser");
const captures = path.join(targetDir, "ember-browser-smoke");
const title = "Show HN: A quieter place for curious minds";
const fixtures = {
    101: { id: 101, type: "story", title, by: "maker", time: 1792573200, score: 284, descendants: 3, url: "https://example.org/ember", kids: [201, 203] },
    102: { id: 102, type: "story", title: "The surprising elegance of small compilers", by: "compilerfan", time: 1792570800, score: 179, descendants: 0, url: "https://example.org/compilers" },
    103: { id: 103, type: "story", title: "Ask HN: What have you learned by building something slowly?", by: "curious", time: 1792567200, score: 142, descendants: 0, text: "<p>What did the process teach you?</p>" },
    201: { id: 201, type: "comment", by: "reader", parent: 101, time: 1792573800, kids: [202], text: "<p>Small details make a <b>meaningful difference</b>. A native reading experience feels wonderful.</p><p>See <a href=\"https://example.org/details\">the design notes</a>.</p>" },
    202: { id: 202, type: "comment", by: "maker", parent: 201, time: 1792574000, text: "<p>Thank you! Here is the tiny core:</p><pre><code>fn main() {\n    println!(\"Stay curious.\");\n}</code></pre>" },
    203: { id: 203, type: "comment", by: "engineer", parent: 101, time: 1792574400, text: "<blockquote>Make the important things easy.</blockquote><p>I keep the design notes available offline.</p><script>globalThis.__emberUnsafeHtml = true</script>" },
};
const initScript = `(() => {
    const original = globalThis.fetch.bind(globalThis);
    const items = ${JSON.stringify(fixtures)};
    globalThis.fetch = async (input, options) => {
        const url = new URL(typeof input === "string" || input instanceof URL ? input : input.url, location.href);
        if (!["hacker-news.firebaseio.com", "hn.algolia.com"].includes(url.hostname)) return original(input, options);
        globalThis.__emberApiRequests = (globalThis.__emberApiRequests ?? 0) + 1;
        if (sessionStorage.getItem("ember-test-offline") === "1") throw new TypeError("Simulated offline connection");
        let value = null;
        if (url.hostname === "hn.algolia.com") {
            globalThis.__emberLastSearch = url.href;
            value = { hits: [{ objectID: "101", title: items[101].title, author: "maker", points: 284, num_comments: 3, created_at_i: 1792573200, url: items[101].url }], nbHits: 1, nbPages: 1, page: 0, hitsPerPage: 30 };
        else if (/stories\\.json$/.test(url.pathname)) value = [101, 102, 103];
        else if (/\\/item\\//.test(url.pathname)) value = items[Number(url.pathname.split("/").pop().replace(".json", ""))] ?? null;
        else if (/\\/user\\//.test(url.pathname)) value = { id: "maker", karma: 1234, created: 1500000000, about: "<p>I build small things.</p>", submitted: [101, 202] };
        return new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
    };
})();`;

async function state(page) { return JSON.parse(await page.evaluate("ember.emberState()")); }
async function waitText(page, text) {
    await page.waitFor(`globalThis.ember?.emberState && JSON.parse(ember.emberState())?.elements.some(e => e.text === ${JSON.stringify(text)})`, 30000);
}
async function click(page, predicate, description) {
    const element = (await state(page)).elements.find(element => element.hit && predicate(element));
    assert(element, `No visible hit target: ${description}`);
    await page.click(element.x + element.width / 2, element.y + element.height / 2);
    await sleep(250);
}
const byName = (page, name) => click(page, element => element.name === name, name);
const byText = (page, text) => click(page, element => element.text === text, text);
const byAutomation = (page, name) => click(page, element => element.automationName === name, name);

async function touchDrag(page, x, y, endX, endY) {
    await page.send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 });
    const point = (x, y) => ({ x, y, id: 1, radiusX: 6, radiusY: 6, force: 1 });
    await page.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [point(x, y)] });
    for (let step = 1; step <= 10; ++step) {
        await page.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [point(x + (endX - x) * step / 10, y + (endY - y) * step / 10)] });
        await sleep(20);
    }
    await page.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await sleep(300);
}

async function reveal(page, predicate, direction = 1) {
    for (let attempt = 0; attempt < 12; ++attempt) {
        const snapshot = await state(page);
        if (snapshot.elements.some(element => element.hit && predicate(element))) return;
        await page.wheel(snapshot.width - 110, snapshot.height * 0.65, 0, 320 * direction);
    }
    throw new Error("Could not scroll the target into view");
}

async function applyTheme(page, index) {
    await reveal(page, element => element.automationName === "Theme", -1);
    await byAutomation(page, "Theme");
    await page.press("Home");
    for (let step = 0; step < index; ++step) await page.press("ArrowDown");
    await page.press("Enter");
    const theme = (await state(page)).elements.find(element => element.automationName === "Theme");
    assert(theme?.selectedIndex === index, `Theme selection is ${theme?.selectedIndex}, expected ${index}`);
    if (index === 3) {
        for (const [label, value] of [["Accent color", "#4EB9A4"], ["Custom background", "#172126"]]) {
            await reveal(page, element => element.automationName === label);
            await byAutomation(page, label);
            await page.press("a", ["ctrl"]);
            await page.type(value);
        }
    }
    await reveal(page, element => element.automationName === "Apply preferences");
    await byAutomation(page, "Apply preferences");
    await sleep(350);
}

async function reviewLocalData(page) {
    const revealAction = async name => {
        await reveal(page, element => element.automationName === name);
        await byAutomation(page, name);
    };
    await revealAction("Import this backup");
    await waitText(page, "Paste an Ember backup first.");
    // Validation puts focus back in the JSON field so the correction can be typed directly.
    await page.type("{}");
    await revealAction("Import this backup");
    await waitText(page, "Replace reading data");
    await byAutomation(page, "Cancel Import this backup");
    assert(!(await state(page)).elements.some(e => e.automationName === "Replace reading data"), "Cancelled restore remained active");

    await reveal(page, element => element.text === "Clear local data");
    await byText(page, "Clear local data");
    await revealAction("Clear cached content");
    await waitText(page, "Remove cached content");
    await page.press("Escape");
    assert(!(await state(page)).elements.some(e => e.automationName === "Remove cached content"), "Escape did not cancel cache removal");
}

async function verifyProfileActivity(page) {
    await byAutomation(page, "View maker’s profile");
    const comment = e => e.automationName?.startsWith("Thank you! Here is the tiny core:");
    await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.hit && e.automationName?.startsWith('Thank you! Here is the tiny core:'))");
    await page.screenshot(path.join(captures, "desktop-profile.png"));
    await page.resize(390, 844);
    await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.name === 'MobileMore' && e.hit)");
    await page.screenshot(path.join(captures, "phone-profile.png"));
    await click(page, comment, "maker’s recent comment");
    await page.waitFor("new URL(location.href).searchParams.get('item') === '202'");
    await waitText(page, "Comment by maker");
    await byName(page, "BackButton");
    await waitText(page, "Recent activity");
    await byAutomation(page, "All comments");
    await page.waitFor("globalThis.__emberLastSearch?.includes('author_maker')");
    const search = new URL(await page.evaluate("globalThis.__emberLastSearch"));
    assert(search.searchParams.get("tags") === "comment,author_maker" && search.pathname.endsWith("/search_by_date"),
        "Profile comments must request recent results scoped to this author");
    await page.resize(1440, 960);
    await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.name === 'NavMore' && e.hit)");
    await byName(page, "NavMore");
    await waitText(page, "Your library");
    await page.screenshot(path.join(captures, "desktop-explore.png"));
}

async function start(width, height) {
    const page = await open(site, { width, height, initScript });
    try { await page.waitForView(); await waitText(page, title); return page; }
    catch (error) { await failure(page, `startup-${width}`); await page.close(); throw error; }
}

async function failure(page, name) {
    fs.mkdirSync(captures, { recursive: true });
    await page.screenshot(path.join(captures, `${name}-failure.png`)).catch(() => {});
    const snapshot = await state(page).catch(() => null);
    fs.writeFileSync(path.join(captures, `${name}-failure.json`), JSON.stringify({ snapshot, log: page.log }, null, 2));
}

await run([
    ["startup recovery keeps the discussion reachable by keyboard on a narrow screen", async () => {
        const page = await open(site, {
            width: 320, height: 420, query: "?item=08",
            initScript: 'Object.defineProperty(globalThis, "WebAssembly", {value: undefined})',
        });
        try {
            await page.waitFor("document.activeElement?.id === 'error-title'");
            assert(await page.evaluate("document.querySelector('#out p').textContent.includes('WebAssembly')"), "Missing capability explanation");
            await page.press("Tab");
            assert(await page.evaluate("document.activeElement?.textContent === 'Reload reader'"), "Reload is unreachable by keyboard");
            await page.press("Tab");
            const link = await page.evaluate(`(() => {
                const { href, textContent } = document.activeElement;
                const { width, height, left, right } = document.activeElement.getBoundingClientRect();
                return { href, textContent, width, height, left, right };
            })()`);
            assert(link.href === "https://news.ycombinator.com/item?id=8", "The fallback lost the discussion ID");
            assert(link.height >= 44 && link.left >= 0 && link.right <= 320, "Fallback link is clipped or too small");
        } finally { await page.close(); }
    }],
    ["desktop renders and opens a rich thread with shareable history", async () => {
        const page = await start(1440, 960);
        try {
            const screen = await state(page);
            assert(screen.elements.some(element => element.name === "NavTop"), "Desktop navigation rail is absent");
            await byName(page, "WelcomeOpen");
            await page.waitFor("location.search.includes('item=101')");
            await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.name === 'FindButton')");
            assert(!(await page.evaluate("globalThis.__emberUnsafeHtml === true")), "HN content executed script");
            await byName(page, "FocusButton");
            assert(!(await state(page)).elements.some(e => e.name === "NavTop"), "The feed rail remains visible in focus reading");
            await byAutomation(page, "Exit focus reading");
            await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.name === 'NavTop')");
            await byName(page, "FindButton");
            await byAutomation(page, "Find in thread");
            await page.type("design notes");
            await waitText(page, "1 of 2 matching comments");
            await page.press("Enter");
            await waitText(page, "2 of 2 matching comments");
            await page.press("Enter", ["shift"]);
            await waitText(page, "1 of 2 matching comments");
            await page.press("Escape");
            assert(!(await state(page)).elements.some(e => e.name === "ThreadFind"), "Escape should close find without leaving the discussion");
            const shot = await page.screenshot(path.join(captures, "desktop-thread.png"));
            assert(colours(shot) > 40, "The screenshot does not contain a rendered reader");
            await verifyProfileActivity(page);
            await byName(page, "ThemeButton");
            await waitText(page, "Appearance & reading");
            await page.screenshot(path.join(captures, "desktop-settings.png"));
            await applyTheme(page, 2);
            const dark = await page.screenshot(path.join(captures, "desktop-dark.png"));
            assert(dark.pixel(4, 4).reduce((sum, channel) => sum + channel, 0) < shot.pixel(4, 4).reduce((sum, channel) => sum + channel, 0) - 80, "Dark theme did not change the reader surface");
            await applyTheme(page, 3);
            await page.screenshot(path.join(captures, "desktop-custom.png"));
            await reviewLocalData(page);
            assert(page.errors.length === 0, page.errors.join("\n"));
        } catch (error) { await failure(page, "desktop"); throw error; }
        finally { await page.close(); }
    }],
    ["phone touch gestures, navigation, downloads, persistence, and offline reload", async () => {
        const page = await start(390, 844);
        try {
            const screen = await state(page);
            const targets = screen.elements.filter(element => /^Mobile/.test(element.name ?? ""));
            assert(targets.length === 5, `Expected five mobile navigation targets, got ${targets.length}`);
            assert(targets.every(element => element.height >= 44 && element.width >= 44), "A mobile navigation target is too small");
            await page.screenshot(path.join(captures, "phone-feed.png"));
            const headline = screen.elements.find(element => element.hit && element.text === title);
            assert(headline, "The first story has no touch target");
            const fingerX = Math.min(screen.width - 35, headline.x + headline.width - 15);
            const fingerY = headline.y + headline.height / 2;
            await touchDrag(page, fingerX - 180, fingerY, fingerX, fingerY);
            await waitText(page, "Story hidden.");
            assert(!(await state(page)).elements.some(element => element.text === title), "Hidden headline remained in the feed");
            await byName(page, "UndoStory");
            await waitText(page, title);
            await byName(page, "DismissStatus");
            await touchDrag(page, fingerX, fingerY, fingerX - 180, fingerY);
            await page.waitFor(`JSON.parse(ember.emberState()).elements.some(e =>
                e.automationName?.startsWith(${JSON.stringify(title)}) && e.helpText === "Saved. Open discussion.")`);
            assert(!(await page.evaluate("new URL(location.href).searchParams.has('item')")), "Swiping a card also opened its discussion");
            await byName(page, "DismissStatus");
            const otherTitle = fixtures[102].title;
            const other = (await state(page)).elements.find(element => element.hit && element.text === otherTitle);
            assert(other, "Second bookmark fixture has no touch target");
            const secondX = Math.min(screen.width - 35, other.x + other.width - 15);
            const secondY = other.y + other.height / 2;
            await touchDrag(page, secondX, secondY, secondX - 180, secondY);
            await waitText(page, "Added to Saved stories. Download the thread to keep all its comments offline.");
            await byName(page, "DismissStatus");
            const heading = (await state(page)).elements.find(element => element.name === "FeedTitle");
            assert(heading, "The feed heading is absent");
            const beforeRefresh = await page.evaluate("globalThis.__emberApiRequests");
            const pullY = heading.y + 2;
            await touchDrag(page, heading.x + 100, pullY, heading.x + 100, pullY + 120);
            await page.waitFor(`globalThis.__emberApiRequests > ${beforeRefresh}`, 10000);
            await waitText(page, title);
            await byText(page, title);
            await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.name === 'SaveButton')");
            await byName(page, "SaveButton");
            await waitText(page, "Removed from Saved stories.");
            await byName(page, "UndoStory");
            await waitText(page, "Change undone.");
            await byAutomation(page, "Remove saved story");
            await waitText(page, "Removed from Saved stories.");
            await byName(page, "SaveButton");
            await waitText(page, "Added to Saved stories. Download the thread to keep all its comments offline.");
            await sleep(400);
            await byName(page, "DownloadButton");
            await waitText(page, "Discussion downloaded. It is available offline in Saved stories.");
            await byName(page, "DismissStatus");
            await page.screenshot(path.join(captures, "phone-thread.png"));
            // A live breakpoint crossing lays out an existing, virtualized discussion.
            // Starting independent windows at these widths does not exercise this path.
            for (const [width, height, rail] of [[700, 420, false], [760, 550, true], [390, 844, false]]) {
                await page.resize(width, height);
                // The viewport changes before the deferred adaptive layout arranges its controls.
                await page.waitFor(`(() => {
                    const view = JSON.parse(ember.emberState());
                    return view.width === ${width} && view.height === ${height}
                        && view.elements.some(e => e.name === 'NavTop') === ${rail}
                        && view.elements.some(e => e.name === 'SaveButton' && e.hit);
                })()`);
                assert(await page.evaluate("new URL(location.href).searchParams.get('item') === '101'"), "Resizing changed the selected URL");
                assert(page.errors.length === 0, page.errors.join("\n"));
            }
            await page.wheel(340, 600, 0, 440);
            await page.screenshot(path.join(captures, "phone-comments.png"));
            await page.wheel(340, 400, 0, -1000);
            await waitText(page, title);
            await page.evaluate("history.back()");
            await page.waitFor("!new URL(location.href).searchParams.has('item')");
            await byName(page, "MobileSaved");
            await waitText(page, title);
            await waitText(page, otherTitle);
            await byAutomation(page, "Show downloaded discussions only");
            await waitText(page, title);
            assert(!(await state(page)).elements.some(e => e.text === otherTitle), "Downloaded filter retained a bookmark without a download");
            await byAutomation(page, "Show all saved stories");
            await waitText(page, otherTitle);
            await byText(page, title);
            await page.waitFor("location.search.includes('item=101')");
            await page.waitFor("navigator.serviceWorker.controller !== null", 30000);
            const shell = await page.evaluate("caches.keys().then(keys => keys.some(key => key.startsWith('ember-shell:')))");
            assert(shell, "The complete offline application shell was not installed");
            await page.evaluate("sessionStorage.setItem('ember-test-offline', '1')");
            await page.send("Network.enable");
            await page.send("Network.emulateNetworkConditions", { offline: true, latency: 0, downloadThroughput: 0, uploadThroughput: 0 });
            const previousDocument = await page.evaluate("performance.timeOrigin");
            await page.send("Page.reload", {});
            await page.waitFor(`performance.timeOrigin !== ${previousDocument}`);
            await page.waitForView();
            await page.waitFor("globalThis.ember?.emberState");
            await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.name === 'FindButton')");
            await waitText(page, title);
            await page.screenshot(path.join(captures, "phone-offline-thread.png"));
            await byName(page, "BackButton");
            await byName(page, "MobileMore");
            await waitText(page, "Your library");
            await page.screenshot(path.join(captures, "phone-explore.png"));
            const requests = await page.evaluate("globalThis.__emberApiRequests ?? 0");
            await byAutomation(page, "Downloaded discussions");
            await waitText(page, title);
            const downloaded = await state(page);
            assert(downloaded.elements.some(e => e.hit && e.automationName === "Show all saved stories"),
                "Explore did not activate the Downloaded filter");
            assert(!downloaded.elements.some(e => e.text === otherTitle), "Explore included an undownloaded bookmark");
            assert(await page.evaluate("globalThis.__emberApiRequests ?? 0") === requests, "Explore fetched its offline library from the network");
            await page.screenshot(path.join(captures, "phone-offline-saved.png"));
            assert(page.errors.length === 0, page.errors.join("\n"));
        } catch (error) { await failure(page, "phone"); throw error; }
        finally { await page.close(); }
    }],
].filter(([name]) => !process.env.EMBER_BROWSER_CASE || name.includes(process.env.EMBER_BROWSER_CASE)));
