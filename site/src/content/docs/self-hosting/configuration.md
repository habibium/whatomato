---
title: Configuration
description: Every environment variable the server reads, what it defaults to and what it changes.
---

The server is configured through environment variables. In development it also reads a `.env` file; `.env.example` in the repository lists them all.

## Variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `DATABASE_URL` | required | Postgres connection string. |
| `APP_URL` | required | Public address of the app, exactly as users open it. |
| `ENCRYPTION_KEY` | required | Encrypts the secrets kept in the database: 32 random bytes in base64, from `openssl rand -base64 32`. |
| `SMTP_URL` | required when `APP_URL` is `https` | SMTP server for verification, password reset and alert emails, such as `smtps://user:password@host:465`. |
| `EMAIL_FROM` | required with `SMTP_URL` | Sender, such as `Whatomato <no-reply@example.com>`. |
| `PORT` | `8000` | Port the server listens on. |
| `CLIENT_IP_HEADER` | unset | Header a trusted proxy sets to the client's address, such as `CF-Connecting-IP`. |
| `MAX_LINKED_ACCOUNTS` | unset | How many WhatsApp accounts may be linked at once. |
| `AGENT_ACCESS` | `on` | `off` turns [agent access](/agents/overview/) off for the instance. |
| `AGENT_READING` | `off` | `on` lets users allow a connection to [read chats](/agents/reading-chats/). |
| `RUST_LOG` | `warn,server=info` in the image | Log filter. |
| `WEB_DIR` | `/app/web/dist` in the image | Where the built web app is served from. |

A development build relaxes two of these: `APP_URL` defaults to `http://localhost:5173` and `ENCRYPTION_KEY` to a fixed development key. A release build, which is what the image runs, refuses to start without either.

## The address: `APP_URL`

`APP_URL` is the public address exactly as users open it. Three things hang off it:

- links in emails are built from it;
- an `https` address makes the cookies `Secure`;
- passkeys are bound to its host, which has to be a domain name rather than an IP address.

Moving an instance to another host leaves every existing passkey unusable.

## Email: `SMTP_URL` and `EMAIL_FROM`

`smtps://` connects on port 465 with TLS from the start. For port 587, use `smtp://user:password@host:587?tls=required`. Percent-encode special characters in the password, so `@` becomes `%40`.

Without `SMTP_URL` the server writes each email, link included, to its log. That is enough for a trial and refused for an `https` address, since anyone who can read the log could use the links.

## The encryption key

`ENCRYPTION_KEY` seals authenticator app keys, WhatsApp device keys, contact names, agent requests waiting for approval, kept messages, and the instance's push keys.

Keep it apart from the database and its backups: together they reveal every secret it protects. Replacing or losing it:

- stops every authenticator app set up so far from working, so those users have to sign in with a recovery code and set two-step verification up again;
- cuts off every linked WhatsApp account, which then has to be [removed and linked again](/self-hosting/backups/#after-losing-the-key-or-restoring-a-backup);
- turns [notifications](/guides/notifications/) off in every browser until each turns them on again.

## Behind a proxy: `CLIENT_IP_HEADER`

Name a header your proxy **overwrites** with the client's address. Without it every visitor arrives from the proxy's address and shares one rate limit.

`X-Forwarded-For` and `Forwarded` are refused, because a proxy appends to those and their first value is whatever the client claimed. The server trusts the header as sent, so the server must only be reachable through the proxy. See [HTTPS and reverse proxies](/self-hosting/https/).

## Capacity: `MAX_LINKED_ACCOUNTS`

Each linked WhatsApp account holds a connection open around the clock. At the limit, users without a linked account can't start linking and are told there's no room; those already linked are unaffected. Unset means no limit.

## Agents: `AGENT_ACCESS` and `AGENT_READING`

With `AGENT_ACCESS=off`, `/mcp` and signing in answer 404, the Agents page says agent access is off, and no agent's scheduled message runs.

With `AGENT_READING=on`, users can allow a connection to read chats, which keeps the last 7 days of those chats' messages, and can choose to keep every chat's history for longer, the history their phone sends when it links included. While it is `off`, no received message is stored and the reading tools are hidden. Reading needs agent access to be on as well.

## Logs: `RUST_LOG`

Keep libraries at `warn`. The WhatsApp library logs phone numbers and pair codes at `info`.
