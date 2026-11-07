import { useMemo, useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQuery } from '@tanstack/react-query'
import { Outlet, createFileRoute, useParams } from '@tanstack/react-router'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { Grid } from '@astryxdesign/core/Grid'
import { List, ListItem } from '@astryxdesign/core/List'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { Spinner } from '@astryxdesign/core/Spinner'
import { Heading, Text } from '@astryxdesign/core/Text'
import { TextInput } from '@astryxdesign/core/TextInput'
import { VStack } from '@astryxdesign/core/VStack'
import { HStack } from '@astryxdesign/core/HStack'
import {
  spacingVars,
  typeScaleVars,
} from '@astryxdesign/core/theme/tokens.stylex'
import { Search } from 'lucide-react'

import { AppFrame } from '@/components/app-frame'
import { ListEmpty } from '@/components/list-empty'
import { OfflineBanner } from '@/components/offline-banner'
import { PageHeading } from '@/components/page-heading'
import { PersonAvatar } from '@/components/person-avatar'
import { Panel } from '@/components/panel'
import type { Contact, ContactBook } from '@/lib/contacts'
import { contactHref, rankContacts, useContactBook } from '@/lib/contacts'
import { messagesQuery } from '@/lib/scheduler'
import { useSessionExpiry } from '@/lib/use-session-expiry'
import { formatPhoneNumber, statusQuery } from '@/lib/whatsapp'

export const Route = createFileRoute('/_authenticated/contacts')({
  component: ContactsPage,
})

const WIDE = '@media (min-width: 1024px)'

const styles = stylex.create({
  columns: {
    alignItems: 'start',
    gridTemplateColumns: {
      default: 'minmax(0, 1fr)',
      [WIDE]: 'minmax(0, 22rem) minmax(0, 1fr)',
    },
  },
  // Beside the details the list scrolls on its own, so a long book never pushes them off screen
  list: {
    maxHeight: {
      default: 'none',
      [WIDE]: `calc(100dvh - 2 * ${spacingVars['--spacing-8']})`,
    },
    overflowY: { default: 'visible', [WIDE]: 'auto' },
    position: { default: 'static', [WIDE]: 'sticky' },
    top: spacingVars['--spacing-8'],
  },
  letter: {
    fontSize: typeScaleVars['--text-heading-3-size'],
    lineHeight: typeScaleVars['--text-heading-3-leading'],
  },
  narrowHidden: { display: { default: 'none', [WIDE]: 'flex' } },
})

function ContactsPage() {
  const { query, book, isImporting } = useContactBook()
  const isSignedOut = useSessionExpiry(query.error, '/contacts')
  const { contactId } = useParams({ strict: false })

  return (
    <AppFrame>
      <VStack gap={6}>
        {query.isRefetchError && !isSignedOut ? <OfflineBanner /> : null}
        <PageHeading title="Contacts" lede={<Summary book={book} />} />

        {book ? (
          book.contacts.length > 0 ? (
            <Grid columnGap={8} rowGap={6} xstyle={styles.columns}>
              {/* On a phone the list and one contact take turns */}
              <VStack
                gap={4}
                xstyle={[styles.list, contactId ? styles.narrowHidden : null]}
              >
                <ContactList
                  contacts={book.contacts}
                  selectedID={contactId ?? null}
                />
              </VStack>
              <VStack xstyle={contactId ? null : styles.narrowHidden}>
                <Outlet />
              </VStack>
            </Grid>
          ) : isImporting ? (
            <Importing />
          ) : (
            <NoContacts book={book} />
          )
        ) : query.isPending ? (
          <VStack gap={2} aria-busy="true">
            {[0, 1, 2, 3, 4].map((index) => (
              <Skeleton key={index} index={index} width="100%" height={48} />
            ))}
          </VStack>
        ) : isSignedOut ? null : (
          <Banner
            status="error"
            title="Couldn't load your contacts."
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

function Summary({ book }: { book: ContactBook | undefined }) {
  if (!book || book.contacts.length === 0) {
    return 'The people saved on your phone, so a message goes to a name.'
  }
  const count = book.contacts.length
  return `${count} ${count === 1 ? 'person' : 'people'} saved on your phone, kept up to date while WhatsApp is linked.`
}

function Importing() {
  return (
    <Panel gap={3}>
      <HStack gap={3} vAlign="center">
        <Spinner size="sm" label="Bringing your contacts over" />
        <Heading level={2}>Bringing your contacts over</Heading>
      </HStack>
      <Text color="secondary">
        Your phone is sending the names it has saved. They show here as they
        arrive.
      </Text>
    </Panel>
  )
}

function NoContacts({ book }: { book: ContactBook }) {
  const status = useQuery(statusQuery).data?.status
  const isLinked = status !== undefined && 'phone_number' in status
  if (!isLinked) {
    return (
      <ListEmpty
        title="Link WhatsApp to see your contacts"
        description="Whatomato keeps a copy of the names saved on your phone, so you can schedule a message to a person instead of typing their number. Unlinking deletes the copy."
        actions={
          <Button label="Link WhatsApp" variant="primary" href="/whatsapp" />
        }
      />
    )
  }
  if (!book.imported) {
    return (
      <ListEmpty
        title="Your contacts come over once WhatsApp reconnects"
        description="The phone shares its contacts while the link is running. Reconnect WhatsApp and they’ll show here."
        actions={<Button label="Go to WhatsApp" href="/whatsapp" />}
      />
    )
  }
  return (
    <ListEmpty
      title="No contacts came over"
      description="Your phone shares the contacts saved in its address book. Ones you save from now on show here as the phone shares them."
      actions={<Button label="Go to messages" href="/messages" />}
    />
  )
}

interface ContactListProps {
  contacts: Contact[]
  selectedID: string | null
}

function ContactList({ contacts, selectedID }: ContactListProps) {
  const [search, setSearch] = useState('')
  const messages = useQuery(messagesQuery)
  const scheduled = useMemo(() => {
    const counts = new Map<string, number>()
    for (const message of messages.data ?? []) {
      if (message.recipient_type === 'contact' && message.next_run_at) {
        counts.set(message.recipient, (counts.get(message.recipient) ?? 0) + 1)
      }
    }
    return counts
  }, [messages.data])

  const query = search.trim()
  const found = query ? rankContacts(contacts, query) : contacts
  const sections = query
    ? [{ key: 'found', heading: null, contacts: found }]
    : byLetter(contacts)

  const row = (contact: Contact) => {
    const count = contact.phone_number
      ? (scheduled.get(contact.phone_number) ?? 0)
      : 0
    return (
      <ListItem
        key={contact.id}
        label={contact.name}
        description={
          contact.phone_number
            ? formatPhoneNumber(contact.phone_number)
            : 'Number not shared'
        }
        startContent={
          <PersonAvatar name={contact.name} pictureID={contact.id} size="md" />
        }
        endContent={
          count > 0 ? (
            <Text type="supporting" hasTabularNumbers>
              {count} scheduled
            </Text>
          ) : undefined
        }
        href={contactHref(contact)}
        isSelected={contact.id === selectedID}
      />
    )
  }

  return (
    <>
      <TextInput
        label="Search contacts"
        isLabelHidden
        size="lg"
        width="100%"
        startIcon={Search}
        hasClear
        placeholder="Search by name or number"
        autoComplete="off"
        value={search}
        onChange={(value) => setSearch(value)}
      />
      {found.length === 0 ? (
        <Text color="secondary">No one saved matches “{query}”.</Text>
      ) : (
        <VStack gap={5}>
          {sections.map((section) => (
            <List
              key={section.key}
              density="compact"
              header={
                section.heading ? (
                  <Heading level={2} xstyle={styles.letter}>
                    {section.heading}
                  </Heading>
                ) : undefined
              }
            >
              {section.contacts.map(row)}
            </List>
          ))}
        </VStack>
      )}
    </>
  )
}

/** A–Z by the first letter of the name, accents folded; anything else goes under "#". */
function byLetter(contacts: Contact[]) {
  const sections: { key: string; heading: string; contacts: Contact[] }[] = []
  for (const contact of contacts) {
    const first = contact.name
      .normalize('NFD')
      .replace(/\p{M}/gu, '')
      .charAt(0)
      .toLocaleUpperCase()
    const heading = /\p{L}/u.test(first) ? first : '#'
    const section = sections.find((item) => item.heading === heading)
    if (section) section.contacts.push(contact)
    else sections.push({ key: heading, heading, contacts: [contact] })
  }
  return sections
}
