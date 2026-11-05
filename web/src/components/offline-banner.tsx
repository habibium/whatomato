import { Banner } from '@astryxdesign/core/Banner'

export function OfflineBanner() {
  return (
    <Banner
      status="warning"
      title="Can't reach Whatomato right now."
      description="What you see here may be out of date. It updates again once the connection is back."
    />
  )
}
