import * as stylex from '@stylexjs/stylex'
import { Avatar } from '@astryxdesign/core/Avatar'
import { HStack } from '@astryxdesign/core/HStack'
import { Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'

import { recipientLabel } from '@/lib/scheduler'
import type { RecipientType } from '@/lib/scheduler'
import { formatPhoneNumber } from '@/lib/whatsapp'

const styles = stylex.create({
  text: { minWidth: 0 },
})

interface RecipientCellProps {
  recipient_type: RecipientType
  recipient: string
  recipient_name?: string | null
  /** Greyed, for a message that's switched off. */
  isMuted?: boolean
}

/** Initials, the name, and under it the number or "Group", so two contacts with one name stay apart. */
export function RecipientCell({
  recipient_type,
  recipient,
  recipient_name,
  isMuted = false,
}: RecipientCellProps) {
  const name = recipientLabel({
    recipient_type,
    recipient,
    recipient_name: recipient_name ?? null,
  })
  const isGroup = recipient_type === 'group'
  const detail = isGroup
    ? 'Group'
    : recipient_name
      ? formatPhoneNumber(recipient)
      : 'Contact'

  return (
    <HStack gap={3} vAlign="center" xstyle={styles.text}>
      <Avatar
        name={name}
        size="md"
        shape={isGroup ? 'rounded' : 'circle'}
        tooltip={false}
      />
      <VStack xstyle={styles.text}>
        <Text
          weight="medium"
          color={isMuted ? 'secondary' : 'primary'}
          maxLines={1}
          hasTabularNumbers
        >
          {name}
        </Text>
        <Text type="supporting" maxLines={1} hasTabularNumbers>
          {detail}
        </Text>
      </VStack>
    </HStack>
  )
}
