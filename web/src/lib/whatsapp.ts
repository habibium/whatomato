import { queryOptions } from '@tanstack/react-query'
import { parsePhoneNumberFromString } from 'libphonenumber-js'

import type { components } from '@/api/schema'

import { api } from '@/api/client'

export type WhatsAppStatus = components['schemas']['WhatsAppStatus']
export type Pairing = Extract<WhatsAppStatus, { status: 'pairing' }>
export type NotLinked = Extract<WhatsAppStatus, { status: 'not_linked' }>
export type Linked = Extract<WhatsAppStatus, { phone_number: string }>
export type Group = components['schemas']['GroupResponse']

export const CONNECTION_DOTS = {
  not_linked: { label: 'Not linked', variant: 'neutral', isPulsing: false },
  pairing: { label: 'Linking', variant: 'accent', isPulsing: true },
  syncing: { label: 'Syncing', variant: 'accent', isPulsing: true },
  connected: { label: 'Connected', variant: 'success', isPulsing: false },
  reconnecting: { label: 'Reconnecting', variant: 'warning', isPulsing: true },
  paused: { label: 'Paused', variant: 'neutral', isPulsing: false },
  stopped: { label: 'Stopped', variant: 'error', isPulsing: false },
} as const

/** A response that wasn't a success, kept apart so a lost session reads differently from an outage. */
export class ApiError extends Error {
  readonly status: number

  constructor(status: number) {
    super(`Request failed with ${status}`)
    this.status = status
  }
}

interface StatusSnapshot {
  status: WhatsAppStatus
  /** Milliseconds to add to this device's clock to read the server's. */
  clockOffset: number
}

// Codes rotate every 20 to 60 seconds, and a sync or a reconnect settles within moments.
const FAST_POLL_MS = 2_000
const SLOW_POLL_MS = 15_000

export const statusQuery = queryOptions({
  queryKey: ['whatsapp', 'status'],
  queryFn: async (): Promise<StatusSnapshot> => {
    const { data, response } = await api.GET('/api/whatsapp/status')
    if (!data) throw new ApiError(response.status)
    return { status: data, clockOffset: clockOffset(response) }
  },
  refetchInterval: (query) => {
    const status = query.state.data?.status.status
    if (query.state.error instanceof ApiError) {
      return query.state.error.status === 401 ? false : SLOW_POLL_MS
    }
    return status === 'pairing' ||
      status === 'syncing' ||
      status === 'reconnecting'
      ? FAST_POLL_MS
      : SLOW_POLL_MS
  },
  retry: (failures, error) =>
    !(error instanceof ApiError && error.status === 401) && failures < 2,
})

const collator = new Intl.Collator(undefined, { sensitivity: 'base' })

export const groupsQuery = queryOptions({
  queryKey: ['whatsapp', 'groups'],
  queryFn: async (): Promise<Group[]> => {
    const { data, response } = await api.GET('/api/whatsapp/groups')
    if (!data) throw new ApiError(response.status)
    return [...data].sort((a, b) => collator.compare(a.name, b.name))
  },
  staleTime: 60_000,
  // 409 means the link dropped, which a status refresh shows better than a retry
  retry: (failures, error) =>
    !(error instanceof ApiError && error.status < 500) && failures < 2,
})

/**
 * A code's expiry and a ban's end come in server time; a phone set a minute fast would otherwise
 * hide every code as expired. `Date` truncates to whole seconds, hence the half second added.
 */
function clockOffset(response: Response) {
  const serverTime = Date.parse(response.headers.get('Date') ?? '')
  if (Number.isNaN(serverTime)) return 0
  // Rounded, so a poll that lands a few milliseconds later doesn't count as new data
  return Math.round((serverTime + 500 - Date.now()) / 1000) * 1000
}

export const NUMBER_HINT = 'Enter the whole number, with the country code'

export function digitsOf(value: string) {
  return value.replace(/\D/g, '')
}

/** The server's rule: formatting characters only, and 8 to 15 digits (E.164). */
export function isPhoneNumber(value: string) {
  return (
    /^\+?[\d\s()-]*$/.test(value.trim()) && /^\d{8,15}$/.test(digitsOf(value))
  )
}

/** The API sends the digits with the country code and no plus. */
export function formatPhoneNumber(digits: string) {
  const international = `+${digits}`
  return (
    parsePhoneNumberFromString(international)?.formatInternational() ??
    international
  )
}

/** Split in two, the way WhatsApp lays out the boxes it asks for. */
export function formatPairCode(code: string) {
  return code.length === 8 ? `${code.slice(0, 4)}-${code.slice(4)}` : code
}
