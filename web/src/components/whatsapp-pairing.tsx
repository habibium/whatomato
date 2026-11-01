import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'
import * as stylex from '@stylexjs/stylex'
import type { StyleXStyles } from '@stylexjs/stylex'
import { Button } from '@astryxdesign/core/Button'
import { Center } from '@astryxdesign/core/Center'
import { Grid } from '@astryxdesign/core/Grid'
import { HStack } from '@astryxdesign/core/HStack'
import { useClipboard } from '@astryxdesign/core/hooks'
import { Icon } from '@astryxdesign/core/Icon'
import { Spinner } from '@astryxdesign/core/Spinner'
import { Text } from '@astryxdesign/core/Text'
import { VisuallyHidden } from '@astryxdesign/core/VisuallyHidden'
import { VStack } from '@astryxdesign/core/VStack'
import {
  borderVars,
  colorVars,
  durationVars,
  easeVars,
  fontWeightVars,
  radiusVars,
  spacingVars,
  textSizeVars,
  typographyVars,
} from '@astryxdesign/core/theme/tokens.stylex'
import { Check, Copy } from 'lucide-react'

import type { Pairing } from '@/lib/whatsapp'
import { formatPairCode, formatPhoneNumber } from '@/lib/whatsapp'

export type LinkMethod = 'qr' | 'phone'

/** The size the server draws WhatsApp's codes at, so each 4px module lands on whole pixels. */
const QR_PX = 276

// The code sits on one line at line-height 1, so the tile can hold its height
// while it waits for one and never jumps when it arrives.
const CODE_SIZE = textSizeVars['--font-size-4xl']
const CODE_SIZE_WIDE = `calc(${textSizeVars['--font-size-5xl']} * 1.3)`
const TILE_PADDING = spacingVars['--spacing-6']

const fadeIn = stylex.keyframes({
  from: { opacity: 0 },
  to: { opacity: 1 },
})

const styles = stylex.create({
  // Side by side the code spans both rows, so Cancel sits right under the
  // steps; stacked on a phone it moves below the code.
  layout: {
    gridTemplateAreas: {
      default: '"steps" "code" "cancel"',
      '@media (min-width: 640px)': '"steps code" "cancel code"',
    },
    gridTemplateColumns: {
      default: 'minmax(0, 1fr)',
      '@media (min-width: 640px)': 'minmax(0, 1fr) auto',
    },
    gridTemplateRows: {
      default: 'auto',
      '@media (min-width: 640px)': 'auto 1fr',
    },
  },
  stepsArea: { gridArea: 'steps' },
  codeArea: { gridArea: 'code' },
  cancelArea: { gridArea: 'cancel' },
  step: {
    borderTopColor: colorVars['--color-border'],
    borderTopStyle: 'solid',
    borderTopWidth: {
      default: borderVars['--border-width'],
      ':first-child': 0,
    },
    paddingBlockEnd: spacingVars['--spacing-3'],
    paddingBlockStart: {
      default: spacingVars['--spacing-3'],
      ':first-child': 0,
    },
  },
  stepNumber: {
    flexShrink: 0,
    minWidth: spacingVars['--spacing-4'],
  },
  tile: {
    borderColor: colorVars['--color-border'],
    borderRadius: radiusVars['--radius-container'],
    borderStyle: 'solid',
    borderWidth: borderVars['--border-width'],
    maxWidth: '100%',
    transitionDuration: {
      default: '0s',
      '@media (prefers-reduced-motion: no-preference)':
        durationVars['--duration-medium'],
    },
    transitionProperty: 'background-color',
    transitionTimingFunction: easeVars['--ease-standard'],
  },
  // --color-on-dark is white in both schemes, as a phone needs dark on white to
  // scan; the SVG brings its own quiet zone, so the tile only frames it.
  qrTile: {
    backgroundColor: colorVars['--color-on-dark'],
    padding: spacingVars['--spacing-1'],
  },
  waitingTile: { backgroundColor: colorVars['--color-background-surface'] },
  qrBox: {
    aspectRatio: '1',
    height: 'auto',
    maxWidth: '100%',
    width: QR_PX,
  },
  qr: {
    animationDuration: durationVars['--duration-medium'],
    animationName: {
      default: 'none',
      '@media (prefers-reduced-motion: no-preference)': fadeIn,
    },
    animationTimingFunction: easeVars['--ease-standard'],
    display: 'block',
  },
  // The tile owns the type and the code inherits it: Text sets its own size,
  // and an override of that can lose to it on stylesheet order.
  codeTile: {
    backgroundColor: colorVars['--color-background-surface'],
    fontFamily: typographyVars['--font-family-code'],
    fontSize: {
      default: CODE_SIZE,
      '@media (min-width: 640px)': CODE_SIZE_WIDE,
    },
    fontVariantNumeric: 'tabular-nums slashed-zero',
    fontWeight: fontWeightVars['--font-weight-medium'],
    letterSpacing: '0.08em',
    lineHeight: 1,
    minHeight: {
      default: `calc(${CODE_SIZE} + 2 * (${TILE_PADDING} + ${borderVars['--border-width']}))`,
      '@media (min-width: 640px)': `calc(${CODE_SIZE_WIDE} + 2 * (${TILE_PADDING} + ${borderVars['--border-width']}))`,
    },
    padding: TILE_PADDING,
  },
  code: {
    animationDuration: durationVars['--duration-medium'],
    animationName: {
      default: 'none',
      '@media (prefers-reduced-motion: no-preference)': fadeIn,
    },
    animationTimingFunction: easeVars['--ease-standard'],
    // One click selects the whole code, dash and all
    userSelect: 'all',
    whiteSpace: 'nowrap',
  },
})

/** How this pairing works, from its code; before the first code, from what this tab asked for. */
export function pairingMethod(
  pairing: Pairing,
  requested: LinkMethod | null,
): LinkMethod | null {
  if (pairing.qr_code) return 'qr'
  if (pairing.pair_code) return 'phone'
  return requested
}

interface PairingPanelProps {
  pairing: Pairing
  method: LinkMethod | null
  clockOffset: number
  /** The digits this tab linked with; after a reload it isn't known. */
  phoneNumber: string | null
  onCancel: () => Promise<void>
}

export function PairingPanel({
  pairing,
  method,
  clockOffset,
  phoneNumber,
  onCancel,
}: PairingPanelProps) {
  const secondsLeft = useSecondsLeft(pairing.expires_at, clockOffset)
  // Past its expiry a code is dead, even before the next poll replaces it
  const isLive = secondsLeft !== 0
  const cancel = (
    <HStack xstyle={styles.cancelArea}>
      <Button label="Cancel" clickAction={onCancel} />
    </HStack>
  )

  if (method === 'qr') {
    return (
      <QrPairing
        qrCode={isLive ? pairing.qr_code : null}
        secondsLeft={secondsLeft}
        cancel={cancel}
      />
    )
  }
  if (method === 'phone') {
    return (
      <CodePairing
        code={isLive ? pairing.pair_code : null}
        secondsLeft={secondsLeft}
        phoneNumber={phoneNumber}
        cancel={cancel}
      />
    )
  }
  return (
    <VStack gap={6} hAlign="start">
      <HStack gap={3} vAlign="center">
        <Spinner size="md" />
        <Text color="secondary">Getting a code from WhatsApp…</Text>
      </HStack>
      {cancel}
    </VStack>
  )
}

interface MethodProps {
  secondsLeft: number | null
  cancel: ReactNode
}

function QrPairing({
  qrCode,
  secondsLeft,
  cancel,
}: MethodProps & { qrCode: string | null }) {
  return (
    <Grid align="start" columnGap={8} rowGap={6} xstyle={styles.layout}>
      <Steps xstyle={styles.stepsArea}>
        <Step number={1}>Open WhatsApp on your phone.</Step>
        <Step number={2}>
          Tap <UiName>Settings</UiName> on iPhone, or <UiName>⋮</UiName> on
          Android.
        </Step>
        <Step number={3}>
          Tap <UiName>Linked devices</UiName>, then{' '}
          <UiName>Link a device</UiName>.
        </Step>
        <Step number={4}>Point your phone at this code.</Step>
      </Steps>
      <VStack gap={3} hAlign="start" xstyle={styles.codeArea}>
        {qrCode ? (
          <Center xstyle={[styles.tile, styles.qrTile]}>
            <img
              // A new code fades in rather than swapping under a phone mid-scan
              key={qrCode}
              {...stylex.props(styles.qrBox, styles.qr)}
              src={qrCode}
              alt="QR code that links your WhatsApp to Whatomato"
              width={QR_PX}
              height={QR_PX}
            />
          </Center>
        ) : (
          <Center xstyle={[styles.tile, styles.qrTile, styles.waitingTile]}>
            <Center xstyle={styles.qrBox}>
              <Spinner size="lg" />
            </Center>
          </Center>
        )}
        <Expiry secondsLeft={secondsLeft} isWaiting={!qrCode} />
      </VStack>
      {cancel}
    </Grid>
  )
}

function CodePairing({
  code,
  secondsLeft,
  phoneNumber,
  cancel,
}: MethodProps & { code: string | null; phoneNumber: string | null }) {
  const { copy, isCopied } = useClipboard({ announce: 'Code copied' })

  return (
    <VStack gap={8}>
      <VStack gap={3}>
        <Center axis="vertical" xstyle={[styles.tile, styles.codeTile]}>
          {code ? (
            <>
              <Text
                key={code}
                type="inherit"
                aria-hidden="true"
                xstyle={styles.code}
              >
                {formatPairCode(code)}
              </Text>
              <VisuallyHidden>Code: {[...code].join(' ')}</VisuallyHidden>
            </>
          ) : (
            <Spinner size="lg" />
          )}
        </Center>
        <HStack gap={4} vAlign="center" wrap="wrap">
          <Button
            label={isCopied ? 'Copied' : 'Copy code'}
            icon={<Icon icon={isCopied ? Check : Copy} size="sm" />}
            isDisabled={!code}
            onClick={() => {
              if (code) void copy(code)
            }}
          />
          <Expiry secondsLeft={secondsLeft} isWaiting={!code} />
        </HStack>
      </VStack>
      <VStack gap={6}>
        <Steps>
          <Step number={1}>
            Open WhatsApp on{' '}
            {phoneNumber ? (
              <>
                the phone with{' '}
                <Text type="inherit" hasTabularNumbers>
                  {formatPhoneNumber(phoneNumber)}
                </Text>
              </>
            ) : (
              'your phone'
            )}
            .
          </Step>
          <Step number={2}>
            Tap <UiName>Settings</UiName> on iPhone, or <UiName>⋮</UiName> on
            Android.
          </Step>
          <Step number={3}>
            Tap <UiName>Linked devices</UiName>, then{' '}
            <UiName>Link a device</UiName>.
          </Step>
          <Step number={4}>
            Tap <UiName>Link with phone number instead</UiName>, and enter the
            code.
          </Step>
        </Steps>
        {cancel}
      </VStack>
    </VStack>
  )
}

function Expiry({
  secondsLeft,
  isWaiting,
}: {
  secondsLeft: number | null
  isWaiting: boolean
}) {
  if (isWaiting) {
    return <Text type="supporting">Getting a code from WhatsApp…</Text>
  }
  if (!secondsLeft) return null
  const minutes = Math.floor(secondsLeft / 60)
  const seconds = String(secondsLeft % 60).padStart(2, '0')
  return (
    <Text type="supporting" hasTabularNumbers>
      Expires in {minutes}:{seconds}
    </Text>
  )
}

function Steps({
  children,
  xstyle,
}: {
  children: ReactNode
  xstyle?: StyleXStyles
}) {
  return (
    <VStack as="ol" xstyle={xstyle}>
      {children}
    </VStack>
  )
}

function Step({ number, children }: { number: number; children: ReactNode }) {
  return (
    <HStack as="li" gap={3} xstyle={styles.step}>
      <Text
        aria-hidden="true"
        color="secondary"
        hasTabularNumbers
        xstyle={styles.stepNumber}
      >
        {number}
      </Text>
      <Text>{children}</Text>
    </HStack>
  )
}

/** A label on WhatsApp's own screens, set apart the way its help pages do. */
function UiName({ children }: { children: ReactNode }) {
  return (
    <Text type="inherit" weight="semibold">
      {children}
    </Text>
  )
}

/** Whole seconds until `expiresAt` on the server's clock; `null` without one. */
function useSecondsLeft(expiresAt: string | null, clockOffset: number) {
  const deadline =
    expiresAt === null ? null : Date.parse(expiresAt) - clockOffset
  const [now, setNow] = useState(Date.now)

  useEffect(() => {
    if (deadline === null || Number.isNaN(deadline)) return
    let timer: number | undefined
    // Wakes when the shown second changes, not on a free-running interval
    const tick = () => {
      const current = Date.now()
      setNow(current)
      if (current < deadline) {
        timer = window.setTimeout(tick, (deadline - current) % 1000 || 1000)
      }
    }
    tick()
    return () => window.clearTimeout(timer)
  }, [deadline])

  if (deadline === null || Number.isNaN(deadline)) return null
  return Math.max(0, Math.ceil((deadline - now) / 1000))
}
