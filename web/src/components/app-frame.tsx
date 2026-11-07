import type { ReactNode } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  useLocation,
  useNavigate,
  useRouteContext,
} from '@tanstack/react-router'
import { AppShell } from '@astryxdesign/core/AppShell'
import { Avatar } from '@astryxdesign/core/Avatar'
import { DropdownMenu } from '@astryxdesign/core/DropdownMenu'
import { HStack } from '@astryxdesign/core/HStack'
import { Link } from '@astryxdesign/core/Link'
import {
  SideNav,
  SideNavItem,
  SideNavSection,
} from '@astryxdesign/core/SideNav'
import { StatusDot } from '@astryxdesign/core/StatusDot'
import { Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'
import {
  colorVars,
  radiusVars,
  spacingVars,
} from '@astryxdesign/core/theme/tokens.stylex'
import {
  CalendarClock,
  FileText,
  History,
  Settings,
  Smartphone,
} from 'lucide-react'

import { api } from '@/api/client'
import { Wordmark } from '@/components/wordmark'
import { HOME_HREF, SIGNIN_HREF, setSessionUser } from '@/lib/session'
import { CONNECTION_DOTS, formatPhoneNumber, statusQuery } from '@/lib/whatsapp'

const WORK = [
  { href: '/messages', label: 'Messages', icon: CalendarClock },
  { href: '/templates', label: 'Templates', icon: FileText },
  { href: '/history', label: 'History', icon: History },
] as const

// AppShell's own `md` breakpoint, where the side nav folds into a drawer
const WIDE = '@media (min-width: 768px)'

const styles = stylex.create({
  // The frame is the wash; the work sits on one sheet a tone step lighter. On a phone the sheet is the screen.
  sheet: {
    backgroundColor: colorVars['--color-background-surface'],
    borderColor: {
      default: 'transparent',
      [WIDE]: colorVars['--color-border'],
    },
    borderRadius: { default: 0, [WIDE]: radiusVars['--radius-container'] },
    borderStyle: 'solid',
    borderWidth: { default: 0, [WIDE]: 1 },
    marginBlock: { default: 0, [WIDE]: spacingVars['--spacing-2'] },
    marginInlineEnd: { default: 0, [WIDE]: spacingVars['--spacing-2'] },
    // Below WIDE the shell's own 48px bar sits above the sheet
    minHeight: {
      default: `calc(100dvh - ${spacingVars['--spacing-12']})`,
      [WIDE]: `calc(100dvh - 2 * ${spacingVars['--spacing-2']})`,
    },
  },
  // A cap keeps rows readable on very wide screens
  content: {
    marginInline: 'auto',
    maxWidth: 1200,
    paddingBlock: {
      default: spacingVars['--spacing-5'],
      '@media (min-width: 768px)': spacingVars['--spacing-8'],
    },
    paddingInline: {
      default: spacingVars['--spacing-4'],
      '@media (min-width: 768px)': spacingVars['--spacing-8'],
    },
    width: '100%',
  },
  brand: { textDecoration: 'none' },
  // A row like the nav items above it, not a centred button
  account: { justifyContent: 'flex-start', minWidth: 0, width: '100%' },
})

export function AppFrame({ children }: { children: ReactNode }) {
  return (
    <AppShell variant="wash" height="auto" sideNav={<AppNav />}>
      <VStack xstyle={styles.sheet}>
        <VStack xstyle={styles.content}>{children}</VStack>
      </VStack>
    </AppShell>
  )
}

function AppNav() {
  const { user } = useRouteContext({ from: '/_authenticated' })
  const pathname = useLocation({ select: (location) => location.pathname })
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { data: whatsapp } = useQuery(statusQuery)
  const status = whatsapp?.status
  const connection = status ? CONNECTION_DOTS[status.status] : null
  const linkedNumber =
    status && 'phone_number' in status && status.phone_number
      ? formatPhoneNumber(status.phone_number)
      : null

  const signOut = async () => {
    try {
      await api.POST('/api/auth/signout')
    } finally {
      // The next person to sign in here mustn't see these messages, even briefly
      queryClient.clear()
      setSessionUser(queryClient, null)
      await navigate({ href: SIGNIN_HREF })
    }
  }

  return (
    <SideNav
      header={
        <VStack gap={1} paddingBlock={2} paddingInline={2}>
          <Link href={HOME_HREF} color="primary" xstyle={styles.brand}>
            <Wordmark />
          </Link>
        </VStack>
      }
      footer={
        <VStack paddingInline={2} paddingBlock={2}>
          <DropdownMenu
            button={{
              label: user.email,
              variant: 'ghost',
              icon: <Avatar name={user.email} size="xsm" tooltip={false} />,
              xstyle: styles.account,
            }}
            placement="above"
            alignment="start"
            items={[
              {
                label: 'Settings',
                onClick: () => void navigate({ href: '/settings' }),
              },
              { label: 'Sign out', onClick: () => void signOut() },
            ]}
          />
        </VStack>
      }
    >
      <SideNavSection title="Messages" isHeaderHidden>
        {WORK.map((page) => (
          <SideNavItem
            key={page.href}
            label={page.label}
            href={page.href}
            icon={page.icon}
            isSelected={page.href === pathname}
          />
        ))}
      </SideNavSection>
      <SideNavSection title="Account">
        <SideNavItem
          label="WhatsApp"
          href="/whatsapp"
          icon={Smartphone}
          isSelected={pathname === '/whatsapp'}
          endContent={
            connection ? (
              <HStack gap={1.5} vAlign="center">
                <StatusDot
                  variant={connection.variant}
                  label={`WhatsApp: ${connection.label}`}
                  isPulsing={connection.isPulsing}
                />
              </HStack>
            ) : undefined
          }
        />
        <SideNavItem
          label="Settings"
          href="/settings"
          icon={Settings}
          isSelected={pathname === '/settings'}
        />
      </SideNavSection>
      {linkedNumber ? (
        <VStack paddingInline={3} paddingBlock={2}>
          <Text type="supporting">Sending from {linkedNumber}</Text>
        </VStack>
      ) : null}
    </SideNav>
  )
}
