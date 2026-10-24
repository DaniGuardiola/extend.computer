import { useNavigate } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { useEffect } from 'react'
import { accountQuery, useClientReady } from './query'

export function useGuestRoute() {
  const navigate = useNavigate()
  const enabled = useClientReady()
  const session = useQuery({ ...accountQuery, enabled, staleTime: 0 })
  useEffect(() => {
    if (enabled && !session.isFetching && session.isSuccess && session.data)
      void navigate({ to: '/account', replace: true })
  }, [enabled, session.isFetching, session.isSuccess, session.data, navigate])
  return (
    enabled &&
    !session.isPending &&
    !session.isFetching &&
    (!session.data || session.isError)
  )
}
