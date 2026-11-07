import { useEffect, useId, useRef, useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { usePopover } from '@astryxdesign/core/Popover'
import { Text } from '@astryxdesign/core/Text'
import { TextArea } from '@astryxdesign/core/TextArea'
import { VStack } from '@astryxdesign/core/VStack'
import {
  colorVars,
  typographyVars,
  radiusVars,
  spacingVars,
} from '@astryxdesign/core/theme/tokens.stylex'

import {
  emptyVariables,
  hasVariables,
  insertVariable,
  matchingVariables,
  preview,
  variableAt,
} from '@/lib/variables'
import type { VariableContext } from '@/lib/variables'
import { VariableHighlights } from '@/components/variable-highlights'

const styles = stylex.create({
  list: {
    listStyle: 'none',
    margin: 0,
    maxHeight: '18rem',
    overflowY: 'auto',
    padding: spacingVars['--spacing-1'],
    width: 'min(28rem, 90vw)',
  },
  option: {
    borderRadius: radiusVars['--radius-element'],
    cursor: 'pointer',
    paddingBlock: spacingVars['--spacing-2'],
    paddingInline: spacingVars['--spacing-3'],
  },
  highlighted: { backgroundColor: colorVars['--color-overlay-hover'] },
  frame: { position: 'relative' },
  token: { fontFamily: typographyVars['--font-family-code'] },
})

interface MessageBodyFieldProps {
  label?: string
  rows?: number
  value: string
  onChange: (value: string) => void
  onBlur: () => void
  htmlName: string
  maxLength: number
  status?: { type: 'error'; message?: string }
  /** For each variable's example and the preview; without it only the hints show. */
  context?: VariableContext | null
}

/**
 * The message text. Typing `{{`, or putting the cursor inside `{{…}}`, lists the variables with
 * what each holds, filtered as the name is typed.
 */
export function MessageBodyField({
  label = 'Message',
  rows = 5,
  value,
  onChange,
  onBlur,
  htmlName,
  maxLength,
  status,
  context = null,
}: MessageBodyFieldProps) {
  const listID = useId()
  const textareaRef = useRef<HTMLTextAreaElement | null>(null)
  const frameRef = useRef<HTMLDivElement | null>(null)
  const [caret, setCaret] = useState<number | null>(null)
  const [highlighted, setHighlighted] = useState(0)
  // The `{{` whose list Escape closed; it stays closed until the cursor leaves it
  const [dismissed, setDismissed] = useState<number | null>(null)
  const popover = usePopover({
    role: 'none',
    hasAutoFocus: false,
    hasLightDismiss: false,
    hasCloseButton: false,
  })

  const open = caret === null ? null : variableAt(value, caret)
  const options = open ? matchingVariables(open.query) : []
  const isOpen = open !== null && options.length > 0 && dismissed !== open.start
  const active = Math.min(highlighted, Math.max(options.length - 1, 0))

  const pick = (name: string) => {
    const textarea = textareaRef.current
    if (!open || caret === null || !textarea) return
    const next = insertVariable(value, caret, open, name)
    onChange(next.text)
    setCaret(next.caret)
    // After React writes the new value, or the browser puts the cursor at the end
    requestAnimationFrame(() => {
      textarea.setSelectionRange(next.caret, next.caret)
    })
  }

  // Kept in a ref so the listener below, added once, always sees this render's state
  const keys = useRef<(event: KeyboardEvent) => void>(() => {})
  keys.current = (event) => {
    if (!isOpen || !open) return
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const step = event.key === 'ArrowDown' ? 1 : -1
      setHighlighted((active + step + options.length) % options.length)
    } else if (event.key === 'Enter' || event.key === 'Tab') {
      const option = options[active]
      if (!option) return
      event.preventDefault()
      pick(option.name)
    } else if (event.key === 'Escape') {
      // Closes the list only, not the dialog the field sits in
      event.preventDefault()
      event.stopPropagation()
      setDismissed(open.start)
    }
  }

  useEffect(() => {
    const textarea = textareaRef.current
    if (!textarea) return
    const track = () => {
      setCaret(
        textarea.selectionStart === textarea.selectionEnd
          ? textarea.selectionStart
          : null,
      )
    }
    const onKeyDown = (event: KeyboardEvent) => keys.current(event)
    const onBlurred = () => setCaret(null)
    // Not `input`: a render from there lands before React applies the keystroke, and React then
    // puts the old text back. Typing moves the cursor through onChange below instead.
    const events = ['keyup', 'click', 'focus', 'select'] as const
    for (const name of events) textarea.addEventListener(name, track)
    textarea.addEventListener('keydown', onKeyDown)
    textarea.addEventListener('blur', onBlurred)
    return () => {
      for (const name of events) textarea.removeEventListener(name, track)
      textarea.removeEventListener('keydown', onKeyDown)
      textarea.removeEventListener('blur', onBlurred)
    }
  }, [])

  // A new `{{`, or more of a name typed, starts the list again: on the variable the cursor is in,
  // else at the top
  const start = open?.start
  const query = open?.query
  const current = open?.current
  useEffect(() => {
    if (query === undefined) return
    const index = matchingVariables(query).findIndex(
      (option) => option.name === current,
    )
    setHighlighted(Math.max(index, 0))
  }, [start, query, current])

  useEffect(() => {
    if (dismissed !== null && open?.start !== dismissed) setDismissed(null)
  }, [open?.start, dismissed])

  const { show, hide } = popover
  useEffect(() => {
    if (isOpen) show({ skipAutoFocus: true })
    else hide()
  }, [isOpen, show, hide])

  // The textarea is Astryx's, so the combobox wiring goes onto it directly
  useEffect(() => {
    const textarea = textareaRef.current
    if (!textarea) return
    textarea.setAttribute('aria-autocomplete', 'list')
    textarea.setAttribute('aria-expanded', String(isOpen))
    textarea.setAttribute('aria-controls', listID)
    const option = isOpen ? options[active] : undefined
    if (option) {
      textarea.setAttribute('aria-activedescendant', `${listID}-${option.name}`)
    } else {
      textarea.removeAttribute('aria-activedescendant')
    }
  })

  const shown = context && hasVariables(value) ? preview(value, context) : null
  const [empty] = context ? emptyVariables(value, context) : []

  return (
    <VStack gap={2}>
      <VStack ref={frameRef} xstyle={styles.frame}>
        <TextArea
          ref={(element: HTMLTextAreaElement | null) => {
            textareaRef.current = element
            popover.triggerRef(element)
          }}
          label={label}
          description="Type {{ to add their name, the date or the time, filled in as it goes out."
          size="lg"
          rows={rows}
          maxLength={maxLength}
          htmlName={htmlName}
          value={value}
          onChange={(next, event) => {
            onChange(next)
            setCaret(event.target.selectionStart)
          }}
          onBlur={onBlur}
          status={status}
        />
        <VariableHighlights
          value={value}
          textareaRef={textareaRef}
          frameRef={frameRef}
        />
      </VStack>
      {shown !== null ? (
        <Text type="supporting" maxLines={3}>
          Goes out as “{shown.trim()}”
        </Text>
      ) : null}
      {empty ? (
        <Text type="supporting">
          {`{{${empty}}} would be empty for this recipient. Write {{${empty}|there}} to say “there” instead.`}
        </Text>
      ) : null}
      {popover.render(
        <VStack
          as="ul"
          role="listbox"
          id={listID}
          aria-label="Variables"
          xstyle={styles.list}
        >
          {options.map((option, index) => {
            const example = context ? option.value(context) : null
            return (
              <VStack
                as="li"
                key={option.name}
                id={`${listID}-${option.name}`}
                role="option"
                aria-selected={index === active}
                gap={0.5}
                xstyle={[styles.option, index === active && styles.highlighted]}
                // Keeps focus, and the cursor, in the textarea
                onMouseDown={(event: React.MouseEvent) => {
                  event.preventDefault()
                  pick(option.name)
                }}
                onMouseEnter={() => setHighlighted(index)}
              >
                <Text weight="medium" xstyle={styles.token}>
                  {`{{${option.name}}}`}
                </Text>
                <Text type="supporting">
                  {example
                    ? `${example} · ${option.hint}`
                    : capitalise(option.hint)}
                </Text>
              </VStack>
            )
          })}
        </VStack>,
        { placement: 'below', alignment: 'start' },
      )}
    </VStack>
  )
}

function capitalise(text: string) {
  return text.charAt(0).toUpperCase() + text.slice(1)
}
