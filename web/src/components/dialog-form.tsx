import type { ReactNode, Ref } from 'react'
import * as stylex from '@stylexjs/stylex'

const styles = stylex.create({
  // Dialog limits its child's height; without this the form grows past it and the footer scrolls away
  form: {
    display: 'flex',
    flexDirection: 'column',
    height: '100%',
    minHeight: 0,
  },
})

interface DialogFormProps {
  ref?: Ref<HTMLFormElement>
  onSubmit: () => void
  children: ReactNode
}

export function DialogForm({ ref, onSubmit, children }: DialogFormProps) {
  return (
    <form
      ref={ref}
      noValidate
      {...stylex.props(styles.form)}
      onSubmit={(event) => {
        event.preventDefault()
        onSubmit()
      }}
    >
      {children}
    </form>
  )
}
