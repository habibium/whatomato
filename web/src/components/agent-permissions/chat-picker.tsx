import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Avatar } from '@astryxdesign/core/Avatar'
import { Typeahead, TypeaheadItem } from '@astryxdesign/core/Typeahead'
import type { SearchSource, SearchableItem } from '@astryxdesign/core/Typeahead'
import { Search } from 'lucide-react'

import { PersonAvatar } from '@/components/person-avatar'
import { chatKey } from '@/lib/agent-grant'
import type { Chat } from '@/lib/agent-grant'
import { matchesContact, rankContacts, useContactBook } from '@/lib/contacts'
import {
  digitsOf,
  formatPhoneNumber,
  groupsQuery,
  isInternational,
  isPhoneNumber,
} from '@/lib/whatsapp'

type ChatItem = SearchableItem<{
  chat: Chat
  detail: string
  pictureID: string | null
}>

function toItem(
  chat: Chat,
  detail: string,
  pictureID: string | null,
): ChatItem {
  return {
    id: chatKey(chat),
    label: chat.name ?? formatPhoneNumber(chat.id),
    auxiliaryData: { chat, detail, pictureID },
  }
}

interface ChatPickerProps {
  /** Keys of the chats already listed, which the picker leaves out. */
  listed: ReadonlySet<string>
  onAdd: (chat: Chat) => void
  isDisabled?: boolean
  disabledMessage?: string
}

/** Contacts and groups by name, or a number typed in full for someone who isn't saved. */
export function ChatPicker({
  listed,
  onAdd,
  isDisabled,
  disabledMessage,
}: ChatPickerProps) {
  const { book, byNumber } = useContactBook()
  const groups = useQuery(groupsQuery)
  // Bumped after each pick, so the field empties for the next one
  const [round, setRound] = useState(0)

  const source = useMemo<SearchSource<ChatItem>>(() => {
    const contactItems = (query: string) =>
      (query
        ? rankContacts(book?.contacts ?? [], query)
        : (book?.contacts ?? [])
      )
        .filter((contact) => contact.phone_number)
        .map((contact) =>
          toItem(
            {
              type: 'contact',
              id: contact.phone_number ?? '',
              name: contact.name,
            },
            formatPhoneNumber(contact.phone_number ?? ''),
            contact.id,
          ),
        )
    const groupItems = (query: string) =>
      (groups.data ?? [])
        .filter((group) =>
          matchesContact(
            {
              id: group.id,
              name: group.name,
              first_name: null,
              username: null,
              phone_number: null,
            },
            query,
          ),
        )
        .map((group) =>
          toItem(
            { type: 'group', id: group.id, name: group.name || null },
            'Group',
            group.id,
          ),
        )
    const unlisted = (items: ChatItem[]) =>
      items.filter((item) => !listed.has(item.id))

    return {
      search: (query) => {
        const items = [...contactItems(query), ...groupItems(query)]
        const typed = digitsOf(query)
        if (
          isPhoneNumber(query) &&
          isInternational(typed) &&
          !byNumber.has(typed)
        ) {
          items.unshift(
            toItem(
              { type: 'contact', id: typed, name: null },
              'Not in your contacts',
              null,
            ),
          )
        }
        return unlisted(items)
      },
      bootstrap: () => unlisted([...contactItems(''), ...groupItems('')]),
    }
  }, [book, groups.data, byNumber, listed])

  const hasSaved = (book?.contacts.length ?? 0) + (groups.data?.length ?? 0) > 0

  return (
    <Typeahead<ChatItem>
      key={round}
      label="Add a chat"
      isLabelHidden
      size="lg"
      width="100%"
      startIcon={Search}
      placeholder={
        hasSaved
          ? 'Search contacts and groups, or type a number'
          : 'Type a number with its country code'
      }
      searchSource={source}
      value={null}
      hasClear={false}
      hasAutoFocus={round > 0}
      hasEntriesOnFocus={hasSaved}
      debounceMs={0}
      maxMenuItems={50}
      emptySearchResultsText="Nothing matches. Type the whole number, with its country code, to add it."
      isDisabled={isDisabled}
      disabledMessage={disabledMessage}
      onChange={(item) => {
        const chat = item?.auxiliaryData?.chat
        if (!chat) return
        onAdd(chat)
        setRound((value) => value + 1)
      }}
      renderItem={(item) => {
        const data = item.auxiliaryData
        return (
          <TypeaheadItem
            item={item}
            icon={
              data?.pictureID ? (
                <PersonAvatar
                  name={item.label}
                  pictureID={data.pictureID}
                  size="sm"
                  shape={data.chat.type === 'group' ? 'rounded' : 'circle'}
                />
              ) : (
                <Avatar size="sm" tooltip={false} />
              )
            }
            description={data?.detail}
          />
        )
      }}
    />
  )
}
