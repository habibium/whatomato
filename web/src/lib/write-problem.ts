import type { components } from '@/api/schema'

import {
  TOO_MANY_ATTEMPTS,
  retryAfterSeconds,
  tryAgainIn,
} from '@/lib/retry-after'

export interface WriteProblem {
  title: string
  description: string
}

const SERVER_FAULT = 'Something went wrong on our side. Try again in a moment.'

/** `failed` becomes the title, as in "Couldn't save." */
export function toWriteProblem(
  result: { response: Response; error?: unknown },
  failed: string,
): WriteProblem {
  const { response, error } = result
  if (response.status === 429) {
    return {
      title: TOO_MANY_ATTEMPTS,
      description: tryAgainIn(retryAfterSeconds(response)),
    }
  }
  const reason = (error as components['schemas']['ErrorBody'] | undefined)
    ?.error
  if (response.status === 409 && reason) {
    return { title: failed, description: `${reason}.` }
  }
  return { title: failed, description: SERVER_FAULT }
}

export function connectionProblem(failed: string): WriteProblem {
  return { title: failed, description: 'Check your connection and try again.' }
}
