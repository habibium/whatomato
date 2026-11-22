---
title: Backups and restores
description: How to back up an instance, and why a restored backup brings back everything except the WhatsApp links.
---

## Back up

Dump the database:

```sh
docker compose exec -T db pg_dump -U whatomato whatomato > whatomato.sql
```

Keep `ENCRYPTION_KEY` somewhere other than with the dumps. The two together reveal every secret the key protects.

## What a restore brings back

A restore brings back the accounts, templates, scheduled messages and history. It doesn't bring back the WhatsApp links. Every user has to link again, for the reason below.

## After losing the key or restoring a backup

Linked devices can't survive either of these:

- **A lost or replaced `ENCRYPTION_KEY`.** The keys can't be opened any more.
- **A database restored from a backup.** The Signal keys in it have moved on since the backup was taken, and reusing them can break decryption and reuse message keys.

So before the server runs on that database, remove the links:

1. Stop the server.
2. Delete the WhatsApp accounts, which takes every key with them:

   ```sql
   DELETE FROM whatsapp_accounts;
   ```

3. Start the server.
4. Tell users to link again, and to remove the old device under **Linked devices** on their phone.

Scheduled messages that come due while an account isn't linked fail and don't go out later, so let users know soon.

:::danger[Never run a copy beside the original]
Don't start a server on a copy of the database while the original runs, for a restore drill for instance, unless the copy has had that `DELETE`. The lock that keeps two servers apart only covers one database, so the copy would connect with the same keys and take the accounts over.
:::
