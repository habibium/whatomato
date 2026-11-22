import { useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { AlertDialog } from '@astryxdesign/core/AlertDialog'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { useAnnounce } from '@astryxdesign/core/hooks'
import { RadioList, RadioListItem } from '@astryxdesign/core/RadioList'
import { Heading, Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'

import { api } from '@/api/client'
import { readingQuery } from '@/lib/agents'
import { historyQuery, isImporting } from '@/lib/whatsapp'
import type { History, MessageHistory } from '@/lib/whatsapp'

const styles = stylex.create({
  measure: { maxWidth: '60ch' },
})

const CHOICES: ReadonlyArray<{
  value: History
  label: string
  description: string
}> = [
  {
    value: 'off',
    label: 'Only chats an agent may read',
    description:
      'The last 7 days of the chats you let an agent read. No other chat is stored.',
  },
  {
    value: { days: 30 },
    label: 'Every chat, the last 30 days',
    description: 'Includes what your phone sends when WhatsApp links.',
  },
  {
    value: { days: 90 },
    label: 'Every chat, the last 3 months',
    description: 'Includes what your phone sends when WhatsApp links.',
  },
  {
    value: { days: 365 },
    label: 'Every chat, the last year',
    description: 'Includes what your phone sends when WhatsApp links.',
  },
  {
    value: 'all',
    label: 'Every chat, everything',
    description:
      'As much as your phone sends when WhatsApp links, and every message after it.',
  },
]

function keyOf(history: History) {
  return typeof history === 'object' ? `days:${history.days}` : history
}

/** Days kept, for comparing two choices; Infinity keeps everything. */
function daysOf(history: History) {
  if (history === 'off') return 7
  if (history === 'all') return Infinity
  return history.days
}

const numbers = new Intl.NumberFormat()

interface MessageHistorySectionProps {
  isLinked: boolean
  /** Unlink and link again, so the phone sends its history. */
  onRelink: () => void
}

/**
 * What Whatomato keeps of the chats for agents to read, and how the phone's history is coming in.
 * Hidden on a server that doesn't let agents read.
 */
export function MessageHistorySection({
  isLinked,
  onRelink,
}: MessageHistorySectionProps) {
  const queryClient = useQueryClient()
  const announce = useAnnounce()
  const query = useQuery(historyQuery)
  const [pending, setPending] = useState<History | null>(null)
  const [isSaving, setIsSaving] = useState(false)
  const [failed, setFailed] = useState(false)

  const history = query.data
  if (!history?.available) return null

  const save = async (keep: History) => {
    setFailed(false)
    setIsSaving(true)
    try {
      const { data } = await api.PUT('/api/whatsapp/history', {
        body: { keep },
      })
      if (!data) throw new Error('not saved')
      queryClient.setQueryData(historyQuery.queryKey, data)
      // The permission editor describes what's kept
      await queryClient.invalidateQueries({ queryKey: readingQuery.queryKey })
      announce('Message history saved.')
    } catch {
      setFailed(true)
    } finally {
      setIsSaving(false)
      setPending(null)
    }
  }

  const choose = (key: string) => {
    const choice = choices(history.keep).find(
      (item) => keyOf(item.value) === key,
    )
    if (!choice || key === keyOf(history.keep)) return
    // Keeping less deletes what falls outside, so it's confirmed first
    if (daysOf(choice.value) < daysOf(history.keep)) {
      setPending(choice.value)
    } else {
      void save(choice.value)
    }
  }

  return (
    <VStack as="section" aria-labelledby="message-history" gap={4}>
      <VStack gap={2}>
        <Heading id="message-history" level={2}>
          Message history
        </Heading>
        <Text color="secondary" xstyle={styles.measure}>
          What Whatomato keeps, encrypted, for the agents you let read your
          chats. An agent still reads only the chats you allow on its page, and
          no further back than you choose there.
        </Text>
      </VStack>
      <RadioList
        label="What Whatomato keeps"
        isLabelHidden
        value={keyOf(history.keep)}
        isDisabled={isSaving}
        onChange={choose}
      >
        {choices(history.keep).map((choice) => (
          <RadioListItem
            key={keyOf(choice.value)}
            value={keyOf(choice.value)}
            label={choice.label}
            description={choice.description}
          />
        ))}
      </RadioList>
      {failed ? (
        <Banner
          status="error"
          title="Couldn't save your message history."
          description="Something went wrong on our side. Try again in a moment."
        />
      ) : null}
      <ImportStatus history={history} isLinked={isLinked} onRelink={onRelink} />
      <AlertDialog
        isOpen={pending !== null}
        onOpenChange={(isOpen) => {
          if (!isOpen) setPending(null)
        }}
        title="Delete older messages?"
        description={
          pending === 'off'
            ? 'Whatomato deletes every chat no agent may read, and anything older than 7 days, within a few minutes. Bringing them back means linking WhatsApp again.'
            : 'Whatomato deletes what falls outside the new length within a few minutes. Bringing it back means linking WhatsApp again.'
        }
        actionLabel="Delete"
        isActionLoading={isSaving}
        onAction={async () => {
          if (pending !== null) await save(pending)
        }}
      />
    </VStack>
  )
}

/** The usual choices, plus the saved one when it was set some other way. */
function choices(current: History) {
  if (CHOICES.some((choice) => keyOf(choice.value) === keyOf(current))) {
    return CHOICES
  }
  const days = typeof current === 'object' ? current.days : 0
  return [
    ...CHOICES,
    {
      value: current,
      label: `Every chat, the last ${days} days`,
      description: 'Includes what your phone sends when WhatsApp links.',
    },
  ]
}

function ImportStatus({
  history,
  isLinked,
  onRelink,
}: {
  history: MessageHistory
  isLinked: boolean
  onRelink: () => void
}) {
  if (history.keep === 'off') return null
  const imported = history.import
  if (!isLinked || !imported) {
    return (
      <Text type="supporting" xstyle={styles.measure}>
        When you link WhatsApp, your phone sends its older messages too.
      </Text>
    )
  }
  if (isImporting(history)) {
    const progress =
      imported.progress === null ? '' : ` ${imported.progress}% sent so far.`
    return (
      <Text type="supporting" aria-live="polite" xstyle={styles.measure}>
        Bringing in older messages from your phone.{progress}{' '}
        {numbers.format(imported.imported)} kept.
      </Text>
    )
  }
  if (imported.progress === null && imported.imported === 0) {
    return (
      <VStack gap={3} hAlign="start">
        <Text type="supporting" xstyle={styles.measure}>
          Your phone sends older messages only while WhatsApp links. Link it
          again to bring them in; new messages are kept from now on either way.
        </Text>
        <Button label="Link WhatsApp again" clickAction={onRelink} />
      </VStack>
    )
  }
  return (
    <Text type="supporting" xstyle={styles.measure}>
      Brought in {numbers.format(imported.imported)} older messages from your
      phone.
    </Text>
  )
}
