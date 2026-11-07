import { keepPreviousData, queryOptions } from '@tanstack/react-query'
import { z } from 'zod'

import type { components } from '@/api/schema'

import { api } from '@/api/client'
import { variableProblem } from '@/lib/variables'
import { ApiError } from '@/lib/whatsapp'

export type ScheduledMessage = components['schemas']['ScheduledMessage']
export type Template = components['schemas']['MessageTemplate']
export type Delivery = components['schemas']['DeliveryResponse']
export type RecipientType = components['schemas']['RecipientType']

export const BROWSER_TIME_ZONE =
  Intl.DateTimeFormat().resolvedOptions().timeZone

export const DELIVERIES_PER_PAGE = 25

export const BODY_MAX = 4096

// Spread into code points, which is how the server counts
export const bodySchema = z
  .string()
  .trim()
  .min(1, 'Write the message')
  .refine(
    (body) => [...body].length <= BODY_MAX,
    `Keep it to ${BODY_MAX} characters`,
  )
  .superRefine((body, context) => {
    const problem = variableProblem(body)
    if (problem) context.addIssue({ code: 'custom', message: problem })
  })

// The server sends about one message every 3 seconds, so a pending row rarely waits long
const PENDING_POLL_MS = 3_000
const MESSAGES_POLL_MS = 30_000

function retryOutages(failures: number, error: Error) {
  return !(error instanceof ApiError && error.status < 500) && failures < 2
}

export const messagesQuery = queryOptions({
  queryKey: ['messages'],
  queryFn: async () => {
    const { data, response } = await api.GET('/api/messages')
    if (!data) throw new ApiError(response.status)
    return data
  },
  // Each run moves a row's next run on, with nothing here to say it happened
  refetchInterval: MESSAGES_POLL_MS,
  retry: retryOutages,
})

export const templatesQuery = queryOptions({
  queryKey: ['templates'],
  queryFn: async () => {
    const { data, response } = await api.GET('/api/templates')
    if (!data) throw new ApiError(response.status)
    return data
  },
  retry: retryOutages,
})

export const DELIVERIES_KEY = ['deliveries']

export function deliveriesQuery(page: number) {
  return queryOptions({
    queryKey: [...DELIVERIES_KEY, page],
    queryFn: async () => {
      const { data, response } = await api.GET('/api/deliveries', {
        params: {
          query: {
            limit: DELIVERIES_PER_PAGE,
            offset: (page - 1) * DELIVERIES_PER_PAGE,
          },
        },
      })
      if (!data) throw new ApiError(response.status)
      return data
    },
    placeholderData: keepPreviousData,
    refetchInterval: (query) =>
      query.state.data?.items.some((item) => item.status === 'pending')
        ? PENDING_POLL_MS
        : false,
    retry: retryOutages,
  })
}

/** The latest few deliveries to one number or group, for a contact's page. */
export function recipientDeliveriesQuery(recipient: string, limit: number) {
  return queryOptions({
    queryKey: [...DELIVERIES_KEY, 'to', recipient, limit],
    queryFn: async () => {
      const { data, response } = await api.GET('/api/deliveries', {
        params: { query: { recipient, limit } },
      })
      if (!data) throw new ApiError(response.status)
      return data
    },
    refetchInterval: (query) =>
      query.state.data?.items.some((item) => item.status === 'pending')
        ? PENDING_POLL_MS
        : false,
    retry: retryOutages,
  })
}
