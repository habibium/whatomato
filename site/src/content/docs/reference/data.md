---
title: What's stored
description: What Whatomato keeps in its database, what is encrypted and what isn't, and what deleting or unlinking removes.
---

Everything lives in the instance's Postgres database. Secrets are encrypted with the instance's `ENCRYPTION_KEY` (XChaCha20-Poly1305), each bound to its account, table and key, so a database dump alone can't take over a device or read a sealed value.

## Your account

- Your email address, and your password as a hash.
- Sessions and emailed links, as SHA-256 hashes of their tokens only. The browser holds its session in an `HttpOnly` cookie.
- For two-step verification: the authenticator app's key, encrypted, and recovery codes as hashes.
- For passkeys: each passkey's public key and name.

## Your WhatsApp link

What a linked device has to keep to stay linked and to encrypt messages:

- the device's identity and the Signal keys and sessions for the people and groups it talks to;
- the app state keys and versions WhatsApp syncs between devices;
- recent sent messages, which WhatsApp can ask a device to send again when a recipient failed to decrypt them;
- the linked number, and when it was linked, paused or stopped.

The keys are encrypted.

## Your contacts

The contacts saved on your phone, which it shares with linked devices: their names, numbers and WhatsApp usernames. Names are encrypted. Each contact's number and WhatsApp address stay in the clear, since they are what a contact is looked up by.

Profile pictures aren't stored. Each is looked up when the app shows it and kept in the server's memory for a few hours.

## What you write

Templates, scheduled messages and the history of what was sent keep the message text and the recipient in plain columns. **These are not encrypted**, so a database dump does show them.

## What isn't stored

Whatomato doesn't download your chat history, and it stores no message you receive, unless the instance has [reading](/agents/reading-chats/) turned on and you allowed a connection to read that chat, or chose to [keep message history](/agents/reading-chats/#keep-message-history).

## Agent access

- Each connection, with the SHA-256 hash of its key and the key's last 4 characters. A connection made by signing in has no key: the address and name of the app are kept, with the hashes of its tokens.
- The connection's permissions as you saved them, chat names and notes included, not encrypted.
- A request waiting for approval: its recipient in the clear and its text encrypted.
- The activity log: every call, with the chat it named and the outcome, never the message text. Deleted after 90 days.
- Messages an agent sends or schedules, kept like any other, with the connection that made them.
- A picture or file an agent sends: what it is and its name in the clear, the file itself encrypted, and only until the message goes out, fails, or its request is answered or expires. Then the file is deleted and only its name stays in the history.
- Pictures and files that arrive aren't stored. A kept message holds, encrypted, only what WhatsApp needs to hand its file over again.

### With reading on

For the chats some connection may read, each message is kept for 7 days; while you keep message history, every chat's messages are kept as long as you chose. Which chat, when, and whether you sent it are in the clear; who wrote it and the text are encrypted. Each message also has search terms, keyed hashes of short slices of its text that don't give the text away. Pictures and files aren't kept, only their captions and file names. View-once messages are never kept. See [Reading chats](/agents/reading-chats/#whats-kept).

The history your phone sends when it links is held encrypted until it's imported, then deleted.

## Notifications

For each browser with notifications on: its address at the browser's push service and the two keys that browser made for encrypting what is sent to it. These aren't encrypted, since they open nothing without the instance's own push key pair, which is made at the first start and kept encrypted.

## What removes what

| You do this | This is deleted |
| --- | --- |
| Unlink WhatsApp, or remove the device on your phone | The device's keys and the copy of your contacts. Scheduled messages, templates and history stay. |
| Delete a scheduled message | The message. What it sent stays in History. |
| Delete a template | The template. Messages made from it keep their text. |
| Turn reading off for a chat | What was kept of that chat, within a few minutes. |
| Revoke an agent connection | The connection, its key or tokens, and its approval requests. Its sends stay in History and its entries stay in the activity log. |
| Turn notifications off in a browser, or sign out of it | That browser's push details. |

There is no way to delete an account from the app yet. Only whoever runs the instance can remove one, by deleting the user's row in the database, which takes everything that belongs to the account with it. Unlink WhatsApp first, so the device is removed from the phone as well.

## For operators

A dump of the database together with `ENCRYPTION_KEY` reveals every secret above. Keep them apart. See [Backups and restores](/self-hosting/backups/).
