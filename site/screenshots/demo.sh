#!/usr/bin/env sh
# Runs the demo the docs' screenshots are taken from: its own database, a server that never reaches
# WhatsApp, and made-up data. README.md has the rest.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
work=${TMPDIR:-/tmp}/whatomato-docs-demo

database_url=$(sed -n 's/^DATABASE_URL=//p' "$root/.env" | tr -d "\"'")
if [ -z "$database_url" ]; then
	echo "DATABASE_URL is missing from $root/.env" >&2
	exit 1
fi
# The dev database's server and login, under a name nothing else uses
postgres=${database_url%/*}
demo_url=$postgres/whatomato_docs_demo
if ! psql "$postgres/postgres" -Atc "SELECT 1 FROM pg_database WHERE datname = 'whatomato_docs_demo'" | grep -q 1; then
	psql "$postgres/postgres" -qc 'CREATE DATABASE whatomato_docs_demo'
fi

mkdir -p "$work"
pnpm --dir "$root/web" exec vite build --outDir "$work/web" --emptyOutDir
cargo build -p server --manifest-path "$root/Cargo.toml"

trap 'kill 0' EXIT INT TERM

# Started outside the repository, so the server can't find .env and fall back to the real database
cd "$work"
env -u ENCRYPTION_KEY PORT=8011 DATABASE_URL="$demo_url" APP_URL=http://localhost:8012 \
	WEB_DIR="$work/web" AGENT_ACCESS=on AGENT_READING=on "$root/target/debug/server" &
until curl -fs http://127.0.0.1:8011/api/health >/dev/null; do sleep 0.5; done

node "$here/proxy.mjs" &
until curl -fs http://127.0.0.1:8012/api/health >/dev/null; do sleep 0.5; done

DEMO_DATABASE_URL="$demo_url" node "$here/seed.mjs"
wait
