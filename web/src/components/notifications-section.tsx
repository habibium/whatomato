import { useEffect, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { List, ListItem } from '@astryxdesign/core/List'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { Text } from '@astryxdesign/core/Text'
import { useToast } from '@astryxdesign/core/Toast'
import { Token } from '@astryxdesign/core/Token'
import { VStack } from '@astryxdesign/core/VStack'

import { api } from '@/api/client'
import { SettingsSection } from '@/components/settings-section'
import { formatDay } from '@/lib/format'
import {
  currentSubscription,
  isPushSupported,
  pushQuery,
  reconcilePush,
  turnOffPush,
  turnOnPush,
} from '@/lib/push'
import type { TurnOnResult } from '@/lib/push'

const PROBLEMS: Record<Exclude<TurnOnResult, 'on'>, string> = {
  blocked:
    'Notifications are blocked for this site. Allow them in your browser’s settings for this site, then try again.',
  unsupported: 'This browser can’t show notifications from a website.',
  refused:
    'This browser’s notification service isn’t one Whatomato knows, so it can’t be turned on here.',
  failed:
    'Couldn’t turn notifications on. Check your connection and try again.',
}

/** Turning browser notifications on and off, for this browser and the account's others. */
export function NotificationsSection() {
  const queryClient = useQueryClient()
  const toast = useToast()
  const status = useQuery(pushQuery)
  const isSupported = isPushSupported()
  // This browser's address at its push service, once it's known to be this account's
  const [here, setHere] = useState<string | null>(null)
  const [problem, setProblem] = useState<string | null>(null)
  const [isBusy, setIsBusy] = useState(false)

  useEffect(() => {
    if (!status.data) return
    let isCurrent = true
    void (async () => {
      const isOn = await reconcilePush(status.data)
      const subscription = isOn ? await currentSubscription() : null
      if (isCurrent) setHere(subscription?.endpoint ?? null)
    })()
    return () => {
      isCurrent = false
    }
  }, [status.data])

  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: pushQuery.queryKey })

  const turnOn = async () => {
    if (!status.data) return
    setProblem(null)
    setIsBusy(true)
    const result = await turnOnPush(status.data.public_key)
    if (result !== 'on') setProblem(PROBLEMS[result])
    await refresh()
    setIsBusy(false)
  }

  const turnOff = async (endpoint: string) => {
    setProblem(null)
    setIsBusy(true)
    await turnOffPush(endpoint)
    await refresh()
    setIsBusy(false)
  }

  const test = async () => {
    setProblem(null)
    try {
      const { data } = await api.POST('/api/push/test')
      if (data && data.sent > 0) {
        toast({ body: 'Sent. It should appear in a moment.', type: 'info' })
      } else {
        setProblem(
          'The test couldn’t be sent. Turn notifications off and on again, then try once more.',
        )
        await refresh()
      }
    } catch {
      setProblem('Couldn’t send the test. Check your connection and try again.')
    }
  }

  const subscriptions = status.data?.subscriptions ?? []
  const isOnHere = here !== null

  return (
    <SettingsSection
      id="notifications"
      title="Notifications"
      description="A notification from your browser when an agent needs your approval, even while Whatomato isn’t open. It names the agent and the chat, never the message."
      status={
        status.data && isSupported ? (
          <Token
            label={isOnHere ? 'On' : 'Off'}
            color={isOnHere ? 'green' : 'gray'}
            size="sm"
          />
        ) : undefined
      }
    >
      {status.isPending ? (
        <Skeleton width="100%" height={40} />
      ) : status.isError ? (
        <Banner
          status="error"
          title="Couldn't load your notification settings."
          description="Check your connection and try again."
          endContent={
            <Button
              label="Try again"
              size="sm"
              clickAction={async () => {
                await status.refetch()
              }}
            />
          }
        />
      ) : (
        <VStack gap={4}>
          {problem ? <Banner status="error" title={problem} /> : null}

          {isSupported ? (
            <HStack gap={2} wrap="wrap">
              {isOnHere ? (
                <>
                  <Button label="Send a test" clickAction={test} />
                  <Button
                    label="Turn off in this browser"
                    variant="secondary"
                    isLoading={isBusy}
                    clickAction={() => turnOff(here)}
                  />
                </>
              ) : (
                <Button
                  label="Turn on in this browser"
                  variant="primary"
                  isLoading={isBusy}
                  clickAction={turnOn}
                />
              )}
            </HStack>
          ) : (
            <Text color="secondary">
              This browser can’t show notifications from a website. On an iPhone
              or iPad, add Whatomato to the Home Screen and open it from there.
            </Text>
          )}

          {subscriptions.length > 0 ? (
            <List hasDividers density="compact">
              {subscriptions.map((subscription) => (
                <ListItem
                  key={subscription.id}
                  label={
                    subscription.endpoint === here
                      ? `${subscription.label} (this browser)`
                      : subscription.label
                  }
                  description={`On since ${formatDay(subscription.created_at)}`}
                  endContent={
                    subscription.endpoint === here ? undefined : (
                      <Button
                        label="Turn off"
                        size="sm"
                        variant="secondary"
                        clickAction={() => turnOff(subscription.endpoint)}
                      />
                    )
                  }
                />
              ))}
            </List>
          ) : null}
        </VStack>
      )}
    </SettingsSection>
  )
}
