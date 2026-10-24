import {
  createContext,
  useContext,
  useId,
  useState,
  type ComponentProps,
} from 'react'

const Validation = createContext<Record<string, string>>({})

function fieldError(input: HTMLInputElement) {
  if (input.disabled || !input.willValidate) return ''
  if (input.validity.valueMissing)
    return input.type === 'email'
      ? 'Enter your email.'
      : input.autocomplete === 'new-password'
        ? 'Enter a password.'
        : 'Enter your password.'
  if (input.validity.typeMismatch) return 'Enter a valid email address.'
  if (input.minLength > 0 && input.value.length < input.minLength)
    return `Use at least ${input.minLength} characters.`
  if (input.maxLength > 0 && input.value.length > input.maxLength)
    return `Use no more than ${input.maxLength} characters.`
  return input.validity.valid ? '' : 'Check this field.'
}

export function ValidatedForm({
  onSubmit,
  children,
  ...props
}: ComponentProps<'form'>) {
  const [errors, setErrors] = useState<Record<string, string>>({})
  return (
    <Validation.Provider value={errors}>
      <form
        {...props}
        noValidate
        onSubmit={(event) => {
          const inputs = Array.from(
            event.currentTarget.querySelectorAll('input'),
          )
          const next: Record<string, string> = {}
          let first: HTMLInputElement | undefined
          for (const input of inputs) {
            const error = fieldError(input)
            if (error) {
              next[input.name] = error
              first ??= input
            }
          }
          setErrors(next)
          if (first) {
            event.preventDefault()
            first.focus({ preventScroll: true })
          } else {
            onSubmit?.(event)
          }
        }}
        onBlur={(event) => {
          const input = event.target
          if (!(input instanceof HTMLInputElement) || !input.name) return
          if (input.value || errors[input.name])
            setErrors((previous) => ({
              ...previous,
              [input.name]: fieldError(input),
            }))
        }}
        onInput={(event) => {
          const input = event.target
          if (!(input instanceof HTMLInputElement) || !errors[input.name])
            return
          setErrors((previous) => ({
            ...previous,
            [input.name]: fieldError(input),
          }))
        }}
      >
        {children}
      </form>
    </Validation.Provider>
  )
}

export function ValidatedInput({
  hint,
  validHint,
  ...props
}: ComponentProps<'input'> & { hint?: string; validHint?: string }) {
  const errors = useContext(Validation)
  const [valid, setValid] = useState(false)
  const id = useId()
  const error = errors[props.name ?? '']
  return (
    <>
      <input
        {...props}
        onInput={(event) => {
          const input = event.currentTarget
          setValid(
            input.value.length > 0 &&
              input.value.length >= input.minLength &&
              input.validity.valid,
          )
          props.onInput?.(event)
        }}
        aria-invalid={error ? true : undefined}
        aria-describedby={
          [props['aria-describedby'], error || hint ? id : undefined]
            .filter(Boolean)
            .join(' ') || undefined
        }
      />
      {(error || hint) && (
        <span
          id={id}
          className={
            hint
              ? `field-feedback${error ? ' field-error' : ''}`
              : 'field-error'
          }
          role={error ? 'alert' : undefined}
        >
          {error || (valid && validHint ? validHint : hint)}
        </span>
      )}
    </>
  )
}
