import { useId, useState } from 'react'
import { CodeBlock } from '@astryxdesign/core/CodeBlock'
import { Tab, TabList } from '@astryxdesign/core/TabList'
import { Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'

import { AGENT_CLIENTS, setupSteps } from '@/lib/agent-setup'
import type { AgentClientID } from '@/lib/agent-setup'

interface AgentSetupStepsProps {
  /** Where agents connect, from `endpointURL()`. */
  url: string
  /** Filled into the snippets when known; otherwise they show a placeholder. */
  accessKey?: string
}

/** Copy-paste steps for each agent, one at a time. */
export function AgentSetupSteps({ url, accessKey }: AgentSetupStepsProps) {
  const [client, setClient] = useState<AgentClientID>('claude-code')
  const steps = setupSteps(client, { url, accessKey })
  const panelID = useId()

  return (
    <VStack gap={5}>
      <TabList
        value={client}
        onChange={(value) => setClient(value as AgentClientID)}
        hasDivider
        role="tablist"
      >
        {AGENT_CLIENTS.map((item) => (
          <Tab
            key={item.id}
            value={item.id}
            label={item.name}
            panelId={panelID}
          />
        ))}
      </TabList>
      <VStack id={panelID} role="tabpanel">
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
                  title={step.file}
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
