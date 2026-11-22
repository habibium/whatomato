---
title: HTTPS and reverse proxies
description: The server speaks plain HTTP, so TLS belongs to a reverse proxy in front of it. This page covers what the proxy has to pass through.
---

The server speaks plain HTTP on its port. Put a reverse proxy in front of it to terminate TLS, and publish the server's port on loopback only, as the [compose file](/self-hosting/quickstart/) does.

## Two settings that have to agree with the proxy

- `APP_URL` is the public address exactly as users open it, starting with `https://`.
- `CLIENT_IP_HEADER` names a header the proxy overwrites with the client's address. Without it every visitor arrives from the proxy's address and shares one rate limit.

## With Caddy

[Caddy](https://caddyserver.com) obtains the certificate itself:

```text
whatomato.example.com {
	reverse_proxy 127.0.0.1:8000 {
		header_up X-Real-IP {remote_host}
	}
}
```

Then add `CLIENT_IP_HEADER: X-Real-IP` to the container's environment.

## Behind Cloudflare

Set `CLIENT_IP_HEADER` to `CF-Connecting-IP`. The server trusts that header as sent, so make sure the origin only accepts connections from Cloudflare. Otherwise anyone who reaches the origin directly can claim any address.

## Paths the proxy has to send to the server

Everything goes to the one server. A proxy that routes by path has to send these to it, without caching or buffering the answers:

| Path | What it is |
| --- | --- |
| `/api` | The web app's API. |
| `/mcp` | Where agents connect. The `Authorization` header has to arrive intact. |
| `/oauth` | Sign-in for apps that connect without a key. |
| `/.well-known/oauth-protected-resource` and `/.well-known/oauth-authorization-server` | The discovery documents those apps read first. |

Every other path is the web app.

## The event stream

A page that is open listens at `/api/agents/events`, a stream that stays open. A proxy mustn't buffer it. The server sends a line every 20 seconds, which keeps the stream from being closed as idle.

## What the server connects out to

- **WhatsApp**, for every linked account.
- **Your SMTP server**, to send email.
- **Have I Been Pwned**, to check new passwords against known breaches. Only the first 5 characters of the password's SHA-1 hash leave the server, and if the service doesn't answer within 3 seconds the password is accepted.
- **Browser push services**, to send notifications: `fcm.googleapis.com`, `updates.push.services.mozilla.com`, `web.push.apple.com` and hosts under `notify.windows.com`.
- **An app's own site**, when a user connects an app by signing in: the server fetches the details the app publishes about itself.

A firewall that limits outgoing connections has to allow these.
