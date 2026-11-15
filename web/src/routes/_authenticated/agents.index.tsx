import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { AlertDialog } from '@astryxdesign/core/AlertDialog'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { useAnnounce, useMediaQuery } from '@astryxdesign/core/hooks'
import { Link } from '@astryxdesign/core/Link'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { StatusDot } from '@astryxdesign/core/StatusDot'
import { Table, pixel, proportional } from '@astryxdesign/core/Table'
import type { TableColumn } from '@astryxdesign/core/Table'
import { Text } from '@astryxdesign/core/Text'
import { useToast } from '@astryxdesign/core/Toast'
import { VisuallyHidden } from '@astryxdesign/core/VisuallyHidden'
import { VStack } from '@astryxdesign/core/VStack'

import { api } from '@/api/client'
import { AppFrame } from '@/components/app-frame'
import { ListEmpty } from '@/components/list-empty'
import { OfflineBanner } from '@/components/offline-banner'
import { PageHeading } from '@/components/page-heading'
import { describeChats } from '@/lib/agent-grant'
import { STATUS_DOTS, agentsQuery, presetsQuery } from '@/lib/agents'
import type { AgentConnection, AgentPreset } from '@/lib/agents'
import { formatDay, formatWhen } from '@/lib/format'
import { useSessionExpiry } from '@/lib/use-session-expiry'
import { connectionProblem, toWriteProblem } from '@/lib/write-problem'
import type { WriteProblem } from '@/lib/write-problem'

export const Route = createFileRoute('/_authenticated/agents/')({
  component: AgentsPage,
})

function AgentsPage() {
  const queryClient = useQueryClient()
  const announce = useAnnounce()
  const toast = useToast()
  const query = useQuery(agentsQuery)
  const overview = query.data
  const presets = useQuery({
    ...presetsQuery,
    enabled: overview?.enabled === true,
  })
  const isSignedOut = useSessionExpiry(query.error, '/agents')
  const [problem, setProblem] = useState<WriteProblem | null>(null)
  const [isPauseAllOpen, setIsPauseAllOpen] = useState(false)
  const [isPausingAll, setIsPausingAll] = useState(false)

  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: agentsQuery.queryKey })

  const toggle = async (connection: AgentConnection) => {
    const isPaused = connection.status === 'paused'
    const failed = isPaused
      ? `Couldn't resume ${connection.name}.`
      : `Couldn't pause ${connection.name}.`
    setProblem(null)
    try {
      const params = { params: { path: { id: connection.id } } }
      const result = isPaused
        ? await api.POST('/api/agents/{id}/resume', params)
        : await api.POST('/api/agents/{id}/pause', params)
      if (result.response.ok) {
        announce(
          isPaused
            ? `${connection.name} resumed.`
            : `${connection.name} paused.`,
        )
      } else {
        setProblem(toWriteProblem(result, failed))
      }
    } catch {
      setProblem(connectionProblem(failed))
    }
    await refresh()
  }

  const pauseAll = async () => {
    const failed = "Couldn't pause your connections."
    setProblem(null)
    setIsPausingAll(true)
    try {
      const result = await api.POST('/api/agents/pause-all')
      if (result.data) {
        const count = result.data.paused
        const body =
          count === 0
            ? 'Nothing was running.'
            : `${count} ${count === 1 ? 'connection' : 'connections'} paused.`
        announce(body)
        toast({ body, type: 'info' })
      } else {
        setProblem(toWriteProblem(result, failed))
      }
    } catch {
      setProblem(connectionProblem(failed))
    }
    await refresh()
    setIsPausingAll(false)
    setIsPauseAllOpen(false)
  }

  const connections = overview?.connections
  const isEnabled = overview?.enabled === true
  const hasActive =
    connections?.some((item) => item.status === 'active') ?? false
  const newConnection = (
    <Button label="New connection" variant="primary" href="/agents/new" />
  )

  return (
    <AppFrame>
      <VStack gap={6}>
        {query.isRefetchError && !isSignedOut ? <OfflineBanner /> : null}

        <PageHeading
          title="Agents"
          lede={
            isEnabled && connections?.length
              ? 'AI agents connected to your account, each with its own access key and permissions.'
              : 'Let an AI agent schedule and send WhatsApp messages for you, within limits you set.'
          }
          actions={
            isEnabled && connections?.length ? (
              <>
                {hasActive ? (
                  <Button
                    label="Pause all"
                    variant="secondary"
                    clickAction={() => setIsPauseAllOpen(true)}
                  />
                ) : null}
                {newConnection}
              </>
            ) : undefined
          }
        />

        {problem ? (
          <Banner
            status="error"
            title={problem.title}
            description={problem.description}
            isDismissable
            onDismiss={() => setProblem(null)}
          />
        ) : null}

        {/* Waiting approvals go here, above the connections; built in the next stage. */}

        {overview ? (
          !overview.enabled ? (
            <ListEmpty
              title="Agent access is off on this server"
              description="Whoever runs this Whatomato has turned off connecting AI agents, so there’s nothing to set up here."
              actions={<Button label="Go to messages" href="/messages" />}
            />
          ) : overview.connections.length > 0 ? (
            <ConnectionTable
              connections={overview.connections}
              presets={presets.data ?? []}
              onToggle={toggle}
            />
          ) : (
            <ListEmpty
              title="No agents connected"
              description="An AI agent, such as Claude Code or Cursor, can schedule and send messages for you once it’s connected. You give it an access key and decide what it may do: which chats, and whether it asks you before each message."
              actions={newConnection}
            />
          )
        ) : query.isPending ? (
          <VStack gap={2} aria-busy="true">
            {[0, 1, 2].map((index) => (
              <Skeleton key={index} index={index} width="100%" height={56} />
            ))}
          </VStack>
        ) : isSignedOut ? null : (
          <Banner
            status="error"
            title="Couldn't load your agents."
            description="Check your connection and try again."
            endContent={
              <Button
                label="Try again"
                size="sm"
                clickAction={async () => {
                  await query.refetch()
                }}
              />
            }
          />
        )}
      </VStack>

      <AlertDialog
        isOpen={isPauseAllOpen}
        onOpenChange={setIsPauseAllOpen}
        title="Pause every connection?"
        description="Every agent is refused from now on, and waiting approvals stay waiting. Nothing is deleted. Resume each connection from this page when you’re ready."
        actionLabel="Pause all"
        actionVariant="primary"
        isActionLoading={isPausingAll}
        onAction={pauseAll}
      />
    </AppFrame>
  )
}

interface ConnectionTableProps {
  connections: AgentConnection[]
  presets: AgentPreset[]
  onToggle: (connection: AgentConnection) => Promise<void>
}

function ConnectionTable({
  connections,
  presets,
  onToggle,
}: ConnectionTableProps) {
  const isNarrow = useMediaQuery('(max-width: 899px)')

  const name: TableColumn<AgentConnection> = {
    key: 'name',
    header: 'Connection',
    width: proportional(3),
    renderCell: (connection) => <NameCell connection={connection} />,
  }
  const status: TableColumn<AgentConnection> = {
    key: 'status',
    header: 'Status',
    width: pixel(168),
    renderCell: (connection) => <Status connection={connection} />,
  }
  const access: TableColumn<AgentConnection> = {
    key: 'access',
    header: 'May do',
    width: proportional(3),
    renderCell: (connection) => (
      <Access connection={connection} presets={presets} />
    ),
  }
  const used: TableColumn<AgentConnection> = {
    key: 'used',
    header: 'Last used',
    width: pixel(168),
    renderCell: (connection) => (
      <Text color="secondary" hasTabularNumbers>
        {connection.last_used_at
          ? formatWhen(connection.last_used_at)
          : 'Not yet'}
      </Text>
    ),
  }
  const action: TableColumn<AgentConnection> = {
    key: 'action',
    header: <VisuallyHidden>Pause or resume</VisuallyHidden>,
    width: pixel(112),
    renderCell: (connection) =>
      connection.status === 'active' || connection.status === 'paused' ? (
        <HStack justify="end">
          <Button
            label={connection.status === 'paused' ? 'Resume' : 'Pause'}
            aria-label={`${connection.status === 'paused' ? 'Resume' : 'Pause'} ${connection.name}`}
            size="sm"
            variant="secondary"
            clickAction={() => onToggle(connection)}
          />
        </HStack>
      ) : null,
  }

  const columns: TableColumn<AgentConnection>[] = isNarrow
    ? [
        {
          key: 'connection',
          header: <VisuallyHidden>Connection</VisuallyHidden>,
          width: proportional(1),
          renderCell: (connection) => (
            <VStack gap={2}>
              <NameCell connection={connection} />
              <Access connection={connection} presets={presets} />
              <HStack gap={3} wrap="wrap" vAlign="center">
                <Status connection={connection} />
                <Text type="supporting" hasTabularNumbers>
                  {connection.last_used_at
                    ? `Used ${formatDay(connection.last_used_at).toLowerCase()}`
                    : 'Not used yet'}
                </Text>
              </HStack>
            </VStack>
          ),
        },
        action,
      ]
    : [name, status, access, used, action]

  return (
    <Table
      data={connections}
      columns={columns}
      idKey="id"
      verticalAlign={isNarrow ? 'top' : 'middle'}
      hasHover
    />
  )
}

function NameCell({ connection }: { connection: AgentConnection }) {
  return (
    <VStack gap={0.5}>
      <Link href={`/agents/${connection.id}`} color="primary">
        <Text weight="medium">{connection.name}</Text>
      </Link>
      <Text type="supporting" hasTabularNumbers>
        Key ending {connection.key_hint}
      </Text>
    </VStack>
  )
}

function Status({ connection }: { connection: AgentConnection }) {
  const dot = STATUS_DOTS[connection.status]
  const detail =
    connection.status === 'expired' && connection.expires_at
      ? `Since ${formatDay(connection.expires_at)}`
      : connection.status === 'active' && connection.expires_at
        ? `Until ${formatDay(connection.expires_at)}`
        : null
  return (
    <HStack gap={2} vAlign="center">
      <StatusDot
        variant={dot.variant}
        label={dot.label}
        tooltip={connection.grant_problem ?? undefined}
      />
      <VStack>
        <Text>{dot.label}</Text>
        {detail ? <Text type="supporting">{detail}</Text> : null}
      </VStack>
    </HStack>
  )
}

function Access({
  connection,
  presets,
}: {
  connection: AgentConnection
  presets: AgentPreset[]
}) {
  if (!connection.grant) {
    return (
      <Text color="secondary">
        {connection.grant_problem ??
          'Its permissions can’t be read, so it can do nothing.'}
      </Text>
    )
  }
  const preset = presets.find((item) => item.id === connection.preset)
  return (
    <VStack gap={0.5}>
      <Text>{preset?.name ?? (connection.preset ? '' : 'Custom')}</Text>
      <Text type="supporting">{describeChats(connection.grant)}</Text>
    </VStack>
  )
}
