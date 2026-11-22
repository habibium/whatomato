---
title: Architecture
description: One Rust server holds the WhatsApp connections, runs the scheduler and serves a React app, with a typed contract between the two.
---

Whatomato is one process. The Rust server answers the API, serves the built web app, holds a WhatsApp connection for every linked account, and runs the scheduler. It needs one Postgres database and nothing else.

```text
                    ┌────────────────────────────── server ──────────────────────────────┐
 browser  ──/──────▶│  web app (static files)                                             │
 browser  ──/api───▶│  REST API ──┐                                                       │
 agent    ──/mcp───▶│  MCP ───────┼─▶ operations ─▶ scheduler ─▶ WhatsApp clients ──────────▶ WhatsApp
 app      ──/oauth─▶│  OAuth ─────┘         │             │              │                 │
                    └───────────────────────┼─────────────┼──────────────┼────────────────┘
                                            ▼             ▼              ▼
                                                      Postgres
```

## The repository

| Path | What it holds |
| --- | --- |
| `server/` | The Rust server: axum for HTTP, sqlx for Postgres, whatsapp-rust for WhatsApp |
| `server/migrations/` | Database migrations, embedded in the binary and applied at startup |
| `web/` | The web app: React, TanStack Router, Query and Form, styled with Astryx and StyleX |
| `site/` | This documentation: Astro and Starlight |
| `openapi.json` | The API contract, generated from the server |
| `Dockerfile` | Builds the one image: the web app, then the server, then a slim runtime |

## The typed API contract

The server is the source of truth. utoipa derives an OpenAPI document from the axum routes, and the web app generates its TypeScript types from that document.

```text
server/src/lib.rs ──cargo run --bin openapi──▶ openapi.json ──openapi-typescript──▶ web/src/api/schema.d.ts
```

Both steps run by themselves in `just dev`: the server regenerates `openapi.json` before every restart, and a Vite plugin watches that file and regenerates the types. Both generated files are committed, and `just check` fails if either is out of date. `just gen-api` runs the generation by hand.

The REST API under `/api` is the web app's own. It uses a session cookie and CSRF protection, and it changes with the app, so it isn't a stable interface for other programs. The interface meant for other programs is [MCP](/reference/mcp-tools/).

## WhatsApp clients

Each linked account gets a supervisor that owns its client: it connects, reconnects with a growing delay when the client dies, and reports a status the rest of the server reads. Keys and sessions are stored in Postgres through the server's own store, encrypted with `ENCRYPTION_KEY`.

Only one server may run a given account's client, so a server connects nothing until it holds a Postgres advisory lock. That is what makes a rolling deploy safe. See [Running it](/self-hosting/operations/).

## The scheduler

One loop ticks every 3 seconds. Each tick it turns the schedules that are due into queued sends, then starts at most one send for each account. That tick is where the pace of one message every 3 seconds comes from.

A send that fails because WhatsApp wasn't connected is retried on a growing delay. A send whose outcome is unknown is never retried. [Schedule a message](/guides/schedule-a-message/#what-happens-when-a-message-is-due) describes the behaviour from the outside.

## Agent access

The MCP endpoint and the web app's agent pages call the same operations, and every operation goes through one decision: given this connection's permissions, its counts and this chat, is the call allowed, does it have to ask, or is it refused. A scheduled message that belongs to an agent goes through the same decision again each time it's due.

Apps that sign in use OAuth 2.1 with PKCE. The server is its own authorization server and identifies clients by the address they publish their details at.

## One image

The `Dockerfile` builds the web app with Node, the server with Rust, and copies both into a slim Debian image that runs as an unprivileged user. The server serves the web app's files itself, precompressed, so there is no separate web server and no JavaScript runs on the server.

The documentation isn't part of the image. It's a static site that deploys by itself, so fixing a page never restarts anyone's WhatsApp connection.
