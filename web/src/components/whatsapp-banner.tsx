import { useQuery } from '@tanstack/react-query'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'

import { statusQuery } from '@/lib/whatsapp'
import type { WhatsAppStatus } from '@/lib/whatsapp'

const LOST = 'Any that come due before then fail, and don’t go out later.'

function describe(status: WhatsAppStatus) {
  switch (status.status) {
    case 'connected':
    case 'syncing':
      return null
    case 'not_linked':
      return {
        status: 'warning',
        title: 'WhatsApp isn’t linked.',
        description: `Scheduled messages won’t send until you link it. ${LOST}`,
        action: 'Link WhatsApp',
      } as const
    case 'pairing':
      return {
        status: 'warning',
        title: 'WhatsApp isn’t linked yet.',
        description: `Scheduled messages won’t send until you finish linking it. ${LOST}`,
        action: 'Finish linking',
      } as const
    case 'paused':
      return {
        status: 'warning',
        title: 'WhatsApp is paused.',
        description: `Scheduled messages won’t send until you reconnect it. ${LOST}`,
        action: 'Open WhatsApp',
      } as const
    case 'reconnecting':
      return {
        status: 'info',
        title: 'Reconnecting to WhatsApp.',
        description:
          'Scheduled messages wait for the connection, and keep trying for about half an hour. There’s nothing you need to do.',
        action: 'Open WhatsApp',
      } as const
    case 'stopped':
      return {
        status: 'error',
        title: 'WhatsApp has stopped.',
        description: `Scheduled messages won’t send until you reconnect it. ${LOST}`,
        action: 'Open WhatsApp',
      } as const
  }
}

export function WhatsAppBanner() {
  const { data } = useQuery(statusQuery)
  const copy = data ? describe(data.status) : null
  if (!copy) return null

  return (
    <Banner
      status={copy.status}
      title={copy.title}
      description={copy.description}
      endContent={<Button label={copy.action} size="sm" href="/whatsapp" />}
    />
  )
}
