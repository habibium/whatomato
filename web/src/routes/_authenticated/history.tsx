import { useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { List, ListItem } from '@astryxdesign/core/List'
import { Pagination } from '@astryxdesign/core/Pagination'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { Heading, Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'
import {
  colorVars,
  spacingVars,
  typeScaleVars,
} from '@astryxdesign/core/theme/tokens.stylex'

import { AppFrame } from '@/components/app-frame'
import { DeliveryStatus } from '@/components/delivery-status'
import { PersonAvatar } from '@/components/person-avatar'
import { ListEmpty } from '@/components/list-empty'
import { OfflineBanner } from '@/components/offline-banner'
import { PageHeading } from '@/components/page-heading'
import { dayKey, formatClock, formatDayHeading } from '@/lib/format'
import { useRecipientLabel, useRecipientPictureID } from '@/lib/contacts'
import { DELIVERIES_PER_PAGE, deliveriesQuery } from '@/lib/scheduler'
import type { Delivery } from '@/lib/scheduler'
import { useSessionExpiry } from '@/lib/use-session-expiry'

export const Route = createFileRoute('/_authenticated/history')({
  component: HistoryPage,
})

const styles = stylex.create({
  failure: { color: colorVars['--color-text-red'] },
  // One slot wide enough for "11:30 PM", so every name starts on the same edge
  time: {
    flexShrink: 0,
    width: { default: '4.25rem', '@media (min-width: 640px)': '4.75rem' },
  },
  start: { paddingTop: spacingVars['--spacing-0-5'] },
  day: {
    fontSize: typeScaleVars['--text-heading-3-size'],
    lineHeight: typeScaleVars['--text-heading-3-leading'],
  },
  // On a phone the time already says which row it is; the message needs the width
  avatar: {
    display: { default: 'none', '@media (min-width: 640px)': 'flex' },
  },
})

function HistoryPage() {
  const [page, setPage] = useState(1)
  const query = useQuery(deliveriesQuery(page))
  const isSignedOut = useSessionExpiry(query.error, '/history')
  const deliveries = query.data

  return (
    <AppFrame>
      <VStack gap={6}>
        {query.isRefetchError && !isSignedOut ? <OfflineBanner /> : null}

        <PageHeading
          title="History"
          lede={
            <Summary deliveries={deliveries?.items} total={deliveries?.total} />
          }
        />

        {deliveries ? (
          deliveries.items.length > 0 ? (
            <VStack gap={6}>
              <DeliveryLog deliveries={deliveries.items} />
              <HStack gap={4} justify="between" vAlign="center" wrap="wrap">
                <Text type="supporting">
                  Sent means WhatsApp accepted the message. It doesn’t prove it
                  reached the phone.
                </Text>
                {deliveries.total > DELIVERIES_PER_PAGE ? (
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
                ) : null}
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
          <VStack gap={2} aria-busy="true">
            {[0, 1, 2, 3, 4].map((index) => (
              <Skeleton key={index} index={index} width="100%" height={56} />
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

function Summary({
  deliveries,
  total,
}: {
  deliveries: Delivery[] | undefined
  total: number | undefined
}) {
  if (!deliveries || total === undefined) return null
  if (total === 0) return 'Everything Whatomato sends shows here.'
  const failed = deliveries.filter((delivery) => delivery.status === 'failed')
  const noun = total === 1 ? 'message' : 'messages'
  // Failures are only counted on the page shown, so they're named only when they're on it
  return failed.length > 0
    ? `${total} ${noun}, scheduled or sent straight away. ${failed.length} on this page failed.`
    : `${total} ${noun}, scheduled or sent straight away.`
}

function DeliveryLog({ deliveries }: { deliveries: Delivery[] }) {
  const days: { key: string; at: string; items: Delivery[] }[] = []
  for (const delivery of deliveries) {
    const at = delivery.sent_at ?? delivery.created_at
    const key = dayKey(at)
    const day = days.at(-1)
    if (day?.key === key) day.items.push(delivery)
    else days.push({ key, at, items: [delivery] })
  }

  return (
    <VStack gap={6}>
      {days.map((day) => (
        <List
          key={day.key}
          hasDividers
          header={
            <Heading level={2} xstyle={styles.day}>
              {formatDayHeading(day.at)}
            </Heading>
          }
        >
          {day.items.map((delivery) => (
            <DeliveryRow key={delivery.id} delivery={delivery} />
          ))}
        </List>
      ))}
    </VStack>
  )
}

function DeliveryRow({ delivery }: { delivery: Delivery }) {
  const name = useRecipientLabel()(delivery)
  const pictureID = useRecipientPictureID()(delivery)
  const isFailed = delivery.status === 'failed'
  return (
    <ListItem
      label={name}
      startContent={
        <HStack gap={3} vAlign="start" xstyle={styles.start}>
          <Text
            color="secondary"
            hasTabularNumbers
            textWrap="nowrap"
            xstyle={styles.time}
          >
            {formatClock(delivery.sent_at ?? delivery.created_at)}
          </Text>
          <HStack xstyle={styles.avatar}>
            <PersonAvatar
              name={name}
              pictureID={pictureID}
              size="sm"
              shape={delivery.recipient_type === 'group' ? 'rounded' : 'circle'}
            />
          </HStack>
        </HStack>
      }
      description={
        <VStack>
          <Text color="secondary" maxLines={1}>
            {delivery.body}
          </Text>
          {delivery.error ? (
            // Text's own colour outranks xstyle, so a failure's red comes from around it
            <VStack xstyle={isFailed ? styles.failure : undefined}>
              <Text
                type="supporting"
                color={isFailed ? 'inherit' : 'secondary'}
              >
                {delivery.error}
              </Text>
            </VStack>
          ) : null}
        </VStack>
      }
      endContent={<DeliveryStatus delivery={delivery} />}
    />
  )
}
