import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useLocation } from '@tanstack/react-router'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { useAnnounce, useMediaQuery } from '@astryxdesign/core/hooks'
import { Icon } from '@astryxdesign/core/Icon'
import { Link } from '@astryxdesign/core/Link'
import { MoreMenu } from '@astryxdesign/core/MoreMenu'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { Switch } from '@astryxdesign/core/Switch'
import { Table, pixel, proportional } from '@astryxdesign/core/Table'
import { Text } from '@astryxdesign/core/Text'
import { Timestamp } from '@astryxdesign/core/Timestamp'
import { VisuallyHidden } from '@astryxdesign/core/VisuallyHidden'
import { VStack } from '@astryxdesign/core/VStack'
import { User, Users } from 'lucide-react'

import type { TableColumn } from '@astryxdesign/core/Table'

import { api } from '@/api/client'
import { AppFrame } from '@/components/app-frame'
import { ConfirmDeleteDialog } from '@/components/confirm-dialog'
import { ListEmpty } from '@/components/list-empty'
import { MessageFormDialog } from '@/components/message-form-dialog'
import { OfflineBanner } from '@/components/offline-banner'
import { PageHeading } from '@/components/page-heading'
import { SendNowDialog } from '@/components/send-now-dialog'
import { WhatsAppBanner } from '@/components/whatsapp-banner'
import { describeCron } from '@/lib/cron'
import {
  BROWSER_TIME_ZONE,
  messagesQuery,
  recipientLabel,
} from '@/lib/scheduler'
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

function MessagesPage() {
  const notice = useLocation({ select: (location) => location.state.notice })
  const queryClient = useQueryClient()
  const announce = useAnnounce()
  const query = useQuery(messagesQuery)
  const isSignedOut = useSessionExpiry(query.error, '/messages')
  const [dialog, setDialog] = useState<OpenDialog | null>(null)
  const [problem, setProblem] = useState<WriteProblem | null>(null)
  const [isSent, setIsSent] = useState(false)

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
  const isEmpty = messages?.length === 0
  const actions = (
    <>
      <Button
        label="Send now"
        size="lg"
        clickAction={() => setDialog({ kind: 'send' })}
      />
      <Button
        label="New message"
        variant="primary"
        size="lg"
        clickAction={() => setDialog({ kind: 'write', message: null })}
      />
    </>
  )

  return (
    <AppFrame>
      <VStack gap={8} paddingBlock={10}>
        {notice === 'email-verified' ? (
          <Banner
            status="success"
            title="Your email is verified."
            description="You're signed in, and the account is ready."
            isDismissable
          />
        ) : null}
        {query.isRefetchError && !isSignedOut ? <OfflineBanner /> : null}
        <WhatsAppBanner />

        <PageHeading
          title="Scheduled messages."
          lede={<Summary messages={messages} />}
          actions={isEmpty ? undefined : actions}
        />

        {isSent ? (
          <Banner
            status="success"
            title="Your message is on its way."
            description={
              <>
                See how it went in{' '}
                <Link
                  href="/history"
                  type="inherit"
                  color="inherit"
                  hasUnderline
                >
                  History
                </Link>
                .
              </>
            }
            isDismissable
            onDismiss={() => setIsSent(false)}
          />
        ) : null}
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
            <MessageTable
              messages={messages}
              onToggle={toggle}
              onEdit={(message) => setDialog({ kind: 'write', message })}
              onDelete={(message) => setDialog({ kind: 'delete', message })}
            />
          ) : (
            <ListEmpty
              title="Nothing scheduled yet"
              description="Write a message once, and it goes out on its own: at a set time, or on a repeat."
              actions={actions}
            />
          )
        ) : query.isPending ? (
          <VStack gap={3} aria-busy="true">
            {[0, 1, 2, 3].map((index) => (
              <Skeleton key={index} index={index} width="100%" height={44} />
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
        onSent={() => setIsSent(true)}
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

function Summary({ messages }: { messages: ScheduledMessage[] | undefined }) {
  const live = (messages ?? []).flatMap((message) =>
    message.next_run_at ? [{ message, at: message.next_run_at }] : [],
  )
  if (live.length === 0) {
    return messages?.length
      ? 'None of these are due to go out.'
      : 'Messages go out from your own number, at a set time or on a repeat.'
  }

  const next = live.reduce((soonest, run) =>
    Date.parse(run.at) < Date.parse(soonest.at) ? run : soonest,
  )
  return (
    <>
      {live.length} on schedule. Next to {recipientLabel(next.message)},{' '}
      <Timestamp
        value={next.at}
        format="date_time"
        type="inherit"
        color="inherit"
      />
      .
    </>
  )
}

interface MessageTableProps {
  messages: ScheduledMessage[]
  onToggle: (message: ScheduledMessage, enabled: boolean) => Promise<void>
  onEdit: (message: ScheduledMessage) => void
  onDelete: (message: ScheduledMessage) => void
}

function MessageTable({
  messages,
  onToggle,
  onEdit,
  onDelete,
}: MessageTableProps) {
  const isNarrow = useMediaQuery('(max-width: 999px)')

  const enabled: TableColumn<ScheduledMessage> = {
    key: 'enabled',
    header: <VisuallyHidden>Enabled</VisuallyHidden>,
    width: pixel(64),
    renderCell: (message) => (
      <Switch
        label={`Enabled, to ${recipientLabel(message)}`}
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
    width: pixel(52),
    renderCell: (message) => (
      <MoreMenu
        label={`Actions for ${recipientLabel(message)}`}
        alignment="end"
        size="sm"
        items={[
          { label: 'Edit', onClick: () => onEdit(message) },
          {
            label: 'Delete',
            variant: 'destructive',
            onClick: () => onDelete(message),
          },
        ]}
      />
    ),
  }

  const columns: TableColumn<ScheduledMessage>[] = isNarrow
    ? [
        {
          key: 'message',
          header: 'Message',
          width: proportional(1),
          renderCell: (message) => (
            <VStack gap={1}>
              <Recipient message={message} />
              <Text color="secondary" maxLines={2}>
                {message.body}
              </Text>
              <Schedule message={message} />
              <NextRun message={message} />
            </VStack>
          ),
        },
        enabled,
        menu,
      ]
    : [
        {
          key: 'recipient',
          header: 'Recipient',
          width: proportional(3),
          renderCell: (message) => <Recipient message={message} />,
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
          header: 'Schedule',
          width: proportional(3),
          renderCell: (message) => <Schedule message={message} />,
        },
        {
          key: 'next',
          header: 'Next run',
          width: pixel(208),
          renderCell: (message) => <NextRun message={message} />,
        },
        enabled,
        menu,
      ]

  return (
    <Table
      data={messages}
      columns={columns}
      idKey="id"
      verticalAlign={isNarrow ? 'top' : 'middle'}
    />
  )
}

function Recipient({ message }: { message: ScheduledMessage }) {
  const isGroup = message.recipient_type === 'group'
  return (
    <HStack gap={2} vAlign="center">
      <Icon icon={isGroup ? Users : User} size="sm" color="secondary" />
      <VisuallyHidden>{isGroup ? 'Group: ' : 'Contact: '}</VisuallyHidden>
      <Text
        weight="medium"
        color={message.enabled ? 'primary' : 'secondary'}
        maxLines={1}
        hasTabularNumbers
      >
        {recipientLabel(message)}
      </Text>
    </HStack>
  )
}

function Schedule({ message }: { message: ScheduledMessage }) {
  if (message.schedule_kind === 'once') {
    return <Text color="secondary">One time</Text>
  }
  return (
    <VStack>
      <Text color="secondary">
        {describeCron(message.cron ?? '') ?? message.cron}
      </Text>
      {message.timezone === BROWSER_TIME_ZONE ? null : (
        <Text type="supporting">{message.timezone}</Text>
      )}
    </VStack>
  )
}

function NextRun({ message }: { message: ScheduledMessage }) {
  if (message.next_run_at) {
    return (
      <Timestamp
        value={message.next_run_at}
        format="date_time"
        type="body"
        color="primary"
      />
    )
  }
  if (message.schedule_kind === 'once' && message.last_run_at) {
    return (
      <Text color="secondary">
        Ran{' '}
        <Timestamp
          value={message.last_run_at}
          format="date_time"
          type="inherit"
          color="inherit"
        />
      </Text>
    )
  }
  return <Text color="secondary">Off</Text>
}
