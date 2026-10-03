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
    return input.type === 'email' ? 'Enter your email.' : 'Enter your password.'
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

export function ValidatedInput(props: ComponentProps<'input'>) {
  const errors = useContext(Validation)
  const id = useId()
  const error = errors[props.name ?? '']
  return (
    <>
      <input
        {...props}
        aria-invalid={error ? true : undefined}
        aria-describedby={
          [props['aria-describedby'], error ? id : undefined]
            .filter(Boolean)
            .join(' ') || undefined
        }
      />
      {error && (
        <span id={id} className="field-error" role="alert">
          {error}
        </span>
      )}
    </>
  )
}
