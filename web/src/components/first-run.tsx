import type { ReactNode } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQuery } from '@tanstack/react-query'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { Icon } from '@astryxdesign/core/Icon'
import { Heading, Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'
import {
  borderVars,
  colorVars,
  spacingVars,
  typeScaleVars,
} from '@astryxdesign/core/theme/tokens.stylex'
import { Circle, CircleCheck } from 'lucide-react'

import { Panel } from '@/components/panel'
import { statusQuery } from '@/lib/whatsapp'

const styles = stylex.create({
  title: {
    fontSize: typeScaleVars['--text-heading-3-size'],
    lineHeight: typeScaleVars['--text-heading-3-leading'],
  },
  step: {
    borderTopColor: colorVars['--color-border'],
    borderTopStyle: 'solid',
    borderTopWidth: borderVars['--border-width'],
    paddingTop: spacingVars['--spacing-4'],
  },
  mark: { paddingTop: spacingVars['--spacing-0-5'] },
  text: { flexBasis: '18rem', flexGrow: 1, minWidth: 0 },
})

interface FirstRunProps {
  onNewMessage: () => void
  onSendNow: () => void
}

/** A new account's Messages screen: the two things that stand between it and a first send. */
export function FirstRun({ onNewMessage, onSendNow }: FirstRunProps) {
  const { data } = useQuery(statusQuery)
  const status = data?.status.status
  const isLinked =
    status !== undefined && status !== 'not_linked' && status !== 'pairing'
  const done = isLinked ? 1 : 0

  return (
    <Panel gap={4}>
      <HStack gap={3} justify="between" vAlign="center" wrap="wrap">
        <Heading level={2} xstyle={styles.title}>
          Get set up
        </Heading>
        <Text type="supporting" hasTabularNumbers>
          {done} of 2 done
        </Text>
      </HStack>
      <VStack as="ol" gap={4}>
        <Step
          isDone={isLinked}
          title="Link your WhatsApp"
          body="Messages go out from your own number, so Whatomato needs to be linked to it. It takes about a minute."
          action={
            isLinked ? null : (
              <Button
                label="Link WhatsApp"
                variant="primary"
                href="/whatsapp"
              />
            )
          }
        />
        <Step
          isDone={false}
          title="Schedule your first message"
          body="Pick who it goes to, what it says and when: once, or on a repeat."
          action={
            <HStack gap={2} wrap="wrap">
              <Button label="Send now" clickAction={onSendNow} />
              <Button
                label="New message"
                variant={isLinked ? 'primary' : 'secondary'}
                clickAction={onNewMessage}
              />
            </HStack>
          }
        />
      </VStack>
    </Panel>
  )
}

interface StepProps {
  isDone: boolean
  title: string
  body: string
  action: ReactNode
}

function Step({ isDone, title, body, action }: StepProps) {
  return (
    <HStack as="li" gap={4} vAlign="center" wrap="wrap" xstyle={styles.step}>
      <HStack gap={3} vAlign="start" xstyle={styles.text}>
        <HStack xstyle={styles.mark}>
          <Icon
            icon={isDone ? CircleCheck : Circle}
            color={isDone ? 'success' : 'secondary'}
            label={isDone ? 'Done' : 'To do'}
          />
        </HStack>
        <VStack gap={0.5}>
          <Text weight="semibold" color={isDone ? 'secondary' : 'primary'}>
            {title}
          </Text>
          <Text color="secondary">{body}</Text>
        </VStack>
      </HStack>
      {action}
    </HStack>
  )
}
