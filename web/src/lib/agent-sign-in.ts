import { queryOptions } from '@tanstack/react-query'

import type { components } from '@/api/schema'

import { api } from '@/api/client'
import { ApiError } from '@/lib/whatsapp'

export type AuthorizationRequest = components['schemas']['AuthorizationRequest']
export type SignInCheck = components['schemas']['SignInCheck']
export type SignInAsk = components['schemas']['SignInAsk']

const FIELDS = [
  'response_type',
  'client_id',
  'redirect_uri',
  'code_challenge',
  'code_challenge_method',
  'state',
  'resource',
  'scope',
] as const

/**
 * The app's request as the server packed it into `?request=`: `q.` and the query the app sent, in
 * base64url. Read here, not by the router, which would turn a `state` of digits into a number.
 */
export function unpackRequest(
  packed: string | undefined,
): AuthorizationRequest | null {
  if (!packed?.startsWith('q.') || packed.length === 2) return null
  try {
    const base64 = packed.slice(2).replaceAll('-', '+').replaceAll('_', '/')
    const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0))
    const params = new URLSearchParams(new TextDecoder().decode(bytes))
    const request: AuthorizationRequest = {}
    for (const field of FIELDS) request[field] = params.get(field)
    return request
  } catch {
    return null
  }
}

export function signInCheckQuery(request: AuthorizationRequest) {
  return queryOptions({
    queryKey: ['agents', 'sign-in', request],
    queryFn: async (): Promise<SignInCheck> => {
      const { data, response } = await api.POST('/api/agents/sign-in/check', {
        body: request,
      })
      if (!data) throw new ApiError(response.status)
      return data
    },
    // The answer decides where the browser goes next, so it's asked once and never from a cache
    staleTime: Infinity,
    gcTime: 0,
    refetchOnWindowFocus: false,
    retry: (failures, error) =>
      !(error instanceof ApiError && error.status < 500) && failures < 2,
  })
}
