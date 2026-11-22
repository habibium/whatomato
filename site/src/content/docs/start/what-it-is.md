---
title: What Whatomato is
description: A web app that sends WhatsApp messages from your own number at the time you choose, and lets an AI agent send for you under rules you set.
---

Whatomato links to your WhatsApp account the way WhatsApp Web does, as a linked device. Once it's linked, a server holds that connection around the clock, so a message can go out at 6 in the morning with your phone asleep and no browser open.

## What you can do with it

- **Schedule a message** to go out once, or on a repeat: chosen weekdays, a day of the month, a date each year, or a time that follows sunrise or sunset. See [Schedule a message](/guides/schedule-a-message/).
- **Send to people and groups.** Pick a contact by the name saved on your phone, type any number that's on WhatsApp, or choose a group your number is in. See [Contacts and groups](/guides/recipients/).
- **Reuse text.** Save what you send often as a template, and let variables such as `{{first_name}}` or `{{date}}` fill themselves in as each message goes out. See [Templates and variables](/guides/templates/).
- **See what happened.** Every send is logged with its outcome, and a failure says why. See [History](/guides/history/).
- **Let an AI agent help.** Claude, ChatGPT, Codex, Cursor and other agents can find chats, send and schedule for you. You choose what each one may do, and by default every send waits for your approval. See [How agent access works](/agents/overview/).

## How it reaches WhatsApp

Whatomato talks to WhatsApp through the open-source [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) library, as a linked device of your own account. Messages come from your number, and your phone keeps working as usual.

It isn't the WhatsApp Business API, and WhatsApp doesn't officially allow apps like it. Read [WhatsApp's rules](/start/whatsapp-risk/) before you link a number.

## What it deliberately doesn't do

- **It doesn't download your chat history** unless you choose to [keep message history](/agents/reading-chats/#keep-message-history), and it stores no message you receive unless you turn on [reading](/agents/reading-chats/) for a chat. Reading is off on the hosted instance.
- **It doesn't send in bulk.** Each account sends at most one message every 3 seconds, and schedules can't be finer than a minute.
- **It doesn't send twice.** A send that may already have gone out is marked as failed instead of being tried again.

## Two ways to use it

I run a hosted instance at [whatomato.habibium.com](https://whatomato.habibium.com). It runs the same image that is published from the [source repository](https://github.com/habibium/whatomato). To use it, start with the [Quickstart](/start/quickstart/).

You can also run your own. Whatomato ships as one Docker image and needs a Postgres database. Start with [Run it with Docker Compose](/self-hosting/quickstart/).

Whatomato is free software under the [GNU AGPL, version 3](/project/license/).
