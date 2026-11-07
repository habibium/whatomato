import { createFileRoute } from '@tanstack/react-router'
import { Heading, Text } from '@astryxdesign/core/Text'

import { Panel } from '@/components/panel'

export const Route = createFileRoute('/_authenticated/contacts/')({
  component: NoneChosen,
})

/** Shown beside the list on a wide screen; on a phone the list stands alone. */
function NoneChosen() {
  return (
    <Panel gap={2}>
      <Heading level={2}>Choose someone</Heading>
      <Text color="secondary">
        See what’s scheduled to them and what went out, or write them a message.
      </Text>
      <Text type="supporting">
        Names come from your phone’s address book. Unlinking WhatsApp deletes
        Whatomato’s copy.
      </Text>
    </Panel>
  )
}
