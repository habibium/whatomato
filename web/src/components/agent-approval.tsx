import { forwardRef, useId, useMemo, useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQueryClient } from '@tanstack/react-query'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { useAnnounce } from '@astryxdesign/core/hooks'
import { Link } from '@astryxdesign/core/Link'
import { Heading, Text } from '@astryxdesign/core/Text'
import { TextArea } from '@astryxdesign/core/TextArea'
import { useToast } from '@astryxdesign/core/Toast'
import { VStack } from '@astryxdesign/core/VStack'
import {
  borderVars,
  colorVars,
  radiusVars,
  spacingVars,
} from '@astryxdesign/core/theme/tokens.stylex'

import type { components } from '@/api/schema'

import { api } from '@/api/client'
import { RecipientCell } from '@/components/recipient-cell'
import { RevealedText } from '@/components/revealed-text'
import { KIND_LABELS, formatRelative } from '@/lib/agent-activity'
import type { AgentApproval } from '@/lib/agent-activity'
import { agentsQuery } from '@/lib/agents'
import {
  formatWhen,
  formatWhenInSentence,
  summariseRecurrence,
  zoneLabel,
} from '@/lib/format'
import { revealHidden } from '@/lib/hidden-characters'
import { formatPhoneNumber } from '@/lib/whatsapp'
import { connectionProblem, toWriteProblem } from '@/lib/write-problem'
import type { WriteProblem } from '@/lib/write-problem'

const styles = stylex.create({
  card: {
    borderColor: colorVars['--color-border'],
    borderRadius: radiusVars['--radius-container'],
    borderStyle: 'solid',
    borderWidth: borderVars['--border-width'],
    minWidth: 0,
    padding: {
      default: spacingVars['--spacing-4'],
      '@media (min-width: 640px)': spacingVars['--spacing-6'],
    },
    scrollMarginBlock: spacingVars['--spacing-8'],
  },
  // The one the link in the email or the agent's answer pointed at
  marked: {
    borderColor: colorVars['--color-accent'],
    boxShadow: `0 0 0 ${borderVars['--border-width']} ${colorVars['--color-accent']}`,
  },
  // Keyboard focus lands on the card when a link opens it; the ring is the marked border
  focus: { outline: 'none' },
  text: { minWidth: 0 },
})

type ErrorBody = components['schemas']['ErrorBody']

const STANDING: Record<
  Exclude<AgentApproval['connection_status'], 'active'>,
  { title: string; description: string }
> = {
  paused: {
    title: 'It came from a connection you have since paused',
    description: 'The agent can’t do anything else until you resume it.',
  },
  expired: {
    title: 'It came from a connection whose key has since expired',
    description: 'The agent can’t do anything else until you replace its key.',
  },
  unusable: {
    title: 'It came from a connection that needs attention',
    description:
      'Its permissions can’t be read, so the agent can’t do anything else until you set them again.',
  },
}

interface ApprovalCardProps {
  approval: AgentApproval
  isMarked?: boolean
}

/** One waiting request: who it goes to and the whole text first, the decision after. */
export const ApprovalCard = forwardRef<HTMLElement, ApprovalCardProps>(
  function ApprovalCard({ approval, isMarked = false }, ref) {
    const queryClient = useQueryClient()
    const announce = useAnnounce()
    const toast = useToast()
    const headingID = useId()
    const [draft, setDraft] = useState<string | null>(null)
    const [reasons, setReasons] = useState<string[]>([])
    const [problem, setProblem] = useState<WriteProblem | null>(null)
    const [busy, setBusy] = useState<'approve' | 'reject' | null>(null)
    const request = approval.request
    const body = request?.body ?? ''
    const revealed = useMemo(() => revealHidden(body), [body])
    const edited = useMemo(
      () => (draft !== null ? revealHidden(draft) : null),
      [draft, body],
    )
    const who =
      approval.recipient_name ??
      (approval.recipient_type === 'contact'
        ? formatPhoneNumber(approval.recipient)
        : 'the group')

    const settle = async () => {
      // ['agents'] covers the navigation count, the approvals and the connection pages
      await queryClient.invalidateQueries({ queryKey: agentsQuery.queryKey })
    }

    const decide = async (action: 'approve' | 'reject') => {
      const failed =
        action === 'approve'
          ? "Couldn't approve this request."
          : "Couldn't reject this request."
      setProblem(null)
      setReasons([])
      setBusy(action)
      try {
        const params = { params: { path: { id: approval.id } } }
        const result =
          action === 'approve'
            ? await api.POST('/api/agents/approvals/{id}/approve', {
                ...params,
                body: draft !== null && draft !== body ? { body: draft } : {},
              })
            : await api.POST('/api/agents/approvals/{id}/reject', params)
        if (result.data) {
          const message = decidedMessage(result.data, who)
          announce(message)
          toast({ body: message, type: 'info' })
          await settle()
          return
        }
        const error = result.error as ErrorBody | undefined
        const status = result.response.status
        if (status === 409) {
          const now = error?.details?.status?.[0]
          const message =
            now === 'expired'
              ? 'This request had already expired.'
              : now === 'approved' || now === 'rejected'
                ? `This request was already ${now}.`
                : 'This request was already answered.'
          announce(message)
          toast({ body: message, type: 'info' })
          await settle()
          return
        }
        if (status === 422 && error?.details?.request?.length) {
          setReasons(error.details.request)
          return
        }
        if (status === 404) {
          toast({ body: 'This request isn’t there any more.', type: 'info' })
          await settle()
          return
        }
        setProblem(toWriteProblem(result, failed))
      } catch {
        setProblem(connectionProblem(failed))
      } finally {
        setBusy(null)
      }
    }

    return (
      <VStack
        as="article"
        ref={ref}
        tabIndex={-1}
        aria-labelledby={headingID}
        gap={5}
        xstyle={[styles.card, styles.focus, isMarked && styles.marked]}
      >
        <HStack gap={4} justify="between" vAlign="start" wrap="wrap">
          <VStack gap={1} xstyle={styles.text}>
            <Heading level={3} id={headingID}>
              {request
                ? KIND_LABELS[approval.kind]
                : 'A request that can’t be read'}
            </Heading>
            <Text color="secondary">
              Asked by {approval.connection_name}{' '}
              {formatWhenInSentence(approval.created_at)}
            </Text>
          </VStack>
          <Text color="secondary" hasTabularNumbers>
            Expires {formatRelative(approval.expires_at)}
          </Text>
        </HStack>

        <RecipientCell
          recipient_type={approval.recipient_type}
          recipient={approval.recipient}
          recipient_name={approval.recipient_name}
        />

        {request?.schedule ? (
          <ScheduleWords
            request={request}
            isChange={approval.kind === 'schedule_update'}
          />
        ) : null}

        {request ? (
          draft === null ? (
            <RevealedText revealed={revealed} />
          ) : (
            <VStack gap={3}>
              <TextArea
                label="Message"
                description="Approving sends your version instead of the agent’s."
                value={draft}
                rows={Math.min(Math.max(draft.split('\n').length, 4), 20)}
                onChange={(value) => setDraft(value)}
                width="100%"
              />
              {edited && edited.total > 0 ? (
                <RevealedText revealed={edited} />
              ) : null}
            </VStack>
          )
        ) : (
          <Banner
            status="error"
            title="Whatomato can’t read what the agent asked for"
            description={
              approval.problem ??
              'It can’t be approved. Reject it, and ask the agent again.'
            }
          />
        )}

        {approval.status === 'waiting' &&
        approval.connection_status !== 'active' ? (
          <Banner
            status="warning"
            title={STANDING[approval.connection_status].title}
            description={`${STANDING[approval.connection_status].description} Approving still sends it.`}
          />
        ) : null}

        {reasons.length ? (
          <Banner
            status="error"
            title="This can’t go ahead"
            description={reasons.join(' ')}
          />
        ) : null}
        {problem ? (
          <Banner
            status="error"
            title={problem.title}
            description={problem.description}
          />
        ) : null}

        <HStack gap={2} justify="end" wrap="wrap">
          {approval.can_reject ? (
            <Button
              label="Reject"
              aria-label={`Reject: ${KIND_LABELS[approval.kind].toLowerCase()} to ${who}`}
              variant="secondary"
              isLoading={busy === 'reject'}
              isDisabled={busy === 'approve'}
              clickAction={() => decide('reject')}
            />
          ) : null}
          {request && approval.can_approve ? (
            draft === null ? (
              <>
                <Button
                  label="Edit"
                  aria-label={`Edit the message to ${who}`}
                  variant="secondary"
                  isDisabled={busy !== null}
                  clickAction={() => setDraft(body)}
                />
                <Button
                  label="Approve"
                  aria-label={`Approve: ${KIND_LABELS[approval.kind].toLowerCase()} to ${who}`}
                  variant="primary"
                  isLoading={busy === 'approve'}
                  isDisabled={busy === 'reject'}
                  clickAction={() => decide('approve')}
                />
              </>
            ) : (
              <>
                <Button
                  label="Undo edits"
                  variant="secondary"
                  isDisabled={busy !== null}
                  clickAction={() => {
                    setDraft(null)
                    setReasons([])
                  }}
                />
                <Button
                  label={draft === body ? 'Approve' : 'Approve edited'}
                  variant="primary"
                  isLoading={busy === 'approve'}
                  isDisabled={busy === 'reject' || draft.trim() === ''}
                  clickAction={() => decide('approve')}
                />
              </>
            )
          ) : null}
        </HStack>
      </VStack>
    )
  },
)

function ScheduleWords({
  request,
  isChange,
}: {
  request: NonNullable<AgentApproval['request']>
  isChange: boolean
}) {
  const schedule = request.schedule
  if (!schedule) return null
  const zone = zoneLabel(request.timezone)
  const how =
    schedule.schedule_kind === 'once'
      ? schedule.run_at
        ? `Once, ${formatWhenInSentence(schedule.run_at)}`
        : 'Once'
      : summariseRecurrence(schedule.recurrence)
  const where = [zone, schedule.city ? `sun times for ${schedule.city}` : null]
    .filter(Boolean)
    .join(', ')
  const next = schedule.next_runs.map(formatWhen)
  return (
    <VStack gap={1}>
      <Text weight="medium">
        {how}
        {where ? `, ${where}` : ''}
        {schedule.enabled ? '' : '. Switched off'}
      </Text>
      {schedule.schedule_kind === 'recurring' && next.length ? (
        <Text color="secondary" hasTabularNumbers>
          Next: {next.join(' · ')}
        </Text>
      ) : null}
      {isChange && schedule.scheduled_message_id ? (
        // Messages has no link to one message, so this opens the list it's in
        <Text type="supporting" color="secondary">
          It changes a message you already scheduled.{' '}
          <Link href="/messages" color="secondary" hasUnderline>
            See it in Messages
          </Link>
        </Text>
      ) : null}
    </VStack>
  )
}

function decidedMessage(approval: AgentApproval, who: string) {
  if (approval.status === 'rejected') return `Rejected. Nothing goes to ${who}.`
  if (approval.kind === 'send')
    return `Approved. The message to ${who} is on its way.`
  return approval.kind === 'schedule_create'
    ? `Approved. The message to ${who} is scheduled.`
    : `Approved. The scheduled message to ${who} is changed.`
}
