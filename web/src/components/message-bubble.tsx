import * as stylex from '@stylexjs/stylex'
import { Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'
import {
  colorVars,
  radiusVars,
  spacingVars,
} from '@astryxdesign/core/theme/tokens.stylex'

const styles = stylex.create({
  // Message text shown as the outgoing bubble it becomes in WhatsApp: pale green, one tight corner
  bubble: {
    backgroundColor: `color-mix(in srgb, ${colorVars['--color-background-green']} 55%, transparent)`,
    borderRadius: radiusVars['--radius-container'],
    borderStartEndRadius: radiusVars['--radius-inner'],
    paddingBlock: spacingVars['--spacing-2'],
    paddingInline: spacingVars['--spacing-3'],
  },
  text: { overflowWrap: 'anywhere', whiteSpace: 'pre-wrap' },
})

interface MessageBubbleProps {
  body: string
  maxLines?: number
}

export function MessageBubble({ body, maxLines }: MessageBubbleProps) {
  return (
    <VStack xstyle={styles.bubble}>
      <Text maxLines={maxLines} xstyle={styles.text}>
        {body}
      </Text>
    </VStack>
  )
}
