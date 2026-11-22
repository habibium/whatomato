---
title: Run it with Docker Compose
description: Whatomato ships as one Docker image and needs a Postgres database. This page takes you from nothing to a running instance.
---

Each release publishes the image as `ghcr.io/habibium/whatomato`, built for `linux/amd64`.

## What you need

- Docker with Compose.
- A Postgres database. The compose file below brings its own; the tests run against Postgres 18.
- For anything past a trial on your own machine: a domain served over HTTPS and an SMTP server. With an `https` address the server won't start without one.

## The compose file

Save this as `compose.yaml`. It runs Whatomato with a database of its own.

```yaml title="compose.yaml"
services:
  whatomato:
    image: ghcr.io/habibium/whatomato:latest
    restart: unless-stopped
    depends_on:
      db:
        condition: service_healthy
    environment:
      DATABASE_URL: postgres://whatomato:${POSTGRES_PASSWORD}@db:5432/whatomato
      APP_URL: https://whatomato.example.com
      ENCRYPTION_KEY: ${ENCRYPTION_KEY}
      SMTP_URL: ${SMTP_URL}
      EMAIL_FROM: Whatomato <no-reply@example.com>
    ports:
      - "127.0.0.1:8000:8000"
    # each WhatsApp client flushes its keys on the way down
    stop_grace_period: 30s

  db:
    image: postgres:18-alpine
    restart: unless-stopped
    environment:
      POSTGRES_USER: whatomato
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
      POSTGRES_DB: whatomato
    volumes:
      - db:/var/lib/postgresql
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U whatomato"]
      interval: 5s
      timeout: 5s
      retries: 10

volumes:
  db:
```

The secrets go in a `.env` file beside it:

```sh title=".env"
# openssl rand -hex 32 (hex, because it goes into DATABASE_URL)
POSTGRES_PASSWORD=
# openssl rand -base64 32
ENCRYPTION_KEY=
SMTP_URL=smtps://user:password@smtp.example.com:465
```

:::danger[Keep the encryption key]
`ENCRYPTION_KEY` protects every secret in the database. Store a copy somewhere other than with your database backups. Losing it cuts off every linked WhatsApp account and every authenticator app. See [Backups and restores](/self-hosting/backups/).
:::

## Start it

```sh
docker compose up -d
```

The server applies its database migrations as it starts. `docker compose ps` reports it healthy once `/api/health` answers, which includes reaching the database.

Open the address you set as `APP_URL` and sign up. [Configuration](/self-hosting/configuration/) describes every variable, and [HTTPS and reverse proxies](/self-hosting/https/) covers putting it behind a proxy.

## Try it on your own machine first

Set `APP_URL` to `http://localhost:8000` and leave `SMTP_URL` empty. The server then writes each email to its log instead of sending it, so the verification link is in:

```sh
docker compose logs whatomato
```

## Before you invite anyone

- **Sign-up is open** to anyone who can reach the instance, and no setting closes it. An instance meant for a few people needs its access limited in front of it, at the proxy or the network.
- **Run one container per database.** [Running it](/self-hosting/operations/) explains why.
- `MAX_LINKED_ACCOUNTS` caps how many WhatsApp accounts the instance holds, if memory is a concern.

## Other architectures

The published image is `linux/amd64` only. On anything else, such as arm64, build it from a checkout and point the compose file at your build:

```sh
docker build -t whatomato .
```

```yaml
services:
  whatomato:
    image: whatomato
```
