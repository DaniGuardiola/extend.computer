// Runs the real AccountService against the real D1 migrations in Node, using
// node:sqlite in place of D1 and Durable Object storage. Local only; no
// Cloudflare runtime, network or email.
import { registerHooks } from 'node:module'
import { DatabaseSync } from 'node:sqlite'
import { readFileSync, readdirSync } from 'node:fs'
import { createHash } from 'node:crypto'

registerHooks({
  resolve(specifier, context, next) {
    if (specifier === 'cloudflare:workers')
      return {
        url: 'data:text/javascript,export class DurableObject{constructor(ctx,env){this.ctx=ctx;this.env=env}}',
        shortCircuit: true,
      }
    return next(specifier, context)
  },
})

const migrations = new URL('../migrations/', import.meta.url)
type Value = string | number | null

class Statement {
  constructor(
    private db: DatabaseSync,
    private sql: string,
    private params: Value[] = [],
  ) {}
  bind(...params: unknown[]) {
    return new Statement(
      this.db,
      this.sql,
      params.map((value) =>
        typeof value === 'boolean' ? Number(value) : (value as Value),
      ),
    )
  }
  runSync() {
    const result = this.db.prepare(this.sql).run(...this.params)
    return {
      success: true,
      results: [],
      meta: { changes: Number(result.changes) },
    }
  }
  async first() {
    return this.db.prepare(this.sql).get(...this.params) ?? null
  }
  async all() {
    return {
      success: true,
      results: this.db.prepare(this.sql).all(...this.params),
    }
  }
  async run() {
    return this.runSync()
  }
}

// D1 enforces foreign keys and runs each batch as one transaction.
export function d1(db: DatabaseSync) {
  return {
    prepare: (sql: string) => new Statement(db, sql),
    async batch(statements: Statement[]) {
      db.exec('BEGIN')
      try {
        const results = statements.map((statement) => statement.runSync())
        db.exec('COMMIT')
        return results
      } catch (error) {
        db.exec('ROLLBACK')
        throw error
      }
    },
  }
}

export function migratedDatabase() {
  const db = new DatabaseSync(':memory:')
  db.exec('PRAGMA foreign_keys = ON')
  for (const name of readdirSync(migrations)
    .filter((f) => f.endsWith('.sql'))
    .sort())
    db.exec(readFileSync(new URL(name, migrations), 'utf8'))
  return db
}

export const digest = (value: string) =>
  createHash('sha256').update(value).digest('hex')

export async function startAccountService(vars: Record<string, string> = {}) {
  const { AccountService, heartbeat } =
    await import('../src/lib/account-service.ts')
  const db = migratedDatabase()
  const background: Promise<unknown>[] = []
  const objects = new Map<string, InstanceType<typeof AccountService>>()
  const storage = new Map<string, DatabaseSync>()
  const env = {
    DB: d1(db),
    SIGNUP_ENABLED: 'true',
    RELAY_ENABLED: 'false',
    PUBLIC_ORIGIN: 'https://extend.computer',
    ...vars,
    ACCOUNTS: {
      getByName(name: string) {
        let object = objects.get(name)
        if (!object) {
          const sql = new DatabaseSync(':memory:')
          storage.set(name, sql)
          const ctx = {
            storage: {
              sql: {
                exec: (query: string, ...bindings: Value[]) => {
                  const rows = sql.prepare(query).all(...bindings)
                  return { toArray: () => rows }
                },
              },
            },
            waitUntil: (promise: Promise<unknown>) => background.push(promise),
          }
          object = new AccountService(ctx as never, env as never)
          objects.set(name, object)
        }
        return object
      },
    },
  }
  return {
    db,
    env,
    heartbeat: (request: Request) => heartbeat(request, env as never),
    // The Worker shards account requests by IP; one shard is enough here.
    service: env.ACCOUNTS.getByName('auth-0'),
    storage: (name: string) => storage.get(name)!,
    settle: () => Promise.all(background.splice(0)),
  }
}

export type Harness = Awaited<ReturnType<typeof startAccountService>>
export const site = 'https://extend.computer'

/** Sends one API request to the service as a browser (cookie) or an app (token). */
export async function call(
  harness: Harness,
  path: string,
  {
    method = 'GET',
    data,
    token,
    cookie,
    origin = site,
    base = site,
  }: {
    method?: string
    data?: unknown
    token?: string
    cookie?: string
    origin?: string | null
    base?: string
  } = {},
) {
  const headers = new Headers({ 'User-Agent': 'Mozilla/5.0 Chrome/130' })
  if (origin) headers.set('Origin', origin)
  if (token) {
    headers.set('Authorization', 'Bearer ' + token)
    headers.set('X-Extend-Client', 'desktop')
  }
  if (cookie) headers.set('Cookie', 'extend_session=' + cookie)
  if (data !== undefined) headers.set('Content-Type', 'application/json')
  const response = await harness.service.fetch(
    new Request(base + '/api/v1' + path, {
      method,
      headers,
      body: data === undefined ? undefined : JSON.stringify(data),
    }),
  )
  return {
    status: response.status,
    cookie: response.headers.get('Set-Cookie'),
    data: response.status === 204 ? null : ((await response.json()) as any),
  }
}
