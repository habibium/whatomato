import * as stylex from '@stylexjs/stylex'
import { useQuery } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { List, ListItem } from '@astryxdesign/core/List'
import { Pagination } from '@astryxdesign/core/Pagination'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { StatusDot } from '@astryxdesign/core/StatusDot'
import { Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'

import { AppFrame } from '@/components/app-frame'
import { BackLink } from '@/components/back-link'
import { ListEmpty } from '@/components/list-empty'
import { OfflineBanner } from '@/components/offline-banner'
import { PageHeading } from '@/components/page-heading'
import { RevealedText } from '@/components/revealed-text'
import { afterwards } from '@/components/waiting-approvals'
import {
  APPROVALS_PER_PAGE,
  APPROVAL_DOTS,
  KIND_LABELS,
  approvalsQuery,
} from '@/lib/agent-activity'
import type { AgentApproval } from '@/lib/agent-activity'
import { agentsQuery } from '@/lib/agents'
import { useRecipientLabel } from '@/lib/contacts'
import { formatWhenInSentence } from '@/lib/format'
import { revealHidden } from '@/lib/hidden-characters'
import { useSessionExpiry } from '@/lib/use-session-expiry'

const styles = stylex.create({
  status: { flexShrink: 0, whiteSpace: 'nowrap' },
})

export const Route = createFileRoute('/_authenticated/agents/approvals')({
  validateSearch: (search): { page?: number } => ({
    page:
      typeof search.page === 'number' && search.page > 1
        ? Math.floor(search.page)
        : undefined,
  }),
  component: EarlierApprovalsPage,
})

function EarlierApprovalsPage() {
  const { page = 1 } = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  const overview = useQuery(agentsQuery)
  const waiting = overview.data?.approvals_waiting
  // Waiting ones come first in the server's order; this view starts after them
  const query = useQuery({
    ...approvalsQuery(
      (waiting ?? 0) + (page - 1) * APPROVALS_PER_PAGE,
      APPROVALS_PER_PAGE,
    ),
    enabled: waiting !== undefined,
  })
  const isSignedOut = useSessionExpiry(
    query.error ?? overview.error,
    '/agents/approvals',
  )
  const data = query.data
  const items = data?.items.filter((item) => item.status !== 'waiting')
  const total = data ? data.total - (waiting ?? 0) : 0

  return (
    <AppFrame>
      <VStack gap={6}>
        {query.isRefetchError && !isSignedOut ? <OfflineBanner /> : null}
        <BackLink href="/agents" label="Agents" />
        <PageHeading
          title="Earlier requests"
          lede={
            data
              ? total > 0
                ? `${total} ${total === 1 ? 'request' : 'requests'} your agents made that you approved or rejected, or that expired.`
                : undefined
              : undefined
          }
        />

        {waiting ? (
          <Banner
            status="info"
            title={`${waiting} ${waiting === 1 ? 'request waits' : 'requests wait'} for you`}
            description="Answer them on the Agents page."
            endContent={
              <Button label="Go to agents" size="sm" href="/agents" />
            }
          />
        ) : null}

        {items ? (
          items.length > 0 ? (
            <VStack gap={6}>
              <List hasDividers>
                {items.map((item) => (
                  <EarlierRow key={item.id} approval={item} />
                ))}
              </List>
              {total > APPROVALS_PER_PAGE ? (
                <HStack justify="end">
                  <Pagination
                    variant="count"
                    size="sm"
                    label="Request pages"
                    page={page}
                    onChange={(next) => navigate({ search: { page: next } })}
                    totalItems={total}
                    pageSize={APPROVALS_PER_PAGE}
                    isDisabled={query.isPlaceholderData}
                  />
                </HStack>
              ) : null}
            </VStack>
          ) : (
            <ListEmpty
              title="No earlier requests"
              description="When an agent asks before sending, its request shows on the Agents page. Once you answer it, or it expires, it moves here."
              actions={<Button label="Go to agents" href="/agents" />}
            />
          )
        ) : query.isPending && !isSignedOut && !overview.isError ? (
          <VStack gap={2} aria-busy="true">
            {[0, 1, 2, 3].map((index) => (
              <Skeleton key={index} index={index} width="100%" height={72} />
            ))}
          </VStack>
        ) : isSignedOut ? null : (
          <Banner
            status="error"
            title="Couldn't load earlier requests."
            description="Check your connection and try again."
            endContent={
              <Button
                label="Try again"
                size="sm"
                clickAction={async () => {
                  await Promise.all([overview.refetch(), query.refetch()])
                }}
              />
            }
          />
        )}
      </VStack>
    </AppFrame>
  )
}

function EarlierRow({ approval }: { approval: AgentApproval }) {
  const who = useRecipientLabel()({
    recipient_type: approval.recipient_type,
    recipient: approval.recipient,
    recipient_name: approval.recipient_name,
  })
  const dot = APPROVAL_DOTS[approval.status]
  return (
    <ListItem
      label={`${KIND_LABELS[approval.kind]} to ${who}`}
      description={
        <VStack gap={1}>
          <Text color="secondary">
            Asked by {approval.connection_name}{' '}
            {formatWhenInSentence(approval.created_at)}. {afterwards(approval)}
          </Text>
          {approval.request ? (
            <RevealedText
              revealed={revealHidden(approval.request.body)}
              isCompact
            />
          ) : approval.problem ? (
            <Text type="supporting">{approval.problem}</Text>
          ) : null}
        </VStack>
      }
      endContent={
        <HStack gap={2} vAlign="center" xstyle={styles.status}>
          <StatusDot
            aria-hidden="true"
            variant={dot.variant}
            label={dot.label}
          />
          <Text>{dot.label}</Text>
        </HStack>
      }
    />
  )
}
