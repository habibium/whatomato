import { mkdir, readFile, writeFile } from 'node:fs/promises'

import { SOURCE_URL } from '../src/consts.mjs'

const source = new URL('../../CHANGELOG.md', import.meta.url)
const target = new URL(
  '../src/content/docs/reference/changelog.md',
  import.meta.url,
)

const frontmatter = `---
title: Changelog
description: What changed in each release of Whatomato.
editUrl: false
tableOfContents:
  maxHeadingLevel: 2
---
`

const longDate = new Intl.DateTimeFormat('en-GB', {
  day: 'numeric',
  month: 'long',
  year: 'numeric',
  timeZone: 'UTC',
})

/** git-cliff's output, reshaped into a page: no title of its own, dated versions, linked commits */
export function toPage(markdown) {
  const body = markdown
    .replace(/^# .*\n+/, '')
    .replace(
      /^## \[(.+?)\] - (\d{4}-\d{2}-\d{2})$/gm,
      (_, version, date) =>
        `\n## ${version}\n\nReleased ${longDate.format(new Date(`${date}T00:00:00Z`))}.`,
    )
    .replace(/^## \[(.+?)\]$/gm, '\n## $1')
    .replace(
      / \(([0-9a-f]{7})\)$/gm,
      (_, hash) => ` ([${hash}](${SOURCE_URL}/commit/${hash}))`,
    )
  return `${frontmatter}${body.trimEnd()}\n`
}

/** Publishes the repository's CHANGELOG.md as the Changelog page, so releases need no docs edit */
export function changelog() {
  return {
    name: 'whatomato-changelog',
    hooks: {
      'astro:config:setup': async ({ addWatchFile }) => {
        addWatchFile(source)
        await mkdir(new URL('./', target), { recursive: true })
        await writeFile(target, toPage(await readFile(source, 'utf8')))
      },
    },
  }
}
