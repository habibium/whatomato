import { keepPreviousData, queryOptions } from '@tanstack/react-query'

import type { components } from '@/api/schema'

import { api } from '@/api/client'
import { ApiError } from '@/lib/whatsapp'

export type AgentApproval = components['schemas']['AgentApproval']
export type AgentActivity = components['schemas']['AgentActivity']
export type AgentConnectionDetail =
  components['schemas']['AgentConnectionDetail']
export type ApprovalKind = components['schemas']['ApprovalKind']
export type ApprovalStatus = components['schemas']['ApprovalStatus']
export type Outcome = components['schemas']['Outcome']

type Dot = {
  label: string
  variant: 'success' | 'warning' | 'error' | 'accent' | 'neutral'
}

export const ACTIVITY_PER_PAGE = 50
export const APPROVALS_PER_PAGE = 25
/** The most a page of approvals holds; waiting ones come first, so this covers all but a flood. */
export const WAITING_MAX = 100

const retry = (failures: number, error: Error) =>
  !(error instanceof ApiError && error.status < 500) && failures < 2

export const connectionQuery = (id: string) =>
  queryOptions({
    queryKey: ['agents', 'connection', id],
    queryFn: async () => {
      const { data, response } = await api.GET('/api/agents/{id}', {
        params: { path: { id } },
      })
      if (!data) throw new ApiError(response.status)
      return data
    },
    retry,
  })

export const approvalsQuery = (offset: number, limit: number) =>
  queryOptions({
    queryKey: ['agents', 'approvals', offset, limit],
    queryFn: async () => {
      const { data, response } = await api.GET('/api/agents/approvals', {
        params: { query: { offset, limit } },
      })
      if (!data) throw new ApiError(response.status)
      return data
    },
    placeholderData: keepPreviousData,
    refetchOnWindowFocus: true,
    retry,
  })

export const approvalQuery = (id: string) =>
  queryOptions({
    queryKey: ['agents', 'approval', id],
    queryFn: async () => {
      const { data, response } = await api.GET('/api/agents/approvals/{id}', {
        params: { path: { id } },
      })
      if (!data) throw new ApiError(response.status)
      return data
    },
    retry,
  })

export const activityQuery = (
  connectionID: string | undefined,
  page: number,
  limit = ACTIVITY_PER_PAGE,
) =>
  queryOptions({
    queryKey: ['agents', 'activity', connectionID ?? 'all', page, limit],
    queryFn: async () => {
      const { data, response } = await api.GET('/api/agents/activity', {
        params: {
          query: {
            connection_id: connectionID,
            limit,
            offset: (page - 1) * limit,
          },
        },
      })
      if (!data) throw new ApiError(response.status)
      return data
    },
    placeholderData: keepPreviousData,
    retry,
  })

/** What the agent asked for, as the owner reads it at a glance. */
export const KIND_LABELS: Record<ApprovalKind, string> = {
  send: 'Send a message now',
  schedule_create: 'Schedule a new message',
  schedule_update: 'Change a scheduled message',
}

export const APPROVAL_DOTS: Record<ApprovalStatus, Dot> = {
  waiting: { label: 'Waiting', variant: 'accent' },
  approved: { label: 'Approved', variant: 'success' },
  rejected: { label: 'Rejected', variant: 'neutral' },
  expired: { label: 'Expired', variant: 'warning' },
}

export const OUTCOME_DOTS: Record<Outcome, Dot> = {
  allowed: { label: 'Allowed', variant: 'success' },
  asked: { label: 'Asked you', variant: 'accent' },
  denied: { label: 'Refused', variant: 'warning' },
  failed: { label: 'Failed', variant: 'error' },
}

const TOOL_PHRASES: Record<string, string> = {
  whoami: 'Checked what it may do',
  find_chats: 'Looked up chats',
  send_message: 'Sent a message',
  list_deliveries: 'Checked sent messages',
  get_approval: 'Checked an approval',
  list_approvals: 'Checked its approvals',
  schedule_message: 'Scheduled a message',
  list_scheduled_messages: 'Looked at scheduled messages',
  update_scheduled_message: 'Changed a scheduled message',
  delete_scheduled_message: 'Deleted a scheduled message',
  list_templates: 'Read templates',
}

/** Activity names what was tried, so a refused send reads "Tried to send a message". */
const TRIED: Record<string, string> = {
  send_message: 'Tried to send a message',
  schedule_message: 'Tried to schedule a message',
  update_scheduled_message: 'Tried to change a scheduled message',
  delete_scheduled_message: 'Tried to delete a scheduled message',
  find_chats: 'Tried to look up chats',
  list_templates: 'Tried to read templates',
  list_scheduled_messages: 'Tried to look at scheduled messages',
}

const ASKED: Record<string, string> = {
  send_message: 'Asked to send a message',
  schedule_message: 'Asked to schedule a message',
  update_scheduled_message: 'Asked to change a scheduled message',
}

export function describeTool(tool: string, outcome: Outcome) {
  if (outcome === 'asked' && ASKED[tool]) return ASKED[tool]
  if ((outcome === 'denied' || outcome === 'failed') && TRIED[tool]) {
    return TRIED[tool]
  }
  // A tool added on the server after this page was built; its raw name means nothing to the owner
  return TOOL_PHRASES[tool] ?? 'Used a feature this page doesn’t know yet'
}

const relative = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' })
const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ['day', 86_400_000],
  ['hour', 3_600_000],
  ['minute', 60_000],
]

/** "in 23 hours", "in 5 minutes"; "now" under a minute. */
export function formatRelative(iso: string, now = Date.now()) {
  const delta = Date.parse(iso) - now
  for (const [unit, size] of UNITS) {
    if (Math.abs(delta) >= size) {
      return relative.format(Math.round(delta / size), unit)
    }
  }
  return relative.format(0, 'minute')
}
