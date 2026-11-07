import type { ReactNode } from 'react'
import * as stylex from '@stylexjs/stylex'
import { VStack } from '@astryxdesign/core/VStack'
import {
  borderVars,
  colorVars,
  radiusVars,
  spacingVars,
} from '@astryxdesign/core/theme/tokens.stylex'

const styles = stylex.create({
  panel: {
    borderColor: colorVars['--color-border'],
    borderRadius: radiusVars['--radius-container'],
    borderStyle: 'solid',
    borderWidth: borderVars['--border-width'],
    minWidth: 0,
    padding: {
      default: spacingVars['--spacing-4'],
      '@media (min-width: 640px)': spacingVars['--spacing-6'],
    },
  },
})

/** One block of work on the sheet: a 1px edge and no shadow, so it reads as part of the page. */
export function Panel({
  children,
  gap = 5,
}: {
  children: ReactNode
  gap?: 2 | 3 | 4 | 5 | 6
}) {
  return (
    <VStack gap={gap} xstyle={styles.panel}>
      {children}
    </VStack>
  )
}
