---
title: WhatsApp's rules
description: WhatsApp doesn't officially allow apps like Whatomato and can ban a number that uses one. This page says what raises that risk and what Whatomato does about it.
---

:::danger[Link a number you can afford to lose]
WhatsApp doesn't officially allow third-party clients. It can ban a number that uses one, for a while or for good. Its terms also don't allow automated messaging.
:::

Whatomato is an unofficial client. It connects as a linked device of your own account through the [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) library. It isn't the WhatsApp Business API, and nothing about it is approved by WhatsApp or Meta.

## What makes a ban likelier

- Sending the same text to many people.
- Messaging people who don't have your number saved.
- Sending at a pace or a regularity no person would.
- Letting an AI agent send without you looking at each message.

A handful of reminders to people who know you is a very different pattern from a broadcast to strangers. Whatomato is built for the first and makes the second hard.

## What Whatomato does to lower the risk

- **A human pace.** Each account sends at most one message every 3 seconds, however many are due.
- **No schedule finer than a minute.**
- **No second copy.** If Whatomato can't tell whether a message went out, it marks the send as failed and doesn't try again.
- **A check before sending.** A number that isn't on WhatsApp fails instead of being messaged blindly.
- **A quiet presence.** A connected account shows as online for 45 seconds once a day, which keeps your phone from flagging the device as inactive. The rest of the time it stays unavailable, so your phone keeps getting its notifications.
- **Agents ask first.** A new agent connection can't send anything without your approval, and no permission lifts its [daily and hourly limits](/reference/limits/#agents).

None of this makes a ban impossible.

## Agents raise the stakes

An agent sending for you is automated messaging. It can also be tricked by text it reads elsewhere, such as a web page, an email or a message someone sent you, into sending something you didn't intend. That is why every send asks you first by default. See [Staying safe](/agents/safety/).

## If WhatsApp bans the number

A temporary ban shows on the **WhatsApp** page as **WhatsApp has banned this number for now**, with the time it ends when WhatsApp gives one, and you get an email. **Reconnect** stays unavailable until then, and pressing it sooner wouldn't lift the ban. Messages that come due meanwhile fail and don't go out later.

Whatomato can't appeal a ban for you. That goes through WhatsApp, from your phone.
