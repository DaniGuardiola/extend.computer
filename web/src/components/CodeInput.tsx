import { useEffect, useRef } from 'react'
import { OTPInput, REGEXP_ONLY_DIGITS } from 'input-otp'

export function CodeInput({
  disabled = false,
  length = 6,
  onComplete,
}: {
  disabled?: boolean
  length?: 6 | 8
  onComplete?: (code: string) => void
}) {
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (!disabled) input.current?.focus({ preventScroll: true })
  }, [disabled])
  return (
    <OTPInput
      ref={input}
      pushPasswordManagerStrategy="none"
      onComplete={onComplete}
      name="code"
      aria-label={length === 8 ? 'Recovery code' : 'One-time code'}
      containerClassName="otp-input"
      maxLength={length}
      minLength={length}
      pattern={REGEXP_ONLY_DIGITS}
      inputMode="numeric"
      autoComplete={length === 8 ? 'off' : 'one-time-code'}
      autoFocus
      required
      readOnly={disabled}
      aria-disabled={disabled}
      pasteTransformer={(value) => value.replace(/\s|-/g, '')}
      render={({ slots }) => (
        <div
          className={'otp-slots' + (length === 8 ? ' otp-slots-recovery' : '')}
          aria-hidden="true"
        >
          {(length === 8 ? [slots.slice(0, 4), slots.slice(4)] : [slots]).map(
            (group, groupIndex) => (
              <div className="otp-group" key={groupIndex}>
                {group.map((slot, index) => (
                  <div
                    key={index}
                    className={'otp-slot' + (slot.isActive ? ' active' : '')}
                  >
                    {slot.char}
                    {slot.hasFakeCaret && <span className="otp-caret" />}
                  </div>
                ))}
              </div>
            ),
          )}
        </div>
      )}
    />
  )
}
