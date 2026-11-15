import * as stylex from '@stylexjs/stylex'
import { HStack } from '@astryxdesign/core/HStack'
import { List, ListItem } from '@astryxdesign/core/List'
import { StatusDot } from '@astryxdesign/core/StatusDot'
import { Heading, Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'
import {
  spacingVars,
  typeScaleVars,
} from '@astryxdesign/core/theme/tokens.stylex'

import { OUTCOME_DOTS, describeTool } from '@/lib/agent-activity'
import type { AgentActivity } from '@/lib/agent-activity'
import { useRecipientLabel } from '@/lib/contacts'
import { dayKey, formatClock, formatDayHeading } from '@/lib/format'

const styles = stylex.create({
  // History's time slot, so the two logs line up the same way
  time: {
    flexShrink: 0,
    width: { default: '4.25rem', '@media (min-width: 640px)': '4.75rem' },
  },
  start: { paddingTop: spacingVars['--spacing-0-5'] },
  day: {
    fontSize: typeScaleVars['--text-heading-3-size'],
    lineHeight: typeScaleVars['--text-heading-3-leading'],
  },
  status: { flexShrink: 0, whiteSpace: 'nowrap' },
})

interface ActivityLogProps {
  items: AgentActivity[]
  /** Off on one connection's page, where every row is the same one. */
  showsConnection?: boolean
}

/** Every call agents made, newest first, under a heading per day. */
export function ActivityLog({
  items,
  showsConnection = true,
}: ActivityLogProps) {
  const days: { key: string; at: string; items: AgentActivity[] }[] = []
  for (const item of items) {
    const key = dayKey(item.created_at)
    const day = days.at(-1)
    if (day?.key === key) day.items.push(item)
    else days.push({ key, at: item.created_at, items: [item] })
  }

  return (
    <VStack gap={6}>
      {days.map((day) => (
        <List
          key={day.key}
          hasDividers
          header={
            <Heading level={3} xstyle={styles.day}>
              {formatDayHeading(day.at)}
            </Heading>
          }
        >
          {day.items.map((item) => (
            <ActivityRow
              key={item.id}
              item={item}
              showsConnection={showsConnection}
            />
          ))}
        </List>
      ))}
    </VStack>
  )
}

function ActivityRow({
  item,
  showsConnection,
}: {
  item: AgentActivity
  showsConnection: boolean
}) {
  const recipientLabel = useRecipientLabel()
  const chat =
    item.recipient_type && item.recipient
      ? recipientLabel({
          recipient_type: item.recipient_type,
          recipient: item.recipient,
          recipient_name: item.recipient_name,
        })
      : null
  const dot = OUTCOME_DOTS[item.outcome]
  const detail = [
    showsConnection
      ? `${item.connection_name}${item.connection_id ? '' : ' (revoked)'}`
      : null,
    chat ? `${item.tool === 'find_chats' ? '' : 'To '}${chat}` : null,
  ]
    .filter(Boolean)
    .join(' · ')

  return (
    <ListItem
      label={describeTool(item.tool, item.outcome)}
      startContent={
        <HStack xstyle={styles.start}>
          <Text
            color="secondary"
            hasTabularNumbers
            textWrap="nowrap"
            xstyle={styles.time}
          >
            {formatClock(item.created_at)}
          </Text>
        </HStack>
      }
      description={
        detail || item.reason_text ? (
          <VStack>
            {detail ? <Text color="secondary">{detail}</Text> : null}
            {item.reason_text ? (
              <Text type="supporting">{item.reason_text}</Text>
            ) : null}
          </VStack>
        ) : undefined
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
