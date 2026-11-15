import { useRef } from 'react'
import { useBlocker } from '@tanstack/react-router'
import { AlertDialog } from '@astryxdesign/core/AlertDialog'
import { Button } from '@astryxdesign/core/Button'
import { CodeBlock } from '@astryxdesign/core/CodeBlock'
import { Divider } from '@astryxdesign/core/Divider'
import { HStack } from '@astryxdesign/core/HStack'
import { Heading, Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'

import { AgentSetupSteps } from '@/components/agent-setup-steps'
import { PageHeading } from '@/components/page-heading'
import { SettingsSection } from '@/components/settings-section'
import { endpointURL } from '@/lib/agents'
import type { AgentKeyIssued } from '@/lib/agents'

interface KeyIssuedProps {
  issued: AgentKeyIssued
  title: string
  lede: string
  /** Leaving on purpose; any other way out first asks whether the key was copied. */
  onDone: () => void
}

/** A freshly made access key, shown this once, with the steps that use it. */
export function KeyIssued({ issued, title, lede, onDone }: KeyIssuedProps) {
  const isDone = useRef(false)
  const blocker = useBlocker({
    shouldBlockFn: () => !isDone.current,
    enableBeforeUnload: () => !isDone.current,
    withResolver: true,
  })
  const { key } = issued

  const finish = () => {
    isDone.current = true
    onDone()
  }

  return (
    <VStack gap={8}>
      <PageHeading title={title} lede={lede} />

      <SettingsSection
        id="access-key"
        title="Access key"
        description="Copy it now. It won’t be shown again: Whatomato keeps only its last four characters. If it’s lost, make a new key from the connection’s page."
      >
        <CodeBlock
          code={key}
          language="plaintext"
          title="Access key"
          width="100%"
          isWrapped
        />
        <Text type="supporting">
          Anyone with this key can do what you allowed, so keep it out of chats,
          screenshots and shared files.
        </Text>
      </SettingsSection>

      <Divider />

      <VStack gap={4}>
        <VStack gap={1}>
          <Heading level={2}>Set up your agent</Heading>
          <Text color="secondary">
            Pick the agent you use. Each step is ready to paste, key included.
          </Text>
        </VStack>
        <AgentSetupSteps url={endpointURL()} accessKey={key} />
      </VStack>

      <Divider />

      <HStack gap={2} justify="end">
        <Button label="Done" variant="primary" clickAction={finish} />
      </HStack>

      <AlertDialog
        isOpen={blocker.status === 'blocked'}
        onOpenChange={(isOpen) => {
          if (!isOpen) blocker.reset?.()
        }}
        title="Leave without the key?"
        description="The access key won’t be shown again. If you haven’t copied it, stay and copy it now; otherwise you’ll need to make a new one."
        actionLabel="Leave"
        cancelLabel="Stay"
        onAction={() => blocker.proceed?.()}
      />
    </VStack>
  )
}
