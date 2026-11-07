import * as stylex from '@stylexjs/stylex'
import { HStack } from '@astryxdesign/core/HStack'
import { StatusDot } from '@astryxdesign/core/StatusDot'
import { Text } from '@astryxdesign/core/Text'

import type { Delivery } from '@/lib/scheduler'

const styles = stylex.create({
  status: { flexShrink: 0, whiteSpace: 'nowrap' },
})

function describeStatus({ status, error }: Delivery) {
  switch (status) {
    case 'sent':
      return { label: 'Sent', variant: 'success', isPulsing: false } as const
    case 'failed':
      return { label: 'Failed', variant: 'error', isPulsing: false } as const
    case 'pending':
      return error
        ? ({
            label: 'Waiting to retry',
            variant: 'warning',
            isPulsing: true,
          } as const)
        : ({ label: 'Sending', variant: 'accent', isPulsing: true } as const)
  }
}

export function DeliveryStatus({ delivery }: { delivery: Delivery }) {
  const status = describeStatus(delivery)
  return (
    <HStack gap={2} vAlign="center" xstyle={styles.status}>
      {/* The word beside it says the same, so the dot itself stays quiet */}
      <StatusDot
        aria-hidden="true"
        variant={status.variant}
        label={status.label}
        isPulsing={status.isPulsing}
      />
      <Text>{status.label}</Text>
    </HStack>
  )
}
