import * as Ariakit from '@ariakit/react'
import { Fingerprint, KeyRound, Trash2 } from 'lucide-react'
import type { Passkey } from '../lib/api'

export function CredentialRow({
  credential,
  disabled,
  onRemove,
  onUsageChange,
}: {
  credential: Passkey
  disabled: boolean
  onRemove: (credential: Passkey) => void
  onUsageChange: (credential: Passkey) => void
}) {
  const factor = credential.purpose === 'factor'
  const label = factor ? 'Remove security key' : 'Remove passkey'
  return (
    <div className="security-row">
      <div>
        {factor ? <KeyRound size={19} /> : <Fingerprint size={19} />}
        <div>
          <h3>{credential.name}</h3>
          <p>
            {factor ? 'Second factor only · ' : ''}
            Added {new Date(credential.created_at * 1000).toLocaleDateString()}
            {credential.rp_id !== location.hostname
              ? ` · ${credential.rp_id}`
              : ''}
          </p>
          {!factor && (
            <label className="credential-usage flex items-center gap-2 mt-3 text-app-muted text-[12px]">
              <input
                type="checkbox"
                checked={credential.purpose === 'login'}
                disabled={disabled}
                aria-label={`Allow sign-in without password: ${credential.name}`}
                onChange={() => onUsageChange(credential)}
              />
              Allow sign-in without password
            </label>
          )}
        </div>
      </div>
      <Ariakit.TooltipProvider showTimeout={0} placement="left">
        <Ariakit.TooltipAnchor
          render={<button type="button" />}
          className="icon-button passkey-remove-desktop"
          aria-label={`${label}: ${credential.name}`}
          disabled={disabled}
          onClick={() => onRemove(credential)}
        >
          <Trash2 size={15} />
        </Ariakit.TooltipAnchor>
        <Ariakit.Tooltip
          className="account-tooltip"
          flip="right top bottom"
          overflowPadding={16}
        >
          <Ariakit.TooltipArrow />
          {label}: {credential.name}
        </Ariakit.Tooltip>
      </Ariakit.TooltipProvider>
      <button
        type="button"
        className="text-button passkey-remove-mobile"
        aria-label={`${label}: ${credential.name}`}
        disabled={disabled}
        onClick={() => onRemove(credential)}
      >
        {label}
      </button>
    </div>
  )
}
