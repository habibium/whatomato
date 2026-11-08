import { useEffect, useRef, useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { Link } from '@astryxdesign/core/Link'
import { List, ListItem } from '@astryxdesign/core/List'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { Heading, Text } from '@astryxdesign/core/Text'
import { useToast } from '@astryxdesign/core/Toast'
import { VStack } from '@astryxdesign/core/VStack'
import { ArrowLeft } from 'lucide-react'

import { DeliveryStatus } from '@/components/delivery-status'
import { MessageFormDialog } from '@/components/message-form-dialog'
import { Panel } from '@/components/panel'
import { PersonAvatar } from '@/components/person-avatar'
import { SendNowDialog } from '@/components/send-now-dialog'
import type { Contact } from '@/lib/contacts'
import { useContactBook } from '@/lib/contacts'
import { formatWhen, summariseRecurrence, zoneLabel } from '@/lib/format'
import { messagesQuery, recipientDeliveriesQuery } from '@/lib/scheduler'
import type { ScheduledMessage } from '@/lib/scheduler'
import { formatPhoneNumber } from '@/lib/whatsapp'

export const Route = createFileRoute('/_authenticated/contacts/$contactId')({
  component: ContactPage,
})

/** Enough to see whether messages to someone go out; the full log is History's. */
const RECENT = 5

const styles = stylex.create({
  name: { minWidth: 0, overflowWrap: 'anywhere' },
  identity: { minWidth: 0 },
  // A landing spot for focus when another contact opens; it isn't a control
  focusTarget: { outline: 'none' },
  back: { display: { default: 'flex', '@media (min-width: 1024px)': 'none' } },
})

type OpenDialog =
  | { kind: 'write'; message: ScheduledMessage | null }
  | { kind: 'send' }

function ContactPage() {
  const { contactId } = Route.useParams()
  const { book } = useContactBook()
  if (!book) {
    return <Skeleton width="100%" height={240} />
  }
  const contact = book.contacts.find((item) => item.id === contactId)
  return contact ? (
    <ContactDetails key={contact.id} contact={contact} />
  ) : (
    <Panel gap={3}>
      <Heading level={2}>This contact isn’t saved any more</Heading>
      <Text color="secondary">
        It was deleted on your phone, or the link to WhatsApp changed.
      </Text>
      <HStack>
        <Button label="All contacts" href="/contacts" />
      </HStack>
    </Panel>
  )
}

function ContactDetails({ contact }: { contact: Contact }) {
  const toast = useToast()
  const headingRef = useRef<HTMLHeadingElement>(null)
  const [dialog, setDialog] = useState<OpenDialog | null>(null)
  const number = contact.phone_number
  const recipient = number
    ? {
        recipient_type: 'contact' as const,
        recipient: number,
        recipient_name: contact.name,
      }
    : null
  const closeDialog = (isOpen: boolean) => {
    if (!isOpen) setDialog(null)
  }

  // Choosing someone in the list moves focus here, so a screen reader hears who opened
  useEffect(() => {
    headingRef.current?.focus({ preventScroll: true })
  }, [])

  return (
    <Panel gap={6}>
      <HStack xstyle={styles.back}>
        <Link href="/contacts" hasUnderline>
          <HStack gap={1} vAlign="center">
            <ArrowLeft size={16} aria-hidden="true" />
            All contacts
          </HStack>
        </Link>
      </HStack>

      <HStack gap={4} vAlign="center">
        <PersonAvatar name={contact.name} pictureID={contact.id} size="lg" />
        <VStack gap={0.5} xstyle={styles.identity}>
          <Heading
            level={2}
            ref={headingRef}
            tabIndex={-1}
            xstyle={[styles.name, styles.focusTarget]}
          >
            {contact.name}
          </Heading>
          <Text color="secondary" hasTabularNumbers>
            {number ? formatPhoneNumber(number) : 'Number not shared'}
            {contact.username ? ` · @${contact.username}` : null}
          </Text>
        </VStack>
      </HStack>

      {recipient ? (
        <HStack gap={2} wrap="wrap">
          <Button
            label="New message"
            variant="primary"
            clickAction={() => setDialog({ kind: 'write', message: null })}
          />
          <Button
            label="Send now"
            clickAction={() => setDialog({ kind: 'send' })}
          />
        </HStack>
      ) : (
        <Text color="secondary">
          WhatsApp hasn’t shared {firstName(contact)}’s number with linked
          devices, so Whatomato can’t send to them.
        </Text>
      )}

      {number ? (
        <>
          <Messages
            number={number}
            name={firstName(contact)}
            onEdit={(message) => setDialog({ kind: 'write', message })}
          />
          <Recent number={number} name={firstName(contact)} />
        </>
      ) : null}

      <MessageFormDialog
        isOpen={dialog?.kind === 'write'}
        onOpenChange={closeDialog}
        message={dialog?.kind === 'write' ? dialog.message : null}
        recipient={recipient}
      />
      <SendNowDialog
        isOpen={dialog?.kind === 'send'}
        onOpenChange={closeDialog}
        recipient={recipient}
        onSent={() =>
          toast({
            body: `Your message to ${firstName(contact)} is on its way.`,
            type: 'info',
          })
        }
      />
    </Panel>
  )
}

function firstName(contact: Contact) {
  return contact.first_name ?? contact.name
}

function Messages({
  number,
  name,
  onEdit,
}: {
  number: string
  name: string
  onEdit: (message: ScheduledMessage) => void
}) {
  const messages = useQuery(messagesQuery)
  if (!messages.data) return null
  // What goes out next first, then the rest by when they last changed
  const theirs = messages.data
    .filter(
      (message) =>
        message.recipient_type === 'contact' && message.recipient === number,
    )
    .sort(
      (a, b) =>
        Number(!a.next_run_at) - Number(!b.next_run_at) ||
        (a.next_run_at && b.next_run_at
          ? Date.parse(a.next_run_at) - Date.parse(b.next_run_at)
          : Date.parse(b.updated_at) - Date.parse(a.updated_at)),
    )

  return (
    <VStack gap={2}>
      <Heading level={3}>Messages</Heading>
      {theirs.length === 0 ? (
        <Text color="secondary">Nothing is scheduled to {name}.</Text>
      ) : (
        <List hasDividers>
          {theirs.map((message) => (
            <ListItem
              key={message.id}
              label={message.body}
              description={<When message={message} />}
              onClick={() => onEdit(message)}
            />
          ))}
        </List>
      )}
    </VStack>
  )
}

/** How it repeats, in the zone it was written in, and when it goes next or went last. */
function When({ message }: { message: ScheduledMessage }) {
  const zone = zoneLabel(message.timezone)
  const repeats =
    message.schedule_kind === 'recurring'
      ? `${summariseRecurrence(message.recurrence)}${zone ? `, ${zone}` : ''}`
      : null
  const next = message.next_run_at
    ? `${repeats ? 'Next ' : 'Once, '}${formatWhen(message.next_run_at)}`
    : message.schedule_kind === 'once' && message.last_run_at
      ? `Went ${formatWhen(message.last_run_at)}`
      : 'Off'
  return (
    <VStack>
      {repeats ? <Text color="secondary">{repeats}</Text> : null}
      <Text
        color={message.next_run_at ? 'primary' : 'secondary'}
        hasTabularNumbers
      >
        {next}
      </Text>
    </VStack>
  )
}

function Recent({ number, name }: { number: string; name: string }) {
  const deliveries = useQuery(recipientDeliveriesQuery(number, RECENT))
  if (!deliveries.data) return null
  const { items, total } = deliveries.data

  return (
    <VStack gap={2}>
      <Heading level={3}>Sent recently</Heading>
      {items.length === 0 ? (
        <Text color="secondary">Nothing has gone to {name} yet.</Text>
      ) : (
        <>
          <List hasDividers>
            {items.map((delivery) => (
              <ListItem
                key={delivery.id}
                label={formatWhen(delivery.sent_at ?? delivery.created_at)}
                description={delivery.error ?? delivery.body}
                endContent={<DeliveryStatus delivery={delivery} />}
              />
            ))}
          </List>
          {total > items.length ? (
            <Text type="supporting" hasTabularNumbers>
              {total - items.length} more in History.
            </Text>
          ) : null}
        </>
      )}
    </VStack>
  )
}
