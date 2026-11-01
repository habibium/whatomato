import { useId, useRef } from 'react'
import * as stylex from '@stylexjs/stylex'
import {
  Field,
  inputStatusBorderStyles,
  inputStatusFocusWithinStyles,
  inputStatusHoverShadowStyles,
  inputWrapperStyles,
} from '@astryxdesign/core/Field'
import { useInputContainer } from '@astryxdesign/core/hooks'
import { Icon } from '@astryxdesign/core/Icon'
import { mergeProps, themeProps } from '@astryxdesign/core/utils'
import {
  colorVars,
  sizeVars,
  typeScaleVars,
  typographyVars,
} from '@astryxdesign/core/theme/tokens.stylex'

// TextInput can set neither type="tel" nor inputMode, which bring up the
// dial pad on phones, so this restates its chrome the way PasswordField does.
const styles = stylex.create({
  shell: { height: sizeVars['--size-element-lg'] },
  input: {
    display: 'block',
    flexGrow: 1,
    flexShrink: 1,
    flexBasis: '0%',
    minWidth: 0,
    borderWidth: 0,
    borderStyle: 'none',
    padding: 0,
    outline: 'none',
    backgroundColor: 'transparent',
    color: colorVars['--color-text-primary'],
    fontFamily: typographyVars['--font-family-body'],
    fontVariantNumeric: 'tabular-nums',
    // Matches TextInput: on touch, below 16px iOS zooms the page on focus.
    fontSize: {
      default: typeScaleVars['--text-body-size'],
      '@media (pointer: coarse)': `max(1rem, ${typeScaleVars['--text-body-size']})`,
    },
    lineHeight: typeScaleVars['--text-body-leading'],
    '::placeholder': { color: colorVars['--color-text-secondary'] },
  },
})

interface PhoneNumberFieldProps {
  label: string
  placeholder?: string
  value: string
  onChange: (value: string) => void
  onBlur: () => void
  htmlName: string
  isInvalid?: boolean
  errorMessage?: string
  hasAutoFocus?: boolean
}

export function PhoneNumberField({
  label,
  placeholder,
  value,
  onChange,
  onBlur,
  htmlName,
  isInvalid = false,
  errorMessage,
  hasAutoFocus = false,
}: PhoneNumberFieldProps) {
  const inputID = useId()
  const messageID = useId()
  const inputRef = useRef<HTMLInputElement>(null)
  const shellRef = useRef<HTMLDivElement>(null)
  const status = isInvalid
    ? ({ type: 'error', message: errorMessage, messageID } as const)
    : undefined
  const { onClick, onMouseUp } = useInputContainer({
    containerRef: shellRef,
    inputRef,
  })

  return (
    <Field label={label} inputID={inputID} isRequired status={status}>
      <div
        ref={shellRef}
        onClick={onClick}
        onMouseUp={onMouseUp}
        // data-size is load-bearing: Field keys the attached message's overlap
        // off `:has(> [data-size])`, so without it the message sits too high.
        {...mergeProps(
          themeProps('text-input', {
            size: 'lg',
            status: status?.type ?? null,
          }),
          stylex.props(
            inputWrapperStyles.base,
            styles.shell,
            isInvalid && inputStatusBorderStyles.error,
            isInvalid && inputStatusHoverShadowStyles.error,
            isInvalid && inputStatusFocusWithinStyles.error,
          ),
        )}
      >
        <input
          {...stylex.props(styles.input)}
          ref={inputRef}
          id={inputID}
          name={htmlName}
          type="tel"
          inputMode="tel"
          autoComplete="tel"
          placeholder={placeholder}
          autoFocus={hasAutoFocus}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          onBlur={onBlur}
          aria-describedby={isInvalid && errorMessage ? messageID : undefined}
          aria-invalid={isInvalid || undefined}
          aria-required="true"
        />
        {isInvalid ? <Icon icon="error" color="error" /> : null}
      </div>
    </Field>
  )
}
