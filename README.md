<img src="web/public/favicon-192.png" alt="" width="96" height="96" />

# Whatomato

Schedule and send WhatsApp messages. Rust backend (`server/`, axum) and a React frontend (`web/`, Vite + TanStack), shipped as one Docker image.

Run it yourself with the image described under [Self-hosting](#self-hosting), or use the hosted service.

## Hosted service

I run a hosted instance at https://whatomato.habibium.com. It runs the same image that is published from this repository.

## Self-hosting

Each release publishes the image as `ghcr.io/habibium/whatomato`, built for `linux/amd64`. It needs a Postgres database (the tests run against 18), and for anything past a trial on your own machine, a domain served over HTTPS and an SMTP server: with an `https` `APP_URL` the server won't start without `SMTP_URL`.

A `compose.yaml` that runs it with a database of its own:

```yaml
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

```sh
# openssl rand -hex 32 (hex, because it goes into DATABASE_URL)
POSTGRES_PASSWORD=
# openssl rand -base64 32
ENCRYPTION_KEY=
SMTP_URL=smtps://user:password@smtp.example.com:465
```

Then `docker compose up -d`. The server applies its migrations as it starts, and `docker compose ps` reports it healthy once `/api/health` answers, which includes reaching the database. Open `APP_URL` and sign up. [Configuration](#configuration) describes every variable.

Sign-up is open to anyone who can reach the instance and no setting closes it, so an instance meant for a few people needs its access limited in front of it. Run one container per database: [Running it](#running-it) explains why, and what a deploy has to respect.

To try it on your own machine first, set `APP_URL` to `http://localhost:8000` and leave `SMTP_URL` empty. The server then writes each email to its log instead of sending it, so the verification link is in `docker compose logs whatomato`.

### HTTPS

The server speaks plain HTTP, so TLS belongs to a reverse proxy in front of it, which is why the example publishes the port on loopback only. Two settings have to agree with the proxy:

- `APP_URL` is the public URL exactly as users open it. Links in emails are built from it, `https` makes the cookies `Secure`, and passkeys are bound to its host, which has to be a domain name rather than an IP address.
- `CLIENT_IP_HEADER` names a header the proxy overwrites with the client's address. Without it every visitor arrives from the proxy's address and shares one rate limit.

With [Caddy](https://caddyserver.com), which obtains the certificate itself:

```caddyfile
whatomato.example.com {
	reverse_proxy 127.0.0.1:8000 {
		header_up X-Real-IP {remote_host}
	}
}
```

and `CLIENT_IP_HEADER: X-Real-IP` added to the container's environment.

Agents connect at `/mcp`, outside `/api`, so a proxy that routes by path has to send `/mcp` to the server as it does `/api`, with the `Authorization` header intact and without caching or buffering the answers. `AGENT_ACCESS=off` turns [agent access](#agent-access) off instead.

### Upgrading

```sh
docker compose pull && docker compose up -d
```

`latest` follows the stable releases, `0.1` the patch releases of one minor version, and a full version such as `0.1.5` never moves; the [changelog](CHANGELOG.md) says what each release changed. Pending migrations run when the new version starts. They only go forward: an older image won't start on a database a newer one has migrated.

### Backups

Dump the database with `docker compose exec -T db pg_dump -U whatomato whatomato > whatomato.sql`, and keep `ENCRYPTION_KEY` somewhere other than with the dumps, since the two together reveal every secret it protects. A restore brings back the accounts, templates, schedules and history, but not the WhatsApp links: follow [After losing the key or restoring a backup](#after-losing-the-key-or-restoring-a-backup).

### Other architectures

The published image is `linux/amd64` only. On anything else, such as arm64, build it from a checkout with `docker build -t whatomato .` and set `image: whatomato`.

## Prerequisites

- Rust nightly (`rustup toolchain install nightly`), plus `cargo install bacon just mprocs git-cliff`
- Node 24+ and `pnpm`
- OpenSSL headers and `pkg-config` (`libssl-dev` on Debian), which the passkey library links against

## Development

```sh
just dev
```

Starts both processes in one terminal via [mprocs](https://github.com/pvolok/mprocs):

| Process  | Command    | URL                                         |
| -------- | ---------- | ------------------------------------------- |
| `server` | `bacon`    | http://localhost:8000 (docs at `/api/docs`) |
| `web`    | `pnpm dev` | http://localhost:5173                       |

mprocs keys: `Ctrl+a` toggles focus between the process list and the terminal, `j`/`k` select a process, `r` restarts it, `q` quits everything. With the terminal focused, keys go to the process, so bacon's shortcuts (`c` clippy, `/` search) still work.

You can also run the two halves in separate panes: `bacon` at the root and `pnpm dev` inside `web/`.

## Typesafe API contract

The backend is the source of truth. utoipa derives an OpenAPI spec from the axum routes; the frontend generates TypeScript types from it.

```
server/src/lib.rs  ──cargo run --bin openapi──▶  openapi.json  ──openapi-typescript──▶  web/src/api/schema.d.ts
```

Both steps run automatically in `just dev`: bacon regenerates `openapi.json` before every server restart, and a Vite plugin watches that file and regenerates `schema.d.ts`. Both generated files are committed, and `just check` fails if either is out of date.

To run the generation by hand: `just gen-api`.

## Commands

| Recipe              | What it does                                                           |
| ------------------- | ---------------------------------------------------------------------- |
| `just dev`          | Start backend and frontend                                             |
| `just check`        | Clippy, tests, OpenAPI freshness, formatting, lint, TS types freshness |
| `just fmt`          | Format Rust and TypeScript                                             |
| `just gen-api`      | Regenerate `openapi.json` and `schema.d.ts`                            |
| `just build`        | Production build: `web/dist` and the release binary                    |
| `just start`        | Run the release binary, serving the SPA and the API on one port        |
| `just docker-build` | Build the production image locally                                     |
| `just changelog`    | Preview the changelog entries since the last release                   |
| `just release`      | Tag a version and push; CI builds, publishes and deploys it            |

Run `just` alone to list recipes.

## Configuration

Environment variables, or a `.env` file in development (see `.env.example`):

| Variable           | Default                                                             | Purpose                                                                                                                                                                                   |
| ------------------ | ------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `DATABASE_URL`     | required                                                            | Postgres connection string                                                                                                                                                                |
| `PORT`             | `8000`                                                              | Listen port                                                                                                                                                                               |
| `WEB_DIR`          | `web/dist`                                                          | Built SPA to serve for non-`/api` paths                                                                                                                                                   |
| `RUST_LOG`         | `warn,server=info` in the image; `warn,server=debug,…` in development | tracing filter. Keep libraries at `warn`: the WhatsApp library logs phone numbers and pair codes at `info`                                                                               |
| `APP_URL`          | required in release builds; `http://localhost:5173` in debug builds | Public URL of the app. Links in emails point here, and an `https` URL makes the cookies `Secure`                                                                                          |
| `SMTP_URL`         | required when `APP_URL` is `https`                                  | SMTP server for verification and password reset emails, e.g. `smtps://user:password@host:465`                                                                                             |
| `EMAIL_FROM`       | required with SMTP                                                  | Sender, e.g. `Whatomato <no-reply@example.com>`                                                                                                                                           |
| `ENCRYPTION_KEY`   | required in release builds; a fixed development key in debug builds | Encrypts secrets kept in the database, such as authenticator app keys and WhatsApp device keys: 32 random bytes in base64, from `openssl rand -base64 32`                                 |
| `CLIENT_IP_HEADER` | unset                                                               | Header a trusted proxy sets to the client's address, e.g. `CF-Connecting-IP` behind Cloudflare. Unset, rate limits key on the peer address. `X-Forwarded-For` and `Forwarded` are refused |
| `MAX_LINKED_ACCOUNTS` | unset                                                               | How many WhatsApp accounts may be linked at once. At the limit, users without a linked account can't start linking; those already linked are unaffected                                   |
| `AGENT_ACCESS`     | `on`                                                                | `off` turns agent access off: `/mcp` and the agent routes answer 404 and no agent's schedule runs                                                                                         |

Migrations in `server/migrations` are embedded in the binary and applied at startup.

Without `SMTP_URL` the server writes each email, link included, to its log, which is enough for development and refused for an `https` `APP_URL`, since anyone who can read the log could use the links. Percent-encode special characters in the SMTP password (`@` becomes `%40`). `smtps://` connects on port 465 with TLS from the start; for port 587 use `smtp://user:password@host:587?tls=required`.

Keep `ENCRYPTION_KEY` apart from the database and its backups: together they reveal every secret it protects. Replacing or losing it stops every authenticator app set up so far from working, and those users have to sign in with a recovery code and set two-step verification up again. It also cuts off every linked WhatsApp account: see [After losing the key or restoring a backup](#after-losing-the-key-or-restoring-a-backup).

## Accounts

Sign-up takes an email address and a password. The account cannot sign in until the address is verified through the emailed link, which works for 24 hours. Opened in the browser that signed up, the link also signs the user in; anywhere else it only verifies the address, so a link opened in someone else's browser cannot sign them into the wrong account. Password reset links work once, for an hour, and end every session and outstanding link of the account. Sessions last 30 days and extend while in use; the browser holds them in an `HttpOnly` cookie, and the database stores only a SHA-256 hash of each session and link token.

Writes under `/api/auth` are rate-limited per client address: a burst of 20, then one every 6 seconds, with IPv6 addresses counted per /64. Sign-in attempts are also limited per account and password re-checks per user, however many addresses they come from: 10, then one every 90 seconds. Second-factor codes, whether at sign-in or while turning two-step verification on, are limited per account to 5, then one every 30 minutes, however many sign-ins they are spread over. A refused request gets a 429 with `Retry-After`. The limits live in memory, so they are per instance and reset on restart. Behind a proxy, set `CLIENT_IP_HEADER`, or every client shares the proxy's address; the server trusts that header as sent, so it must only be reachable through the proxy.

Sign-up and password reset refuse passwords listed in Have I Been Pwned's [Pwned Passwords](https://haveibeenpwned.com/API/v3#PwnedPasswords). Only the first 5 characters of the password's SHA-1 hash leave the server. When the service does not answer within 3 seconds, the password is accepted and a warning logged.

Two-step verification is optional and works with any authenticator app (TOTP: SHA-1, 6 digits, 30-second steps, one step of clock drift allowed). Turning it on, turning it off and replacing the recovery codes ask for the password again and email the owner, and turning it on also ends every other session. A new key waits 15 minutes for the first code from the app, and five wrong codes end that setup. With it on, the password starts a sign-in that waits five minutes for a code from the app or one of ten single-use recovery codes, and each app code works once. Five wrong codes end the sign-in and email the owner, since whoever tried had the password; that email goes out at most once every 6 hours per account. Starting over from the password does not buy more guesses: an account takes 30 wrong codes a day across all its sign-ins, and only a right code or a password reset clears the count. A sign-in with a recovery code emails the owner too, with how many codes are left. A password reset ends a sign-in that is waiting for its code but leaves two-step verification on. The app's key is encrypted with `ENCRYPTION_KEY` (XChaCha20-Poly1305, with the user's id bound in), and recovery codes and waiting sign-ins are stored only as SHA-256 hashes.

Passkeys are added under Security settings, after the password is entered again, and sign in without an email address or password: from the "Sign in with a passkey" button, or from the browser's autofill on the email field. A passkey always verifies its user with a PIN or biometrics, so a passkey sign-in needs no other step. Each passkey is bound to the host of `APP_URL`, its relying party ID, so moving the app to another host leaves existing passkeys unusable. Removing one asks for the password too, and either change emails the owner. A registration or sign-in challenge stays on the server for five minutes, named by an `HttpOnly` cookie, and works once.

## WhatsApp

Each user links one WhatsApp account as a linked device, from the WhatsApp page, by scanning a QR code or by typing a code on the phone. The server then holds a connection for that account around the clock and reconnects it after a restart, so messages can go out with no browser open. Linking and unlinking ask for the password again, and a user can start 5 links, then one every 10 minutes.

> [!WARNING]
> WhatsApp doesn't officially allow third-party clients. It can ban a number that uses one, for a while or for good, and sending the same text to many people or messaging people who don't have your number makes that likelier. Link a number you can afford to lose.

Whatomato talks to WhatsApp through the [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) library, as a linked device of the user's own account. It isn't the WhatsApp Business API.

### What's stored

Postgres holds what a linked device has to keep to stay linked and to encrypt messages:

- the device's identity and the Signal keys and sessions for the people and groups it talks to;
- the app state keys and versions WhatsApp syncs between devices;
- recent sent messages, which WhatsApp can ask a device to send again when a recipient failed to decrypt them;
- the linked number, and when it was linked, paused or stopped;
- the contacts saved on the phone, which it shares with linked devices: their names, numbers and WhatsApp usernames, so a message can go to a name. The first connect after linking replays the whole address book once; after that each change on the phone comes through as it's made.

Profile pictures of saved contacts and groups aren't stored: each is looked up when the app shows it, at most two at a time, and kept in the server's memory for a few hours.

Everything secret is encrypted with `ENCRYPTION_KEY` (XChaCha20-Poly1305), bound to its account, table and key, so a database dump alone can't take over a device. Contact names are encrypted the same way. Lookup keys, timestamps and flags stay in the clear, and those include each contact's number and WhatsApp address. Whatomato doesn't download chat history or store the messages it receives.

What a user writes is not encrypted: templates, scheduled messages and the delivery history keep the message text and the recipient in plain columns, so a database dump does show those. Deleting a schedule keeps its history.

For [agent access](#agent-access), Postgres keeps each connection with only the SHA-256 hash of its key and the key's last 4 characters, and its permissions as they were saved, chat names and notes included, unencrypted. A request waiting for approval keeps its recipient in the clear and its text sealed with `ENCRYPTION_KEY`, and goes when its connection is revoked. The activity log records every call an agent makes, with the chat it named and the outcome but never the message text, and is deleted after 90 days. Messages an agent sends or schedules are kept like any other, with the connection that made them.

Unlinking asks WhatsApp to remove the device, then deletes the account's row and, through the cascade, every key. When WhatsApp can't be reached the keys go anyway, and the page tells the user to remove the device from their phone. A link, an unlink, a removal from the phone, another session taking over the keys and a temporary ban each email the owner.

### Staying linked

Disconnecting pauses the account and keeps the link, across restarts too. WhatsApp logs out a linked device that stays offline too long, so a paused account connects for about two minutes once a week. It sends nothing, and if WhatsApp can't be reached it tries again an hour later. An account stopped by a takeover or a ban stays offline until its owner reconnects it, and WhatsApp may log out those devices in the end. WhatsApp also logs out every linked device when the phone itself stays unused for about two weeks, which nothing here can prevent.

A connected account shows as online for 45 seconds once a day, which keeps the phone from flagging the device as inactive. The rest of the time it stays unavailable, so the phone keeps getting notifications.

### Scheduling

A message goes out once at a set time, or repeatedly on a five-field cron schedule read in the time zone it was created in. Minutes are the finest unit, and each account sends at most one message every 3 seconds, however many are due, since sending like a machine is one way to get banned. A server that was down sends what it missed once when it's back, and a repeating message then moves on to its next time.

A message or template can hold variables, filled in as each message is queued: `{{first_name}}` and `{{name}}` from the name saved on the phone (or the group's name), and `{{day}}`, `{{date}}`, `{{time}}`, `{{month}}` and `{{year}}` for when it's due, in its own time zone. A fallback goes after a bar, so `{{first_name|there}}` says "there" when there's no name. Saving refuses a variable that doesn't exist, and the history keeps the text as it went out.

A delivery reads as sent once WhatsApp acknowledges it, which doesn't prove it reached the phone. A contact is checked first, so a number that isn't on WhatsApp fails instead of appearing to work. While WhatsApp is reconnecting a send waits and tries again for about half an hour. It fails straight away when the account is paused, stopped or unlinked, because only its owner can fix that. Anything that may already have gone out, such as a send the server restarted during or one WhatsApp never confirmed, fails with a message saying so rather than risking a second copy.

### Running it

Only one server may run a given account's client: two on the same keys knock each other off and can corrupt them. So a server connects no account and sends nothing until it holds a Postgres advisory lock, which it lets go after stopping its clients, and either order of starting and stopping works during a deploy.

Give the container at least 30 seconds to stop (`docker stop --time 30`, `stop_grace_period` in Compose or Swarm), because each client flushes its keys on the way down and Docker's default of 10 can cut that short.

### After losing the key or restoring a backup

Linked devices can't survive either of these, so users have to link again:

- **A lost or replaced `ENCRYPTION_KEY`:** the keys can't be opened any more.
- **A database restored from a backup:** the Signal keys in it have moved on since the backup, and reusing them can break decryption and reuse message keys.

Stop the server, then delete the accounts, which takes every key with them:

```sql
DELETE FROM whatsapp_accounts;
```

Start the server, tell users to link again, and have them remove the old device under Linked devices on the phone.

Never start a server on a copy of the database while the original runs, such as for a restore drill, unless the copy has had that `DELETE`: the advisory lock only covers one database, so the copy would connect with the same keys and take the accounts over.

## Agent access

You can let an AI agent, such as Claude Code, Codex, OpenCode, Antigravity or Cursor, use your account. Depending on what you allow, it can find chats by name, send a message, schedule messages, and check whether they went out. It can't read your chats, since Whatomato doesn't store the messages you receive. This is optional: if you never create a connection, nothing changes.

### Connecting an agent

On the Agents page, create a connection: give it a name, choose its permissions and enter your password. The page then shows its access key once, so copy it before you leave. Below the key are the setup steps for each client. Agents talk to Whatomato over MCP, the protocol these tools use to reach outside services, at your site's address followed by `/mcp`. For Claude Code it's one command:

```sh
claude mcp add --transport http whatomato https://whatomato.example.com/mcp \
  --header "Authorization: Bearer $WHATOMATO_KEY"
```

### Permissions

A connection starts from one of these, which you can then change:

- Ask before sending, the default: the agent can find chats, read templates and manage its own schedules, and every send waits for you.
- View only: it can find chats, read templates and see what it sent, but not send or schedule.
- Send without asking: it sends without asking, up to 20 messages a day per chat.
- Full access: it sends without asking, messages people who aren't saved contacts, and manages every schedule and delivery, not just its own.

A connection covers all chats or only the ones you select. Each chat has a rule: whether the agent may send there (no, ask me first, or yes), an optional daily limit, and an optional note for the agent, such as "reply in Bangla". The agent sees the note, but nothing enforces it. A separate setting covers people who aren't saved contacts: never, ask me first, or allowed.

Any change to a connection's permissions asks for your password again and emails you. A connection can also be given an expiry date.

### Approvals

When a send or a schedule needs your approval, it appears at the top of the Agents page with the recipient and the whole text, and you get an email, at most one an hour. You can approve it, edit the text and then approve it, or reject it. Your approval is final, even if you've since changed the connection's permissions or paused it, so the request shows whether the connection is still active. A request nobody answers expires after 24 hours.

### Limits

No permission lifts these:

- A connection sends at most 200 messages in 24 hours, counting the requests still waiting for approval.
- It can have at most 20 requests waiting for approval.
- Over a chat's daily limit, a send is refused rather than turned into a request.
- An agent can't schedule a message more often than once an hour. You can, from the web app.
- A connection makes at most 120 calls a minute, and an account has at most 20 connections.

An agent's schedule is checked again each time it's due. If the connection is paused, expired or over a limit, or the chat's rule now says no, that run is skipped and shows as a failed delivery that says why.

An agent can never link or unlink WhatsApp, change your account or security settings, create or change connections, approve its own requests, or read your chats.

### Pausing and revoking

You can pause one connection or all of them at once, and resume them later. A paused agent can't do anything, and its schedules don't run. Revoking a connection deletes it and its waiting requests; it offers to delete the schedules the agent created too, and does so unless you untick it. The history of what it sent stays. The activity log on the Agents page lists every call each agent made.

> [!WARNING]
> WhatsApp's terms don't allow automated messaging, and it bans accounts for it. An agent sending for you raises that risk, and messages to people who haven't saved your number are the riskiest. An agent can also be tricked by text it reads elsewhere, such as a web page or an email, into sending something you didn't intend, which is why every send asks you first by default. Anything you tell or show an agent, including the chat names and notes it gets from Whatomato, goes to the company that runs its model.

## License

Whatomato is licensed under the GNU Affero General Public License, version 3 only ([`AGPL-3.0-only`](LICENSE)). Anyone who runs a modified version as a network service must offer its users the source of that version. The image links `wacore-libsignal`, part of whatsapp-rust, whose sources are `AGPL-3.0-only` despite its MIT package metadata ([upstream issue](https://github.com/oxidezap/whatsapp-rust/issues/1828)); this project's own license covers that.

The tests ported from [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) keep its MIT license; see [`LICENSES/whatsapp-rust-MIT.txt`](LICENSES/whatsapp-rust-MIT.txt).
