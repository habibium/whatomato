import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useLocation } from '@tanstack/react-router'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { useAnnounce, useMediaQuery } from '@astryxdesign/core/hooks'
import { Link } from '@astryxdesign/core/Link'
import { MoreMenu } from '@astryxdesign/core/MoreMenu'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { Switch } from '@astryxdesign/core/Switch'
import { Tab, TabList } from '@astryxdesign/core/TabList'
import { Table, pixel, proportional } from '@astryxdesign/core/Table'
import { Text } from '@astryxdesign/core/Text'
import { useToast } from '@astryxdesign/core/Toast'
import { VisuallyHidden } from '@astryxdesign/core/VisuallyHidden'
import { VStack } from '@astryxdesign/core/VStack'

import type { TableColumn } from '@astryxdesign/core/Table'

import { api } from '@/api/client'
import { AppFrame } from '@/components/app-frame'
import { ConfirmDeleteDialog } from '@/components/confirm-dialog'
import { FirstRun } from '@/components/first-run'
import { MessageFormDialog } from '@/components/message-form-dialog'
import { OfflineBanner } from '@/components/offline-banner'
import { PageHeading } from '@/components/page-heading'
import { RecipientCell } from '@/components/recipient-cell'
import { SendNowDialog } from '@/components/send-now-dialog'
import { WhatsAppBanner } from '@/components/whatsapp-banner'
import {
  formatWhen,
  formatWhenInSentence,
  summariseCron,
  zoneLabel,
} from '@/lib/format'
import { useRecipientLabel } from '@/lib/contacts'
import { messagesQuery } from '@/lib/scheduler'
import type { ScheduledMessage } from '@/lib/scheduler'
import { useSessionExpiry } from '@/lib/use-session-expiry'
import { connectionProblem, toWriteProblem } from '@/lib/write-problem'
import type { WriteProblem } from '@/lib/write-problem'

export const Route = createFileRoute('/_authenticated/messages')({
  component: MessagesPage,
})

type OpenDialog =
  | { kind: 'write'; message: ScheduledMessage | null }
  | { kind: 'delete'; message: ScheduledMessage }
  | { kind: 'send' }

type View = 'scheduled' | 'off' | 'past'

const VIEWS: { value: View; label: string }[] = [
  { value: 'scheduled', label: 'Scheduled' },
  { value: 'off', label: 'Off' },
  { value: 'past', label: 'Past' },
]

/** A one-time message that has run can't be switched back on, so it gets its own view. */
function viewOf(message: ScheduledMessage): View {
  if (message.next_run_at) return 'scheduled'
  if (message.schedule_kind === 'once' && message.last_run_at) return 'past'
  return 'off'
}

const time = (iso: string | null | undefined) => (iso ? Date.parse(iso) : 0)

const ORDER: Record<
  View,
  (a: ScheduledMessage, b: ScheduledMessage) => number
> = {
  scheduled: (a, b) => time(a.next_run_at) - time(b.next_run_at),
  off: (a, b) => time(b.updated_at) - time(a.updated_at),
  past: (a, b) => time(b.last_run_at) - time(a.last_run_at),
}

function MessagesPage() {
  const notice = useLocation({ select: (location) => location.state.notice })
  const queryClient = useQueryClient()
  const announce = useAnnounce()
  const toast = useToast()
  const query = useQuery(messagesQuery)
  const isSignedOut = useSessionExpiry(query.error, '/messages')
  const [dialog, setDialog] = useState<OpenDialog | null>(null)
  const [problem, setProblem] = useState<WriteProblem | null>(null)
  const [view, setView] = useState<View>('scheduled')

  const closeDialog = (isOpen: boolean) => {
    if (!isOpen) setDialog(null)
  }
  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: messagesQuery.queryKey })

  const toggle = async (message: ScheduledMessage, enabled: boolean) => {
    const failed = enabled
      ? "Couldn't turn this message on."
      : "Couldn't turn this message off."
    setProblem(null)
    try {
      const result = await api.PATCH('/api/messages/{id}', {
        params: { path: { id: message.id } },
        body: { enabled },
      })
      if (!result.data) setProblem(toWriteProblem(result, failed))
    } catch {
      setProblem(connectionProblem(failed))
    }
    await refresh()
  }

  const remove = async (message: ScheduledMessage) => {
    const failed = "Couldn't delete this message."
    setProblem(null)
    try {
      const result = await api.DELETE('/api/messages/{id}', {
        params: { path: { id: message.id } },
      })
      // 404: someone already deleted it, which is what was asked for
      if (result.response.ok || result.response.status === 404) {
        announce('Message deleted.')
        toast({ body: 'Message deleted.', type: 'info' })
      } else {
        setProblem(toWriteProblem(result, failed))
      }
    } catch {
      setProblem(connectionProblem(failed))
    }
    await refresh()
    setDialog(null)
  }

  const messages = query.data
  const counts = { scheduled: 0, off: 0, past: 0 }
  for (const message of messages ?? []) counts[viewOf(message)] += 1
  const shown = (messages ?? [])
    .filter((message) => viewOf(message) === view)
    .sort(ORDER[view])
  const hasViews = counts.off + counts.past > 0

  const actions = (
    <>
      <Button
        label="Send now"
        clickAction={() => setDialog({ kind: 'send' })}
      />
      <Button
        label="New message"
        variant="primary"
        clickAction={() => setDialog({ kind: 'write', message: null })}
      />
    </>
  )

  return (
    <AppFrame>
      <VStack gap={6}>
        {notice === 'email-verified' ? (
          <Banner
            status="success"
            title="Your email is verified."
            description="You're signed in, and the account is ready."
            isDismissable
          />
        ) : null}
        {query.isRefetchError && !isSignedOut ? <OfflineBanner /> : null}

        <PageHeading
          title="Messages"
          lede={<Summary messages={messages} />}
          actions={messages?.length === 0 ? undefined : actions}
        />

        {messages?.length === 0 ? null : <WhatsAppBanner />}

        {problem ? (
          <Banner
            status="error"
            title={problem.title}
            description={problem.description}
            isDismissable
            onDismiss={() => setProblem(null)}
          />
        ) : null}

        {messages ? (
          messages.length > 0 ? (
            <VStack gap={2}>
              {hasViews ? (
                <TabList
                  value={view}
                  onChange={(value) => setView(value as View)}
                  hasDivider
                >
                  {VIEWS.map((option) => (
                    <Tab
                      key={option.value}
                      value={option.value}
                      label={option.label}
                      endContent={
                        <Text type="supporting" hasTabularNumbers>
                          {counts[option.value]}
                        </Text>
                      }
                    />
                  ))}
                </TabList>
              ) : null}
              {shown.length > 0 ? (
                <MessageTable
                  view={view}
                  messages={shown}
                  onToggle={toggle}
                  onEdit={(message) => setDialog({ kind: 'write', message })}
                  onDelete={(message) => setDialog({ kind: 'delete', message })}
                />
              ) : (
                <VStack paddingBlock={8}>
                  <Text color="secondary">{EMPTY_VIEW[view]}</Text>
                </VStack>
              )}
            </VStack>
          ) : (
            <FirstRun
              onNewMessage={() => setDialog({ kind: 'write', message: null })}
              onSendNow={() => setDialog({ kind: 'send' })}
            />
          )
        ) : query.isPending ? (
          <VStack gap={2} aria-busy="true">
            {[0, 1, 2, 3, 4].map((index) => (
              <Skeleton key={index} index={index} width="100%" height={56} />
            ))}
          </VStack>
        ) : isSignedOut ? null : (
          <Banner
            status="error"
            title="Couldn't load your messages."
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

      <MessageFormDialog
        isOpen={dialog?.kind === 'write'}
        onOpenChange={closeDialog}
        message={dialog?.kind === 'write' ? dialog.message : null}
      />
      <SendNowDialog
        isOpen={dialog?.kind === 'send'}
        onOpenChange={closeDialog}
        onSent={() =>
          toast({
            body: 'Your message is on its way.',
            type: 'info',
            endContent: (
              <Link href="/history" hasUnderline>
                See it in History
              </Link>
            ),
          })
        }
      />
      <ConfirmDeleteDialog
        isOpen={dialog?.kind === 'delete'}
        onOpenChange={closeDialog}
        title="Delete this message?"
        description="It won't send again. What it already sent stays in History."
        onConfirm={async () => {
          if (dialog?.kind === 'delete') await remove(dialog.message)
        }}
      />
    </AppFrame>
  )
}

const EMPTY_VIEW: Record<View, string> = {
  scheduled:
    'Nothing is due to go out. Switch a message on, or write a new one.',
  off: 'Every message is switched on.',
  past: 'No one-time message has run yet. History shows how each send went.',
}

function Summary({ messages }: { messages: ScheduledMessage[] | undefined }) {
  const recipientLabel = useRecipientLabel()
  if (!messages) return null
  const due = messages
    .filter((message) => message.next_run_at)
    .sort(ORDER.scheduled)
  const [next] = due
  if (!next?.next_run_at) {
    return messages.length
      ? 'Nothing is due to go out.'
      : 'Messages go out from your own number, at a set time or on a repeat.'
  }
  return (
    <>
      {due.length} scheduled. Next to {recipientLabel(next)}{' '}
      {formatWhenInSentence(next.next_run_at)}.
    </>
  )
}

interface MessageTableProps {
  view: View
  messages: ScheduledMessage[]
  onToggle: (message: ScheduledMessage, enabled: boolean) => Promise<void>
  onEdit: (message: ScheduledMessage) => void
  onDelete: (message: ScheduledMessage) => void
}

function MessageTable({
  view,
  messages,
  onToggle,
  onEdit,
  onDelete,
}: MessageTableProps) {
  const isNarrow = useMediaQuery('(max-width: 1099px)')
  const recipientLabel = useRecipientLabel()

  const enabled: TableColumn<ScheduledMessage> = {
    key: 'enabled',
    header: <VisuallyHidden>On</VisuallyHidden>,
    width: pixel(56),
    renderCell: (message) =>
      viewOf(message) === 'past' ? null : (
        <Switch
          label={`Send to ${recipientLabel(message)}`}
          isLabelHidden
          size="sm"
          value={message.enabled}
          changeAction={(next) => onToggle(message, next)}
        />
      ),
  }
  const menu: TableColumn<ScheduledMessage> = {
    key: 'actions',
    header: <VisuallyHidden>Actions</VisuallyHidden>,
    width: pixel(48),
    renderCell: (message) => (
      <MoreMenu
        label={`Actions for ${recipientLabel(message)}`}
        alignment="end"
        size="sm"
        items={[
          {
            label: viewOf(message) === 'past' ? 'Schedule again' : 'Edit',
            onClick: () => onEdit(message),
          },
          {
            label: 'Delete',
            variant: 'destructive',
            onClick: () => onDelete(message),
          },
        ]}
      />
    ),
  }
  const when: TableColumn<ScheduledMessage> = {
    key: 'when',
    header: WHEN_HEADER[view],
    width: pixel(184),
    renderCell: (message) => <When message={message} view={view} />,
  }

  const columns: TableColumn<ScheduledMessage>[] = isNarrow
    ? [
        {
          key: 'message',
          header: <VisuallyHidden>Message</VisuallyHidden>,
          width: proportional(1),
          renderCell: (message) => (
            <VStack gap={2}>
              <RecipientCell {...message} isMuted={!message.enabled} />
              <Text color="secondary" maxLines={2}>
                {message.body}
              </Text>
              <HStack gap={2} vAlign="start" wrap="wrap">
                <When message={message} view={view} />
                <Schedule message={message} />
              </HStack>
            </VStack>
          ),
        },
        enabled,
        menu,
      ]
    : [
        {
          key: 'recipient',
          header: 'To',
          width: proportional(3),
          renderCell: (message) => (
            <RecipientCell {...message} isMuted={!message.enabled} />
          ),
        },
        {
          key: 'body',
          header: 'Message',
          width: proportional(4),
          renderCell: (message) => (
            <Text color="secondary" maxLines={1}>
              {message.body}
            </Text>
          ),
        },
        {
          key: 'schedule',
          header: 'Repeats',
          width: proportional(3),
          renderCell: (message) => <Schedule message={message} />,
        },
        when,
        enabled,
        menu,
      ]

  return (
    <Table
      data={messages}
      columns={columns}
      idKey="id"
      verticalAlign={isNarrow ? 'top' : 'middle'}
      hasHover
    />
  )
}

const WHEN_HEADER: Record<View, string> = {
  scheduled: 'Next send',
  off: 'Status',
  past: 'Ran',
}

function When({ message, view }: { message: ScheduledMessage; view: View }) {
  if (view === 'scheduled' && message.next_run_at) {
    return (
      <Text weight="medium" hasTabularNumbers textWrap="nowrap">
        {formatWhen(message.next_run_at)}
      </Text>
    )
  }
  if (view === 'past' && message.last_run_at) {
    return (
      <Text color="secondary" hasTabularNumbers textWrap="nowrap">
        {formatWhen(message.last_run_at)}
      </Text>
    )
  }
  return <Text color="secondary">Off</Text>
}

function Schedule({ message }: { message: ScheduledMessage }) {
  const zone = zoneLabel(message.timezone)
  const label =
    message.schedule_kind === 'once'
      ? 'Once'
      : summariseCron(message.cron ?? '')
  return (
    <VStack>
      <Text color="secondary" maxLines={1}>
        {label}
      </Text>
      {zone && message.schedule_kind === 'recurring' ? (
        <Text type="supporting">{zone}</Text>
      ) : null}
    </VStack>
  )
}
