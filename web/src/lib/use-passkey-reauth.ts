import { useCallback, useEffect, useRef, useState } from 'react'

import { $api, api } from '@/api/client'
import { getPasskey, hasPasskeySupport, toPasskeyFailure } from '@/lib/passkey'
import type { WebAuthnJSON } from '@/lib/passkey'

/** What a change behind the password takes: the password, or a passkey in its place. */
export type Reauth = { password: string } | { passkey: WebAuthnJSON }

export type PasskeyAnswer = { passkey: WebAuthnJSON } | 'cancelled' | 'failed'

// The server keeps a challenge for five minutes; an older one is fetched again.
const CHALLENGE_REUSE_MS = 4 * 60_000

/**
 * Offers the user's passkeys in place of the password, where the browser can use them and the
 * account has any. `isActive` while the form asking is open.
 */
export function usePasskeyReauth(isActive: boolean) {
  const [isSupported] = useState(hasPasskeySupport)
  const passkeys = $api.useQuery('get', '/api/auth/passkeys', undefined, {
    enabled: isSupported,
  })
  const isAvailable = isSupported && (passkeys.data?.length ?? 0) > 0
  const challenge = useRef<{
    publicKey: Promise<WebAuthnJSON>
    fetchedAt: number
  }>(null)

  const prepare = useCallback(() => {
    const current = challenge.current
    if (current && Date.now() - current.fetchedAt < CHALLENGE_REUSE_MS) {
      return current.publicKey
    }
    const publicKey = api
      .POST('/api/auth/passkeys/reauth/start')
      .then(({ data }) => {
        if (!data) throw new Error('Could not start a passkey check')
        return data.publicKey
      })
    challenge.current = { publicKey, fetchedAt: Date.now() }
    publicKey.catch(() => {
      if (challenge.current?.publicKey === publicKey) challenge.current = null
    })
    return publicKey
  }, [])

  // Fetched ahead of the click, so the browser's prompt follows the click with no request in
  // between: Safari only shows it while the click still counts as user activation.
  useEffect(() => {
    if (isActive && isAvailable) prepare().catch(() => {})
  }, [isActive, isAvailable, prepare])

  const answer = useCallback(async (): Promise<PasskeyAnswer> => {
    let publicKey: WebAuthnJSON
    try {
      publicKey = await prepare()
    } catch {
      return 'failed'
    }
    try {
      const passkey = await getPasskey(publicKey, 'optional')
      // Sending it spends the challenge, whatever the server makes of it.
      challenge.current = null
      return { passkey }
    } catch (error) {
      return toPasskeyFailure(error) === 'cancelled' ? 'cancelled' : 'failed'
    }
  }, [prepare])

  return { isAvailable, answer }
}
