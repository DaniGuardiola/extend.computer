import { QueryObserver } from '@tanstack/react-query'
import test from 'node:test'
import assert from 'node:assert/strict'
import { createQueryClient, resetSessionQueries } from '../src/lib/query'

test('query clients isolate requests and share concurrent reads within a client', async () => {
  const first = createQueryClient()
  const second = createQueryClient()
  let reads = 0
  const options = {
    queryKey: ['account'],
    queryFn: async () => {
      reads++
      await new Promise((resolve) => setTimeout(resolve, 5))
      return { id: 'first-user' }
    },
  }
  const [a, b] = await Promise.all([
    first.fetchQuery(options),
    first.fetchQuery(options),
  ])
  assert.deepEqual(a, b)
  assert.equal(reads, 1)
  assert.equal(second.getQueryData(['account']), undefined)
  first.clear()
  second.clear()
})

test('clearing a session cancels outstanding reads and cannot repopulate its cache', async () => {
  const client = createQueryClient()
  let signal: AbortSignal | undefined
  let complete!: (value: string) => void
  const pending = client
    .fetchQuery({
      queryKey: ['sessions'],
      queryFn: (context) => {
        signal = context.signal
        return new Promise<string>((resolve) => {
          complete = resolve
        })
      },
    })
    .catch(() => undefined)
  client.clear()
  assert.equal(signal?.aborted, true)
  complete('old-user-data')
  await pending
  assert.equal(client.getQueryData(['sessions']), undefined)
})

test('failed queries do not retry or refetch on focus/reconnect', async () => {
  const client = createQueryClient()
  let reads = 0
  await assert.rejects(
    client.fetchQuery({
      queryKey: ['failed'],
      queryFn: async () => {
        reads++
        throw new Error('Offline')
      },
    }),
    /Offline/,
  )
  assert.equal(reads, 1)
  const defaults = client.getDefaultOptions()
  assert.equal(defaults.queries?.refetchOnWindowFocus, false)
  assert.equal(defaults.queries?.refetchOnReconnect, false)
  assert.equal(defaults.mutations?.retry, false)
  client.clear()
})

test('sign-in resets active guest observers and drops inactive data from the previous identity', async () => {
  const client = createQueryClient()
  let identity: string | null = null
  const options = { queryKey: ['account'], queryFn: async () => identity }
  const observer = new QueryObserver(client, options)
  const unsubscribe = observer.subscribe(() => {})
  await client.fetchQuery(options)
  client.setQueryData(['devices'], ['previous-user-device'])
  identity = 'new-user'
  await resetSessionQueries(client)
  assert.equal(observer.getCurrentResult().data, 'new-user')
  assert.equal(client.getQueryData(['devices']), undefined)
  unsubscribe()
  client.clear()
})
