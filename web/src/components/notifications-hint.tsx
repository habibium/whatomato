import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'

import { isPushSupported, pushQuery, reconcilePush } from '@/lib/push'

const DISMISSED_KEY = 'wa-notifications-hint-dismissed'

function wasDismissed() {
  try {
    return localStorage.getItem(DISMISSED_KEY) === '1'
  } catch {
    return false
  }
}

/** Points at the switch in Settings, for a browser that could notify about approvals and doesn't. */
export function NotificationsHint() {
  const isCandidate = isPushSupported() && !wasDismissed()
  const status = useQuery({ ...pushQuery, enabled: isCandidate })
  const [isOff, setIsOff] = useState(false)

  useEffect(() => {
    if (!status.data) return
    let isCurrent = true
    void reconcilePush(status.data).then((isOn) => {
      if (isCurrent) setIsOff(!isOn)
    })
    return () => {
      isCurrent = false
    }
  }, [status.data])

  if (!isCandidate || !isOff) return null
  return (
    <Banner
      status="info"
      title="Hear about approvals as they come."
      description="This browser can notify you when an agent needs your approval, even while Whatomato isn’t open."
      endContent={
        <Button
          label="Turn on notifications"
          size="sm"
          href="/settings#notifications"
        />
      }
      isDismissable
      onDismiss={() => {
        try {
          localStorage.setItem(DISMISSED_KEY, '1')
        } catch {
          // Without storage it shows again next time, which is no harm
        }
        setIsOff(false)
      }}
    />
  )
}
