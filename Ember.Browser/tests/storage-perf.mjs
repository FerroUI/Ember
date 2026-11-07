// CHROME=/path/to/chrome node Ember.Browser/tests/storage-perf.mjs
// Compare storage bridge calls on identical records, without network or renderer work.
import { copyFile, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { open } from "../../FerroUI/scripts/browser/harness.mjs";

const site = await mkdtemp(path.join(os.tmpdir(), "ember-idb-bench-"));
let page;
try {
    await copyFile(new URL("../wwwroot/ember-platform.js", import.meta.url), path.join(site, "ember-platform.js"));
    await writeFile(path.join(site, "index.html"), "<!doctype html><title>Ember storage benchmark</title>");
    page = await open(site);
    const results = await page.evaluate(`(async () => {
        const db = await import("./ember-platform.js");
        const keys = Array.from({ length: 1001 }, (_, i) => "cache:item:" + (i + 1));
        const text = "x".repeat(1024);
        for (let i = 0; i < keys.length; i += 32) {
            await Promise.all(keys.slice(i, i + 32).map((key, j) => db.databasePut(key,
                JSON.stringify({ fetched_at: 1, value: { id: i + j + 1, text } }))));
        }
        async function measure(batched) {
            const transaction = IDBDatabase.prototype.transaction;
            let transactions = 0, checksum = 0;
            IDBDatabase.prototype.transaction = function (...args) {
                transactions++;
                return transaction.apply(this, args);
            };
            const read = raw => { checksum += JSON.parse(raw).value.id; };
            const start = performance.now();
            try {
                read(await db.databaseGet(keys[0]));
                const width = batched ? 32 : 8;
                for (let i = 1; i < keys.length; i += width) {
                    const group = keys.slice(i, i + width);
                    const values = batched ? await db.databaseGetMany(group) : await Promise.all(group.map(db.databaseGet));
                    values.forEach(read);
                }
                if (batched) {
                    for (let i = 0; i < keys.length; i += 32) {
                        (await db.databaseGetMany(keys.slice(i, i + 32))).forEach(read);
                    }
                } else {
                    for (const key of keys) read(await db.databaseGet(key));
                }
                const ms = performance.now() - start;
                if (checksum !== keys.length * (keys.length + 1)) throw new Error("Read checksum mismatch");
                return { ms, transactions };
            } finally {
                IDBDatabase.prototype.transaction = transaction;
            }
        }
        await measure(false);
        await measure(true);
        const scalar = [], batch = [];
        for (let sample = 0; sample < 9; sample++) {
            for (const batched of sample % 2 ? [true, false] : [false, true]) {
                (batched ? batch : scalar).push(await measure(batched));
            }
        }
        return { scalar, batch };
    })()`);
    for (const [name, samples] of Object.entries(results)) {
        samples.sort((a, b) => a.ms - b.ms);
        const { ms, transactions } = samples[Math.floor(samples.length / 2)];
        console.log(`${name}: ${ms.toFixed(2)} ms median, ${transactions} transactions (${samples.length} samples)`);
    }
} finally {
    if (page) await page.close();
    await rm(site, { recursive: true, force: true });
}
