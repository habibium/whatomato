import type { ReactNode } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  useLocation,
  useNavigate,
  useRouteContext,
} from '@tanstack/react-router'
import { DropdownMenu } from '@astryxdesign/core/DropdownMenu'
import { HStack } from '@astryxdesign/core/HStack'
import { Icon } from '@astryxdesign/core/Icon'
import { StatusDot } from '@astryxdesign/core/StatusDot'
import { TopNavItem } from '@astryxdesign/core/TopNav'
import {
  colorVars,
  fontWeightVars,
} from '@astryxdesign/core/theme/tokens.stylex'

import { api } from '@/api/client'
import { SiteFrame } from '@/components/site-frame'
import { HOME_HREF, SIGNIN_HREF, setSessionUser } from '@/lib/session'
import { CONNECTION_DOTS, statusQuery } from '@/lib/whatsapp'

const WHATSAPP_HREF = '/whatsapp'

const PAGES = [
  { href: '/messages', label: 'Messages' },
  { href: '/templates', label: 'Templates' },
  { href: '/history', label: 'History' },
  { href: WHATSAPP_HREF, label: 'WhatsApp' },
] as const

const SETTINGS = { href: '/settings', label: 'Settings' } as const

const styles = stylex.create({
  // Reads as one more nav item, not as a button beside them
  account: {
    color: colorVars['--color-text-secondary'],
    fontWeight: fontWeightVars['--font-weight-medium'],
  },
  accountCurrent: {
    backgroundColor: colorVars['--color-neutral'],
    color: colorVars['--color-text-primary'],
    fontWeight: fontWeightVars['--font-weight-semibold'],
  },
  // The wordmark leaves room for one trigger on a phone, so below 640 the whole
  // nav folds into a menu named after the page you're on.
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
})

export function AppFrame({ children }: { children: ReactNode }) {
  return (
    <SiteFrame homeHref={HOME_HREF} action={<AppNav />}>
      {children}
    </SiteFrame>
  )
}

function AppNav() {
  const { user } = useRouteContext({ from: '/_authenticated' })
  const pathname = useLocation({ select: (location) => location.pathname })
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { data: whatsapp } = useQuery(statusQuery)
  const connection = whatsapp ? CONNECTION_DOTS[whatsapp.status.status] : null

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

  const toMenuItem = (page: { href: string; label: string }) => ({
    label: page.label,
    description: page.href === WHATSAPP_HREF ? connection?.label : undefined,
    endContent:
      page.href === pathname ? <Icon icon="check" size="sm" /> : undefined,
    onClick: () => void navigate({ href: page.href }),
  })
  const signOutItem = { label: 'Sign out', onClick: () => void signOut() }
  const current = [...PAGES, SETTINGS].find((page) => page.href === pathname)

  return (
    <>
      <HStack
        as="nav"
        aria-label="Main"
        gap={1}
        vAlign="center"
        xstyle={styles.wideOnly}
      >
        {PAGES.map((page) => (
          <TopNavItem
            key={page.href}
            label={page.label}
            href={page.href}
            isSelected={page.href === pathname}
            icon={
              page.href === WHATSAPP_HREF && connection ? (
                <StatusDot
                  variant={connection.variant}
                  label={`WhatsApp: ${connection.label}`}
                  tooltip={connection.label}
                  isPulsing={connection.isPulsing}
                />
              ) : undefined
            }
          />
        ))}
        <DropdownMenu
          button={{
            label: 'Account',
            variant: 'ghost',
            xstyle:
              pathname === SETTINGS.href
                ? styles.accountCurrent
                : styles.account,
          }}
          alignment="end"
          items={[
            {
              type: 'section',
              title: user.email,
              items: [toMenuItem(SETTINGS), signOutItem],
            },
          ]}
        />
      </HStack>
      <HStack xstyle={styles.narrowOnly}>
        <DropdownMenu
          button={{ label: current?.label ?? 'Menu', variant: 'ghost' }}
          alignment="end"
          presentation="adaptive"
          items={[
            ...PAGES.map(toMenuItem),
            { type: 'divider' },
            toMenuItem(SETTINGS),
            signOutItem,
          ]}
        />
      </HStack>
    </>
  )
}
