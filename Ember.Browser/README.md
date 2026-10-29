# Ember browser host

This host runs the same Rust/FerroUI application as the native hosts. The story list,
navigation, threaded comments and rich content are FerroUI controls rendered into a
canvas. HTML is used only for the loading and startup-error screen.

## Build and run

From the repository root:

```sh
FerroUI/scripts/browser/setup.sh /path/to/browser-tools
source /path/to/browser-tools/env.sh
scripts/build-browser.sh
python3 -m http.server 8080 --directory "${CARGO_TARGET_DIR:-target}/browser/ember-browser"
```

The site is built under `$CARGO_TARGET_DIR` (`target` when it is not set).

Open `http://localhost:8080/`. The viewport is dynamic and carries `viewport-fit=cover`
for mobile safe areas; the app handles touch input through FerroUI's browser platform.
`?item=8863` opens a story. Browser Back/Forward and hash navigation are routed back
to the application's navigation service. Opening a story updates the shareable URL
without reloading WebAssembly.

The wrapper (`scripts/build-browser.sh` of this repository) accepts the options of FerroUI's `FerroUI/scripts/build-browser.sh`, including `--out`,
`--debug`, `--features vello`, `--threads` and `--both`. The default backend is the
framework's Skia backend. `--features vello` adds FerroUI's alternative renderer,
selected with `?Renderer=Vello`. All drawing remains behind the FerroUI rendering API.

For a development build on a machine with limited memory, optimize the Rust code
before the WebAssembly binding step. An entirely unoptimized framework module is
much larger:

```sh
CARGO_PROFILE_DEV_OPT_LEVEL=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 \
  CARGO_BUILD_JOBS=1 scripts/build-browser.sh --debug
```

## Database and offline behavior

`platform.rs` implements the reader core's `HttpClient` and `Store` contracts. Promises
are awaited through FerroUI's completion helper, so requests and storage do not block
the UI thread. Network requests omit credentials, use a 20-second timeout, reject
unexpected redirects and cap decoded response bodies at 8 MiB. Only the two explicit
HTTPS API providers are accepted.

`ember-platform.js` stores versioned records in the origin-scoped `ferroui-ember`
IndexedDB database. Each write resolves after the transaction commits. An aborted
transaction, unavailable database or quota failure rejects the operation; the adapter
does not claim an unsuccessful write was saved. Cache records and durable local
reading state use the namespaces defined by `ember-core`.

The default non-threaded build precaches a complete, content-versioned application
shell. It extends the existing FerroUI worker, preserving the platform's file-download
handler. An incomplete precache fails installation so the previous complete shell
remains available. API responses are not cached by the service worker: the repository
owns expiry and offline fallback in IndexedDB. Browser quotas and user site-data
clearing still apply. A shell must have installed successfully during an online visit
before the first offline launch.

Threaded and combined builds retain IndexedDB data but do not add shell precaching:
the framework's worker owns cross-origin isolation in those modes. Use the default
build when installable offline startup is required.

## Verification

```sh
cd Ember.Browser
npm ci
npm test
```

The tests cover provider URL restrictions, response limits and status failures,
transaction completion and abort, concurrent persistence, prefix indexes, URL history,
complete offline shell installation, failed precaching and separation from the
framework's download/isolation handlers. They use `fake-indexeddb` for a real IndexedDB
request/transaction lifecycle rather than an in-memory mock of the storage contract.

`emberState()` is a read-only WebAssembly export for interaction tests. It reports the
actual visible FerroUI controls, their CSS-pixel bounds and hit-test availability.

After building the site, run the compiled application in Chrome with FerroUI's
browser harness:

```sh
CHROME=/path/to/chrome node Ember.Browser/tests/browser-smoke.mjs
```

The interaction checks supply deterministic HN responses at the network boundary.
They exercise real pointer, keyboard and touch input, swipe-to-save, pull-to-refresh,
rendering, URL history, themes, phone navigation, discussion downloads, IndexedDB
persistence and service-worker startup with the network disabled. The runner serves
`$CARGO_TARGET_DIR/browser/ember-browser` unless a site is given as its argument, and screenshots
and failure diagnostics are written under `$CARGO_TARGET_DIR/ember-browser-smoke` (`target` when
`CARGO_TARGET_DIR` is not set).

## Other hosts

```sh
cargo run -p ember-desktop
```

The Android and iOS hosts compile in this workspace; FerroUI's packaging scripts
(`FerroUI/scripts/android/apk.sh`, `FerroUI/scripts/ios/bundle.sh`) do not yet package an
application of another workspace.

Android stores the database under the activity's private files directory and requests
the Internet permission. iOS uses `Library/Application Support/Ember` within its app
sandbox. Native package builds require the corresponding SDK/NDK or macOS/Xcode.
Browser validation does not establish signed-device behavior on Android or iOS.
