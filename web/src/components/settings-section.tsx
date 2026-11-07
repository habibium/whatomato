import type { ReactNode } from 'react'
import * as stylex from '@stylexjs/stylex'
import { Grid } from '@astryxdesign/core/Grid'
import { HStack } from '@astryxdesign/core/HStack'
import { Heading, Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'
import { typeScaleVars } from '@astryxdesign/core/theme/tokens.stylex'

import { Panel } from '@/components/panel'

const styles = stylex.create({
  // The title and what it's for on the left, the controls beside it; stacked on a phone
  row: {
    alignItems: 'start',
    gridTemplateColumns: {
      default: 'minmax(0, 1fr)',
      '@media (min-width: 960px)': 'minmax(0, 18rem) minmax(0, 1fr)',
    },
  },
  title: {
    fontSize: typeScaleVars['--text-heading-3-size'],
    lineHeight: typeScaleVars['--text-heading-3-leading'],
  },
})

interface SettingsSectionProps {
  id: string
  title: string
  description: ReactNode
  /** A status beside the title: "On", "Off". */
  status?: ReactNode
  children: ReactNode
}

export function SettingsSection({
  id,
  title,
  description,
  status,
  children,
}: SettingsSectionProps) {
  return (
    <VStack as="section" aria-labelledby={id}>
      <Grid columnGap={8} rowGap={4} xstyle={styles.row}>
        <VStack gap={1.5}>
          <HStack gap={2} vAlign="center">
            <Heading id={id} level={2} xstyle={styles.title}>
              {title}
            </Heading>
            {status}
          </HStack>
          <Text color="secondary" textWrap="pretty">
            {description}
          </Text>
        </VStack>
        <Panel gap={4}>{children}</Panel>
      </Grid>
    </VStack>
  )
}
