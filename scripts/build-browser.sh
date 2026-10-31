#!/bin/bash
# Builds the browser site of Ember with FerroUI's script (FerroUI/scripts/build-browser.sh, with the
# same options), then gives the default, non-threaded site its offline application shell.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [[ "${EMBER_LOW_MEMORY:-0}" == 1 ]]; then
    debug=false
    for arg in "$@"; do
        case "$arg" in
            --debug) debug=true ;;
            --threads|--both) echo "EMBER_LOW_MEMORY supports single-threaded development builds." >&2; exit 2 ;;
        esac
    done
    if [[ "$debug" != true ]]; then
        echo "Use EMBER_LOW_MEMORY=1 with --debug." >&2; exit 2
    fi

    # Resolve the pinned CLI and the SDK's Binaryen before adding the shim to PATH.
    EMBER_REAL_BINDGEN="$(command -v wasm-bindgen)" || { echo "Activate FerroUI's browser toolchain first." >&2; exit 1; }
    if [[ -n "${EMSDK:-}" && -x "$EMSDK/upstream/bin/wasm-opt" ]]; then
        EMBER_WASM_OPT="$EMSDK/upstream/bin/wasm-opt"
    else
        EMBER_WASM_OPT="$(command -v wasm-opt)" || { echo "Cannot find wasm-opt from the Emscripten SDK." >&2; exit 1; }
    fi
    export EMBER_REAL_BINDGEN EMBER_WASM_OPT PATH="$ROOT/scripts/browser-low-memory:$PATH"
    export CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_OPT_LEVEL=1 CARGO_PROFILE_DEV_DEBUG=0
    export CARGO_PROFILE_DEV_CODEGEN_UNITS=256 CARGO_INCREMENTAL=0
    export BINARYEN_CORES=1 RAYON_NUM_THREADS=1 EMCC_CORES=1 MALLOC_ARENA_MAX=2
    export EMCC_CFLAGS="${EMCC_CFLAGS:+$EMCC_CFLAGS }-Wl,--strip-all"
fi
FERROUI_BROWSER_WORKSPACE="$ROOT" "$ROOT/FerroUI/scripts/build-browser.sh" ember-browser "$@"
node "$ROOT/Ember.Browser/write-offline-manifest.mjs" "$ROOT" "$@"
