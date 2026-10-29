#!/bin/bash
# Builds the browser site of Ember with FerroUI's script (FerroUI/scripts/build-browser.sh, with the
# same options), then gives the default, non-threaded site its offline application shell.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FERROUI_BROWSER_WORKSPACE="$ROOT" "$ROOT/FerroUI/scripts/build-browser.sh" ember-browser "$@"
node "$ROOT/Ember.Browser/write-offline-manifest.mjs" "$ROOT" "$@"
