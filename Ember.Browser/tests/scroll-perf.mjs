// CHROME=/path/to/chrome node Ember.Browser/tests/scroll-perf.mjs SITE [SAMPLES]
// Warm main-thread rAF gaps and wheel timestamp to next rAF callback.
// Visible rows are inspected only before and after each input train.
import { setTimeout as sleep } from "node:timers/promises";
import { open } from "../../FerroUI/scripts/browser/harness.mjs";

const [site, repetitions = "5"] = process.argv.slice(2);
const samples = Number(repetitions);
if (!site || !Number.isSafeInteger(samples) || samples < 1) {
    throw new Error("Specify an extracted release site and a positive sample count");
}

const initScript = `(() => {
    const original = fetch.bind(globalThis);
    const ids = Array.from({ length: 180 }, (_, i) => 1001 + i);
    const kids = Array.from({ length: 119 }, (_, i) => 2000 + i);
    const text = "<p>On a fast reader, moving to the next discussion should be immediate. <em>Nested details</em> and <code>source trees</code> still need to remain readable.</p><p>We keep the selected page and reading position while navigating.</p>";
    globalThis.fetch = async (input, options) => {
        const url = new URL(input instanceof Request ? input.url : input, location.href);
        if (url.hostname !== "hacker-news.firebaseio.com") return original(input, options);
        const id = Number(/\\/item\\/(\\d+)\\.json$/.exec(url.pathname)?.[1]);
        let data = ids;
        if (id >= 2000) {
            data = { id, type: "comment", by: "user" + id, parent: 1001, text, kids: [], time: 1792573200 };
        } else if (id) {
            data = { id, type: "story", by: "reader", title: "Scroll story " + id,
                url: "https://example.org/" + id, score: 214, time: 1792573200,
                descendants: id === 1001 ? kids.length : 0, kids: id === 1001 ? kids : [] };
        }
        return new Response(JSON.stringify(data), { headers: { "Content-Type": "application/json" } });
    };

    let recording, frameRequest, nextWheel;
    function frame(timestamp) {
        const now = performance.now();
        recording.frames.push(timestamp);
        while (nextWheel < recording.wheels.length) recording.wheels[nextWheel++].nextFrame = now;
        frameRequest = requestAnimationFrame(frame);
    }
    addEventListener("wheel", event => {
        if (recording) recording.wheels.push({ timestamp: event.timeStamp, nextFrame: null });
    }, { passive: true, capture: true });
    globalThis.startScrollSample = () => {
        recording = { frames: [], wheels: [] };
        nextWheel = 0;
        frameRequest = requestAnimationFrame(frame);
    };
    globalThis.endScrollSample = () => {
        cancelAnimationFrame(frameRequest);
        const result = recording;
        recording = null;
        return result;
    };
})();`;

const page = await open(site, { width: 1280, height: 800, initScript });
const elements = "JSON.parse(ember.emberState()).elements";
const wheelCount = 90;

async function firstVisible(pane) {
    const prefix = pane === "feed" ? "Scroll story " : "View user";
    return page.evaluate(`${elements}.find(e => e.hit && e.automationName?.startsWith(${JSON.stringify(prefix)}))?.automationName`);
}

function percentile(values, fraction) {
    const sorted = [...values].sort((a, b) => a - b);
    const index = Math.min(sorted.length - 1, Math.floor(sorted.length * fraction));
    return Number(sorted[index].toFixed(2));
}

async function scroll(pane, deltaY) {
    const from = await firstVisible(pane);
    await page.evaluate("startScrollSample()");
    const start = performance.now();
    const sends = [];
    // Schedule from the same clock even when acknowledgements take longer than
    // a refresh interval. Chromium may coalesce wheel events under load.
    for (let i = 0; i < wheelCount; i++) {
        const delay = start + i * (1000 / 60) - performance.now();
        if (delay > 0) await sleep(delay);
        sends.push(page.send("Input.dispatchMouseEvent", {
            type: "mouseWheel", pointerType: "mouse", x: pane === "feed" ? 345 : 980,
            y: 550, deltaX: 0, deltaY,
        }));
    }
    await Promise.all(sends);
    await sleep(100);
    const { frames, wheels } = await page.evaluate("endScrollSample()");
    const to = await firstVisible(pane);
    if (!from || !to || from === to || !wheels.length || wheels.at(-1).nextFrame === null) {
        throw new Error(`Incomplete ${pane} scroll: ${from} → ${to}, ${wheels.length} delivered events`);
    }
    const active = frames.filter(t => t >= wheels[0].timestamp && t <= wheels.at(-1).nextFrame);
    if (active.length < 2) throw new Error(`Too few animation frames in the ${pane} sample`);
    const gaps = active.slice(1).map((timestamp, i) => timestamp - active[i]);
    const input = wheels.map(event => event.nextFrame - event.timestamp);
    return {
        from, to, deliveredEvents: wheels.length, frameCount: active.length,
        frameGapP50Ms: percentile(gaps, 0.5), frameGapP95Ms: percentile(gaps, 0.95),
        maxFrameGapMs: Number(Math.max(...gaps).toFixed(2)), gapsOver25Ms: gaps.filter(gap => gap > 25).length,
        inputToRafP50Ms: percentile(input, 0.5), inputToRafP95Ms: percentile(input, 0.95),
    };
}

try {
    await page.waitForView();
    await page.waitFor(`${elements}.some(e => e.text === "Unread · 30")`);
    await page.waitFor("!!navigator.serviceWorker.controller", 60000);
    for (let count = 60; count <= 180; count += 30) {
        const button = await page.evaluate(`${elements}.find(e => e.name === "LoadMoreButton" && e.hit)`);
        if (!button) throw new Error("Missing Load more before " + count);
        await page.click(button.x + button.width / 2, button.y + button.height / 2);
        await page.waitFor(`${elements}.some(e => e.text === "Unread · ${count}")`);
    }
    for (const pane of ["feed", "thread"]) {
        if (pane === "thread") {
            await page.evaluate('location.hash = "item=1001"');
            await page.waitFor(`${elements}.some(e => e.automationName === "View user2000’s profile")`, 30000);
        }
        await scroll(pane, 12);
        await scroll(pane, -12);
        const forward = [];
        for (let sample = 0; sample < samples; sample++) {
            forward.push(await scroll(pane, 12));
            await scroll(pane, -12);
        }
        console.log(JSON.stringify({ pane, wheelCount, deltaY: 12, forward }));
    }
    if (page.errors.length) throw new Error(page.errors.join("\n"));
} finally {
    await page.close();
}
