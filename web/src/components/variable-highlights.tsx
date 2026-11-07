import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { RefObject } from 'react'
import * as stylex from '@stylexjs/stylex'
import { colorVars, radiusVars } from '@astryxdesign/core/theme/tokens.stylex'

import { isVariable } from '@/lib/variables'

/** The textarea's padding box and type, so the layer's text lands exactly on the textarea's. */
interface Box {
  top: string
  left: string
  width: string
  height: string
  padding: string
  fontFamily: string
  fontSize: string
  fontWeight: string
  lineHeight: string
  letterSpacing: string
}

const styles = stylex.create({
  layer: (box: Box) => ({
    boxSizing: 'border-box',
    color: 'transparent',
    fontFamily: box.fontFamily,
    fontSize: box.fontSize,
    fontWeight: box.fontWeight,
    height: box.height,
    left: box.left,
    letterSpacing: box.letterSpacing,
    lineHeight: box.lineHeight,
    overflow: 'hidden',
    overflowWrap: 'break-word',
    padding: box.padding,
    pointerEvents: 'none',
    position: 'absolute',
    top: box.top,
    whiteSpace: 'pre-wrap',
    width: box.width,
  }),
  // A shadow, not padding, widens the tint, since padding would push the text out of line
  variable: {
    backgroundColor: `color-mix(in srgb, ${colorVars['--color-background-blue']} 45%, transparent)`,
    borderRadius: radiusVars['--radius-inner'],
    boxShadow: `0 0 0 2px color-mix(in srgb, ${colorVars['--color-background-blue']} 45%, transparent)`,
    color: colorVars['--color-text-blue'],
  },
  unknown: {
    backgroundColor: `color-mix(in srgb, ${colorVars['--color-background-red']} 45%, transparent)`,
    borderRadius: radiusVars['--radius-inner'],
    boxShadow: `0 0 0 2px color-mix(in srgb, ${colorVars['--color-background-red']} 45%, transparent)`,
    color: colorVars['--color-text-red'],
  },
})

interface VariableHighlightsProps {
  value: string
  textareaRef: RefObject<HTMLTextAreaElement | null>
  frameRef: RefObject<HTMLElement | null>
}

/**
 * Draws each `{{…}}` over the textarea in blue, or red when it isn't a variable. A textarea can't
 * colour parts of its own text, so this layer repeats the text in the same box and type, with
 * everything but the variables transparent, and scrolls with it.
 */
export function VariableHighlights({
  value,
  textareaRef,
  frameRef,
}: VariableHighlightsProps) {
  const layerRef = useRef<HTMLDivElement>(null)
  const [box, setBox] = useState<Box | null>(null)

  useLayoutEffect(() => {
    const textarea = textareaRef.current
    const frame = frameRef.current
    if (!textarea || !frame) return
    const measure = () => {
      const area = textarea.getBoundingClientRect()
      const outer = frame.getBoundingClientRect()
      const computed = getComputedStyle(textarea)
      setBox({
        top: `${area.top - outer.top + textarea.clientTop}px`,
        left: `${area.left - outer.left + textarea.clientLeft}px`,
        // The client box leaves out a scrollbar, which narrows the lines the same way
        width: `${textarea.clientWidth}px`,
        height: `${textarea.clientHeight}px`,
        padding: computed.padding,
        fontFamily: computed.fontFamily,
        fontSize: computed.fontSize,
        fontWeight: computed.fontWeight,
        lineHeight: computed.lineHeight,
        letterSpacing: computed.letterSpacing,
      })
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(textarea)
    observer.observe(frame)
    void document.fonts.ready.then(measure)
    return () => observer.disconnect()
    // The value too: a scrollbar appearing narrows the lines without resizing the textarea
  }, [textareaRef, frameRef, value])

  useEffect(() => {
    const textarea = textareaRef.current
    if (!textarea) return
    const follow = () => {
      if (layerRef.current) layerRef.current.scrollTop = textarea.scrollTop
    }
    follow()
    textarea.addEventListener('scroll', follow)
    return () => textarea.removeEventListener('scroll', follow)
  }, [textareaRef, value])

  if (!box || !value.includes('{{')) return null

  const parts = value.split(/(\{\{[^{}\n]*\}\})/)
  return (
    <div ref={layerRef} aria-hidden="true" {...stylex.props(styles.layer(box))}>
      {parts.map((part, index) =>
        index % 2 === 1 ? (
          <span
            key={index}
            {...stylex.props(
              isVariable(part) ? styles.variable : styles.unknown,
            )}
          >
            {part}
          </span>
        ) : (
          part
        ),
      )}
      {/* A final line break only takes up a line once something follows it */}
      {value.endsWith('\n') ? ' ' : null}
    </div>
  )
}
