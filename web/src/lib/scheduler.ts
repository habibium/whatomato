import { useEffect, useState } from 'react'
import { keepPreviousData, queryOptions, useQuery } from '@tanstack/react-query'
import { z } from 'zod'

import type { components } from '@/api/schema'

import { api } from '@/api/client'
import { toFieldErrors } from '@/lib/form-errors'
import type { Recurrence } from '@/lib/schedule'
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

/** The server's answer for a schedule not yet saved: when it goes out, or why it never would. */
export interface NextRuns {
  runs: string[]
  /** For a sun schedule, the time zone's main city. */
  city: string | null
  problem: string | null
}

const PREVIEW_SETTLE_MS = 250

function nextRunsQuery(recurrence: string, timezone: string) {
  return queryOptions({
    queryKey: ['next-runs', recurrence, timezone],
    queryFn: async (): Promise<NextRuns> => {
      const { data, error, response } = await api.GET(
        '/api/messages/next-runs',
        { params: { query: { recurrence, timezone } } },
      )
      if (data) return { ...data, problem: null }
      const fields = toFieldErrors(error, ['recurrence', 'timezone'] as const)
      const problem = fields?.recurrence?.[0] ?? fields?.timezone?.[0]
      if (problem) return { runs: [], city: null, problem: problem.message }
      throw new ApiError(response.status)
    },
    enabled: recurrence !== '',
    placeholderData: keepPreviousData,
    // The runs move on as time does, but slowly next to how long a form stays open
    staleTime: 60_000,
    retry: retryOutages,
  })
}

/** Asks once the controls stop changing, so typing a cron expression doesn't ask on every key. */
export function useNextRuns(recurrence: Recurrence | null, timezone: string) {
  const text = recurrence ? JSON.stringify(recurrence) : ''
  const [settled, setSettled] = useState(text)
  useEffect(() => {
    const timer = setTimeout(() => setSettled(text), PREVIEW_SETTLE_MS)
    return () => clearTimeout(timer)
  }, [text])
  return useQuery(nextRunsQuery(settled, timezone))
}
