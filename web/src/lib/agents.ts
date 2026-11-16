import { queryOptions } from '@tanstack/react-query'

import type { components } from '@/api/schema'

import { api } from '@/api/client'
import { ApiError } from '@/lib/whatsapp'

export type AgentsOverview = components['schemas']['AgentsOverview']
export type AgentConnection = components['schemas']['AgentConnection']
export type AgentPreset = components['schemas']['AgentPreset']
export type AgentKeyIssued = components['schemas']['AgentKeyIssued']
export type ConnectionStatus = components['schemas']['ConnectionStatus']
export type PresetID = components['schemas']['Preset']

// Slow enough to be free, fast enough that a waiting approval shows up while the tab sits open
const OVERVIEW_POLL_MS = 60_000

export const agentsQuery = queryOptions({
  queryKey: ['agents'],
  queryFn: async (): Promise<AgentsOverview> => {
    const { data, response } = await api.GET('/api/agents')
    if (!data) throw new ApiError(response.status)
    return data
  },
  refetchInterval: OVERVIEW_POLL_MS,
  refetchOnWindowFocus: true,
  retry: (failures, error) =>
    !(error instanceof ApiError && error.status < 500) && failures < 2,
})

/** Whether this server lets connections read chats, and for how long messages are kept. */
export const readingQuery = queryOptions({
  queryKey: ['agents', 'reading'],
  queryFn: async () => {
    const { data, response } = await api.GET('/api/agents/reading')
    if (!data) throw new ApiError(response.status)
    return data
  },
  staleTime: Infinity,
  retry: (failures, error) =>
    !(error instanceof ApiError && error.status < 500) && failures < 2,
})

export const presetsQuery = queryOptions({
  queryKey: ['agents', 'presets'],
  queryFn: async (): Promise<AgentPreset[]> => {
    const { data, response } = await api.GET('/api/agents/presets')
    if (!data) throw new ApiError(response.status)
    return data
  },
  staleTime: Infinity,
  retry: (failures, error) =>
    !(error instanceof ApiError && error.status < 500) && failures < 2,
})

export const STATUS_DOTS: Record<
  ConnectionStatus,
  { label: string; variant: 'success' | 'neutral' | 'warning' | 'error' }
> = {
  active: { label: 'Active', variant: 'success' },
  paused: { label: 'Paused', variant: 'neutral' },
  expired: { label: 'Expired', variant: 'warning' },
  unusable: { label: 'Needs attention', variant: 'error' },
}

/** The address an agent connects to; in development Vite proxies it to the server. */
export function endpointURL(origin = window.location.origin) {
  return `${origin}/mcp`
}
