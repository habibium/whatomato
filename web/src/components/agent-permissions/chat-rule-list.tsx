import { Fragment, useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { Divider } from '@astryxdesign/core/Divider'
import { FieldStatus } from '@astryxdesign/core/FieldStatus'
import { Grid } from '@astryxdesign/core/Grid'
import { HStack } from '@astryxdesign/core/HStack'
import { IconButton } from '@astryxdesign/core/IconButton'
import { Pagination } from '@astryxdesign/core/Pagination'
import { Text } from '@astryxdesign/core/Text'
import { TextInput } from '@astryxdesign/core/TextInput'
import { VStack } from '@astryxdesign/core/VStack'
import { MessageSquareText, Search, X } from 'lucide-react'

import { DailyLimitField, NoteField, SendControl } from './rule-fields'
import { PersonAvatar } from '@/components/person-avatar'
import { WARNINGS, chatKey } from '@/lib/agent-grant'
import type { ChatRule, Rule } from '@/lib/agent-grant'
import { matchesContact, useContactBook } from '@/lib/contacts'
import { formatPhoneNumber } from '@/lib/whatsapp'

const PAGE_SIZE = 25
// Past this many, finding one chat by eye takes longer than typing its name
const FILTER_FROM = 8
const WIDE = '@media (min-width: 720px)'

const styles = stylex.create({
  // Who, may it send, the limit, then the row's two buttons; on a phone the controls wrap under the name
  row: {
    alignItems: 'center',
    gridTemplateAreas: {
      default: '"who actions" "send send" "limit limit"',
      [WIDE]: '"who send limit actions"',
    },
    gridTemplateColumns: {
      default: 'minmax(0, 1fr) auto',
      [WIDE]: 'minmax(0, 1fr) auto 10rem auto',
    },
  },
  who: { gridArea: 'who', minWidth: 0 },
  send: { gridArea: 'send' },
  limit: { gridArea: 'limit' },
  actions: { gridArea: 'actions' },
  text: { minWidth: 0, overflowWrap: 'anywhere' },
})

interface ChatRuleListProps {
  rules: ChatRule[]
  onChange: (key: string, change: Partial<Rule>) => void
  onRemove: (key: string) => void
}

/** One row per chat; filtered and paged so 500 stay usable. */
export function ChatRuleList({ rules, onChange, onRemove }: ChatRuleListProps) {
  const { byNumber } = useContactBook()
  const [filter, setFilter] = useState('')
  const [page, setPage] = useState(1)

  const nameOf = (rule: ChatRule) =>
    (rule.chat.type === 'contact' ? byNumber.get(rule.chat.id)?.name : null) ??
    rule.chat.name ??
    (rule.chat.type === 'contact'
      ? formatPhoneNumber(rule.chat.id)
      : 'Unnamed group')

  const query = filter.trim()
  const found = query
    ? rules.filter((rule) =>
        matchesContact(
          {
            id: rule.chat.id,
            name: nameOf(rule),
            first_name: null,
            username: null,
            phone_number: rule.chat.type === 'contact' ? rule.chat.id : null,
          },
          query,
        ),
      )
    : rules
  const pages = Math.max(1, Math.ceil(found.length / PAGE_SIZE))
  const current = Math.min(page, pages)
  const shown = found.slice((current - 1) * PAGE_SIZE, current * PAGE_SIZE)

  return (
    <VStack gap={3}>
      {rules.length > FILTER_FROM ? (
        <TextInput
          label="Find a chat in this list"
          isLabelHidden
          width="100%"
          startIcon={Search}
          hasClear
          placeholder={`Find among ${rules.length} chats`}
          autoComplete="off"
          value={filter}
          onChange={(value) => {
            setFilter(value)
            setPage(1)
          }}
        />
      ) : null}
      {shown.length === 0 && query ? (
        <Text color="secondary">No chat in this list matches “{query}”.</Text>
      ) : (
        <VStack as="ul" gap={3}>
          {shown.map((rule, index) => {
            const key = chatKey(rule.chat)
            return (
              <Fragment key={key}>
                {index > 0 ? <Divider /> : null}
                <ChatRuleRow
                  rule={rule}
                  name={nameOf(rule)}
                  pictureID={
                    rule.chat.type === 'group'
                      ? rule.chat.id
                      : (byNumber.get(rule.chat.id)?.id ?? null)
                  }
                  isSaved={
                    rule.chat.type === 'group' || byNumber.has(rule.chat.id)
                  }
                  onChange={(change) => onChange(key, change)}
                  onRemove={() => onRemove(key)}
                />
              </Fragment>
            )
          })}
        </VStack>
      )}
      {found.length > PAGE_SIZE ? (
        <HStack justify="end">
          <Pagination
            variant="count"
            size="sm"
            label="Chat pages"
            page={current}
            onChange={setPage}
            totalItems={found.length}
            pageSize={PAGE_SIZE}
          />
        </HStack>
      ) : null}
    </VStack>
  )
}

interface ChatRuleRowProps {
  rule: ChatRule
  name: string
  pictureID: string | null
  isSaved: boolean
  onChange: (change: Partial<Rule>) => void
  onRemove: () => void
}

function ChatRuleRow({
  rule,
  name,
  pictureID,
  isSaved,
  onChange,
  onRemove,
}: ChatRuleRowProps) {
  const [isNoteOpen, setIsNoteOpen] = useState(false)
  // Warned only where "Yes" was just chosen; a long list of allowed chats isn't a wall of warnings
  const [justAllowed, setJustAllowed] = useState(false)
  const isGroup = rule.chat.type === 'group'
  const detail = isGroup
    ? 'Group'
    : isSaved
      ? formatPhoneNumber(rule.chat.id)
      : rule.chat.name
        ? `${formatPhoneNumber(rule.chat.id)} · not in your contacts`
        : 'Not in your contacts'
  const hasNote = rule.note !== null || isNoteOpen

  return (
    <VStack as="li" gap={3}>
      <Grid columnGap={4} rowGap={3} xstyle={styles.row}>
        <HStack gap={3} vAlign="center" xstyle={styles.who}>
          <PersonAvatar
            name={name}
            pictureID={pictureID}
            size="md"
            shape={isGroup ? 'rounded' : 'circle'}
          />
          <VStack xstyle={styles.text}>
            <Text weight="medium" xstyle={styles.text}>
              {name}
            </Text>
            <Text type="supporting" hasTabularNumbers xstyle={styles.text}>
              {detail}
            </Text>
          </VStack>
        </HStack>
        <VStack xstyle={styles.send}>
          <SendControl
            label={`May it send to ${name}`}
            isLabelHidden
            size="sm"
            value={rule.send}
            onChange={(send) => {
              setJustAllowed(send === 'yes')
              onChange({ send })
            }}
          />
        </VStack>
        <VStack xstyle={styles.limit}>
          <DailyLimitField
            label={`Daily limit for ${name}`}
            isLabelHidden
            size="sm"
            width="100%"
            value={rule.daily_limit}
            isDisabled={rule.send === 'no'}
            onChange={(daily_limit) => onChange({ daily_limit })}
          />
        </VStack>
        <HStack gap={1} xstyle={styles.actions}>
          <IconButton
            label={
              hasNote ? `Note for ${name} is open` : `Add a note for ${name}`
            }
            tooltip={hasNote ? undefined : 'Add a note'}
            icon={<MessageSquareText />}
            variant="ghost"
            size="sm"
            isDisabled={hasNote}
            onClick={() => setIsNoteOpen(true)}
          />
          <IconButton
            label={`Remove ${name}`}
            tooltip="Remove"
            icon={<X />}
            variant="ghost"
            size="sm"
            onClick={onRemove}
          />
        </HStack>
      </Grid>
      {justAllowed && rule.send === 'yes' ? (
        <FieldStatus
          type="warning"
          variant="detached"
          message={WARNINGS.sendChat}
        />
      ) : null}
      {hasNote ? (
        <NoteField
          label={`Note for ${name}`}
          size="sm"
          hasAutoFocus={isNoteOpen && rule.note === null}
          value={rule.note}
          onChange={(note) => {
            setIsNoteOpen(true)
            onChange({ note })
          }}
        />
      ) : null}
    </VStack>
  )
}
