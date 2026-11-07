import { createFileRoute } from '@tanstack/react-router'
import { Divider } from '@astryxdesign/core/Divider'
import { HStack } from '@astryxdesign/core/HStack'
import { Link } from '@astryxdesign/core/Link'
import { MetadataList, MetadataListItem } from '@astryxdesign/core/MetadataList'
import { Text } from '@astryxdesign/core/Text'
import { Timestamp } from '@astryxdesign/core/Timestamp'
import { VStack } from '@astryxdesign/core/VStack'

import { AppFrame } from '@/components/app-frame'
import { PageHeading } from '@/components/page-heading'
import { PasskeysSection } from '@/components/passkeys-section'
import { SettingsSection } from '@/components/settings-section'
import { ThemeToggle } from '@/components/theme-toggle'
import { TwoFactorSection } from '@/components/two-factor-section'
import { SOURCE_URL } from '@/lib/source'

export const Route = createFileRoute('/_authenticated/settings')({
  component: Settings,
})

function Settings() {
  const { user } = Route.useRouteContext()

  return (
    <AppFrame>
      <VStack gap={8}>
        <PageHeading
          title="Settings"
          lede="Whoever gets into this account can send WhatsApp messages from your number, so a password alone shouldn't be enough."
        />

        <SettingsSection
          id="account"
          title="Account"
          description="The email you sign in with. Security alerts go to it too."
        >
          <MetadataList label={{ position: 'start', width: 144 }}>
            <MetadataListItem label="Email">
              <Text>{user.email}</Text>
            </MetadataListItem>
            <MetadataListItem label="Verified">
              {user.verified_at ? (
                <Timestamp
                  value={user.verified_at}
                  format="date_long"
                  type="body"
                  color="primary"
                />
              ) : (
                <Text color="secondary">Not yet</Text>
              )}
            </MetadataListItem>
            <MetadataListItem label="Member since">
              <Timestamp
                value={user.created_at}
                format="date_long"
                type="body"
                color="primary"
              />
            </MetadataListItem>
          </MetadataList>
        </SettingsSection>

        <Divider />
        <TwoFactorSection />
        <Divider />
        <PasskeysSection />
        <Divider />

        <SettingsSection
          id="appearance"
          title="Appearance"
          description="Light, dark, or whatever this device is set to."
        >
          <HStack>
            <ThemeToggle hasLabels />
          </HStack>
        </SettingsSection>

        <Divider />

        <SettingsSection
          id="about"
          title="About"
          description="Whatomato is free software under the GNU AGPL. You can read, run and change its code."
        >
          <MetadataList label={{ position: 'start', width: 144 }}>
            <MetadataListItem label="Version">
              <Text hasTabularNumbers>{__APP_VERSION__}</Text>
            </MetadataListItem>
            <MetadataListItem label="Source code">
              <Link href={SOURCE_URL} target="_blank" hasUnderline>
                View on GitHub
              </Link>
            </MetadataListItem>
          </MetadataList>
        </SettingsSection>
      </VStack>
    </AppFrame>
  )
}
