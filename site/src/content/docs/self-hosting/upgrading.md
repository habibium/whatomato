---
title: Upgrading
description: Pull the new image and start it. Pending database migrations run by themselves, and they only go forward.
---

```sh
docker compose pull && docker compose up -d
```

Pending migrations run when the new version starts. The [changelog](/reference/changelog/) says what each release changed.

## Which tag to follow

| Tag | Moves when |
| --- | --- |
| `latest` | A stable release is published. |
| `0.4` | A patch release of that minor version is published. |
| `0.4.0` | Never. |

Pre-releases are published under their own version, such as `0.5.0-beta.1`, and under a channel tag: `alpha`, `beta` or `rc`. `latest` skips them.

## Migrations only go forward

An older image won't start on a database a newer one has migrated. To be able to go back, take a [backup](/self-hosting/backups/) before you upgrade, and remember what a restore does to linked WhatsApp accounts.

## Give it time to stop

During an upgrade the old container has to stop cleanly: each WhatsApp client flushes its keys on the way down. The compose file sets `stop_grace_period: 30s` for this. See [Running it](/self-hosting/operations/).
