// @ts-check
import { readFileSync } from 'node:fs'

import starlight from '@astrojs/starlight'
import { defineConfig } from 'astro/config'
import starlightLinksValidator from 'starlight-links-validator'
import starlightLlmsTxt from 'starlight-llms-txt'

import { changelog } from './integrations/changelog.mjs'
import { HOME_PATH, SITE_URL, SOURCE_URL } from './src/consts.mjs'

// openapi.json carries the Cargo workspace version, as it does for the web app
const appVersion = JSON.parse(
  readFileSync(new URL('../openapi.json', import.meta.url), 'utf8'),
).info.version

const description =
  'Whatomato schedules and sends WhatsApp messages from your own number, and lets AI agents do the same within the permissions you set.'

export default defineConfig({
  site: SITE_URL,
  trailingSlash: 'always',
  // There is no front page; public/_redirects does the same on Cloudflare with a 301
  redirects: { '/': HOME_PATH },
  vite: {
    define: { __APP_VERSION__: JSON.stringify(appVersion) },
  },
  integrations: [
    changelog(),
    starlight({
      title: 'Whatomato',
      description,
      favicon: '/favicon-32.png',
      head: [
        {
          tag: 'link',
          attrs: {
            rel: 'icon',
            type: 'image/png',
            sizes: '192x192',
            href: '/favicon-192.png',
          },
        },
        {
          tag: 'link',
          attrs: { rel: 'apple-touch-icon', href: '/apple-touch-icon.png' },
        },
      ],
      editLink: { baseUrl: `${SOURCE_URL}/edit/main/site/` },
      customCss: ['./src/styles/fonts.css', './src/styles/theme.css'],
      components: {
        Footer: './src/components/Footer.astro',
        Header: './src/components/Header.astro',
        PageTitle: './src/components/PageTitle.astro',
        Pagination: './src/components/Pagination.astro',
        SiteTitle: './src/components/SiteTitle.astro',
        SocialIcons: './src/components/SocialIcons.astro',
      },
      expressiveCode: {
        themes: ['github-dark', 'github-light'],
        useStarlightUiThemeColors: true,
        // No terminal window dressing on shell blocks
        defaultProps: { frame: 'code' },
        styleOverrides: {
          borderColor: 'var(--wa-line)',
          borderRadius: 'var(--wa-radius-element)',
          codeBackground: 'var(--wa-code)',
          codeFontSize: '0.8125rem',
          frames: {
            editorActiveTabIndicatorTopColor: 'transparent',
            editorTabBarBackground: 'var(--wa-code)',
            frameBoxShadowCssValue: 'none',
            terminalBackground: 'var(--wa-code)',
            terminalTitlebarBackground: 'var(--wa-code)',
          },
        },
      },
      plugins: [
        starlightLinksValidator(),
        starlightLlmsTxt({
          projectName: 'Whatomato',
          description,
          details:
            'Whatomato is free software (AGPL-3.0-only) that runs as one Docker image with a Postgres database. It links to WhatsApp as a linked device of the user’s own account through the whatsapp-rust library; it is not the WhatsApp Business API. Agents reach an account over MCP at `/mcp` on the instance, with an access key or by signing in with OAuth.',
          customSets: [
            {
              label: 'Agent access',
              description:
                'connecting an AI agent over MCP, permissions, approvals, limits and the tool reference',
              paths: ['agents/**', 'reference/mcp-tools'],
            },
            {
              label: 'Self-hosting',
              description:
                'running your own instance: Docker Compose, configuration, HTTPS, upgrades and backups',
              paths: ['self-hosting/**'],
            },
          ],
          demote: ['reference/changelog'],
          exclude: ['reference/changelog'],
        }),
      ],
      sidebar: [
        {
          label: 'Start here',
          items: ['start/what-it-is', 'start/quickstart', 'start/whatsapp-risk'],
        },
        {
          label: 'Use Whatomato',
          items: [
            'guides/link-whatsapp',
            'guides/schedule-a-message',
            'guides/recipients',
            'guides/templates',
            'guides/history',
            'guides/account-security',
            'guides/notifications',
          ],
        },
        {
          label: 'Agents',
          items: [
            'agents/overview',
            'agents/connect',
            'agents/permissions',
            'agents/approvals',
            'agents/reading-chats',
            'agents/manage',
            'agents/safety',
          ],
        },
        {
          label: 'Self-hosting',
          items: [
            'self-hosting/quickstart',
            'self-hosting/configuration',
            'self-hosting/https',
            'self-hosting/upgrading',
            'self-hosting/backups',
            'self-hosting/operations',
          ],
        },
        {
          label: 'Reference',
          items: [
            'reference/schedules',
            'reference/variables',
            'reference/mcp-tools',
            'reference/limits',
            'reference/data',
            'reference/changelog',
          ],
        },
        {
          label: 'Project',
          items: ['project/development', 'project/architecture', 'project/license'],
        },
      ],
    }),
  ],
})
