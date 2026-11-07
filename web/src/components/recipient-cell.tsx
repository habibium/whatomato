import * as stylex from '@stylexjs/stylex'
import { HStack } from '@astryxdesign/core/HStack'
import { Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'

import { PersonAvatar } from '@/components/person-avatar'
import {
  useContactBook,
  useRecipientLabel,
  useRecipientPictureID,
} from '@/lib/contacts'
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
  const { book, byNumber } = useContactBook()
  const name = useRecipientLabel()({
    recipient_type,
    recipient,
    recipient_name: recipient_name ?? null,
  })
  const pictureID = useRecipientPictureID()({ recipient_type, recipient })
  const isGroup = recipient_type === 'group'
  const isNamed = byNumber.has(recipient) || Boolean(recipient_name)
  const detail = isGroup
    ? 'Group'
    : isNamed
      ? formatPhoneNumber(recipient)
      : book?.imported
        ? 'Not in your contacts'
        : 'Phone number'

  return (
    <HStack gap={3} vAlign="center" xstyle={styles.text}>
      <PersonAvatar
        name={name}
        pictureID={pictureID}
        size="md"
        shape={isGroup ? 'rounded' : 'circle'}
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
