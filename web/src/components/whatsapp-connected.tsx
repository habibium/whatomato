import { useEffect, useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQuery } from '@tanstack/react-query'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { useAnnounce } from '@astryxdesign/core/hooks'
import { Icon } from '@astryxdesign/core/Icon'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { Heading, Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'
import {
  borderVars,
  colorVars,
  spacingVars,
} from '@astryxdesign/core/theme/tokens.stylex'

import { api } from '@/api/client'
import { ApiError, groupsQuery } from '@/lib/whatsapp'

/** Enough to show the list is real without pushing the page below the fold. */
const PREVIEW_COUNT = 8

const styles = stylex.create({
  measure: { maxWidth: '45ch' },
  row: {
    borderTopColor: colorVars['--color-border'],
    borderTopStyle: 'solid',
    borderTopWidth: borderVars['--border-width'],
    paddingBlock: spacingVars['--spacing-2'],
  },
})

interface ConnectedSectionProps {
  /** A 409: the link dropped since the last status, so it needs reading again. */
  onStale: () => void
}

export function TestMessageSection({ onStale }: ConnectedSectionProps) {
  const [result, setResult] = useState<'sent' | 'failed' | null>(null)
  const announce = useAnnounce()

  const send = async () => {
    setResult(null)
    try {
      const { response } = await api.POST('/api/whatsapp/test-message')
      if (response.ok) {
        setResult('sent')
        announce('Test message sent to your Message yourself chat.')
      } else if (response.status === 409) {
        onStale()
      } else {
        setResult('failed')
      }
    } catch {
      setResult('failed')
    }
  }

  return (
    <VStack as="section" aria-labelledby="test-message" gap={4}>
      <VStack gap={2}>
        <Heading id="test-message" level={2}>
          Test message
        </Heading>
        <Text color="secondary" xstyle={styles.measure}>
          Send yourself a short message to check that Whatomato can send from
          your number.
        </Text>
      </VStack>
      <HStack gap={4} vAlign="center" wrap="wrap">
        <Button label="Send a test message" clickAction={send} />
        {result === 'sent' ? (
          <HStack gap={1.5} vAlign="center">
            <Icon icon="check" color="success" size="sm" />
            <Text color="secondary">Sent to your “Message yourself” chat.</Text>
          </HStack>
        ) : null}
      </HStack>
      {result === 'failed' ? (
        <Banner
          status="error"
          title="Couldn't send the test message."
          description="Something went wrong on our side. Try again in a moment."
        />
      ) : null}
    </VStack>
  )
}

export function GroupsSection({ onStale }: ConnectedSectionProps) {
  const groups = useQuery(groupsQuery)
  const [isExpanded, setIsExpanded] = useState(false)
  const isStale =
    groups.error instanceof ApiError && groups.error.status === 409

  useEffect(() => {
    if (isStale) onStale()
  }, [isStale, onStale])

  const list = groups.data ?? []
  const shown = isExpanded ? list : list.slice(0, PREVIEW_COUNT)

  return (
    <VStack as="section" aria-labelledby="groups" gap={4}>
      <VStack gap={2}>
        <Heading id="groups" level={2}>
          Groups
        </Heading>
        <Text color="secondary" xstyle={styles.measure}>
          {groups.data === undefined
            ? 'Scheduled messages can go to any group this number is in.'
            : list.length === 0
              ? 'This number isn’t in any groups yet. Once it is, scheduled messages can go to them.'
              : `This number is in ${list.length} ${list.length === 1 ? 'group' : 'groups'}. Scheduled messages can go to any of them.`}
        </Text>
      </VStack>

      {groups.isPending ? (
        <VStack gap={3} aria-hidden="true">
          {[0, 1, 2].map((index) => (
            <Skeleton key={index} index={index} width="40%" height={20} />
          ))}
        </VStack>
      ) : groups.isError ? (
        isStale ? null : (
          <Banner
            status="error"
            title="Couldn't load your groups."
            description="Something went wrong on our side."
            endContent={
              <Button
                label="Try again"
                size="sm"
                clickAction={async () => {
                  await groups.refetch()
                }}
              />
            }
          />
        )
      ) : list.length > 0 ? (
        <VStack gap={4} hAlign="start">
          <VStack as="ul" width="100%">
            {shown.map((group) => (
              <VStack as="li" key={group.id} xstyle={styles.row}>
                <Text maxLines={1}>{group.name || 'Unnamed group'}</Text>
              </VStack>
            ))}
          </VStack>
          {list.length > PREVIEW_COUNT ? (
            <Button
              label={isExpanded ? 'Show fewer' : `Show all ${list.length}`}
              size="sm"
              onClick={() => setIsExpanded((expanded) => !expanded)}
            />
          ) : null}
        </VStack>
      ) : null}
    </VStack>
  )
}
