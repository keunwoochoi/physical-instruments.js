#!/usr/bin/env bash
# Serve the repo for the playground (worklets need http, not file://).
set -e
cd "$(dirname "$0")/../.."
PORT="${1:-8173}"
echo
echo "  instruments.js playground:  http://localhost:${PORT}/apps/playground/"
echo
exec python3 -m http.server "$PORT" --bind 127.0.0.1
