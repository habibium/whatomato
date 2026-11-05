import { useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { useMediaQuery } from '@astryxdesign/core/hooks'
import { Pagination } from '@astryxdesign/core/Pagination'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { StatusDot } from '@astryxdesign/core/StatusDot'
import { Table, pixel, proportional } from '@astryxdesign/core/Table'
import { Text } from '@astryxdesign/core/Text'
import { Timestamp } from '@astryxdesign/core/Timestamp'
import { VStack } from '@astryxdesign/core/VStack'
import { colorVars } from '@astryxdesign/core/theme/tokens.stylex'

import type { TableColumn } from '@astryxdesign/core/Table'

import { AppFrame } from '@/components/app-frame'
import { ListEmpty } from '@/components/list-empty'
import { OfflineBanner } from '@/components/offline-banner'
import { PageHeading } from '@/components/page-heading'
import {
  DELIVERIES_PER_PAGE,
  deliveriesQuery,
  recipientLabel,
} from '@/lib/scheduler'
import type { Delivery } from '@/lib/scheduler'
import { useSessionExpiry } from '@/lib/use-session-expiry'

export const Route = createFileRoute('/_authenticated/history')({
  component: HistoryPage,
})

const styles = stylex.create({
  failure: { color: colorVars['--color-text-red'] },
})

function HistoryPage() {
  const [page, setPage] = useState(1)
  const query = useQuery(deliveriesQuery(page))
  const isSignedOut = useSessionExpiry(query.error, '/history')
  const deliveries = query.data

  return (
    <AppFrame>
      <VStack gap={8} paddingBlock={10}>
        {query.isRefetchError && !isSignedOut ? <OfflineBanner /> : null}

        <PageHeading
          title="Delivery history."
          lede="Everything Whatomato sent, scheduled or straight away. Sent means WhatsApp accepted it, which doesn’t prove it reached the phone."
        />

        {deliveries ? (
          deliveries.items.length > 0 ? (
            <VStack gap={4}>
              <DeliveryTable deliveries={deliveries.items} />
              <HStack justify="end">
                <Pagination
                  variant="count"
                  size="sm"
                  label="History pages"
                  page={page}
                  onChange={setPage}
                  totalItems={deliveries.total}
                  pageSize={DELIVERIES_PER_PAGE}
                  isDisabled={query.isPlaceholderData}
                />
              </HStack>
            </VStack>
          ) : (
            <ListEmpty
              title="Nothing sent yet"
              description="Messages show here once they go out, scheduled or sent straight away."
              actions={<Button label="Go to messages" href="/messages" />}
            />
          )
        ) : query.isPending ? (
          <VStack gap={3} aria-busy="true">
            {[0, 1, 2, 3, 4].map((index) => (
              <Skeleton key={index} index={index} width="100%" height={44} />
            ))}
          </VStack>
        ) : isSignedOut ? null : (
          <Banner
            status="error"
            title="Couldn't load your history."
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
    </AppFrame>
  )
}

function DeliveryTable({ deliveries }: { deliveries: Delivery[] }) {
  const isNarrow = useMediaQuery('(max-width: 899px)')

  const columns: TableColumn<Delivery>[] = isNarrow
    ? [
        {
          key: 'delivery',
          header: 'Delivery',
          width: proportional(1),
          renderCell: (delivery) => (
            <VStack gap={1}>
              <HStack gap={3} justify="between" vAlign="center">
                <Recipient delivery={delivery} />
                <Status delivery={delivery} />
              </HStack>
              <Body delivery={delivery} />
              <When delivery={delivery} />
            </VStack>
          ),
        },
      ]
    : [
        {
          key: 'status',
          header: 'Status',
          width: pixel(160),
          renderCell: (delivery) => <Status delivery={delivery} />,
        },
        {
          key: 'recipient',
          header: 'Recipient',
          width: proportional(2),
          renderCell: (delivery) => <Recipient delivery={delivery} />,
        },
        {
          key: 'body',
          header: 'Message',
          width: proportional(4),
          renderCell: (delivery) => <Body delivery={delivery} />,
        },
        {
          key: 'when',
          header: 'When',
          width: pixel(208),
          renderCell: (delivery) => <When delivery={delivery} />,
        },
      ]

  return (
    <Table
      data={deliveries}
      columns={columns}
      idKey="id"
      verticalAlign={isNarrow ? 'top' : 'middle'}
    />
  )
}

function describeStatus({ status, error }: Delivery) {
  switch (status) {
    case 'sent':
      return { label: 'Sent', variant: 'success', isPulsing: false } as const
    case 'failed':
      return { label: 'Failed', variant: 'error', isPulsing: false } as const
    case 'pending':
      return error
        ? ({
            label: 'Waiting to retry',
            variant: 'warning',
            isPulsing: true,
          } as const)
        : ({ label: 'Sending', variant: 'accent', isPulsing: true } as const)
  }
}

function Status({ delivery }: { delivery: Delivery }) {
  const status = describeStatus(delivery)
  return (
    <HStack gap={2} vAlign="center">
      {/* The word beside it says the same, so the dot itself stays quiet */}
      <StatusDot
        aria-hidden="true"
        variant={status.variant}
        label={status.label}
        isPulsing={status.isPulsing}
      />
      <Text>{status.label}</Text>
    </HStack>
  )
}

function Recipient({ delivery }: { delivery: Delivery }) {
  return (
    <Text weight="medium" maxLines={1} hasTabularNumbers>
      {recipientLabel(delivery)}
    </Text>
  )
}

function Body({ delivery }: { delivery: Delivery }) {
  const isFailed = delivery.status === 'failed'
  return (
    <VStack>
      <Text color="secondary" maxLines={1}>
        {delivery.body}
      </Text>
      {delivery.error ? (
        // Text's own colour outranks xstyle, so a failure's red comes from around it
        <VStack xstyle={isFailed ? styles.failure : undefined}>
          <Text type="supporting" color={isFailed ? 'inherit' : 'secondary'}>
            {delivery.error}
          </Text>
        </VStack>
      ) : null}
    </VStack>
  )
}

function When({ delivery }: { delivery: Delivery }) {
  return (
    <Timestamp
      value={delivery.sent_at ?? delivery.created_at}
      format="date_time"
      type="body"
      color="secondary"
    />
  )
}
