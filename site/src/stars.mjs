import { SOURCE_URL } from './consts.mjs'

export const REPO = new URL(SOURCE_URL).pathname.slice(1)

/** @returns {Promise<number | null>} */
async function fetchStars() {
  /** @type {Record<string, string>} */
  const headers = {
    Accept: 'application/vnd.github+json',
    'User-Agent': 'whatomato-docs',
  }
  if (process.env.GITHUB_TOKEN) {
    headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`
  }
  try {
    const response = await fetch(`https://api.github.com/repos/${REPO}`, {
      headers,
      signal: AbortSignal.timeout(5000),
    })
    if (!response.ok) return null
    const { stargazers_count } = await response.json()
    return typeof stargazers_count === 'number' ? stargazers_count : null
  } catch {
    // An offline build still succeeds; the page fetches the count itself
    return null
  }
}

// Evaluated once per build, not once per page
export const stars = fetchStars()
