export function SessionSignOutChoice({
  checked,
  onChange,
}: {
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <label className="session-sign-out-choice flex items-start gap-[10px] mt-5 cursor-pointer">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span>
        Sign out other sessions
        <small>
          Choose this if someone else may have accessed your account. This
          session stays signed in.
        </small>
      </span>
    </label>
  )
}
