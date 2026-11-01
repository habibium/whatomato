import { useCallback, useEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { Divider } from '@astryxdesign/core/Divider'
import { Grid } from '@astryxdesign/core/Grid'
import { HStack } from '@astryxdesign/core/HStack'
import { useAnnounce } from '@astryxdesign/core/hooks'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { StatusDot } from '@astryxdesign/core/StatusDot'
import { Heading, Text } from '@astryxdesign/core/Text'
import { Timestamp } from '@astryxdesign/core/Timestamp'
import { VisuallyHidden } from '@astryxdesign/core/VisuallyHidden'
import { VStack } from '@astryxdesign/core/VStack'
import {
  borderVars,
  colorVars,
  durationVars,
  easeVars,
  spacingVars,
} from '@astryxdesign/core/theme/tokens.stylex'

import { api } from '@/api/client'
import {
  ConfirmPasswordDialog,
  toConfirmOutcome,
} from '@/components/confirm-password-dialog'
import type { ConfirmOutcome } from '@/components/confirm-password-dialog'
import { SiteFrame } from '@/components/site-frame'
import {
  GroupsSection,
  TestMessageSection,
} from '@/components/whatsapp-connected'
import { PairingPanel, pairingMethod } from '@/components/whatsapp-pairing'
import type { LinkMethod } from '@/components/whatsapp-pairing'
import { PhoneLinkDialog } from '@/components/whatsapp-phone-dialog'
import type { PhoneLinkOutcome } from '@/components/whatsapp-phone-dialog'
import { toFieldErrors } from '@/lib/form-errors'
import { HOME_HREF, setSessionUser } from '@/lib/session'
import { ApiError, formatPhoneNumber, statusQuery } from '@/lib/whatsapp'
import type { Linked, WhatsAppStatus } from '@/lib/whatsapp'

export const Route = createFileRoute('/_authenticated/whatsapp')({
  component: WhatsAppPage,
})

const COLUMN_WIDTH = 560

const enter = stylex.keyframes({
  from: { opacity: 0, transform: 'translateY(6px)' },
  to: { opacity: 1, transform: 'translateY(0)' },
})

const styles = stylex.create({
  measure: { maxWidth: '45ch' },
  // Focused only from code, as a place to land; it isn't a control
  focusTarget: { outline: 'none' },
  // Each status gets a fresh view; easing it in shows the page changed by itself
  enter: {
    animationDuration: durationVars['--duration-medium'],
    animationFillMode: 'both',
    animationName: {
      default: 'none',
      '@media (prefers-reduced-motion: no-preference)': enter,
    },
    animationTimingFunction: easeVars['--ease-standard'],
  },
  fact: {
    borderTopColor: colorVars['--color-border'],
    borderTopStyle: 'solid',
    borderTopWidth: {
      default: borderVars['--border-width'],
      ':first-child': 0,
    },
    gridTemplateColumns: {
      default: 'minmax(0, 1fr)',
      '@media (min-width: 640px)': '160px minmax(0, 1fr)',
    },
    paddingBlockEnd: spacingVars['--spacing-4'],
    paddingBlockStart: {
      default: spacingVars['--spacing-4'],
      ':first-child': 0,
    },
  },
  narrowOnly: {
    display: {
      default: 'inline',
      '@media (min-width: 640px)': 'none',
    },
  },
})

type Command = 'pause' | 'resume' | 'cancel'

const COMMANDS = {
  pause: () => api.POST('/api/whatsapp/pause'),
  resume: () => api.POST('/api/whatsapp/resume'),
  cancel: () => api.POST('/api/whatsapp/link/cancel'),
}

const COMMAND_FAILED: Record<Command, string> = {
  pause: "Couldn't pause WhatsApp.",
  resume: "Couldn't reconnect WhatsApp.",
  cancel: "Couldn't stop linking.",
}

type Notice =
  | 'unlinked'
  | 'still-on-phone'
  | 'pairing-ended'
  | 'unlinked-elsewhere'

interface LinkRequest {
  method: LinkMethod
  phoneNumber: string | null
}

function WhatsAppPage() {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const announce = useAnnounce()
  const query = useQuery(statusQuery)
  const [dialog, setDialog] = useState<'qr' | 'phone' | 'unlink' | null>(null)
  const [request, setRequest] = useState<LinkRequest | null>(null)
  const [notice, setNotice] = useState<Notice | null>(null)
  const [failed, setFailed] = useState<Command | null>(null)
  // A change this tab asked for, so its effect isn't reported as coming from elsewhere
  const expecting = useRef<'cancel' | 'unlink' | null>(null)
  const seen = useRef<WhatsAppStatus['status'] | null>(null)
  const headingRef = useRef<HTMLHeadingElement>(null)

  const refresh = useCallback(
    () => queryClient.invalidateQueries({ queryKey: statusQuery.queryKey }),
    [queryClient],
  )

  const isSignedOut =
    query.error instanceof ApiError && query.error.status === 401
  useEffect(() => {
    if (!isSignedOut) return
    setSessionUser(queryClient, null)
    void navigate({ to: '/signin', search: { redirect: '/whatsapp' } })
  }, [isSignedOut, navigate, queryClient])

  const status = query.data?.status
  const clockOffset = query.data?.clockOffset ?? 0
  const method =
    status?.status === 'pairing'
      ? pairingMethod(status, request?.method ?? null)
      : null
  const copy = status ? describe(status, method) : null

  useEffect(() => {
    if (!status || !copy) return
    const before = seen.current
    seen.current = status.status
    if (before === null || before === status.status) return

    if (status.status === 'not_linked') {
      if (before === 'pairing' && expecting.current !== 'cancel') {
        setNotice('pairing-ended')
      } else if (before !== 'pairing' && expecting.current !== 'unlink') {
        setNotice('unlinked-elsewhere')
      }
    }
    expecting.current = null
    setFailed(null)
    // Focus fell to the body with the view it was in; landing on the new
    // heading reads it out, so it is announced only when focus is elsewhere
    if (document.activeElement === document.body) {
      headingRef.current?.focus({ preventScroll: true })
    } else {
      announce(copy.title)
    }
    // Only a change of status is news; codes and numbers refresh within one
  }, [status?.status])

  const run = async (command: Command) => {
    setFailed(null)
    if (command === 'cancel') expecting.current = 'cancel'
    try {
      const { response } = await COMMANDS[command]()
      // 409: the status moved on by itself, which the refresh shows
      if (!response.ok && response.status !== 409) {
        throw new ApiError(response.status)
      }
    } catch {
      expecting.current = null
      setFailed(command)
    }
    await refresh()
  }

  const startLink = async (password: string, phoneNumber: string | null) => {
    const result = await api.POST('/api/whatsapp/link', {
      body: phoneNumber
        ? { password, phone_number: phoneNumber }
        : { password },
    })
    if (result.response.ok) {
      setRequest({ method: phoneNumber ? 'phone' : 'qr', phoneNumber })
      setNotice(null)
    }
    // 409: another tab got there first, and the refresh shows how far it got
    if (result.response.ok || result.response.status === 409) {
      await refresh()
      return 'confirmed' as const
    }
    return result
  }

  const linkByQr = async (password: string): Promise<ConfirmOutcome> => {
    const result = await startLink(password, null)
    return result === 'confirmed' ? result : toConfirmOutcome(result)
  }

  const linkByPhone = async (
    digits: string,
    password: string,
  ): Promise<PhoneLinkOutcome> => {
    const result = await startLink(password, digits)
    if (result === 'confirmed') return result
    if (toFieldErrors(result.error, ['phone_number'] as const)) {
      return 'invalid-number'
    }
    return toConfirmOutcome(result)
  }

  const unlink = async (password: string): Promise<ConfirmOutcome> => {
    expecting.current = 'unlink'
    try {
      const result = await api.POST('/api/whatsapp/unlink', {
        body: { password },
      })
      if (result.data) {
        setNotice(
          result.data.removed_from_phone ? 'unlinked' : 'still-on-phone',
        )
      }
      if (result.data || result.response.status === 409) {
        await refresh()
        return 'confirmed'
      }
      expecting.current = null
      return toConfirmOutcome(result)
    } catch (error) {
      expecting.current = null
      throw error
    }
  }

  return (
    <SiteFrame
      action={<Button label="Dashboard" variant="ghost" href={HOME_HREF} />}
    >
      <VStack gap={8} maxWidth={COLUMN_WIDTH} paddingBlock={10}>
        {notice ? (
          <NoticeBanner
            key={notice}
            notice={notice}
            onDismiss={() => setNotice(null)}
          />
        ) : null}
        {query.isRefetchError && !isSignedOut ? (
          <Banner
            status="warning"
            title="Can't reach Whatomato right now."
            description="What you see here may be out of date. It updates again once the connection is back."
          />
        ) : null}

        {status && copy ? (
          <VStack key={status.status} gap={8} xstyle={styles.enter}>
            <VStack gap={6}>
              <VStack gap={3}>
                <Heading
                  ref={headingRef}
                  tabIndex={-1}
                  level={1}
                  type="display-3"
                  textWrap="balance"
                  xstyle={styles.focusTarget}
                >
                  {copy.title}
                </Heading>
                <Text color="secondary" size="lg" xstyle={styles.measure}>
                  {copy.lede}
                </Text>
              </VStack>
              {status.status === 'not_linked' ? (
                <LinkChoices
                  onLinkByQr={() => setDialog('qr')}
                  onLinkByPhone={() => setDialog('phone')}
                />
              ) : null}
            </VStack>

            <Divider />

            {status.status === 'not_linked' ? (
              <Facts />
            ) : (
              <VStack gap={5}>
                {status.status === 'pairing' ? (
                  <PairingPanel
                    pairing={status}
                    method={method}
                    clockOffset={clockOffset}
                    phoneNumber={
                      method === 'phone' ? (request?.phoneNumber ?? null) : null
                    }
                    onCancel={() => run('cancel')}
                  />
                ) : (
                  <>
                    <LinkDetails status={status} />
                    <LinkActions
                      status={status}
                      clockOffset={clockOffset}
                      onUnlink={() => setDialog('unlink')}
                      onCommand={run}
                    />
                  </>
                )}
                {failed ? (
                  <Banner
                    status="error"
                    title={COMMAND_FAILED[failed]}
                    description="Something went wrong on our side. Try again in a moment."
                  />
                ) : null}
              </VStack>
            )}

            {status.status === 'connected' ? (
              <>
                <Divider />
                <TestMessageSection onStale={refresh} />
                <Divider />
                <GroupsSection onStale={refresh} />
              </>
            ) : null}
          </VStack>
        ) : query.isPending ? (
          <LoadingHero />
        ) : isSignedOut ? null : (
          <Banner
            status="error"
            title="Couldn't load your WhatsApp link."
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

      <ConfirmPasswordDialog
        isOpen={dialog === 'qr'}
        onOpenChange={(isOpen) => setDialog(isOpen ? 'qr' : null)}
        title="Link with a QR code"
        description="Enter your password to start. You'll need your phone next."
        confirmLabel="Continue"
        onConfirm={linkByQr}
      />
      <PhoneLinkDialog
        isOpen={dialog === 'phone'}
        onOpenChange={(isOpen) => setDialog(isOpen ? 'phone' : null)}
        onConfirm={linkByPhone}
      />
      <ConfirmPasswordDialog
        isOpen={dialog === 'unlink'}
        onOpenChange={(isOpen) => setDialog(isOpen ? 'unlink' : null)}
        title="Unlink WhatsApp?"
        description="Whatomato logs out of your WhatsApp and deletes its keys. Scheduled messages stop until you link again."
        confirmLabel="Unlink"
        confirmVariant="destructive"
        onConfirm={unlink}
      />
    </SiteFrame>
  )
}

interface PageCopy {
  title: string
  lede: ReactNode
}

function describe(status: WhatsAppStatus, method: LinkMethod | null): PageCopy {
  switch (status.status) {
    case 'not_linked':
      return {
        title: 'Link your WhatsApp.',
        lede: 'Once it’s linked, scheduled messages go out from your own number, as if you’d sent them from your phone.',
      }
    case 'pairing':
      return method === 'qr'
        ? {
            title: 'Scan the code with your phone.',
            lede: 'Use the phone that has the WhatsApp account you want to send from.',
          }
        : method === 'phone'
          ? {
              title: 'Enter the code on your phone.',
              lede: 'WhatsApp asks for it when you link a device with a phone number instead of a QR code.',
            }
          : {
              title: 'Linking your WhatsApp.',
              lede: 'Whatomato is getting a code from WhatsApp, which takes a few seconds.',
            }
    case 'syncing':
      return {
        title: 'WhatsApp is linked.',
        lede: 'It’s syncing with Whatomato for the first time, which can take a few minutes. You can leave this page while it does.',
      }
    case 'connected':
      return {
        title: 'WhatsApp is connected.',
        lede: 'Scheduled messages go out from your number, as if you’d sent them from your phone.',
      }
    case 'reconnecting':
      return {
        title: 'Reconnecting to WhatsApp.',
        lede: 'The connection dropped, and Whatomato is reconnecting on its own. There’s nothing you need to do.',
      }
    case 'paused':
      return {
        title: 'WhatsApp is paused.',
        lede: 'Whatomato is disconnected but still linked, so reconnecting needs no new code. Nothing is sent meanwhile, and WhatsApp logs out a device that stays offline too long.',
      }
    case 'stopped':
      switch (status.reason) {
        case 'replaced':
          return {
            title: 'Another session took over.',
            lede: 'Something else connected with the keys that link Whatomato to your WhatsApp. Unlink, then link again for new keys.',
          }
        case 'temporary_ban':
          return {
            title: 'WhatsApp has banned this number for now.',
            lede: status.until ? (
              <>
                The ban lasts until <BanEnd until={status.until} />. Reconnect
                after that; reconnecting sooner won’t lift it.
              </>
            ) : (
              'The ban is temporary. Reconnect once it’s over; reconnecting sooner won’t lift it.'
            ),
          }
        case 'outdated':
          return {
            title: 'Whatomato needs an update.',
            lede: 'WhatsApp stopped accepting the version Whatomato connects with. It needs an update on our side, so try reconnecting later.',
          }
        case 'refused':
          return {
            title: 'WhatsApp refused the connection.',
            lede: 'It didn’t say why. Try reconnecting; if it keeps happening, unlink and link again.',
          }
      }
  }
}

/** The end of a ban in the user's own time, naming the day only when it isn't today or tomorrow. */
function BanEnd({ until }: { until: string }) {
  const startOfDay = (date: Date) =>
    new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime()
  // Rounded, as a day across a clock change runs 23 or 25 hours
  const days = Math.round(
    (startOfDay(new Date(until)) - startOfDay(new Date())) / 86_400_000,
  )
  if (days !== 0 && days !== 1) {
    return (
      <Timestamp
        value={until}
        format="date_time"
        type="inherit"
        color="inherit"
      />
    )
  }
  return (
    <>
      <Timestamp value={until} format="time" type="inherit" color="inherit" />{' '}
      {days === 0 ? 'today' : 'tomorrow'}
    </>
  )
}

function LinkChoices({
  onLinkByQr,
  onLinkByPhone,
}: {
  onLinkByQr: () => void
  onLinkByPhone: () => void
}) {
  return (
    <VStack gap={3} hAlign="start">
      <HStack gap={2} wrap="wrap">
        <Button
          label="Link with QR code"
          variant="primary"
          size="lg"
          clickAction={onLinkByQr}
        />
        <Button
          label="Link with phone number instead"
          variant="secondary"
          size="lg"
          clickAction={onLinkByPhone}
        />
      </HStack>
      <Text type="supporting" xstyle={styles.narrowOnly}>
        On the phone that has WhatsApp? A QR code needs a second screen, so link
        with the phone number.
      </Text>
    </VStack>
  )
}

interface LinkActionsProps {
  status: Linked
  clockOffset: number
  onUnlink: () => void
  onCommand: (command: Command) => Promise<void>
}

function LinkActions({
  status,
  clockOffset,
  onUnlink,
  onCommand,
}: LinkActionsProps) {
  const banEnded = useHasPassed(
    status.status === 'stopped' ? status.until : null,
    clockOffset,
  )

  const pause = (
    <Button
      key="pause"
      label="Pause"
      tooltip="Disconnect for now and keep the link"
      clickAction={() => onCommand('pause')}
    />
  )
  const reconnect = (variant: 'primary' | 'secondary' | 'ghost') => (
    <Button
      key="reconnect"
      label="Reconnect"
      variant={variant}
      isDisabled={!banEnded}
      clickAction={() => onCommand('resume')}
    />
  )
  const unlink = (variant: 'primary' | 'secondary' | 'ghost') => (
    <Button
      key="unlink"
      label="Unlink"
      variant={variant}
      clickAction={onUnlink}
    />
  )

  const buttons =
    status.status === 'syncing'
      ? [unlink('secondary')]
      : status.status === 'connected' || status.status === 'reconnecting'
        ? [pause, unlink('ghost')]
        : status.status === 'paused'
          ? [reconnect('primary'), unlink('ghost')]
          : status.reason === 'replaced'
            ? [unlink('primary'), reconnect('ghost')]
            : status.reason === 'outdated'
              ? [reconnect('secondary'), unlink('ghost')]
              : [reconnect('primary'), unlink('ghost')]

  return (
    <HStack gap={2} wrap="wrap">
      {buttons}
    </HStack>
  )
}

const FACTS = [
  {
    term: 'A linked device',
    body: 'Whatomato joins your account the way WhatsApp Web does. Your phone keeps working as usual.',
  },
  {
    term: 'Your chats',
    body: 'Whatomato never downloads your chat history or stores the messages you receive.',
  },
  {
    term: 'Stopping',
    body: 'Pause or unlink here at any time, or remove Whatomato under Linked devices on your phone.',
  },
  {
    term: 'WhatsApp’s rules',
    body: 'WhatsApp doesn’t officially allow apps like Whatomato, and can ban a number that uses one.',
  },
]

function Facts() {
  return (
    <VStack>
      {FACTS.map((fact) => (
        <Grid key={fact.term} columnGap={4} rowGap={1} xstyle={styles.fact}>
          <Text type="label">{fact.term}</Text>
          <Text color="secondary" xstyle={styles.measure}>
            {fact.body}
          </Text>
        </Grid>
      ))}
    </VStack>
  )
}

const DOTS = {
  syncing: { label: 'Syncing', variant: 'accent', isPulsing: true },
  connected: { label: 'Connected', variant: 'success', isPulsing: false },
  reconnecting: { label: 'Reconnecting', variant: 'warning', isPulsing: true },
  paused: { label: 'Paused', variant: 'neutral', isPulsing: false },
  stopped: { label: 'Stopped', variant: 'error', isPulsing: false },
} as const

function LinkDetails({ status }: { status: Linked }) {
  const dot = DOTS[status.status]
  return (
    <VStack gap={1.5}>
      <Text size="2xl" hasTabularNumbers>
        <VisuallyHidden>Linked number </VisuallyHidden>
        {formatPhoneNumber(status.phone_number)}
      </Text>
      <HStack gap={2} vAlign="center">
        {/* The word beside it says the same, so the dot itself stays quiet */}
        <StatusDot
          aria-hidden="true"
          variant={dot.variant}
          label={dot.label}
          isPulsing={dot.isPulsing}
        />
        <Text color="secondary">{dot.label}</Text>
      </HStack>
    </VStack>
  )
}

function LoadingHero() {
  return (
    <VStack gap={3} aria-busy="true">
      <Skeleton width="70%" height={36} />
      <Skeleton width="90%" height={20} index={1} />
      <Skeleton width="55%" height={20} index={2} />
    </VStack>
  )
}

function NoticeBanner({
  notice,
  onDismiss,
}: {
  notice: Notice
  onDismiss: () => void
}) {
  switch (notice) {
    case 'unlinked':
      return (
        <Banner
          status="success"
          title="WhatsApp is unlinked."
          description="Whatomato logged out of your WhatsApp and deleted its keys."
          isDismissable
          onDismiss={onDismiss}
        />
      )
    case 'still-on-phone':
      return (
        <Banner
          status="warning"
          title="Remove Whatomato from your phone too."
          description="WhatsApp couldn’t be reached, so it still lists Whatomato. On your phone, open Linked devices, tap Whatomato, then Log out."
          isDismissable
          onDismiss={onDismiss}
        />
      )
    case 'pairing-ended':
      return (
        <Banner
          status="info"
          title="Linking stopped."
          description="Your phone didn’t link before the code ran out. Start again for a new one."
          isDismissable
          onDismiss={onDismiss}
        />
      )
    case 'unlinked-elsewhere':
      return (
        <Banner
          status="warning"
          title="WhatsApp is no longer linked."
          description="Whatomato was removed under Linked devices on your phone, or WhatsApp logged it out."
          isDismissable
          onDismiss={onDismiss}
        />
      )
  }
}

/** Whether `time`, on the server's clock, has gone by; true without one. */
function useHasPassed(time: string | null, clockOffset: number) {
  const deadline = time === null ? null : Date.parse(time) - clockOffset
  const [now, setNow] = useState(Date.now)
  const hasPassed =
    deadline === null || Number.isNaN(deadline) || now >= deadline

  useEffect(() => {
    if (hasPassed || deadline === null) return
    // setTimeout overflows past about 24.8 days; a longer wait checks again
    const timer = window.setTimeout(
      () => setNow(Date.now()),
      Math.min(Math.max(0, deadline - Date.now()), 2 ** 31 - 1),
    )
    return () => window.clearTimeout(timer)
  }, [deadline, hasPassed, now])

  return hasPassed
}
