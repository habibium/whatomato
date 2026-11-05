import * as stylex from '@stylexjs/stylex'
import { createFileRoute } from '@tanstack/react-router'
import { Divider } from '@astryxdesign/core/Divider'
import { MetadataList, MetadataListItem } from '@astryxdesign/core/MetadataList'
import { Heading, Text } from '@astryxdesign/core/Text'
import { Timestamp } from '@astryxdesign/core/Timestamp'
import { VStack } from '@astryxdesign/core/VStack'

import { AppFrame } from '@/components/app-frame'
import { PasskeysSection } from '@/components/passkeys-section'
import { TwoFactorSection } from '@/components/two-factor-section'

export const Route = createFileRoute('/_authenticated/settings')({
  component: Settings,
})

const COLUMN_WIDTH = 560

const styles = stylex.create({
  measure: { maxWidth: '45ch' },
})

function Settings() {
  const { user } = Route.useRouteContext()

  return (
    <AppFrame>
      <VStack gap={8} maxWidth={COLUMN_WIDTH} paddingBlock={10}>
        <VStack gap={3}>
          <Heading level={1} type="display-3" textWrap="balance">
            Keep your account secure.
          </Heading>
          <Text color="secondary" size="lg" xstyle={styles.measure}>
            Whoever gets into this account can read and send your WhatsApp
            messages, so a password alone shouldn't be enough.
          </Text>
        </VStack>

        <Divider />

        <MetadataList
          title={<Heading level={2}>Your account</Heading>}
          label={{ position: 'start', width: 160 }}
        >
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

        <Divider />

        <TwoFactorSection />

        <Divider />

        <PasskeysSection />
      </VStack>
    </AppFrame>
  )
}
