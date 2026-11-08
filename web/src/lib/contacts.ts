import { useMemo } from 'react'
import { queryOptions, useQuery } from '@tanstack/react-query'

import type { components } from '@/api/schema'

import { api } from '@/api/client'
import type { ScheduledMessage } from '@/lib/scheduler'
import { ApiError, formatPhoneNumber, statusQuery } from '@/lib/whatsapp'

export type Contact = components['schemas']['ContactResponse']
export type ContactBook = components['schemas']['ContactBook']

// A first import is a few seconds of syncing on the phone's side
const IMPORT_POLL_MS = 3_000
// The server keeps a found picture for hours; one it couldn't get may come once WhatsApp connects
const PICTURE_FRESH_MS = 60 * 60_000
const NO_PICTURE_FRESH_MS = 5 * 60_000
const collator = new Intl.Collator(undefined, { sensitivity: 'base' })

export const contactsQuery = queryOptions({
  queryKey: ['whatsapp', 'contacts'],
  queryFn: async (): Promise<ContactBook> => {
    const { data, response } = await api.GET('/api/whatsapp/contacts')
    if (!data) throw new ApiError(response.status)
    return {
      ...data,
      contacts: [...data.contacts].sort((a, b) =>
        collator.compare(a.name, b.name),
      ),
    }
  },
  staleTime: 60_000,
  retry: (failures, error) =>
    !(error instanceof ApiError && error.status < 500) && failures < 2,
})

/** The address book, polled while a running link is still bringing it over. */
export function useContactBook() {
  const status = useQuery(statusQuery).data?.status.status
  const isRunning =
    status === 'connected' || status === 'syncing' || status === 'reconnecting'
  const query = useQuery({
    ...contactsQuery,
    refetchInterval: (current) =>
      isRunning && current.state.data?.imported === false
        ? IMPORT_POLL_MS
        : false,
  })
  const book = query.data
  const byNumber = useMemo(() => {
    const map = new Map<string, Contact>()
    for (const contact of book?.contacts ?? []) {
      if (contact.phone_number) map.set(contact.phone_number, contact)
    }
    return map
  }, [book])
  return {
    query,
    book,
    byNumber,
    isImporting: isRunning && book?.imported === false,
  }
}

/** A contact's or group's picture as an object URL, or `null` when there's none to show. */
export function pictureQuery(id: string) {
  return queryOptions({
    queryKey: ['whatsapp', 'picture', id],
    queryFn: async (): Promise<string | null> => {
      const { data, response } = await api.GET('/api/whatsapp/pictures/{id}', {
        params: { path: { id } },
        parseAs: 'blob',
      })
      return response.ok && data ? URL.createObjectURL(data) : null
    },
    staleTime: (query) =>
      query.state.data ? PICTURE_FRESH_MS : NO_PICTURE_FRESH_MS,
    // Object URLs stay valid for the page's life, so a cached one never goes stale underneath
    gcTime: Infinity,
    retry: false,
  })
}

type Recipient = Pick<
  ScheduledMessage,
  'recipient_type' | 'recipient' | 'recipient_name'
>

/**
 * What to call a recipient: the name on the phone now, else the one saved with the message, else the
 * number. A rename on the phone shows everywhere at once.
 */
export function useRecipientLabel() {
  const { byNumber } = useContactBook()
  return (recipient: Recipient) => {
    const contact =
      recipient.recipient_type === 'contact'
        ? byNumber.get(recipient.recipient)
        : undefined
    if (contact) return contact.name
    if (recipient.recipient_name) return recipient.recipient_name
    return recipient.recipient_type === 'contact'
      ? formatPhoneNumber(recipient.recipient)
      : 'Unnamed group'
  }
}

/** Whose picture a recipient shows: the saved contact's, or the group's own. */
export function useRecipientPictureID() {
  const { byNumber } = useContactBook()
  return ({
    recipient_type,
    recipient,
  }: Pick<Recipient, 'recipient_type' | 'recipient'>) =>
    recipient_type === 'group'
      ? recipient
      : (byNumber.get(recipient)?.id ?? null)
}

/**
 * Matches on any part of the name or username, or on digits of the number. A leading 0 is dropped
 * from typed digits, so the local "07700 900123" finds the stored 447700900123.
 */
export function matchesContact(contact: Contact, query: string) {
  const text = fold(query.trim())
  if (!text) return true
  if (
    [contact.name, contact.first_name, contact.username].some(
      (field) => field && fold(field).includes(text),
    )
  ) {
    return true
  }
  const digits = query.replace(/\D/g, '').replace(/^0+/, '')
  return (
    !/\p{L}/u.test(query) &&
    digits.length >= 2 &&
    (contact.phone_number ?? '').includes(digits)
  )
}

/** Names first by whether they start with the query, so "an" finds Anna before Joanna. */
export function rankContacts(contacts: Contact[], query: string) {
  const text = fold(query.trim())
  const starts = (contact: Contact) =>
    fold(contact.name)
      .split(/\s+/)
      .some((word) => word.startsWith(text))
  const matches = contacts.filter((contact) => matchesContact(contact, query))
  return [
    ...matches.filter(starts),
    ...matches.filter((contact) => !starts(contact)),
  ]
}

function fold(text: string) {
  return text.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase()
}

/** Who a contact page links to: the id in a URL segment. */
export function contactHref(contact: Pick<Contact, 'id'>) {
  return `/contacts/${encodeURIComponent(contact.id)}`
}

/** The words initials come from: "Dr Evans (dentist)" gives DE, not D(. */
export function initialsOf(name: string) {
  const words = name
    .replace(/\(.*?\)|\[.*?\]/g, ' ')
    .replace(/[^\p{L}\p{N}\s]/gu, ' ')
    .trim()
  return words || name
}
