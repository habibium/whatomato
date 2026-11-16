import { useRef, useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useForm } from '@tanstack/react-form'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  createFileRoute,
  useBlocker,
  useNavigate,
} from '@tanstack/react-router'
import { AlertDialog } from '@astryxdesign/core/AlertDialog'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { CheckboxInput } from '@astryxdesign/core/CheckboxInput'
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog'
import { Divider } from '@astryxdesign/core/Divider'
import { HStack } from '@astryxdesign/core/HStack'
import { useAnnounce } from '@astryxdesign/core/hooks'
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout'
import { Link } from '@astryxdesign/core/Link'
import { List, ListItem } from '@astryxdesign/core/List'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { StatusDot } from '@astryxdesign/core/StatusDot'
import { Heading, Text } from '@astryxdesign/core/Text'
import { TextInput } from '@astryxdesign/core/TextInput'
import { useToast } from '@astryxdesign/core/Toast'
import { VStack } from '@astryxdesign/core/VStack'
import {
  borderVars,
  colorVars,
  spacingVars,
} from '@astryxdesign/core/theme/tokens.stylex'
import { z } from 'zod'

import type { components } from '@/api/schema'

import { api } from '@/api/client'
import { ActivityLog } from '@/components/agent-activity-log'
import { KeyIssued } from '@/components/agent-key-issued'
import { PermissionEditor } from '@/components/agent-permissions'
import { AgentSetupSteps } from '@/components/agent-setup-steps'
import { AppFrame } from '@/components/app-frame'
import { BackLink } from '@/components/back-link'
import { ConfirmPasswordDialog } from '@/components/confirm-password-dialog'
import type { ConfirmOutcome } from '@/components/confirm-password-dialog'
import { OfflineBanner } from '@/components/offline-banner'
import { PageHeading } from '@/components/page-heading'
import { SettingsSection } from '@/components/settings-section'
import { activityQuery, connectionQuery } from '@/lib/agent-activity'
import type { AgentConnectionDetail } from '@/lib/agent-activity'
import {
  cleanGrant,
  describeChats,
  matchingPreset,
  sameGrant,
} from '@/lib/agent-grant'
import type { Grant } from '@/lib/agent-grant'
import {
  STATUS_DOTS,
  agentsQuery,
  endpointURL,
  presetsQuery,
} from '@/lib/agents'
import type { AgentKeyIssued, AgentPreset } from '@/lib/agents'
import { formatWhen } from '@/lib/format'
import { toFieldErrors, toFieldStatus } from '@/lib/form-errors'
import { retryAfterSeconds } from '@/lib/retry-after'
import { useSessionExpiry } from '@/lib/use-session-expiry'
import { ApiError } from '@/lib/whatsapp'
import { connectionProblem, toWriteProblem } from '@/lib/write-problem'
import type { WriteProblem } from '@/lib/write-problem'

export const Route = createFileRoute('/_authenticated/agents/$connectionId')({
  component: ConnectionPage,
})

const DEFAULT_PRESET = 'ask_before_sending'
const NAME_MAX = 60
const RECENT = 5

const styles = stylex.create({
  // Stays in view while the permissions above it scroll, so unsaved changes can't be missed
  saveBar: {
    backgroundColor: colorVars['--color-background-surface'],
    borderBlockStartColor: colorVars['--color-border'],
    borderBlockStartStyle: 'solid',
    borderBlockStartWidth: borderVars['--border-width'],
    bottom: 0,
    paddingBlock: spacingVars['--spacing-4'],
    position: 'sticky',
    zIndex: 1,
  },
  value: { textAlign: 'end' },
  grow: { flexGrow: 1, minWidth: 0 },
})

type ErrorBody = components['schemas']['ErrorBody']

function ConnectionPage() {
  const { connectionId } = Route.useParams()
  const query = useQuery(connectionQuery(connectionId))
  const presets = useQuery(presetsQuery)
  const isSignedOut = useSessionExpiry(query.error, `/agents/${connectionId}`)
  const [issued, setIssued] = useState<AgentKeyIssued | null>(null)

  if (issued) {
    return (
      <AppFrame>
        <KeyIssued
          issued={issued}
          title={`A new key for ${issued.connection.name}`}
          lede="The old key has stopped working. Give the agent this one in its place."
          onDone={() => setIssued(null)}
        />
      </AppFrame>
    )
  }

  const isMissing =
    query.error instanceof ApiError && query.error.status === 404

  return (
    <AppFrame>
      <VStack gap={6}>
        {query.isRefetchError && !isSignedOut ? <OfflineBanner /> : null}
        <BackLink href="/agents" label="Agents" />
        {query.data && presets.data ? (
          <ConnectionDetails
            key={query.data.id}
            connection={query.data}
            presets={presets.data}
            onKeyIssued={setIssued}
          />
        ) : isMissing ? (
          <VStack gap={2}>
            <Heading level={1}>This connection isn’t here</Heading>
            <Text color="secondary">
              It was revoked, or the link is wrong. Its activity stays in the
              log.
            </Text>
            <HStack gap={2}>
              <Button label="All agents" href="/agents" />
            </HStack>
          </VStack>
        ) : query.isPending || presets.isPending ? (
          <VStack gap={4} aria-busy="true">
            <Skeleton width="40%" height={32} />
            {[0, 1, 2].map((index) => (
              <Skeleton key={index} index={index} width="100%" height={160} />
            ))}
          </VStack>
        ) : isSignedOut ? null : (
          <Banner
            status="error"
            title="Couldn't load this connection."
            description="Check your connection and try again."
            endContent={
              <Button
                label="Try again"
                size="sm"
                clickAction={async () => {
                  await Promise.all([query.refetch(), presets.refetch()])
                }}
              />
            }
          />
        )}
      </VStack>
    </AppFrame>
  )
}

interface ConnectionDetailsProps {
  connection: AgentConnectionDetail
  presets: AgentPreset[]
  onKeyIssued: (issued: AgentKeyIssued) => void
}

function ConnectionDetails({
  connection,
  presets,
  onKeyIssued,
}: ConnectionDetailsProps) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const announce = useAnnounce()
  const toast = useToast()
  const [problem, setProblem] = useState<WriteProblem | null>(null)
  const [draft, setDraft] = useState<Grant | null>(null)
  const [grantProblems, setGrantProblems] = useState<string[]>([])
  const [dialog, setDialog] = useState<'save' | 'rotate' | 'revoke' | null>(
    null,
  )
  const [isToggling, setIsToggling] = useState(false)
  const isLeaving = useRef(false)

  const stored = connection.grant
  const editing = draft ?? stored
  const isDirty = draft !== null && (!stored || !sameGrant(draft, stored))
  const blocker = useBlocker({
    shouldBlockFn: () => isDirty && !isLeaving.current,
    enableBeforeUnload: () => isDirty && !isLeaving.current,
    withResolver: true,
  })

  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: agentsQuery.queryKey })

  const toggle = async () => {
    const isPaused = connection.status === 'paused'
    const failed = isPaused
      ? `Couldn't resume ${connection.name}.`
      : `Couldn't pause ${connection.name}.`
    setProblem(null)
    setIsToggling(true)
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
    setIsToggling(false)
  }

  const saveGrant = async (password: string): Promise<ConfirmOutcome> => {
    if (!draft) return 'confirmed'
    const failed = "Couldn't save the permissions."
    let result
    try {
      result = await api.PATCH('/api/agents/{id}', {
        params: { path: { id: connection.id } },
        body: { grant: cleanGrant(draft), password },
      })
    } catch {
      setProblem(connectionProblem(failed))
      return 'confirmed'
    }
    if (result.data) {
      await refresh()
      setDraft(null)
      setGrantProblems([])
      announce('Permissions saved.')
      toast({ body: 'Permissions saved.', type: 'info' })
      return 'confirmed'
    }
    const { response, error } = result
    if (response.status === 429) {
      return { kind: 'rate-limited', retryAfter: retryAfterSeconds(response) }
    }
    if (toFieldErrors(error, ['password'] as const)) return 'incorrect-password'
    const grant = (error as ErrorBody | undefined)?.details?.grant ?? []
    if (grant.length) {
      setGrantProblems(grant)
      return 'confirmed'
    }
    setProblem(toWriteProblem(result, failed))
    return 'confirmed'
  }

  const rotate = async (password: string): Promise<ConfirmOutcome> => {
    const failed = "Couldn't replace the key."
    let result
    try {
      result = await api.POST('/api/agents/{id}/rotate-key', {
        params: { path: { id: connection.id } },
        body: { password },
      })
    } catch {
      setProblem(connectionProblem(failed))
      return 'confirmed'
    }
    if (result.data) {
      await refresh()
      onKeyIssued(result.data)
      return 'confirmed'
    }
    const { response, error } = result
    if (response.status === 429) {
      return { kind: 'rate-limited', retryAfter: retryAfterSeconds(response) }
    }
    if (toFieldErrors(error, ['password'] as const)) return 'incorrect-password'
    setProblem(toWriteProblem(result, failed))
    return 'confirmed'
  }

  const revoke = async (deleteSchedules: boolean) => {
    const failed = `Couldn't revoke ${connection.name}.`
    setProblem(null)
    try {
      const result = await api.DELETE('/api/agents/{id}', {
        params: {
          path: { id: connection.id },
          query: { delete_schedules: deleteSchedules },
        },
      })
      if (result.data) {
        isLeaving.current = true
        // Leave first: refreshing while this page is mounted refetches the revoked id and logs a 404
        await navigate({
          to: '/agents',
          state: {
            revoked: {
              name: connection.name,
              deleted: result.data.schedules_deleted,
              kept: result.data.schedules_kept,
            },
          },
        })
        queryClient.removeQueries({
          queryKey: ['agents', 'connection', connection.id],
        })
        queryClient.removeQueries({
          queryKey: ['agents', 'activity', connection.id],
        })
        await refresh()
        return
      }
      setProblem(toWriteProblem(result, failed))
    } catch {
      setProblem(connectionProblem(failed))
    }
    setDialog(null)
  }

  const canToggle =
    connection.status === 'active' || connection.status === 'paused'
  const dot = STATUS_DOTS[connection.status]

  return (
    <VStack gap={8}>
      <PageHeading
        title={connection.name}
        lede={headline(connection, presets)}
        actions={
          canToggle ? (
            <Button
              label={connection.status === 'paused' ? 'Resume' : 'Pause'}
              variant={connection.status === 'paused' ? 'primary' : 'secondary'}
              tooltip={
                connection.status === 'paused'
                  ? undefined
                  : 'Refuse the agent, and hold the messages it scheduled, until you resume it'
              }
              isLoading={isToggling}
              clickAction={toggle}
            />
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

      <SettingsSection
        id="connection-overview"
        title="Overview"
        description={
          connection.signed_in
            ? 'The app that signed in, when it was last used, and how close it is to its limits.'
            : 'Its key, when it was last used, and how close it is to its limits.'
        }
      >
        <List hasDividers density="compact">
          <Fact label="Status">
            <HStack gap={2} vAlign="center" justify="end">
              <StatusDot
                aria-hidden="true"
                variant={dot.variant}
                label={dot.label}
              />
              <Text>{dot.label}</Text>
            </HStack>
          </Fact>
          {connection.signed_in ? (
            <Fact label="Signed in by">
              {`${connection.signed_in.client_name} (${connection.signed_in.client_host})`}
            </Fact>
          ) : (
            <Fact label="Access key">Ending {connection.key_hint}</Fact>
          )}
          <Fact label="Created">{formatWhen(connection.created_at)}</Fact>
          <Fact label="Last used">
            {connection.last_used_at
              ? formatWhen(connection.last_used_at)
              : 'Not yet'}
          </Fact>
          <Fact label={connection.status === 'expired' ? 'Expired' : 'Expires'}>
            {connection.expires_at
              ? formatWhen(connection.expires_at)
              : 'Never'}
          </Fact>
          <Fact label="Messages in the last 24 hours">
            {connection.usage.sends_last_24_hours} of{' '}
            {connection.usage.daily_ceiling}
          </Fact>
          <Fact label="Requests waiting for you">
            {connection.usage.approvals_waiting} of{' '}
            {connection.usage.approvals_waiting_max}
          </Fact>
        </List>
        <Text type="supporting">
          Past either limit, the agent is refused until the count drops.
          {connection.usage.approvals_waiting > 0 ? (
            <>
              {' '}
              <Link href="/agents" hasUnderline>
                Answer waiting requests
              </Link>
            </>
          ) : null}
        </Text>
      </SettingsSection>

      <Divider />

      <RenameSection connection={connection} onProblem={setProblem} />

      <Divider />

      {editing ? (
        <VStack gap={0}>
          <PermissionEditor
            value={editing}
            onChange={(grant) => {
              setGrantProblems([])
              setDraft(grant)
            }}
            presets={presets}
            problems={grantProblems}
          />
          {isDirty ? (
            <HStack
              gap={3}
              justify="between"
              vAlign="center"
              wrap="wrap"
              xstyle={styles.saveBar}
              role="region"
              aria-label="Unsaved permissions"
            >
              <Text weight="medium">
                {stored
                  ? 'You changed its permissions. Nothing is saved yet.'
                  : 'These permissions aren’t saved yet. Until they are, it can do nothing.'}
              </Text>
              <HStack gap={2}>
                <Button
                  label="Discard changes"
                  variant="secondary"
                  clickAction={() => {
                    setDraft(null)
                    setGrantProblems([])
                  }}
                />
                <Button
                  label="Save permissions"
                  variant="primary"
                  clickAction={() => setDialog('save')}
                />
              </HStack>
            </HStack>
          ) : null}
        </VStack>
      ) : (
        <SettingsSection
          id="connection-permissions"
          title="What it may do"
          description="Its permissions over your account."
        >
          <Banner
            status="error"
            title="Its permissions can’t be read, so it can do nothing"
            description={`${connection.grant_problem ?? 'They were stored in a form this version of Whatomato doesn’t understand.'} Give it permissions again to use it. You start from “Ask before sending” and can change anything before you save.`}
          />
          <HStack>
            <Button
              label="Give it permissions again"
              variant="primary"
              clickAction={() => {
                const preset =
                  presets.find((item) => item.id === DEFAULT_PRESET) ??
                  presets.at(0)
                if (preset) setDraft(cleanGrant(preset.grant))
              }}
            />
          </HStack>
        </SettingsSection>
      )}

      <Divider />

      {connection.signed_in ? null : (
        <>
          <VStack as="section" gap={4} aria-labelledby="setup-steps">
            <VStack gap={1}>
              <Heading level={2} id="setup-steps">
                Set up another machine
              </Heading>
              <Text color="secondary">
                The same steps as when you created it. The key isn’t shown
                again, so put yours where YOUR_ACCESS_KEY is, or replace the key
                below.
              </Text>
            </VStack>
            <AgentSetupSteps url={endpointURL()} />
          </VStack>

          <Divider />
        </>
      )}

      <RecentActivity connection={connection} />

      <Divider />

      {connection.signed_in ? null : (
        <>
          <SettingsSection
            id="replace-key"
            title="Replace the key"
            description="For a key that leaked or was lost. The old key stops working at once, and every agent using it is refused until you give it the new one."
          >
            <HStack>
              <Button
                label="Replace the key"
                clickAction={() => setDialog('rotate')}
              />
            </HStack>
          </SettingsSection>

          <Divider />
        </>
      )}

      <SettingsSection
        id="revoke"
        title="Revoke"
        description={
          connection.signed_in
            ? 'Deletes this connection and signs the app out for good. Its activity stays in the log.'
            : 'Deletes this connection and its key for good. Its activity stays in the log.'
        }
      >
        <HStack>
          <Button
            label={`Revoke ${connection.name}`}
            variant="destructive"
            clickAction={() => setDialog('revoke')}
          />
        </HStack>
      </SettingsSection>

      <ConfirmPasswordDialog
        isOpen={dialog === 'save'}
        onOpenChange={(isOpen) => {
          if (!isOpen) setDialog(null)
        }}
        title="Confirm it’s you"
        description="Changing what an agent may do needs your password."
        confirmLabel="Save permissions"
        onConfirm={saveGrant}
      />
      <ConfirmPasswordDialog
        isOpen={dialog === 'rotate'}
        onOpenChange={(isOpen) => {
          if (!isOpen) setDialog(null)
        }}
        title="Replace the access key?"
        description={`The key ending ${connection.key_hint} stops working as soon as you confirm. You’ll see the new key once, on the next screen.`}
        confirmLabel="Replace the key"
        confirmVariant="destructive"
        onConfirm={rotate}
      />
      <RevokeDialog
        isOpen={dialog === 'revoke'}
        onOpenChange={(isOpen) => {
          if (!isOpen) setDialog(null)
        }}
        connection={connection}
        onRevoke={revoke}
      />
      <AlertDialog
        isOpen={blocker.status === 'blocked'}
        onOpenChange={(isOpen) => {
          if (!isOpen) blocker.reset?.()
        }}
        title="Leave without saving?"
        description="Your changes to its permissions will be lost. The agent keeps the permissions it had."
        actionLabel="Leave"
        cancelLabel="Stay"
        onAction={() => blocker.proceed?.()}
      />
    </VStack>
  )
}

function headline(
  connection: AgentConnectionDetail,
  presets: readonly AgentPreset[],
) {
  switch (connection.status) {
    case 'paused':
      return 'Paused. The agent is refused, and the messages it scheduled don’t go out, until you resume it.'
    case 'expired':
      return 'Its key has expired, so the agent is refused and the messages it scheduled don’t go out. Replace the key to use it again.'
    case 'unusable':
      return 'Its permissions can’t be read, so the agent can do nothing until you set them again.'
    default: {
      if (!connection.grant) return null
      const preset = matchingPreset(connection.grant, presets)
      return `Active. ${preset?.name ?? 'Custom permissions'}: ${describeChats(connection.grant).toLowerCase()}.`
    }
  }
}

function Fact({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <ListItem
      label={label}
      endContent={
        typeof children === 'string' || Array.isArray(children) ? (
          <Text hasTabularNumbers xstyle={styles.value}>
            {children}
          </Text>
        ) : (
          children
        )
      }
    />
  )
}

const nameSchema = z.object({
  name: z
    .string()
    .trim()
    .min(1, 'Give it a name')
    .max(NAME_MAX, `Keep it to ${NAME_MAX} characters`),
})

function RenameSection({
  connection,
  onProblem,
}: {
  connection: AgentConnectionDetail
  onProblem: (problem: WriteProblem | null) => void
}) {
  const queryClient = useQueryClient()
  const announce = useAnnounce()
  const form = useForm({
    defaultValues: { name: connection.name },
    validators: { onSubmit: nameSchema },
    canSubmitWhenInvalid: true,
    onSubmit: async ({ value, formApi }) => {
      const name = value.name.trim()
      if (name === connection.name) return
      const failed = "Couldn't rename the connection."
      onProblem(null)
      try {
        const result = await api.PATCH('/api/agents/{id}', {
          params: { path: { id: connection.id } },
          body: { name },
        })
        if (result.data) {
          await queryClient.invalidateQueries({
            queryKey: agentsQuery.queryKey,
          })
          formApi.reset({ name: result.data.name })
          announce(`Renamed to ${result.data.name}.`)
          return
        }
        const fields = toFieldErrors(result.error, ['name'] as const)
        if (fields) {
          formApi.setErrorMap({ onSubmit: { fields } })
          return
        }
        onProblem(toWriteProblem(result, failed))
      } catch {
        onProblem(connectionProblem(failed))
      }
    },
  })

  return (
    <SettingsSection
      id="connection-name"
      title="Name"
      description="What the agent is, so you can tell your connections apart. Renaming doesn’t need your password."
    >
      <form
        noValidate
        onSubmit={(event) => {
          event.preventDefault()
          void form.handleSubmit()
        }}
      >
        <HStack gap={2} vAlign="start">
          <VStack xstyle={styles.grow}>
            <form.Field name="name">
              {(field) => (
                <TextInput
                  label="Name"
                  isLabelHidden
                  width="100%"
                  htmlName={field.name}
                  autoComplete="off"
                  value={field.state.value}
                  onChange={(value) => field.handleChange(value)}
                  onBlur={field.handleBlur}
                  status={toFieldStatus(field.state.meta)}
                />
              )}
            </form.Field>
          </VStack>
          <form.Subscribe
            selector={(state) => [
              state.isSubmitting,
              state.values.name.trim() !== connection.name,
            ]}
          >
            {([isSubmitting, isChanged]) => (
              <Button
                type="submit"
                label="Rename"
                isLoading={isSubmitting}
                isDisabled={!isChanged}
              />
            )}
          </form.Subscribe>
        </HStack>
      </form>
    </SettingsSection>
  )
}

function RecentActivity({ connection }: { connection: AgentConnectionDetail }) {
  const query = useQuery(activityQuery(connection.id, 1, RECENT))
  const activity = query.data
  return (
    <VStack as="section" gap={4} aria-labelledby="recent-activity">
      <HStack gap={4} justify="between" vAlign="center" wrap="wrap">
        <VStack gap={1}>
          <Heading level={2} id="recent-activity">
            Recent activity
          </Heading>
          <Text color="secondary">
            What it did or tried. Message text is never kept.
          </Text>
        </VStack>
        {activity && activity.total > 0 ? (
          <Button
            label={
              activity.total > RECENT
                ? `See all ${activity.total}`
                : 'Open the activity log'
            }
            href={`/agents/activity?connection=${connection.id}`}
          />
        ) : null}
      </HStack>
      {activity ? (
        activity.items.length > 0 ? (
          <ActivityLog items={activity.items} showsConnection={false} />
        ) : (
          <Text color="secondary">
            Nothing yet. Once the agent connects, what it does shows here.
          </Text>
        )
      ) : query.isPending ? (
        <Skeleton width="100%" height={120} />
      ) : (
        <Text color="secondary">The activity couldn’t be loaded.</Text>
      )}
    </VStack>
  )
}

function RevokeDialog({
  isOpen,
  onOpenChange,
  connection,
  onRevoke,
}: {
  isOpen: boolean
  onOpenChange: (isOpen: boolean) => void
  connection: AgentConnectionDetail
  onRevoke: (deleteSchedules: boolean) => Promise<void>
}) {
  const [deleteSchedules, setDeleteSchedules] = useState(true)
  const [isRevoking, setIsRevoking] = useState(false)
  const count = connection.schedules
  const close = () => onOpenChange(false)

  return (
    <Dialog
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      purpose="form"
      width={480}
    >
      <Layout
        header={
          <DialogHeader
            title={`Revoke ${connection.name}?`}
            subtitle="Its access key stops working at once and the connection is deleted. This can’t be undone; to use the agent again, create a new connection."
            onOpenChange={close}
          />
        }
        content={
          count > 0 ? (
            <LayoutContent>
              <VStack gap={3}>
                <Text>
                  It created or changed {count} scheduled{' '}
                  {count === 1 ? 'message that is' : 'messages that are'} still
                  on Messages.
                </Text>
                <CheckboxInput
                  label={`Delete ${count === 1 ? 'it' : `all ${count}`} too`}
                  description={
                    deleteSchedules
                      ? 'They won’t go out again. What they already sent stays in History.'
                      : 'They keep going out, and show as your own.'
                  }
                  value={deleteSchedules}
                  onChange={setDeleteSchedules}
                />
              </VStack>
            </LayoutContent>
          ) : undefined
        }
        footer={
          <LayoutFooter>
            <HStack gap={2} justify="end">
              <Button label="Cancel" variant="secondary" clickAction={close} />
              <Button
                label="Revoke"
                variant="destructive"
                isLoading={isRevoking}
                clickAction={async () => {
                  setIsRevoking(true)
                  await onRevoke(count > 0 && deleteSchedules)
                  setIsRevoking(false)
                }}
              />
            </HStack>
          </LayoutFooter>
        }
      />
    </Dialog>
  )
}
