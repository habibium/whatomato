# Whatomato documentation

The public documentation site, built with [Astro](https://astro.build) and [Starlight](https://starlight.astro.build). It is a static site that deploys by itself, apart from the app image, so fixing a page never restarts anyone's WhatsApp connection.

## Commands

Run these from the repository root.

| Command | What it does |
| --- | --- |
| `pnpm --dir site install` | Install the dependencies, once |
| `just docs` | Serve the site at http://localhost:4321, reloading as pages change |
| `just check-site` | What CI runs: type-check, then build, which fails on a broken internal link |
| `pnpm --dir site preview` | Serve the last build |

## Layout

| Path | What it holds |
| --- | --- |
| `src/content/docs/` | The pages, as Markdown. The folder is the URL |
| `astro.config.mjs` | Site settings and the sidebar, listed by hand |
| `src/consts.mjs` | The site's address, the app's address, the source address and the first page |
| `src/stars.mjs` | Fetches the repository's star count for the header, once per build |
| `src/styles/theme.css` | The app's colours, type and framed sheet, carried over to Starlight |
| `src/components/` | The Starlight components replaced to match the app, and `Screenshot`. `Header` centres the search box; `SocialIcons` holds the star count and **Open app**, in the header and the phone menu |
| `src/assets/screenshots/` | The pictures of the app, each as a light and a dark file |
| `screenshots/` | The demo the pictures are taken from, and the scripts that take them |
| `integrations/changelog.mjs` | Writes the Changelog page from the repository's `CHANGELOG.md` |
| `public/_headers` | Cache and security headers Cloudflare applies |
| `public/_redirects` | Sends `/` to the first page with a 301, since the site has no front page |
| `wrangler.jsonc` | Where and how the site deploys |

## Writing a page

1. Add a Markdown file under `src/content/docs/`, with a `title` and a `description`. The description is shown under the title as the page's opening line.
2. Add its slug to the sidebar in `astro.config.mjs`.
3. Link to other pages with absolute paths that end in a slash, such as `/guides/templates/`.

Use `.mdx` only when a page needs a component, such as `Steps`, `Tabs` or `Screenshot`. In MDX a bare `{` starts an expression, so keep `{{variables}}` inside code.

A picture of the app goes in with `<Screenshot name="messages-list" alt="..." />`, after `import Screenshot from '../../../components/Screenshot.astro'`. It shows the light or the dark file to match the reader's theme. [screenshots/README.md](screenshots/README.md) says how the pictures are taken, and from what.

The site has no front page: `/` redirects to `HOME_PATH` in `src/consts.mjs`, by `public/_redirects` on Cloudflare and by the `redirects` setting in `astro.config.mjs` everywhere else. Change both together.

The header's star count is fetched when the site builds, with `GITHUB_TOKEN` if it is set, and each page asks GitHub again at most once an hour. A build that can't reach GitHub shows the button without a count.

The site also publishes itself for language models at `/llms.txt`, `/llms-full.txt` and `/llms-small.txt`.

## Deploying

The site is served by Cloudflare as static assets of a Worker with no script. `.github/workflows/docs.yml` builds it on every pull request that touches it, and builds and deploys it on every push to `main` that does. A release changes `CHANGELOG.md` and `openapi.json`, so it redeploys the docs with the new version and changelog.

One-time setup:

1. In Cloudflare, create an API token from the **Edit Cloudflare Workers** template, limited to the account and to the zone the docs host is in.
2. In the repository's settings on GitHub, create an environment named `docs` and add two secrets to it: `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.
3. Run the **Docs** workflow by hand, or push a change under `site/`.

The first deploy creates the Worker, the DNS record for the host in `wrangler.jsonc` and its certificate. Until the secrets exist, the workflow builds the site and skips the deploy.

To serve the site from another address, change `SITE_URL` in `src/consts.mjs` and the route in `wrangler.jsonc` together.

To deploy from your own machine instead: `pnpm --dir site run deploy`, after `pnpm --dir site exec wrangler login`.
