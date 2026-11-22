---
title: Limits
description: Every number Whatomato enforces, from how fast it sends to how many sign-in attempts an account takes.
---

## Sending

| Limit | Value |
| --- | --- |
| Pace for each account | One message every 3 seconds at most |
| Finest schedule | One minute |
| Tries for a send that fails while WhatsApp reconnects | 6, over about half an hour: after 1, 2, 4, 8 and 15 minutes |
| Wait for WhatsApp to acknowledge a message | 20 seconds, then the send fails as unconfirmed |
| Wait to check that a number is on WhatsApp | 30 seconds |

## Messages and templates

| Limit | Value |
| --- | --- |
| Message or template text | 4096 characters |
| Template name | 100 characters |
| Phone number | 8 to 15 digits, with its country code |
| How far a [variable](/reference/variables/#moving-a-time) can move a time | 366 days |
| Sunrise or sunset offset | 12 hours either way |
| Scheduled messages and templates you can have | No limit |
| History rows shown a page | 25 |

## Accounts

| Limit | Value |
| --- | --- |
| Password length | 8 to 128 characters |
| Session | 30 days, extended while in use |
| Email verification link | 24 hours, single use |
| Password reset link | 1 hour, single use |
| Emailed links | One a minute for each account and purpose |
| Password sign-in attempts for an account | 10, then one every 90 seconds |
| Password re-checks for a signed-in user | 10, then one every 90 seconds |
| Two-step code step | 5 minutes, 5 wrong codes |
| Wrong two-step codes for an account | 30 a day |
| Two-step setup | 15 minutes, 5 wrong codes |
| Recovery codes | 10, each single use |
| Passkeys | 20 for each account |
| Passkey prompt | 5 minutes |

### Writes for each address

Each part of the API limits the writes one client address may make: a burst of 20, then one every 6 seconds. IPv6 addresses are counted per /64. Signing in, WhatsApp, messages, templates, notifications and agents each have a bucket of their own.

A refused request gets a `429` with `Retry-After`, and the app shows **Too many attempts** with how long to wait.

These limits, and the sign-in ones above, are kept in the server's memory. They are per instance and reset when it restarts.

## WhatsApp

| Limit | Value |
| --- | --- |
| WhatsApp accounts for each Whatomato account | 1 |
| Link attempts | 5, then one every 10 minutes |
| One linking attempt | 5 minutes; QR codes run out after about 3 |
| Paused account's keep-alive | About 2 minutes once a week |
| Daily presence | 45 seconds online once a day |
| WhatsApp accounts on an instance | Set by its operator with `MAX_LINKED_ACCOUNTS`; no limit by default |

## Agents

No permission lifts these.

| Limit | Value |
| --- | --- |
| Connections for each account | 20 |
| Messages a connection sends in 24 hours | 200, counting requests still waiting for approval |
| Requests waiting for approval | 20 for each connection |
| Scheduled messages a connection holds | 50 |
| Shortest gap between an agent schedule's runs | 1 hour |
| Daily limit you can set for a chat | 1 to 200 |
| Requests to `/mcp` a minute | 120 for each connection |
| Chat searches a minute | 20 for each connection |
| Matches a chat search returns | 20 |
| People tagged in one message | 50 |
| Chats listed in one connection's permissions | 500 |
| Note for a chat | 500 characters |
| Connection name | 60 characters |
| Key expiry | 1 to 365 days, or never |
| Approval request | Expires after 24 hours |
| Activity log | Kept for 90 days |
| Messages kept for [reading](/agents/reading-chats/) | 7 days, or as long as you keep [message history](/agents/reading-chats/#keep-message-history): up to 3,650 days, or everything |
| A picture or file an agent opens or sends | 16 MB |
| How far back a connection may read | 1 to 3,650 days, or 1 to 120 months, or everything kept |
| Messages one search opens | 5,000 holding the query's terms, or the latest 3,000 for a two-letter query |

Over a chat's daily limit, a send is refused rather than turned into a request. Creating a scheduled message also needs room under the 200 and under the chat's daily limit, even though nothing is sent yet.

You can schedule a message more often than hourly yourself, from the web app.

### Sign-in tokens

| Token | Lasts |
| --- | --- |
| Authorization code | 60 seconds |
| Access token | 1 hour |
| Refresh token | 90 days without use |

## Notifications

| Limit | Value |
| --- | --- |
| Browsers with notifications on | The 10 most recent for each account |
| How long a notification waits for an offline browser | 24 hours |
