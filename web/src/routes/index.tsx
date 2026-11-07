import type { ReactNode } from 'react'
import * as stylex from '@stylexjs/stylex'
import { createFileRoute } from '@tanstack/react-router'
import { Avatar } from '@astryxdesign/core/Avatar'
import { Button } from '@astryxdesign/core/Button'
import { Card } from '@astryxdesign/core/Card'
import { Divider } from '@astryxdesign/core/Divider'
import { Grid } from '@astryxdesign/core/Grid'
import { HStack } from '@astryxdesign/core/HStack'
import { StatusDot } from '@astryxdesign/core/StatusDot'
import { Table, pixel, proportional } from '@astryxdesign/core/Table'
import { Heading, Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'
import {
  borderVars,
  colorVars,
  durationVars,
  easeVars,
  spacingVars,
  textSizeVars,
} from '@astryxdesign/core/theme/tokens.stylex'

import type { TableColumn } from '@astryxdesign/core/Table'

import { MessageBubble } from '@/components/message-bubble'
import { SiteFrame } from '@/components/site-frame'

export const Route = createFileRoute('/')({ component: Home })

const SIGNUP_HREF = '/signup'
const SIGNIN_HREF = '/signin'

interface Row extends Record<string, unknown> {
  id: string
  when: string
  to: string
  detail: string
  isGroup: boolean
  repeats: string
}

// The Messages screen as a new account sees it after a week: sample people, real product wording
const rows: Row[] = [
  {
    id: 'r1',
    when: 'Today, 09:45',
    to: 'Design team',
    detail: 'Group',
    isGroup: true,
    repeats: 'Weekdays at 09:45',
  },
  {
    id: 'r2',
    when: 'Today, 17:30',
    to: 'Karim Haddad',
    detail: '+44 7700 900789',
    isGroup: false,
    repeats: 'Once',
  },
  {
    id: 'r3',
    when: 'Tue, 12:00',
    to: 'Five-a-side',
    detail: 'Group',
    isGroup: true,
    repeats: 'Tuesdays at 12:00',
  },
  {
    id: 'r4',
    when: '28 Nov, 09:00',
    to: 'Amina Rahman',
    detail: '+44 7700 900123',
    isGroup: false,
    repeats: 'Monthly on the 28th',
  },
]

const capabilities = [
  {
    term: 'Once, or on a repeat',
    body: 'Pick a date and time, or repeat every day, on chosen weekdays, or once a month. Each message keeps the time zone it was written in.',
  },
  {
    term: 'Contacts and groups',
    body: 'Pick someone by the name saved on your phone, type any number on WhatsApp, or choose a group your number is in. A number that isn’t on WhatsApp is caught before anything goes out.',
  },
  {
    term: 'Templates',
    body: 'Save the text you send often, and start a message from it in one click. Write {{first_name}} or {{date}} and each message fills them in as it goes out.',
  },
  {
    term: 'History',
    body: 'Every send is logged with its outcome. Sent means WhatsApp acknowledged it; a failure says why.',
  },
]

const limits = [
  {
    term: 'It never reads your chats',
    body: 'Whatomato doesn’t download your chat history or store the messages you receive. It keeps the names and numbers of your saved contacts, encrypted, and deletes them when you unlink.',
  },
  {
    term: 'It sends at a human pace',
    body: 'At most one message every three seconds from your number, which keeps it looking like you.',
  },
  {
    term: 'It never sends twice',
    body: 'A send that may already have gone out is marked as failed, not tried again.',
  },
]

const steps = [
  {
    term: 'Link',
    body: 'Scan a QR code with WhatsApp, the same way you open WhatsApp Web. Your phone keeps working as usual.',
  },
  {
    term: 'Write',
    body: 'Choose who it goes to by name, write the message, and pick when: once, or on a repeat.',
  },
  {
    term: 'Leave it',
    body: 'It goes out on time from your number. History shows what was sent and what wasn’t.',
  },
]

const fadeUp = stylex.keyframes({
  from: { opacity: 0, transform: 'translateY(10px)' },
  to: { opacity: 1, transform: 'translateY(0)' },
})

const styles = stylex.create({
  defRow: {
    borderTopColor: colorVars['--color-border'],
    borderTopStyle: 'solid',
    borderTopWidth: {
      default: borderVars['--border-width'],
      ':first-child': 0,
    },
    columnGap: spacingVars['--spacing-8'],
    gridTemplateColumns: {
      default: 'minmax(0, 1fr)',
      '@media (min-width: 640px)': '220px minmax(0, 1fr)',
    },
    paddingBlock: spacingVars['--spacing-5'],
    rowGap: spacingVars['--spacing-2'],
  },
  stepGrid: {
    columnGap: spacingVars['--spacing-8'],
    gridTemplateColumns: {
      default: 'minmax(0, 1fr)',
      '@media (min-width: 720px)': 'repeat(3, minmax(0, 1fr))',
    },
    rowGap: spacingVars['--spacing-6'],
  },
  // `ch` resolves against the element's own font size, so this holds a 40-60
  // character line only while it sits on the text itself, never on a wrapper.
  measure: { maxWidth: '48ch' },
  headline: { maxWidth: '32ch' },
  wideOnly: {
    display: {
      default: 'none',
      '@media (min-width: 640px)': 'flex',
    },
  },
  narrowOnly: {
    display: {
      default: 'flex',
      '@media (min-width: 640px)': 'none',
    },
  },
  // Below 640 the trailing header action is the ghost button, whose own inline
  // padding would otherwise hold its label off the grid edge the wordmark sits on.
  headerActions: {
    marginInlineEnd: {
      default: `calc(${spacingVars['--spacing-3']} * -1)`,
      '@media (min-width: 640px)': 0,
    },
  },
  // display-1 tops out at 42px, which reads as a heading rather than a masthead
  // on a wide page; the step up stays derived from the type scale.
  heroTitle: {
    fontSize: {
      default: textSizeVars['--font-size-4xl'],
      '@media (min-width: 560px)': textSizeVars['--font-size-5xl'],
      '@media (min-width: 900px)': `calc(${textSizeVars['--font-size-5xl']} * 1.4)`,
      '@media (min-width: 1180px)': `calc(${textSizeVars['--font-size-5xl']} * 1.62)`,
    },
    letterSpacing: '-0.035em',
    lineHeight: 1.04,
    maxWidth: '14ch',
  },
  // The copy beside the preview on a wide page, above it on a narrow one
  hero: {
    alignItems: 'center',
    columnGap: spacingVars['--spacing-10'],
    gridTemplateColumns: {
      default: 'minmax(0, 1fr)',
      '@media (min-width: 1000px)': 'minmax(0, 1fr) minmax(0, 26rem)',
    },
    rowGap: spacingVars['--spacing-8'],
  },
  reveal: {
    animationDuration: durationVars['--duration-medium'],
    animationFillMode: 'both',
    animationName: {
      default: fadeUp,
      '@media (prefers-reduced-motion: reduce)': 'none',
    },
    animationTimingFunction: easeVars['--ease-standard'],
  },
  revealLate: { animationDelay: durationVars['--duration-fast-min'] },
  // Card's own background matches the body in dark mode, so the preview would
  // disappear into the page without this.
  panel: { backgroundColor: colorVars['--color-background-surface'] },
  chatTime: { alignSelf: 'flex-end' },
})

const columns: TableColumn<Row>[] = [
  {
    key: 'when',
    header: 'Next send',
    width: pixel(132),
    renderCell: (row) => (
      <Text hasTabularNumbers weight="medium" textWrap="nowrap">
        {row.when}
      </Text>
    ),
  },
  {
    key: 'to',
    header: 'To',
    width: proportional(1),
    renderCell: (row) => (
      <HStack gap={3} vAlign="center">
        <Avatar
          name={row.to}
          size="sm"
          shape={row.isGroup ? 'rounded' : 'circle'}
          tooltip={false}
        />
        <VStack>
          <Text weight="medium" maxLines={1}>
            {row.to}
          </Text>
          <Text type="supporting" hasTabularNumbers maxLines={1}>
            {row.detail}
          </Text>
        </VStack>
      </HStack>
    ),
  },
  {
    key: 'repeats',
    header: 'Repeats',
    width: proportional(1),
    renderCell: (row) => (
      <Text color="secondary" maxLines={1}>
        {row.repeats}
      </Text>
    ),
  },
]

const compactColumns: TableColumn<Row>[] = [
  {
    key: 'to',
    header: 'Next send',
    width: proportional(1),
    renderCell: (row) => (
      <VStack gap={0.5}>
        <HStack gap={3} justify="between" vAlign="center">
          <Text weight="medium" maxLines={1}>
            {row.to}
          </Text>
          <Text hasTabularNumbers weight="medium" textWrap="nowrap">
            {row.when}
          </Text>
        </HStack>
        <Text type="supporting">{row.repeats}</Text>
      </VStack>
    ),
  },
]

interface PageSectionProps {
  headingId: string
  title: string
  lede: string
  children: ReactNode
}

function PageSection({ headingId, title, lede, children }: PageSectionProps) {
  return (
    <VStack as="section" aria-labelledby={headingId} gap={8} paddingBlock={10}>
      <VStack gap={3}>
        <Heading
          id={headingId}
          level={2}
          type="display-3"
          textWrap="balance"
          xstyle={styles.headline}
        >
          {title}
        </Heading>
        <Text color="secondary" size="lg" xstyle={styles.measure}>
          {lede}
        </Text>
      </VStack>
      {children}
    </VStack>
  )
}

function DefinitionRows({
  items,
}: {
  items: { term: string; body: string }[]
}) {
  return (
    <VStack gap={0}>
      {items.map((item) => (
        <Grid key={item.term} xstyle={styles.defRow}>
          <Text type="label">{item.term}</Text>
          <Text color="secondary" xstyle={styles.measure}>
            {item.body}
          </Text>
        </Grid>
      ))}
    </VStack>
  )
}

/** The message as it lands in the chat, so the page shows the output and not only the tool. */
function ChatPreview() {
  const body =
    'Morning all, stand-up starts in 15 minutes. Post your update here if you can’t join.'
  return (
    <Card padding={5} xstyle={styles.panel} data-ui>
      <VStack gap={4}>
        <HStack gap={3} vAlign="center">
          <Avatar
            name="Design team"
            size="md"
            shape="rounded"
            tooltip={false}
          />
          <VStack>
            <Text weight="medium">Design team</Text>
            <Text type="supporting">Weekdays at 09:45, from your number</Text>
          </VStack>
        </HStack>
        <Divider isFullBleed />
        {['Monday', 'Tuesday', 'Wednesday'].map((day) => (
          <VStack key={day} gap={1}>
            <MessageBubble body={body} />
            <Text type="supporting" hasTabularNumbers xstyle={styles.chatTime}>
              {day}, 09:45
            </Text>
          </VStack>
        ))}
      </VStack>
    </Card>
  )
}

function Home() {
  return (
    <SiteFrame
      action={
        <HStack gap={2} vAlign="center" xstyle={styles.headerActions}>
          <Button label="Sign in" variant="ghost" href={SIGNIN_HREF} />
          <Button
            label="Create account"
            variant="secondary"
            href={SIGNUP_HREF}
            xstyle={styles.wideOnly}
          />
        </HStack>
      }
    >
      <VStack gap={0}>
        <VStack
          as="section"
          aria-labelledby="hero-title"
          paddingBlockEnd={10}
          paddingBlockStart={10}
        >
          <Grid xstyle={styles.hero}>
            <VStack gap={8}>
              <VStack gap={5} xstyle={styles.reveal}>
                <Heading
                  id="hero-title"
                  level={1}
                  type="display-1"
                  xstyle={styles.heroTitle}
                >
                  Your WhatsApp, on a schedule.
                </Heading>
                <Text color="secondary" size="lg" xstyle={styles.measure}>
                  Whatomato sends WhatsApp messages from your own number at the
                  time you choose: once, or every week, month or weekday. To
                  contacts and to groups.
                </Text>
              </VStack>
              <VStack
                gap={3}
                hAlign="start"
                xstyle={[styles.reveal, styles.revealLate]}
              >
                <Button
                  label="Create account"
                  variant="primary"
                  size="lg"
                  href={SIGNUP_HREF}
                />
                <Text type="supporting">
                  Free and open source. You link it by scanning a QR code, the
                  same way WhatsApp Web does.
                </Text>
              </VStack>
            </VStack>
            <VStack xstyle={[styles.reveal, styles.revealLate]}>
              <ChatPreview />
            </VStack>
          </Grid>
        </VStack>

        <Divider />

        <PageSection
          headingId="schedule"
          title="Everything that goes out next, in order."
          lede="The Messages screen lists what's due, soonest first, with who it goes to and how often it repeats. Switch one off, edit it, or send something right away."
        >
          <Card padding={4} xstyle={styles.panel} data-ui>
            <HStack justify="between" paddingBlockEnd={3} vAlign="center">
              <Text type="label">Messages</Text>
              <HStack gap={2} vAlign="center">
                <StatusDot
                  aria-hidden="true"
                  variant="success"
                  label="Connected"
                />
                <Text type="supporting">WhatsApp connected</Text>
              </HStack>
            </HStack>
            <VStack xstyle={styles.wideOnly}>
              <Divider isFullBleed />
              <Table
                data={rows}
                columns={columns}
                idKey="id"
                density="spacious"
              />
            </VStack>
            <VStack xstyle={styles.narrowOnly}>
              <Divider isFullBleed />
              <Table
                data={rows}
                columns={compactColumns}
                idKey="id"
                density="spacious"
              />
            </VStack>
            <Divider isFullBleed />
            <HStack justify="between" paddingBlockStart={3} vAlign="center">
              <Text type="supporting">4 scheduled</Text>
              <Text type="supporting" hasTabularNumbers>
                Next to Design team today at 09:45
              </Text>
            </HStack>
          </Card>
        </PageSection>

        <Divider />

        <PageSection
          headingId="capabilities"
          title="What it does."
          lede="Scheduling, and the few things around it that make scheduling dependable."
        >
          <DefinitionRows items={capabilities} />
        </PageSection>

        <Divider />

        <PageSection
          headingId="limits"
          title="What it won’t do."
          lede="It sends from your account, so it behaves the way you would."
        >
          <DefinitionRows items={limits} />
        </PageSection>

        <Divider />

        <PageSection
          headingId="how-it-works"
          title="Three steps, then it runs without you."
          lede="Linking takes about a minute. After that the schedule runs on its own, and every message stays yours to change or stop."
        >
          <Grid xstyle={styles.stepGrid}>
            {steps.map((step) => (
              <VStack key={step.term} gap={2}>
                <Text type="label">{step.term}</Text>
                <Text color="secondary">{step.body}</Text>
              </VStack>
            ))}
          </Grid>
        </PageSection>

        <Divider />

        <PageSection
          headingId="get-started"
          title="Link your account and schedule the first message."
          lede="Creating an account takes an email address and a password. You can connect WhatsApp straight after, or come back to it later."
        >
          <VStack gap={3} hAlign="start">
            <Button
              label="Create account"
              variant="primary"
              size="lg"
              href={SIGNUP_HREF}
            />
          </VStack>
        </PageSection>
      </VStack>
    </SiteFrame>
  )
}
