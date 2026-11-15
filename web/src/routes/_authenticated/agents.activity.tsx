import { useQuery } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { Pagination } from '@astryxdesign/core/Pagination'
import { Selector } from '@astryxdesign/core/Selector'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'

import { ActivityLog } from '@/components/agent-activity-log'
import { AppFrame } from '@/components/app-frame'
import { BackLink } from '@/components/back-link'
import { ListEmpty } from '@/components/list-empty'
import { OfflineBanner } from '@/components/offline-banner'
import { PageHeading } from '@/components/page-heading'
import { ACTIVITY_PER_PAGE, activityQuery } from '@/lib/agent-activity'
import { agentsQuery } from '@/lib/agents'
import { useSessionExpiry } from '@/lib/use-session-expiry'

const ALL = 'all'

export const Route = createFileRoute('/_authenticated/agents/activity')({
  validateSearch: (search): { connection?: string; page?: number } => ({
    connection:
      typeof search.connection === 'string' ? search.connection : undefined,
    page:
      typeof search.page === 'number' && search.page > 1
        ? Math.floor(search.page)
        : undefined,
  }),
  component: ActivityPage,
})

function ActivityPage() {
  const { connection, page = 1 } = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  const overview = useQuery(agentsQuery)
  const query = useQuery(activityQuery(connection, page))
  const isSignedOut = useSessionExpiry(query.error, '/agents/activity')
  const connections = overview.data?.connections ?? []
  const chosen = connections.find((item) => item.id === connection)
  const activity = query.data

  const options = [
    { value: ALL, label: 'All connections' },
    ...connections.map((item) => ({ value: item.id, label: item.name })),
  ]

  return (
    <AppFrame>
      <VStack gap={6}>
        {query.isRefetchError && !isSignedOut ? <OfflineBanner /> : null}
        <BackLink href="/agents" label="Agents" />

        <PageHeading
          title="Activity"
          lede={
            activity
              ? activity.total === 0
                ? 'Every call an agent makes shows here, allowed or not. Message text is never kept.'
                : `${activity.total} ${activity.total === 1 ? 'call' : 'calls'}${chosen ? ` by ${chosen.name}` : ' by your agents'} in the last 90 days, newest first. Message text is never kept.`
              : undefined
          }
        />

        {connections.length > 1 || connection ? (
          <HStack>
            <Selector
              label="Connection"
              isLabelHidden
              width={280}
              options={options}
              value={connection ?? ALL}
              onChange={(value) =>
                navigate({
                  search: {
                    connection: value === ALL ? undefined : value,
                    page: undefined,
                  },
                })
              }
            />
          </HStack>
        ) : null}

        {activity ? (
          activity.items.length > 0 ? (
            <VStack gap={6}>
              <ActivityLog
                items={activity.items}
                showsConnection={!connection}
              />
              {activity.total > ACTIVITY_PER_PAGE ? (
                <HStack justify="end">
                  <Pagination
                    variant="count"
                    size="sm"
                    label="Activity pages"
                    page={page}
                    onChange={(next) =>
                      navigate({ search: (prev) => ({ ...prev, page: next }) })
                    }
                    totalItems={activity.total}
                    pageSize={ACTIVITY_PER_PAGE}
                    isDisabled={query.isPlaceholderData}
                  />
                </HStack>
              ) : null}
            </VStack>
          ) : (
            <ListEmpty
              title={
                chosen
                  ? `${chosen.name} hasn’t done anything yet`
                  : 'No activity yet'
              }
              description="Once an agent connects with its access key, each thing it does or tries shows here."
              actions={<Button label="Go to agents" href="/agents" />}
            />
          )
        ) : query.isPending ? (
          <VStack gap={2} aria-busy="true">
            {[0, 1, 2, 3, 4].map((index) => (
              <Skeleton key={index} index={index} width="100%" height={56} />
            ))}
          </VStack>
        ) : isSignedOut ? null : (
          <Banner
            status="error"
            title="Couldn't load the activity."
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
        {activity && activity.items.length > 0 ? (
          <Text type="supporting">
            Refused means your permissions stopped it. Asked you means it waited
            for your approval.
          </Text>
        ) : null}
      </VStack>
    </AppFrame>
  )
}
