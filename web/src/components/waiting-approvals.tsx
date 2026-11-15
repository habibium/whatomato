import { useEffect, useRef } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { Heading, Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'

import { ApprovalCard } from '@/components/agent-approval'
import {
  APPROVAL_DOTS,
  WAITING_MAX,
  approvalsQuery,
} from '@/lib/agent-activity'
import { formatWhenInSentence } from '@/lib/format'

interface WaitingApprovalsProps {
  /** From the overview, so nothing is fetched while nothing waits. */
  waiting: number
  /** The approval a link pointed at. */
  markedID?: string
}

export function WaitingApprovals({ waiting, markedID }: WaitingApprovalsProps) {
  const query = useQuery({
    ...approvalsQuery(0, WAITING_MAX),
    enabled: waiting > 0 || markedID !== undefined,
  })
  const markedRef = useRef<HTMLElement>(null)
  const hasScrolled = useRef(false)
  const items = query.data?.items ?? []
  const shown = items.filter((item) => item.status === 'waiting')
  const marked = markedID
    ? items.find((item) => item.id === markedID)
    : undefined
  const isMarkedWaiting = marked?.status === 'waiting'

  useEffect(() => {
    if (!isMarkedWaiting || hasScrolled.current || !markedRef.current) return
    hasScrolled.current = true
    markedRef.current.scrollIntoView({ block: 'start', behavior: 'smooth' })
    markedRef.current.focus({ preventScroll: true })
  }, [isMarkedWaiting])

  const answered =
    markedID && query.data && !isMarkedWaiting ? (
      marked ? (
        <Banner
          status="info"
          title={`That request was ${APPROVAL_DOTS[marked.status].label.toLowerCase()}${marked.decided_at ? ` ${formatWhenInSentence(marked.decided_at)}` : ''}`}
          description={`${marked.connection_name} asked to message ${marked.recipient_name ?? marked.recipient}. ${afterwards(marked)}`}
          endContent={
            <Button
              label="Earlier requests"
              size="sm"
              href="/agents/approvals"
            />
          }
        />
      ) : (
        <Banner
          status="info"
          title="That request isn’t waiting any more"
          description="It was answered, expired, or its connection was revoked. Earlier requests shows what became of each."
          endContent={
            <Button
              label="Earlier requests"
              size="sm"
              href="/agents/approvals"
            />
          }
        />
      )
    ) : null

  if (shown.length === 0) return answered

  return (
    <VStack as="section" gap={4} aria-labelledby="waiting-approvals">
      {answered}
      <VStack gap={1}>
        <Heading level={2} id="waiting-approvals">
          {shown.length === 1
            ? 'One request waits for you'
            : `${shown.length} requests wait for you`}
        </Heading>
        <Text color="secondary">
          Read who it goes to and the whole message. Nothing goes out until you
          approve it.
        </Text>
      </VStack>
      <VStack gap={4}>
        {shown.map((approval) => (
          <ApprovalCard
            key={approval.id}
            ref={approval.id === markedID ? markedRef : undefined}
            approval={approval}
            isMarked={approval.id === markedID}
          />
        ))}
      </VStack>
    </VStack>
  )
}

export function afterwards(approval: {
  status: string
  kind: string
  delivery_status: string | null
  scheduled_message_id: string | null
}) {
  if (approval.status === 'rejected') return 'Nothing was sent.'
  if (approval.status === 'expired')
    return 'No one answered in time, so nothing was sent.'
  if (approval.status !== 'approved') return ''
  if (approval.kind === 'send') {
    switch (approval.delivery_status) {
      case 'sent':
        return 'The message went out.'
      case 'failed':
        return 'The message failed to send.'
      case 'pending':
        return 'The message is on its way.'
      default:
        return 'The message isn’t in History any more.'
    }
  }
  return approval.scheduled_message_id
    ? approval.kind === 'schedule_create'
      ? 'The message was scheduled.'
      : 'The scheduled message was changed.'
    : 'That scheduled message has since been deleted.'
}
