import { useCallback, useState } from 'react'

export function useErrorFeedback() {
  const [feedback, setFeedback] = useState({ error: '', attempt: 0 })
  const setError = useCallback((error: string) => {
    setFeedback((previous) => ({
      error,
      attempt: previous.attempt + (error ? 1 : 0),
    }))
  }, [])
  return [feedback.error, setError, feedback.attempt] as const
}

export function ErrorFeedback({
  error,
  attempt,
}: {
  error: string
  attempt: number
}) {
  return error ? (
    <p
      key={attempt}
      role="alert"
      className="message error-message auth-error-enter"
    >
      {error}
    </p>
  ) : null
}
