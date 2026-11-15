import { Fragment } from 'react'
import * as stylex from '@stylexjs/stylex'
import { Banner } from '@astryxdesign/core/Banner'
import { Text } from '@astryxdesign/core/Text'
import { VStack } from '@astryxdesign/core/VStack'
import {
  borderVars,
  colorVars,
  typographyVars,
  radiusVars,
  spacingVars,
  typeScaleVars,
} from '@astryxdesign/core/theme/tokens.stylex'

import { describeHidden } from '@/lib/hidden-characters'
import type { RevealedText as Revealed } from '@/lib/hidden-characters'

const styles = stylex.create({
  // Same pale bubble as MessageBubble, but never clipped: no max height, no line limit
  bubble: {
    backgroundColor: `color-mix(in srgb, ${colorVars['--color-background-green']} 55%, transparent)`,
    borderRadius: radiusVars['--radius-container'],
    borderStartEndRadius: radiusVars['--radius-inner'],
    paddingBlock: spacingVars['--spacing-3'],
    paddingInline: spacingVars['--spacing-4'],
  },
  text: { overflowWrap: 'anywhere', whiteSpace: 'pre-wrap' },
  mark: {
    backgroundColor: colorVars['--color-background-orange'],
    borderRadius: radiusVars['--radius-inner'],
    color: colorVars['--color-text-orange'],
    fontFamily: typographyVars['--font-family-code'],
    fontSize: typeScaleVars['--text-supporting-size'],
    marginInline: borderVars['--border-width'],
    paddingInline: spacingVars['--spacing-0-5'],
    whiteSpace: 'nowrap',
  },
  // A blank-line run is its own line, so the text after it stays where it is in the message
  block: { display: 'inline-block' },
})

interface RevealedTextProps {
  revealed: Revealed
  /** For a row in a list: no notice and no bubble, the marks still drawn. */
  isCompact?: boolean
}

/** A message as written, with each hidden or unusual character drawn as a mark. */
export function RevealedText({
  revealed,
  isCompact = false,
}: RevealedTextProps) {
  const text = (
    <Text type={isCompact ? 'supporting' : undefined} xstyle={styles.text}>
      {revealed.segments.map((segment, index) =>
        segment.kind === 'text' ? (
          <Fragment key={index}>{segment.text}</Fragment>
        ) : segment.kind === 'newline' ? (
          <Fragment key={index}>{'\n'}</Fragment>
        ) : (
          <Text
            key={index}
            role="img"
            aria-label={segment.label}
            xstyle={[
              styles.mark,
              segment.type === 'blank-lines' && styles.block,
            ]}
          >
            {segment.symbol}
          </Text>
        ),
      )}
    </Text>
  )
  if (isCompact) return text
  return (
    <VStack gap={3}>
      {revealed.total > 0 ? (
        <Banner
          status="warning"
          title={`This text holds ${revealed.total} hidden or unusual ${revealed.total === 1 ? 'character' : 'characters'}`}
          description={`${capitalise(describeHidden(revealed.counts))}, marked in orange below. Text can hide behind them, so read it all before you approve.`}
        />
      ) : null}
      <VStack xstyle={styles.bubble}>{text}</VStack>
    </VStack>
  )
}

function capitalise(text: string) {
  return text.charAt(0).toUpperCase() + text.slice(1)
}
