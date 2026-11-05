import type { ReactNode } from 'react'
import * as stylex from '@stylexjs/stylex'
import { Divider } from '@astryxdesign/core/Divider'
import { HStack } from '@astryxdesign/core/HStack'
import { Heading, Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'

const styles = stylex.create({
  measure: { maxWidth: '45ch' },
})

interface ListEmptyProps {
  title: string
  description: string
  actions: ReactNode
}

/** Astryx's EmptyState centres itself, which breaks from the page's left line. */
export function ListEmpty({ title, description, actions }: ListEmptyProps) {
  return (
    <VStack gap={6}>
      <Divider />
      <VStack gap={4} hAlign="start">
        <VStack gap={2}>
          <Heading level={2}>{title}</Heading>
          <Text color="secondary" xstyle={styles.measure}>
            {description}
          </Text>
        </VStack>
        <HStack gap={2} wrap="wrap">
          {actions}
        </HStack>
      </VStack>
    </VStack>
  )
}
