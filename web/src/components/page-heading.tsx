import type { ReactNode } from 'react'
import * as stylex from '@stylexjs/stylex'
import { HStack } from '@astryxdesign/core/HStack'
import { Heading, Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'

const styles = stylex.create({
  measure: { maxWidth: '68ch' },
  text: { minWidth: 0 },
})

interface PageHeadingProps {
  title: string
  /** One line about the data on this page, never about the interface. */
  lede?: ReactNode
  actions?: ReactNode
}

/** The title row: a noun at 24px, the page's finding under it, its actions at the end. */
export function PageHeading({ title, lede, actions }: PageHeadingProps) {
  return (
    <HStack gap={4} justify="between" vAlign="center" wrap="wrap">
      <VStack gap={1} xstyle={styles.text}>
        <Heading level={1}>{title}</Heading>
        {lede ? (
          <Text color="secondary" textWrap="pretty" xstyle={styles.measure}>
            {lede}
          </Text>
        ) : null}
      </VStack>
      {actions ? (
        <HStack gap={2} wrap="wrap">
          {actions}
        </HStack>
      ) : null}
    </HStack>
  )
}
