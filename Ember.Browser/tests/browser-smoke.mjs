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
    250: { id: 250, type: "comment", by: "archivist", parent: 101, time: 1792574450, text: "<p>Margin notes are part of the archive.</p>" },
    203: { id: 203, type: "comment", by: "engineer", parent: 101, time: 1792574400, text: "<blockquote>Make the important things easy.</blockquote><p>I keep the design notes available offline.</p><script>globalThis.__emberUnsafeHtml = true</script>" },
};
const initScript = `(() => {
    const original = globalThis.fetch.bind(globalThis);
    const items = globalThis.__emberFixtureItems = ${JSON.stringify(fixtures)};
    globalThis.fetch = async (input, options) => {
        const url = new URL(typeof input === "string" || input instanceof URL ? input : input.url, location.href);
        if (!["hacker-news.firebaseio.com", "hn.algolia.com"].includes(url.hostname)) return original(input, options);
        globalThis.__emberApiRequests = (globalThis.__emberApiRequests ?? 0) + 1;
        if (sessionStorage.getItem("ember-test-offline") === "1") throw new TypeError("Simulated offline connection");
        let value = null;
        if (url.hostname === "hn.algolia.com") {
            globalThis.__emberLastSearch = url.href;
            const scoped = url.searchParams.get("tags") === "comment,story_101";
            const hit = scoped
                ? { objectID: "250", story_id: 101, story_title: items[101].title, comment_text: items[250].text, author: items[250].by, _tags: ["comment"] }
                : { objectID: "101", title: items[101].title, author: "maker", points: 284, num_comments: 3, created_at_i: 1792573200, url: items[101].url };
            value = { hits: [hit], nbHits: 1, nbPages: 1, page: 0, hitsPerPage: 30 };
        } else if (/stories\\.json$/.test(url.pathname)) value = [101, 102, 103];
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

async function touchDrag(page, x, y, endX, endY, beforeRelease) {
    await page.send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 });
    const point = (x, y) => ({ x, y, id: 1, radiusX: 6, radiusY: 6, force: 1 });
    const move = (x, y) => page.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [point(x, y)] });
    await page.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [point(x, y)] });
    for (let step = 1; step <= 10; ++step) {
        await move(x + (endX - x) * step / 10, y + (endY - y) * step / 10);
        await sleep(20);
    }
    if (beforeRelease) await beforeRelease(move);
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
    ["feed tabs resume their paged viewport and shortcuts target the focused story", async () => {
        const feeds = initScript + `(() => {
            const original = globalThis.fetch;
            const ids = Array.from({ length: 60 }, (_, i) => 1001 + i);
            for (const id of ids) globalThis.__emberFixtureItems[id] = {
                id, type: "story", by: "editor", title: "Archive story " + id, kids: []
            };
            globalThis.fetch = (input, options) => {
                const url = new URL(typeof input === "string" || input instanceof URL ? input : input.url, location.href);
                if (url.hostname === "hacker-news.firebaseio.com" && url.pathname === "/v0/topstories.json") {
                    return Promise.resolve(new Response(JSON.stringify(ids)));
                }
                return original(input, options);
            };
        })();`;
        for (const width of [390, 1280]) {
            const phone = width < 760;
            const page = await open(site, { width, height: 844, initScript: feeds });
            try {
                await page.waitForView();
                await waitText(page, "Archive story 1001");
                await byName(page, "LoadMoreButton");
                await page.waitFor("!JSON.parse(ember.emberState()).elements.some(e => e.name === 'LoadMoreButton' && e.hit)");
                let anchor;
                for (let step = 0; step < 16 && !anchor; step++) {
                    await page.wheel(phone ? 160 : 350, 470, 0, 520);
                    anchor = (await state(page)).elements.find(e => {
                        const match = /^Archive story (\d+),/.exec(e.automationName ?? "");
                        return e.hit && match && Number(match[1]) > 1030 && Number(match[1]) < 1058
                            && e.y > 130 && e.y + e.height < 730;
                    });
                }
                assert(anchor, "A fully visible story on the second page was not found");
                await byName(page, phone ? "MobileNew" : "NavNew");
                await waitText(page, title);
                await byName(page, phone ? "MobileHome" : "NavTop");
                await page.waitFor(`JSON.parse(ember.emberState()).elements.some(e => e.hit
                    && e.automationName === ${JSON.stringify(anchor.automationName)} && Math.abs(e.y - ${anchor.y}) < 2)`, 10000);
                assert(!(await state(page)).elements.some(e => e.name === "LoadMoreButton" && e.hit),
                    "The restored feed lost its final page");
                if (!phone) {
                    const id = Number(/^Archive story (\d+),/.exec(anchor.automationName)[1]);
                    await byAutomation(page, anchor.automationName);
                    await page.waitFor(`new URL(location.href).searchParams.get("item") === "${id}"`);
                    // ArrowDown focuses the next native ListBoxItem while the prior discussion stays open.
                    await page.press("ArrowDown");
                    await page.press("s");
                    await page.waitFor(`JSON.parse(ember.emberState()).elements.some(e =>
                        e.automationName?.startsWith("Archive story ${id + 1},") && e.helpText?.startsWith("Saved"))`, 10000);
                    await page.press("j");
                    await page.waitFor(`new URL(location.href).searchParams.get("item") === "${id + 2}"`, 10000);
                }
                assert(page.errors.length === 0, page.errors.join("\n"));
            } catch (error) { await failure(page, `feed-tabs-${width}`); throw error; }
            finally { await page.close(); }
        }
    }],
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
            await page.resize(2080, 960);
            await page.waitFor(`(() => {
                const view = JSON.parse(ember.emberState());
                const back = view.elements.find(e => e.name === "BackButton" && e.hit);
                const focus = view.elements.find(e => e.name === "FocusButton" && e.hit);
                return view.width === 2080 && back && focus && focus.x + focus.width - back.x <= 860;
            })()`);
            await byName(page, "FocusButton");
            assert(!(await state(page)).elements.some(e => e.name === "NavTop"), "The feed rail remains visible in focus reading");
            await byAutomation(page, "Exit focus reading");
            await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.name === 'NavTop')");
            await page.resize(1440, 960);
            await page.waitFor("JSON.parse(ember.emberState()).width === 1440");
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
    ["phone and desktop retain Find on repeated navigation and reveal a reply in context", async () => {
        for (const width of [390, 1280]) {
            const page = await start(width, 844);
            try {
                await page.evaluate("location.hash = 'id=0101'");
                await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.name === 'FindButton' && e.hit)");
                await byName(page, "FindButton");
                await page.type("tiny core");
                await waitText(page, "1 matching comment");
                // Browsers may deliver both events when traversing a hash URL.
                await page.evaluate("dispatchEvent(new PopStateEvent('popstate'))");
                await waitText(page, "1 matching comment");
                const context = e => e.name === "ReadInContext";
                await reveal(page, context);
                await page.screenshot(path.join(captures, `find-context-${width}.png`));
                if (width < 760) {
                    const button = (await state(page)).elements.find(e => e.hit && context(e));
                    await page.send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 });
                    await page.send("Input.dispatchTouchEvent", {
                        type: "touchStart",
                        touchPoints: [{ x: button.x + button.width / 2, y: button.y + button.height / 2 }],
                    });
                    await page.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
                } else {
                    await byName(page, "ThreadFind");
                    await page.press("Enter", ["ctrl"]);
                }
                await page.waitFor("!JSON.parse(ember.emberState()).elements.some(e => e.name === 'ThreadFind')");
                // Only the selected nested reply contains a code block.
                await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.automationName === 'Wrap code lines')");
                await byName(page, "BackButton");
                await page.waitFor("!location.hash && !new URL(location.href).searchParams.has('item')");
                assert(page.errors.length === 0, page.errors.join("\n"));
            } catch (error) { await failure(page, `context-${width}`); throw error; }
            finally { await page.close(); }
        }
    }],
    ["settings retain preference drafts and discard them across desktop and phone layouts", async () => {
        const page = await start(1440, 960);
        const field = label => element => element.automationName === label;
        const readText = async label => (await state(page)).elements.find(field(label))?.text;
        try {
            await byName(page, "ThemeButton");
            await reveal(page, field("Ocean accent"));
            await byAutomation(page, "Ocean accent");
            await byName(page, "ThemeButton");
            assert(await readText("Accent color") === "#2864A7", "Opening Settings again lost the draft accent");
            await byName(page, "UnreadButton");
            await page.resize(390, 844);
            await page.waitFor("JSON.parse(ember.emberState()).width === 390");
            await reveal(page, field("Discard preference edits"));
            await page.screenshot(path.join(captures, "phone-preference-draft.png"));
            await byAutomation(page, "Discard preference edits");
            await waitText(page, "Preference edits discarded.");
            await reveal(page, field("Accent color"), -1);
            assert(await readText("Accent color") === "#FF6B35", "Discard did not restore the saved accent");

            await reveal(page, field("Forest accent"));
            await byAutomation(page, "Forest accent");
            await reveal(page, field("Apply preferences"));
            await byAutomation(page, "Apply preferences");
            await waitText(page, "Preferences applied.");
            const saved = await page.evaluate(`import("./ember-platform.js")
                .then(async m => JSON.parse(await m.databaseGet("reader:state"))?.preferences)`);
            assert(saved?.accent === "#216E58" && saved.hide_read === true,
                "Applying the draft lost the live unread filter or saved the wrong accent");
            assert(page.errors.length === 0, page.errors.join("\n"));
        } catch (error) { await failure(page, "settings-draft"); throw error; }
        finally { await page.close(); }
    }],
    ["an offline download can be started from a feed, canceled from another tab and retried", async () => {
        const page = await start(390, 844);
        try {
            await page.evaluate(`(() => {
                const original = globalThis.fetch;
                const gate = new Promise(resolve => { globalThis.__releaseReply = resolve; });
                globalThis.fetch = async (input, options) => {
                    const url = new URL(typeof input === "string" || input instanceof URL ? input : input.url, location.href);
                    if (url.hostname === "hacker-news.firebaseio.com" && url.pathname === "/v0/item/201.json") {
                        globalThis.__waitingForReply = true;
                        await gate;
                    }
                    return original(input, options);
                };
            })()`);
            const download = async () => {
                const row = (await state(page)).elements.find(e => e.hit && e.text === title);
                const x = row.x + row.width / 2, y = row.y + row.height / 2;
                for (const [type, buttons] of [["mousePressed", 2], ["mouseReleased", 0]]) {
                    await page.send("Input.dispatchMouseEvent", { type, button: "right", buttons, x, y, clickCount: 1 });
                }
                await waitText(page, "Download for offline reading");
                await byText(page, "Download for offline reading");
            };
            await download();
            await page.waitFor("globalThis.__waitingForReply === true");
            await byName(page, "MobileSaved");
            await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.name === 'CancelTransfer' && e.hit)");
            await page.screenshot(path.join(captures, "phone-offline-progress.png"));
            await byName(page, "CancelTransfer");
            await waitText(page, "Download canceled.");
            await page.evaluate("globalThis.__releaseReply()");
            await byName(page, "MobileHome");
            await download();
            await waitText(page, "Discussion downloaded. It is available offline in Saved stories.");
            const saved = await page.evaluate(`import("./ember-platform.js").then(async m => JSON.parse(await m.databaseGet("reader:state")))`);
            assert(saved.downloaded_threads["101"] && !saved.items["101"].read_at, "Saving a feed download should leave it unread");
            assert(page.errors.length === 0, page.errors.join("\n"));
        } catch (error) { await failure(page, "transfer"); throw error; }
        finally { await page.close(); }
    }],
    ["storage cleanup releases temporary items and preserves verified downloads", async () => {
        const page = await start(1024, 820);
        try {
            await byName(page, "WelcomeOpen");
            await waitText(page, title);
            await byName(page, "DownloadButton");
            await waitText(page, "Discussion downloaded. It is available offline in Saved stories.");
            await byName(page, "ThemeButton");
            await reveal(page, element => element.text === "Clear local data");
            await byText(page, "Clear local data");
            const activate = async label => {
                await reveal(page, element => element.automationName === label);
                await byAutomation(page, label);
            };
            await activate("Check storage use");
            await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.text?.endsWith('6 items · 1 download'))");
            await activate("Clean temporary cache");
            await waitText(page, "Clean temporary data");
            await page.press("Escape");
            const cached = id => page.evaluate(`import('./ember-platform.js').then(m => m.databaseGet('cache:item:${id}'))`);
            assert(await cached(102), "Cancelling cleanup lost a cached story");
            await activate("Clean temporary cache");
            await activate("Clean temporary data");
            await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.text?.endsWith('4 items · 1 download'))");
            assert(!await cached(102) && await cached(101) && await cached(203),
                "Cleanup must preserve the downloaded root and replies and release ordinary cached stories");
            await page.screenshot(path.join(captures, "tablet-offline-storage.png"));
            assert(page.errors.length === 0, page.errors.join("\n"));
        } catch (error) { await failure(page, "storage"); throw error; }
        finally { await page.close(); }
    }],
    ["phone readers wrap and copy a code block and follow an internal rich link", async () => {
        const page = await start(390, 844);
        try {
            const source = "fn measure(column: &str, gutter: usize, margin: usize) -> usize {\n    column.len() + 2 * margin + gutter\n}\n";
            const html = "<p>A width calculation:</p><pre>" +
                source.replaceAll("&", "&amp;").replaceAll(">", "&gt;") +
                "</pre><p>See <a href='/item?id=202'>the related reply</a>.</p>";
            await page.evaluate(`(() => {
                globalThis.__emberFixtureItems[201].text = ${JSON.stringify(html)};
                globalThis.__emberFixtureItems[202].text = "<p>That is the same layout function I use.</p>";
            })()`);
            await byText(page, title);
            await reveal(page, e => e.automationName === "Wrap code lines");
            await byAutomation(page, "Wrap code lines");
            await page.setPermission("clipboard-read", "granted");
            await page.setPermission("clipboard-write", "granted");
            await byAutomation(page, "Copy code");
            await waitText(page, "Code copied.");
            assert(await page.evaluate("navigator.clipboard.readText()") === source,
                "Wrapping changed the code copied to the clipboard");
            await byName(page, "DismissStatus");
            await reveal(page, e => e.automationName === "the related reply");
            await page.screenshot(path.join(captures, "phone-code-tools.png"));
            await byAutomation(page, "the related reply");
            await waitText(page, "Comment by maker");
            assert(await page.evaluate("new URL(location.href).searchParams.get('item')") === "202",
                "A rich HN permalink must open its exact item in the reader");
            await byName(page, "BackButton");
            await waitText(page, title);
            assert(await page.evaluate("new URL(location.href).searchParams.get('item')") === "101",
                "Back should restore the original discussion");
            assert(page.errors.length === 0, page.errors.join("\n"));
        } catch (error) { await failure(page, "code-tools"); throw error; }
        finally { await page.close(); }
    }],
    ["discussion search opens an indexed comment beyond the loaded thread", async () => {
        const page = await start(390, 844);
        try {
            await byText(page, title);
            await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.automationName === 'More story actions' && e.hit)");
            await byAutomation(page, "More story actions");
            await page.press("Home");
            await page.press("ArrowDown");
            await page.press("Enter");
            await waitText(page, "Discussion 101");
            await byAutomation(page, "Search terms");
            await page.type("margin notes");
            await page.screenshot(path.join(captures, "phone-discussion-search.png"));
            await page.press("Enter");
            await waitText(page, "Margin notes are part of the archive.");
            const query = new URL(await page.evaluate("globalThis.__emberLastSearch"));
            assert(query.searchParams.get("tags") === "comment,story_101", "Discussion scope was lost");
            assert(query.searchParams.get("query") === "margin notes", "Search terms were lost");
            await byText(page, "Margin notes are part of the archive.");
            await waitText(page, "Comment by archivist");
            assert(await page.evaluate("new URL(location.href).searchParams.get('item') === '250'"), "Search opened the containing story instead of the exact comment");
            assert(page.errors.length === 0, page.errors.join("\n"));
        } catch (error) { await failure(page, "scoped-search"); throw error; }
        finally { await page.close(); }
    }],
    ["nested replies can jump over a subthread and open a single reply on a phone", async () => {
        const page = await start(390, 844);
        try {
            await page.evaluate(`(() => {
                const items = globalThis.__emberFixtureItems;
                items[201].kids.push(204);
                items[202].kids = [205];
                items[202].by = "designer";
                items[204] = { id: 204, parent: 201, type: "comment", by: "visitor",
                    text: "<p>I kept the same reading position after following the discussion.</p>" };
                items[205] = { id: 205, parent: 202, type: "comment", by: "typesetter",
                    text: "<p>A longer exchange about the details of the implementation.</p>".repeat(20) };
            })()`);
            await byText(page, title);
            const menuAt = async author => {
                const byline = e => e.hit && e.automationName === `View ${author}’s profile`;
                await reveal(page, byline);
                const { elements } = await state(page);
                const row = elements.filter(byline).at(-1);
                const menu = elements.find(e => e.hit && e.automationName === "More comment actions"
                    && Math.abs((e.y + e.height / 2) - (row.y + row.height / 2)) < 12);
                assert(menu, `Missing actions for the reply by ${author}`);
                await page.click(menu.x + menu.width / 2, menu.y + menu.height / 2);
            };
            // The flyout has its own popup root. Use its native keyboard navigation.
            await menuAt("designer");
            await page.press("Home");
            for (let i = 0; i < 4; ++i) await page.press("ArrowDown");
            await page.screenshot(path.join(captures, "phone-reply-navigation.png"));
            await page.press("Enter");
            await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.hit && e.automationName === 'View visitor’s profile')");
            assert(await page.evaluate("new URL(location.href).searchParams.get('item')") === "101",
                "Jumping to another reply should keep the containing discussion open");
            await menuAt("visitor");
            await page.press("Home");
            await page.press("ArrowDown");
            await page.press("Enter");
            await waitText(page, "Comment by visitor");
            assert(await page.evaluate("new URL(location.href).searchParams.get('item')") === "204",
                "The standalone thread must use the selected comment's permalink");
            await byName(page, "BackButton");
            await page.waitFor("new URL(location.href).searchParams.get('item') === '101'");
            assert(page.errors.length === 0, page.errors.join("\n"));
        } catch (error) { await failure(page, "reply-navigation"); throw error; }
        finally { await page.close(); }
    }],
    ["returning readers catch up on new nested replies and copy their link", async () => {
        const page = await start(1024, 820);
        try {
            await byName(page, "WelcomeOpen");
            await byAutomation(page, "Collapse 1 reply");
            await byName(page, "BackButton");
            await page.waitFor("!new URL(location.href).searchParams.has('item')");
            await page.evaluate(`(() => {
                const items = globalThis.__emberFixtureItems;
                items[101].descendants++;
                items[201].kids.push(204);
                items[204] = { id: 204, parent: 201, type: "comment", by: "visitor", time: Math.floor(Date.now() / 1000), text: "<p>A new nested reply for returning readers.</p>" };
            })()`);
            await byText(page, title);
            await waitText(page, "3 comments");
            await byName(page, "RefreshButton");
            await waitText(page, "4 comments");
            await byName(page, "NewReplyFilter");
            await waitText(page, "1 new reply");
            await waitText(page, "Earlier reply");
            const nested = (await state(page)).elements.find(e => e.hit && e.automationName === "View visitor’s profile");
            assert(nested, "The new reply is missing under its previously collapsed parent");
            assert(!(await state(page)).elements.some(e => e.automationName === "View engineer’s profile"),
                "A branch without new replies should be filtered out");
            await page.screenshot(path.join(captures, "tablet-new-replies.png"));
            const menu = (await state(page)).elements.find(e => e.hit && e.automationName === "More comment actions"
                && Math.abs((e.y + e.height / 2) - (nested.y + nested.height / 2)) < 12);
            assert(menu, "No overflow next to the new reply");
            await page.click(menu.x + menu.width / 2, menu.y + menu.height / 2);
            await page.setPermission("clipboard-read", "granted");
            await page.setPermission("clipboard-write", "granted");
            // Flyout menus can use a separate popup root. Select the first action by keyboard.
            await page.press("Home");
            await page.press("Enter");
            await waitText(page, "Comment link copied.");
            assert(await page.evaluate("navigator.clipboard.readText()") === "https://news.ycombinator.com/item?id=204", "The copy action used the wrong comment");
            await byName(page, "NewReplyFilter");
            await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.automationName === 'Expand comment and 2 replies')");
            assert(!(await state(page)).elements.some(e => e.automationName === "View visitor’s profile"),
                "Clearing the scope lost the original collapsed state");
            assert(page.errors.length === 0, page.errors.join("\n"));
        } catch (error) { await failure(page, "new-replies"); throw error; }
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
            await touchDrag(page, fingerX, fingerY, fingerX - 170, fingerY, async move => {
                await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.name === 'SwipeAction' && e.text === 'Save')");
                await page.screenshot(path.join(captures, "phone-swipe-preview.png"));
                // Retreat across the threshold before releasing the same touch.
                await move(fingerX - 32, fingerY);
                await sleep(60);
            });
            assert(!(await state(page)).elements.some(e => e.automationName?.startsWith(title) && e.helpText === "Saved. Open discussion."), "Retracting the swipe still saved the story");
            assert(!(await state(page)).elements.some(e => e.name === "SwipeAction"), "Swipe feedback stayed visible after release");
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
            for (const [width, height, rail] of [[320, 640, false], [700, 420, false], [844, 390, false], [844, 550, true], [760, 550, true], [390, 844, false]]) {
                await page.resize(width, height);
                // The viewport changes before the deferred adaptive layout arranges its controls.
                await page.waitFor(`(() => {
                    const view = JSON.parse(ember.emberState());
                    return view.width === ${width} && view.height === ${height}
                        && view.elements.some(e => e.name === 'NavTop') === ${rail}
                        && view.elements.some(e => e.name === 'SaveButton' && e.hit);
                })()`);
                if (width === 320) {
                    assert(!(await state(page)).elements.some(e => e.name === "DownloadButton" && e.hit), "Download still crowds the narrow toolbar");
                    await byName(page, "ToolbarMore");
                    await waitText(page, "Focus reading");
                    await byText(page, "Focus reading");
                    await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.automationName === 'Exit focus reading' && e.hit)");
                    await byAutomation(page, "Exit focus reading");
                }
                if (width === 844 && height === 390) {
                    const controls = (await state(page)).elements;
                    const first = controls.find(e => e.name === "MobileHome" && e.hit);
                    const last = controls.find(e => e.name === "MobileMore" && e.hit);
                    assert(first && last && first.x === last.x && first.y < last.y, "Landscape primary tabs must form a usable side strip");
                    await page.screenshot(path.join(captures, "landscape-reader.png"));
                }
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
            await byName(page, "LibraryFindButton");
            await page.type("curious maker example.org");
            await page.press("Enter");
            await page.waitFor("JSON.parse(ember.emberState()).elements.some(e => e.name === 'FeedSummary' && e.text?.startsWith('1 match'))");
            await page.screenshot(path.join(captures, "phone-library-find.png"));
            await byText(page, title);
            await page.waitFor("new URL(location.href).searchParams.get('item') === '101' && JSON.parse(ember.emberState()).elements.some(e => e.name === 'FindButton' && e.hit)");
            await byName(page, "BackButton");
            await waitText(page, title);
            const result = await state(page);
            assert(result.elements.some(e => e.name === "LibraryFind" && e.text === "curious maker example.org"), "Returning from a match discarded the collection query");
            await byName(page, "CloseLibraryFind");
            await waitText(page, title);
            assert(!(await state(page)).elements.some(e => e.name === "LibraryFind"), "Closing collection Find did not restore browsing");
            assert(await page.evaluate("globalThis.__emberApiRequests ?? 0") === requests, "Searching and reopening the downloaded discussion used the network");

            // Downloads and bookmarks are independent. Exercise both collections offline.
            await byText(page, title);
            await byAutomation(page, "Remove saved story");
            await waitText(page, "Removed from Saved stories.");
            await byName(page, "BackButton");
            await waitText(page, title);
            await byAutomation(page, "Show all saved stories");
            await waitText(page, otherTitle);
            assert(!(await state(page)).elements.some(e => e.text === title), "Unsaving retained the item in ordinary bookmarks");
            await byAutomation(page, "Show downloaded discussions only");
            await waitText(page, title);
            await byText(page, title);
            await waitText(page, "3 comments");
            await byAutomation(page, "Manage downloaded discussion");
            // The platform flyout may be hosted outside the main canvas control tree.
            await page.press("End");
            await page.press("Enter");
            await waitText(page, "Removed from Downloads. Temporary pages can be reclaimed in Settings.");
            await byName(page, "BackButton");
            await waitText(page, "No downloaded discussions");
            await byName(page, "RetryButton");
            await waitText(page, otherTitle);
            assert(!(await state(page)).elements.some(e => e.text === title), "Releasing a download restored its bookmark");

            assert(page.errors.length === 0, page.errors.join("\n"));
        } catch (error) { await failure(page, "phone"); throw error; }
        finally { await page.close(); }
    }],
].filter(([name]) => !process.env.EMBER_BROWSER_CASE || name.includes(process.env.EMBER_BROWSER_CASE)));
