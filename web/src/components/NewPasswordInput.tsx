import type { ComponentProps } from 'react'
import { ValidatedInput } from './ValidatedForm'

type Props = Omit<
  ComponentProps<'input'>,
  'type' | 'autoComplete' | 'minLength' | 'maxLength' | 'required'
>

export function NewPasswordInput(props: Props) {
  return (
    <ValidatedInput
      {...props}
      type="password"
      autoComplete="new-password"
      required
      minLength={12}
      maxLength={1024}
      hint="At least 12 characters."
      validHint="Nice, that works!"
    />
  )
}
