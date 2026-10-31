import * as stylex from '@stylexjs/stylex'
import { HStack } from '@astryxdesign/core/HStack'
import { Text } from '@astryxdesign/core/Text'
import { spacingVars } from '@astryxdesign/core/theme/tokens.stylex'

import logo from '@/assets/whatomato-mark.webp'

export const PRODUCT_NAME = 'Whatomato'

/** The source image is 96px square, enough for the large mark on a 3x screen */
const LOGO_PX = 96

interface WordmarkProps {
  size?: 'md' | 'lg'
}

const styles = stylex.create({
  // Astryx has no image component
  mark: { display: 'block', flexShrink: 0 },
  markMd: {
    height: spacingVars['--spacing-6'],
    width: spacingVars['--spacing-6'],
  },
  markLg: {
    height: spacingVars['--spacing-8'],
    width: spacingVars['--spacing-8'],
  },
  name: { letterSpacing: '-0.012em' },
})

export function Wordmark({ size = 'md' }: WordmarkProps) {
  const isLarge = size === 'lg'
  return (
    <HStack as="span" gap={2} vAlign="center">
      <img
        {...stylex.props(styles.mark, isLarge ? styles.markLg : styles.markMd)}
        src={logo}
        alt=""
        width={LOGO_PX}
        height={LOGO_PX}
      />
      <Text
        weight="semibold"
        size={isLarge ? 'lg' : undefined}
        textWrap="nowrap"
        xstyle={styles.name}
      >
        {PRODUCT_NAME}
      </Text>
    </HStack>
  )
}
