---
title: Development
description: Set up a working copy, run the server and the web app together, and find the commands the project uses.
---

The source is at [github.com/habibium/whatomato](https://github.com/habibium/whatomato). It holds a Rust server in `server/`, a React web app in `web/`, and this documentation in `site/`.

## Prerequisites

- Rust nightly (`rustup toolchain install nightly`), and the tools the recipes call: `cargo install bacon just mprocs git-cliff`
- Node 24 or newer, and `pnpm`
- OpenSSL headers and `pkg-config` (`libssl-dev` on Debian), which the passkey library links against
- A Postgres database to develop against

Copy `.env.example` to `.env` and set `DATABASE_URL`. Everything else has a development default.

## Run it

```sh
just dev
```

This starts both halves in one terminal through [mprocs](https://github.com/pvolok/mprocs):

| Process | Command | Address |
| --- | --- | --- |
| `server` | `bacon` | `http://localhost:8000`, with the API browser at `/api/docs` |
| `web` | `pnpm dev` | `http://localhost:5173` |

Open the web address. The Vite dev server proxies `/api`, `/mcp` and the sign-in paths to the Rust server.

In mprocs, `Ctrl+a` moves between the process list and the terminal, `j` and `k` select a process, `r` restarts it and `q` quits everything. With the terminal focused, keys go to the process, so bacon's own shortcuts still work.

You can also run the two halves in separate panes: `bacon` at the root and `pnpm dev` inside `web/`.

Without `SMTP_URL`, each email is written to the server's log, link included, which is how you verify a development account.

## Commands

| Recipe | What it does |
| --- | --- |
| `just dev` | Start the server and the web app |
| `just check` | Everything CI runs: clippy, tests, the API contract, formatting, lint and the documentation build |
| `just fmt` | Format Rust and TypeScript |
| `just gen-api` | Regenerate `openapi.json` and the web app's types |
| `just build` | Production build of the web app and the server |
| `just start` | Run the release binary, serving the web app and the API on one port |
| `just docker-build` | Build the production image locally |
| `just docs` | Run this documentation site locally |
| `just changelog` | Preview the changelog entries since the last release |
| `just release` | Tag a version and push; CI builds, publishes and deploys it |

`just` alone lists them.

## The documentation

These pages are Markdown files under `site/src/content/docs`, built with [Starlight](https://starlight.astro.build).

```sh
just docs
```

serves them at `http://localhost:4321` and reloads as you edit. `pnpm --dir site build` builds the site and checks every internal link, which is what CI runs. Each page has an **Edit page** link at the bottom that opens its source.

The changelog page is written from `CHANGELOG.md` when the site builds, so a release needs no documentation edit of its own.

## Tests

`cargo test --workspace` runs the server's tests. Tests that touch the database create a throwaway database each, on the server `DATABASE_URL` points at.

The web app has no test suite yet; `pnpm --dir web typecheck` and `pnpm --dir web lint` are its checks.
