import type { ReactNode } from 'react'
import * as stylex from '@stylexjs/stylex'
import { HStack } from '@astryxdesign/core/HStack'
import { Heading, Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'

const styles = stylex.create({
  measure: { maxWidth: '60ch' },
})

interface PageHeadingProps {
  title: string
  lede: ReactNode
  actions?: ReactNode
}

export function PageHeading({ title, lede, actions }: PageHeadingProps) {
  return (
    <HStack gap={6} justify="between" vAlign="end" wrap="wrap">
      <VStack gap={3}>
        <Heading level={1} type="display-3" textWrap="balance">
          {title}
        </Heading>
        <Text
          color="secondary"
          size="lg"
          textWrap="pretty"
          xstyle={styles.measure}
        >
          {lede}
        </Text>
      </VStack>
      {actions ? (
        <HStack gap={2} wrap="wrap">
          {actions}
        </HStack>
      ) : null}
    </HStack>
  )
}
