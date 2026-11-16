import { useId, useState } from 'react'
import { CodeBlock } from '@astryxdesign/core/CodeBlock'
import { useMediaQuery } from '@astryxdesign/core/hooks'
import { Selector } from '@astryxdesign/core/Selector'
import { Tab, TabList } from '@astryxdesign/core/TabList'
import { Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'

import {
  AGENT_CLIENTS,
  SIGN_IN_CLIENTS,
  setupSteps,
  signInSteps,
} from '@/lib/agent-setup'
import type { AgentClientID, SignInClientID } from '@/lib/agent-setup'

interface AgentSetupStepsProps {
  /** Where agents connect, from `endpointURL()`. */
  url: string
  /** Filled into the snippets when known; otherwise they show a placeholder. */
  accessKey?: string
  /** The steps for apps that sign in and need no key, in place of those for a key. */
  isSigningIn?: boolean
}

/** Copy-paste steps for each agent, one at a time. */
export function AgentSetupSteps({
  url,
  accessKey,
  isSigningIn = false,
}: AgentSetupStepsProps) {
  const clients = isSigningIn ? SIGN_IN_CLIENTS : AGENT_CLIENTS
  const [client, setClient] = useState<string>(
    isSigningIn ? 'claude' : 'claude-code',
  )
  const steps = isSigningIn
    ? signInSteps(client as SignInClientID, url)
    : setupSteps(client as AgentClientID, { url, accessKey })
  const panelID = useId()
  // On a touch screen the scrolling tab strip shows no sign of the clients past the edge
  const isNarrow = useMediaQuery('(max-width: 639px)')

  return (
    <VStack gap={5}>
      {isNarrow ? (
        <Selector
          label="Agent"
          size="lg"
          width="100%"
          presentation="adaptive"
          options={clients.map((item) => ({
            value: item.id,
            label: item.name,
          }))}
          value={client}
          onChange={(value) => setClient(value)}
        />
      ) : (
        <TabList
          value={client}
          onChange={(value) => setClient(value)}
          hasDivider
          role="tablist"
        >
          {clients.map((item) => (
            <Tab
              key={item.id}
              value={item.id}
              label={item.name}
              panelId={panelID}
            />
          ))}
        </TabList>
      )}
      <VStack id={panelID} role={isNarrow ? undefined : 'tabpanel'}>
        <VStack as="ol" gap={5}>
          {steps.map((step, index) => (
            <VStack as="li" key={`${client}-${index}`} gap={2}>
              <Text>
                {steps.length > 1 ? `${index + 1}. ` : null}
                {step.text}
              </Text>
              {step.code ? (
                <CodeBlock
                  code={step.code}
                  language={step.language ?? 'plaintext'}
                  title={step.file ?? step.title}
                  hasLanguageLabel={false}
                  width="100%"
                  isWrapped
                />
              ) : null}
            </VStack>
          ))}
        </VStack>
      </VStack>
    </VStack>
  )
}
