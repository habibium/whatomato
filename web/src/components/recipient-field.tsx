import { useId, useMemo, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Avatar } from '@astryxdesign/core/Avatar'
import { Button } from '@astryxdesign/core/Button'
import { Field } from '@astryxdesign/core/Field'
import { HStack } from '@astryxdesign/core/HStack'
import {
  SegmentedControl,
  SegmentedControlItem,
} from '@astryxdesign/core/SegmentedControl'
import { Selector } from '@astryxdesign/core/Selector'
import { Text } from '@astryxdesign/core/Text'
import { Typeahead, TypeaheadItem } from '@astryxdesign/core/Typeahead'
import type { SearchSource, SearchableItem } from '@astryxdesign/core/Typeahead'
import { VStack } from '@astryxdesign/core/VStack'
import type { z } from 'zod'

import { PersonAvatar } from '@/components/person-avatar'
import type { Contact } from '@/lib/contacts'
import { rankContacts, useContactBook } from '@/lib/contacts'
import type { RecipientType } from '@/lib/scheduler'
import {
  ApiError,
  NUMBER_HINT,
  digitsOf,
  formatPhoneNumber,
  groupsQuery,
  isInternational,
  isPhoneNumber,
} from '@/lib/whatsapp'

/** Past this many groups, scrolling for one takes longer than typing its name. */
const SEARCHABLE_FROM = 8

export interface Recipient {
  recipient_type: RecipientType
  recipient: string
  recipient_name: string | null
}

export function checkRecipient(
  { recipient_type, recipient }: Recipient,
  context: z.RefinementCtx,
) {
  const message =
    recipient_type === 'group'
      ? !recipient && 'Choose a group'
      : !recipient.trim()
        ? 'Choose a contact, or type a number with its country code'
        : /\p{L}/u.test(recipient)
          ? 'Choose a contact from the list, or type a number'
          : !isPhoneNumber(recipient) && NUMBER_HINT
  if (message) {
    context.addIssue({ code: 'custom', path: ['recipient'], message })
  }
}

interface RecipientFieldProps {
  value: Recipient
  onChange: (value: Recipient) => void
  status?: { type: 'error'; message?: string }
}

export function RecipientField({
  value,
  onChange,
  status,
}: RecipientFieldProps) {
  const id = useId()
  const previous = useRef<Recipient | null>(null)

  // Flipping back brings the other choice back: a saved group may have no list to pick it from
  const switchTo = (type: RecipientType) => {
    const restored = previous.current
    previous.current = value
    onChange(
      restored?.recipient_type === type
        ? restored
        : { recipient_type: type, recipient: '', recipient_name: null },
    )
  }

  return (
    <VStack gap={2}>
      <Field label="Send to" inputID={id} isGroupLabel>
        <SegmentedControl
          label="Send to"
          layout="fill"
          size="lg"
          value={value.recipient_type}
          onChange={(type) => switchTo(type as RecipientType)}
        >
          <SegmentedControlItem value="contact" label="Contact" />
          <SegmentedControlItem value="group" label="Group" />
        </SegmentedControl>
      </Field>
      {value.recipient_type === 'contact' ? (
        <ContactPicker value={value} onChange={onChange} status={status} />
      ) : (
        <GroupPicker value={value} onChange={onChange} status={status} />
      )}
    </VStack>
  )
}

type ContactItem = SearchableItem<{
  number: string | null
  contact: Contact | null
}>

function itemFor(contact: Contact): ContactItem {
  return {
    id: contact.id,
    label: contact.name,
    auxiliaryData: { number: contact.phone_number, contact },
  }
}

function numberItem(digits: string, label?: string | null): ContactItem {
  return {
    id: `number:${digits}`,
    label: label || formatPhoneNumber(digits),
    auxiliaryData: { number: digits, contact: null },
  }
}

/**
 * One field for both ways people remember someone: a name from the phone's contacts, or a number
 * typed in full. A typed number counts without being picked from the list.
 */
function ContactPicker({
  value,
  onChange,
  status,
}: Pick<RecipientFieldProps, 'value' | 'onChange' | 'status'>) {
  const { book, byNumber, isImporting } = useContactBook()
  const [isTyped, setIsTyped] = useState(false)
  const contacts = useMemo(() => book?.contacts ?? [], [book])

  const digits = digitsOf(value.recipient)
  const known = byNumber.get(digits)
  const chosen =
    !isTyped && digits
      ? known
        ? itemFor(known)
        : numberItem(digits, value.recipient_name)
      : null

  const source = useMemo<SearchSource<ContactItem>>(
    () => ({
      search: (query) => {
        const found = rankContacts(contacts, query)
        // A contact without a number can't be sent to; it's listed last so its absence isn't a mystery
        const items = [
          ...found.filter((contact) => contact.phone_number),
          ...found.filter((contact) => !contact.phone_number),
        ].map(itemFor)
        const typed = digitsOf(query)
        if (
          isPhoneNumber(query) &&
          isInternational(typed) &&
          !byNumber.has(typed)
        ) {
          items.unshift(numberItem(typed))
        }
        return items
      },
      bootstrap: () =>
        contacts.filter((contact) => contact.phone_number).map(itemFor),
    }),
    [contacts, byNumber],
  )

  // A name alone can't tell two Karims apart
  const hint = known
    ? isTyped
      ? `That’s ${known.name} in your contacts.`
      : `Sends to ${formatPhoneNumber(digits)}`
    : isImporting
      ? 'Your contacts are coming over from your phone. Type a number meanwhile.'
      : book && contacts.length === 0
        ? book.imported
          ? 'No saved contacts came over from your phone. Type a number with its country code.'
          : 'Link WhatsApp to search your contacts by name. Until then, type a number.'
        : null

  return (
    <VStack gap={2}>
      <Typeahead<ContactItem>
        label="Contact"
        isLabelHidden
        size="lg"
        width="100%"
        placeholder={
          contacts.length > 0
            ? 'Search contacts, or type a number'
            : '+44 7700 900123'
        }
        searchSource={source}
        value={chosen}
        hasEntriesOnFocus={contacts.length > 0}
        debounceMs={0}
        // The whole book scrolls in the menu; pictures load only for the rows in view
        maxMenuItems={Math.max(contacts.length + 1, 8)}
        emptySearchResultsText="No contact matches. Type the whole number to send to it."
        onChange={(item) => {
          const number = item?.auxiliaryData?.number
          if (item && !number) return
          setIsTyped(false)
          onChange({
            recipient_type: 'contact',
            recipient: number ? formatPhoneNumber(number) : '',
            recipient_name: item?.auxiliaryData?.contact?.name ?? null,
          })
        }}
        onChangeQuery={(query) => {
          // Editing a picked contact fires this too; leaving the field puts the contact back
          if (chosen) return
          setIsTyped(true)
          onChange({
            recipient_type: 'contact',
            recipient: query,
            recipient_name: null,
          })
        }}
        renderItem={(item) => {
          const number = item.auxiliaryData?.number
          const contact = item.auxiliaryData?.contact
          return (
            <TypeaheadItem
              item={item}
              icon={
                contact ? (
                  <PersonAvatar
                    name={contact.name}
                    pictureID={contact.id}
                    size="sm"
                  />
                ) : (
                  <Avatar size="sm" tooltip={false} />
                )
              }
              description={
                number
                  ? contact
                    ? formatPhoneNumber(number)
                    : 'Not in your contacts'
                  : 'WhatsApp hasn’t shared their number'
              }
              isDisabled={!number}
            />
          )
        }}
        status={status}
      />
      {hint ? <Text type="supporting">{hint}</Text> : null}
    </VStack>
  )
}

const NOT_CONNECTED =
  "WhatsApp isn't connected, so your groups can't load. Connect it on the WhatsApp page, or send to a phone number."
const NOT_CONNECTED_SAVED =
  "WhatsApp isn't connected, so this is the group you saved. Connect it on the WhatsApp page to choose another."

function GroupPicker({
  value,
  onChange,
  status,
}: Pick<RecipientFieldProps, 'value' | 'onChange' | 'status'>) {
  const groups = useQuery(groupsQuery)
  const isNotConnected =
    groups.error instanceof ApiError && groups.error.status === 409
  const list = groups.data ?? []
  const options = list.map((group) => ({
    value: group.id,
    label: group.name || 'Unnamed group',
  }))
  // A saved group may not be on the list: it can't load, or the number has left
  if (value.recipient && !list.some((group) => group.id === value.recipient)) {
    options.unshift({
      value: value.recipient,
      label: value.recipient_name || 'Unnamed group',
    })
  }

  const warning = isNotConnected
    ? value.recipient
      ? NOT_CONNECTED_SAVED
      : NOT_CONNECTED
    : groups.isError
      ? "Couldn't load your groups."
      : undefined

  return (
    <VStack gap={2} hAlign="start">
      <Selector
        label="Group"
        isLabelHidden
        size="lg"
        width="100%"
        placeholder="Choose a group"
        emptyText="This number isn't in any groups yet."
        options={options}
        value={value.recipient || undefined}
        onChange={(id) =>
          onChange({
            ...value,
            recipient: id,
            recipient_name: list.find((group) => group.id === id)?.name || null,
          })
        }
        isLoading={groups.isPending}
        isReadOnly={groups.isError}
        hasSearch={options.length > SEARCHABLE_FROM}
        renderOption={(option) => (
          <HStack gap={2} vAlign="center">
            <PersonAvatar
              name={option.label ?? option.value}
              pictureID={option.value}
              size="sm"
              shape="rounded"
            />
            <Text maxLines={1}>{option.label ?? option.value}</Text>
          </HStack>
        )}
        status={
          status ??
          (warning ? { type: 'warning', message: warning } : undefined)
        }
      />
      {groups.isError && !isNotConnected ? (
        <Button
          label="Try again"
          size="sm"
          clickAction={async () => {
            await groups.refetch()
          }}
        />
      ) : null}
    </VStack>
  )
}
