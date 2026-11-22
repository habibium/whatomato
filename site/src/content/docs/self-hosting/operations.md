---
title: Running it
description: What a deploy has to respect, how to tell the server is healthy, and the limits that live in its memory.
---

## One server per database

Only one server may run a given account's WhatsApp client. Two on the same keys knock each other off and can corrupt them.

So a server connects no account and sends nothing until it holds a Postgres advisory lock, and it lets the lock go only after stopping its clients. During a deploy, either order of starting the new container and stopping the old one works: the new one waits its turn.

Don't scale the service to more than one replica: only the server that holds the lock runs the WhatsApp clients.

## Give it 30 seconds to stop

Each WhatsApp client flushes its keys on the way down, and Docker's default of 10 seconds can cut that short. Give the container at least 30:

- `docker stop --time 30`
- `stop_grace_period: 30s` in Compose or Swarm

## Health

`/api/health` answers once the server is up and can reach the database, and reports the running version. The image's own health check calls it every 30 seconds.

```sh
curl -fsS https://whatomato.example.com/api/health
```

## After a restart

The server reconnects every linked account by itself, so messages keep going out with no browser open. Whatever came due while it was down is sent once when it's back, however late. A send that was in progress when the server stopped is marked as failed with a note that it may or may not have gone out, and isn't retried.

## Rate limits live in memory

The limits on sign-in, on writes and on agent calls are kept in the server's memory. They are per instance and reset on restart. [Limits](/reference/limits/) lists them.

Behind a proxy, set [`CLIENT_IP_HEADER`](/self-hosting/configuration/#behind-a-proxy-client_ip_header), or every visitor shares the proxy's address and one limit.

## Sign-up is open

Anyone who can reach the instance can create an account, and no setting closes that. For an instance meant for a few people, limit access in front of it: an allowlist at the proxy, an identity-aware proxy, or a private network.

`MAX_LINKED_ACCOUNTS` doesn't stop sign-ups. It caps how many WhatsApp accounts can be linked at once.

## Logs

The image logs at `warn,server=info`. Keep libraries at `warn`: the WhatsApp library logs phone numbers and pair codes at `info`.

Request logs carry the path only, never the query string, since emailed links carry their tokens there.

## A user who is locked out

Someone who loses both their authenticator app and their recovery codes can't get back in by themselves, and the app has no admin page. Once you're sure who is asking, turn two-step verification off for them in the database. Email addresses are stored in lower case.

```sql
DELETE FROM totp_secrets
WHERE user_id = (SELECT id FROM users WHERE email = 'person@example.com');

DELETE FROM recovery_codes
WHERE user_id = (SELECT id FROM users WHERE email = 'person@example.com');
```

They can then sign in with their password and set it up again. A passkey they still hold signs them in without any of this.
