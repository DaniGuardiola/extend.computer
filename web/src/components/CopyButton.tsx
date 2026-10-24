import { useState } from 'react'
import { Check, Copy } from 'lucide-react'

export function CopyButton({
  value,
  label = 'Copy',
  ariaLabel = label,
  className = 'text-button link-button',
  onError,
  showIcon = false,
}: {
  value: string
  label?: string
  ariaLabel?: string
  className?: string
  onError?: (failed: boolean) => void
  showIcon?: boolean
}) {
  const [copies, setCopies] = useState(0)
  return (
    <button
      type="button"
      className={className}
      aria-label={copies ? 'Copied!' : ariaLabel}
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(value)
          setCopies((count) => count + 1)
          onError?.(false)
        } catch {
          setCopies(0)
          onError?.(true)
        }
      }}
    >
      {copies > 0 && (
        <Check key={`check-${copies}`} size={16} aria-hidden="true" />
      )}
      {!copies && showIcon && <Copy size={16} aria-hidden="true" />}
      <span role="status" key={`label-${copies}`}>
        {copies ? 'Copied!' : label}
      </span>
    </button>
  )
}
