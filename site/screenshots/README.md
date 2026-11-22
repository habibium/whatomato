# Screenshots

The pictures in the docs are taken from a demo copy of the app that has its own database and never
connects to WhatsApp. Everything in them is made up: the account, the contacts, the numbers (from
the range Ofcom keeps for drama) and the messages. Never take one from a real account.

Each shot is two files in `site/src/assets/screenshots`, `<name>.light.webp` and
`<name>.dark.webp`, and a page shows the pair with `<Screenshot name="<name>" alt="..." />`. They are
taken at twice their size and stored as lossless WebP.

## Start the demo

```sh
site/screenshots/demo.sh
```

It creates the `whatomato_docs_demo` database next to the one in `.env`, builds the web app, starts
a server on port 8011 with `proxy.mjs` in front of it on port 8012, and runs `seed.mjs`. Open
<http://localhost:8012> and sign in as `maya@example.com` with `tomato-ladder-quartz-91`.
`alex@example.com` has the same password and an empty account. Run it again to start over with
fresh data; Ctrl+C stops it.

`proxy.mjs` answers every `/api/whatsapp` request itself. A browser sees a connected account unless
it opens one of these first, which changes the answer for that browser only:

```text
http://localhost:8012/__demo/whatsapp/not_linked
http://localhost:8012/__demo/whatsapp/connected
http://localhost:8012/__demo/whatsapp/paused
http://localhost:8012/__demo/whatsapp/stopped
```

Linking from the WhatsApp page works too: it shows a QR code or a pairing code that links nothing.

## Take a shot

Three scripts drive [agent-browser](https://github.com/vercel-labs/agent-browser), and `shoot.sh`
also needs ImageMagick's `magick`. Each takes a browser name first; every name is a separate browser
with its own sign-in.

```sh
cd site/screenshots
./signin.sh a                        # open the demo in browser "a" and sign in as maya
./browser.sh a open http://localhost:8012/templates
./browser.sh a snapshot -i -c        # lists what can be clicked, each with a ref such as @e12
./browser.sh a click @e12
./shoot.sh a templates-list          # saves templates-list.light.webp and templates-list.dark.webp
```

`shoot.sh <browser> <name> [target]` captures what the browser shows at that moment, in both
colour schemes. The target is one of:

| Target | Captures |
| --- | --- |
| `page` | The page's content without the side navigation. The default. |
| `dialog` | The open dialog. |
| `window` | The whole window. For sign-in pages, and for a menu or list that opens outside a dialog. |
| `region:<heading>` | The section under a heading, by the heading's exact text. |
| a CSS selector | One element, such as `table`. |

Other `browser.sh` commands worth knowing:

```sh
./browser.sh a fill @e7 "text"       # replace a field's text
./browser.sh a type @e7 "text"       # type key by key, which opens suggestion lists
./browser.sh a press Escape          # also Enter, Tab, ArrowDown
./browser.sh a find role button click --name "New message"
./browser.sh a find text "Jonas Berg" click
./browser.sh a wait --text "Scheduled"
./browser.sh a wait 600
./browser.sh a scroll down 400
./browser.sh a get url
./browser.sh a close
```

Refs change whenever the page changes, so take a new snapshot after every click that opens or
closes something.

`boiler.jpg`, the photo on the demo's request to Jonas Berg, is by Salah Ait Mokhtar on
[Unsplash](https://unsplash.com/photos/a-close-up-of-a-pressure-gauge-attached-to-a-pipe--GqvJ5g-Z4A).

## Rules

- Open only `http://localhost:8012`. Not the dev server, not the hosted instance, nothing else.
- Type only the made-up people and text the shot list gives. Contacts: Aisha Rahman, Ben Whitaker,
  Carlos Mendes, Dr Priya Patel, Elena Petrova, Farid Hossain, Grace Liu, Hannah Schmidt, Imran
  Chowdhury, Jonas Berg, Leo Martin, Mum, Olivia Bennett, Sam Taylor, Tomás García, Yuki Tanaka.
  Groups: Book Club, Family, Flat 4B, Product Team, Sunday Football, Year 6 Parents.
- Fill forms in, and leave them unsent unless the shot says to send. Never approve, reject, revoke
  or delete: the other shots need the data as it is.
- Look at every `.light.webp` after taking it. Retake it if the subject is cut off, a menu is half
  open, a field shows a red error that the shot didn't ask for, or a notice covers the subject.
- No mouse pointer, no text selection, and no focus ring on a field the shot isn't about: click an
  empty part of the dialog or press Tab away first if one shows.

## The shots

Names are fixed: the pages refer to them.

### Messages (browser `a`, signed in as maya)

The recurring form is taller than the usual window: run `./browser.sh a set viewport 1200 1400 2`
before opening the dialog for `message-variables`, `message-recurring`, `message-sun` and
`message-cron`, and `./browser.sh a set viewport 1200 1000 2` after them.

| Name | Set-up | Target |
| --- | --- | --- |
| `messages-list` | `/messages`, the **Scheduled** tab. | `page` |
| `messages-past` | The **Past** tab. | `page` |
| `messages-row-menu` | Back on **Scheduled**, open the `...` menu of the row for **Mum**. | `window` |
| `message-form` | **New message**. Contact: search `Jonas`, pick **Jonas Berg**. Message: `Hi {{first_name}}, are we still on for Thursday at 6?`. **One time**, 27 November 2026, 9:00 AM. | `dialog` |
| `recipient-picker` | A fresh **New message**. Type `an` into the contact field so the matching contacts list is open. | `dialog`, or `window` if the list isn't in the picture |
| `recipient-number` | Same field, with `+1 212 555 0155` typed instead, and its **Not in your contacts** suggestion showing. The field only offers a number that is valid for its country, which the drama range isn't; 555-01xx numbers are kept for fiction too. | as above |
| `recipient-group` | **Send to: Group**, with the group list open. | as above |
| `message-variables` | Contact **Aisha Rahman**. Type `Hi {{` into the message so the list of variables is open. | as above |
| `message-recurring` | Contact **Sam Taylor**, message `Gym at 7 tonight?`, **Recurring**, every week on Monday, Wednesday and Friday at 6:00 PM. | `dialog` |
| `message-sun` | Same dialog, with the schedule changed to 45 minutes before sunset, every day. | `dialog` |
| `message-cron` | Same dialog, with the schedule changed to the custom or cron option and `0 9 1 * *` typed in. Skip if there is no such option. | `dialog` |
| `send-now` | Close the dialog. **Send now**, contact **Mum**, message `On my way!`. | `dialog` |

### Contacts, templates and history (browser `d`, signed in as maya)

| Name | Set-up | Target |
| --- | --- | --- |
| `contacts-search` | `/contacts`, with `an` typed into the search field. | `page` |
| `contact-detail` | Clear the search and open **Jonas Berg**. | `page` |
| `templates-list` | `/templates`. | `page` |
| `template-form` | **New template**. Name `Class reminder`, message `Hi {{first_name\|there}}, class starts at {{time + 30m}} today. See you there!`. | `dialog` |
| `template-use` | Close it. Press **Use** on **Payment reminder** and capture what opens. | `dialog` |
| `history-list` | `/history`. | `page` |

### Account and WhatsApp (browsers `b` and `b2`)

Browser `b` starts signed out.

| Name | Set-up | Target |
| --- | --- | --- |
| `signup` | `/signup`, signed out. | `window` |
| `signin` | `/signin`, signed out. | `window` |
| `forgot-password` | `/forgot-password`, signed out. | `window` |
| `first-run` | `./signin.sh b alex@example.com`, open `/__demo/whatsapp/not_linked`, then `/messages`. | `page` |
| `whatsapp-not-linked` | `/whatsapp`. | `page` |
| `whatsapp-link-password` | Start linking with a QR code. If it asks for the password, type it and capture before confirming. | `dialog` |
| `whatsapp-qr` | Confirm. The page now shows a QR code. | `page` |
| `whatsapp-pair-code` | Cancel linking. Link with a phone number instead, `+44 7700 900142`, and confirm the password. The page shows an 8-character code. | `page` |
| `whatsapp-connected` | Cancel linking. Open `/__demo/whatsapp/connected`, then `/whatsapp`. | `page` |
| `whatsapp-history` | The same page. | `region:Message history` |
| `whatsapp-paused` | Open `/__demo/whatsapp/paused`, then `/whatsapp`. | `page` |
| `whatsapp-paused-messages` | In browser `b2` (maya, who has messages): open `/__demo/whatsapp/paused`, then `/messages`, for the notice it shows. Open `/__demo/whatsapp/connected` afterwards. | `page` |
| `whatsapp-stopped` | Open `/__demo/whatsapp/stopped`, then `/whatsapp`. Afterwards open `/__demo/whatsapp/connected` again. | `page` |
| `settings-two-step` | `./signin.sh b2`, then `/settings`. | `region:Two-step verification` |
| `settings-passkeys` | The same page. | `region:Passkeys` |
| `settings-notifications` | The same page. | `region:Notifications` |
| `two-step-setup` | **Turn on** under Two-step verification, confirm the password, and capture the set-up dialog with its QR code. Then close it without entering a code. | `dialog` |

### Agents (browser `c`, signed in as maya)

| Name | Set-up | Target |
| --- | --- | --- |
| `agents-requests` | `/agents`. | `region:3 requests wait for you` |
| `agents-connections` | The same page. | `table` |
| `agents-connect-app` | The same page. | `region:Connect an app that signs in` |
| `approval-edit` | **Edit** on the request to **Sam Taylor**, which turns its text into a field. Press **Undo edits** afterwards. | `article:first-of-type` |
| `approval-attachment` | `/agents`, the request to **Jonas Berg**, which carries a photo. | `article:has(img)` |
| `approvals-earlier` | `/agents/approvals`. | `page` |
| `agent-activity` | `/agents/activity`. | `page` |
| `agent-new-presets` | `/agents/new`, with `Cursor` typed as the name. | `region:What it may do` |
| `agent-new-chats` | **Only selected chats**. Under **Add a chat**, add **Product Team** and set it to **Yes**, 5 a day, note `Work chat. Keep it short.`. | `region:Chats` |
| `agent-reading` | Tick **Read** on the Product Team row, which also shows **How far back it may read** and the switch for opening pictures and files. Untick it after the shot. | `region:Chats` |
| `agent-new-strangers` | The same page. | `region:People who aren’t saved contacts` |
| `agent-new-account` | The same page. | `region:The account` |
| `agent-new-expiry` | The same page. | `region:Expiry` |
| `agent-key` | Create the connection and confirm the password. Capture the page that shows the new key. | `page` |
| `agent-overview` | `/agents`, then open **Morning briefing**. | `region:Overview` |
| `agent-replace-key` | The same page. | `region:Replace the key` |
| `agent-revoke` | Press **Revoke** there and capture the confirmation. Then cancel. | `dialog` |
