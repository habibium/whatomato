import * as stylex from '@stylexjs/stylex'
import { useQuery } from '@tanstack/react-query'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { StatusDot } from '@astryxdesign/core/StatusDot'
import { Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'
import {
  colorVars,
  radiusVars,
  spacingVars,
} from '@astryxdesign/core/theme/tokens.stylex'

import { statusQuery } from '@/lib/whatsapp'
import type { WhatsAppStatus } from '@/lib/whatsapp'

const LOST = 'Any that come due before then fail, and don’t go out later.'

const styles = stylex.create({
  // A quiet row: the dot carries the status, so the notice never outshouts the page's own data
  notice: {
    borderColor: colorVars['--color-border'],
    borderRadius: radiusVars['--radius-container'],
    borderStyle: 'solid',
    borderWidth: 1,
    paddingBlock: spacingVars['--spacing-3'],
    paddingInline: spacingVars['--spacing-4'],
  },
  dot: { paddingTop: spacingVars['--spacing-1'] },
  text: { flexBasis: '20rem', flexGrow: 1, minWidth: 0 },
})

function describe(status: WhatsAppStatus) {
  switch (status.status) {
    case 'connected':
    case 'syncing':
      return null
    case 'not_linked':
      return {
        dot: 'warning',
        title: 'WhatsApp isn’t linked.',
        description: `Scheduled messages won’t send until you link it. ${LOST}`,
        action: 'Link WhatsApp',
      } as const
    case 'pairing':
      return {
        dot: 'warning',
        title: 'WhatsApp isn’t linked yet.',
        description: `Scheduled messages won’t send until you finish linking it. ${LOST}`,
        action: 'Finish linking',
      } as const
    case 'paused':
      return {
        dot: 'warning',
        title: 'WhatsApp is paused.',
        description: `Scheduled messages won’t send until you reconnect it. ${LOST}`,
        action: 'Reconnect',
      } as const
    case 'reconnecting':
      return {
        dot: 'accent',
        title: 'Reconnecting to WhatsApp.',
        description:
          'Scheduled messages wait for the connection, and keep trying for about half an hour. There’s nothing you need to do.',
        action: null,
      } as const
    case 'stopped':
      return {
        dot: 'error',
        title: 'WhatsApp has stopped.',
        description: `Scheduled messages won’t send until you reconnect it. ${LOST}`,
        action: 'Open WhatsApp',
      } as const
  }
}

export function WhatsAppBanner() {
  const { data } = useQuery(statusQuery)
  const copy = data ? describe(data.status) : null
  if (!copy) return null

  return (
    <HStack
      role="status"
      gap={3}
      vAlign="center"
      wrap="wrap"
      xstyle={styles.notice}
    >
      <HStack gap={3} vAlign="start" xstyle={styles.text}>
        <HStack xstyle={styles.dot}>
          <StatusDot
            variant={copy.dot}
            label={copy.title}
            isPulsing={copy.dot === 'accent'}
          />
        </HStack>
        <VStack gap={0.5}>
          <Text weight="semibold">{copy.title}</Text>
          <Text color="secondary" textWrap="pretty">
            {copy.description}
          </Text>
        </VStack>
      </HStack>
      {copy.action ? (
        <Button label={copy.action} size="sm" href="/whatsapp" />
      ) : null}
    </HStack>
  )
}
