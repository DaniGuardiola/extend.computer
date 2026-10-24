import { testOrigin, createTestHeaders } from './test-config.mjs'
// Real capability detection and passwordless enforcement on the standalone server.
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from 'node:net'
const origin = testOrigin
test(
  'automatic-registration',
  { tag: '@integration' },
  async ({ newContext, newPage }) => {
    const socket = createServer()
    await new Promise((resolve) => socket.listen(0, '127.0.0.1', resolve))
    const port = socket.address().port
    await new Promise((resolve) => socket.close(resolve))
    const directory = await mkdtemp(join(tmpdir(), 'extend-registration-'))
    const server = spawn('../server/target/debug/extend-computer-server', [], {
      env: {
        ...process.env,
        EXTEND_BIND: `127.0.0.1:${port}`,
        EXTEND_ORIGIN: origin,
        EXTEND_SIGNUP_ENABLED: 'true',
        EXTEND_DATABASE: join(directory, 'account.sqlite3'),
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    try {
      await new Promise((resolve, reject) => {
        server.stdout.once('data', resolve)
        server.once('error', reject)
        server.once('exit', (code) =>
          reject(new Error(`Server exited: ${code}`)),
        )
      })
      const context = await newContext({
        extraHTTPHeaders: createTestHeaders(),
      })
      const page = await context.newPage()
      await page.goto(origin + '/login')
      const cdp = await context.newCDPSession(page)
      await cdp.send('WebAuthn.enable')
      const request = async (path, data, token, challenge) => {
        const response = await context.request.post(
          `http://127.0.0.1:${port}/v1${path}`,
          {
            headers: {
              Origin: origin,
              ...(token ? { Authorization: `Bearer ${token}` } : {}),
              ...(challenge ? { 'X-Extend-Challenge': challenge } : {}),
            },
            data,
          },
        )
        return {
          status: response.status(),
          body: await response.json(),
          challenge: response.headers()['x-extend-challenge'],
        }
      }
      const password = 'disposable-automatic-registration-password'
      const signup = await request('/auth/signup', {
        email: `automatic-${Date.now()}@example.invalid`,
        password,
      })
      assert.equal(signup.status, 201)
      let token = signup.body.token
      assert.equal(
        (await request('/mfa/reauth', { password }, token)).status,
        200,
      )
      const modern = await cdp.send('WebAuthn.addVirtualAuthenticator', {
        options: {
          protocol: 'ctap2',
          transport: 'internal',
          hasResidentKey: true,
          hasUserVerification: true,
          isUserVerified: true,
          automaticPresenceSimulation: true,
        },
      })
      const ceremony = async (register, verified = true) => {
        const path = register ? '/passkeys/register/' : '/passkeys/login/'
        const options = await request(
          path + 'options',
          register ? { auto: true } : {},
          register ? token : undefined,
        )
        assert.equal(options.status, 200)
        if (register) {
          assert.equal(
            options.body.authenticatorSelection.residentKey,
            'preferred',
          )
          assert.equal(
            options.body.authenticatorSelection.userVerification,
            'preferred',
          )
          assert.equal(
            options.body.authenticatorSelection.authenticatorAttachment,
            undefined,
          )
        }
        const credential = await page.evaluate(
          async ({ options, register, verified }) => {
            const decode = (value) =>
              Uint8Array.from(
                atob(value.replace(/-/g, '+').replace(/_/g, '/')),
                (c) => c.charCodeAt(0),
              )
            const publicKey = {
              ...options,
              challenge: decode(options.challenge),
            }
            if (register) {
              publicKey.user = { ...options.user, id: decode(options.user.id) }
              publicKey.excludeCredentials = options.excludeCredentials.map(
                (key) => ({ ...key, id: decode(key.id) }),
              )
            } else if (!verified) publicKey.userVerification = 'discouraged'
            return (
              await navigator.credentials[register ? 'create' : 'get']({
                publicKey,
              })
            ).toJSON()
          },
          { options: options.body, register, verified },
        )
        return request(
          path + 'verify',
          credential,
          register ? token : undefined,
          options.challenge,
        )
      }
      const loginKey = await ceremony(true)
      assert.equal(loginKey.status, 201, loginKey.body.error)
      assert.equal(loginKey.body.purpose, 'login')
      assert.equal(loginKey.body.recovery_codes.length, 10)
      const enabled = await context.request.get(
        `http://127.0.0.1:${port}/v1/mfa`,
        {
          headers: { Authorization: `Bearer ${token}` },
        },
      )
      assert.equal((await enabled.json()).enabled, true)
      await cdp.send('WebAuthn.setUserVerified', {
        authenticatorId: modern.authenticatorId,
        isUserVerified: false,
      })
      assert.equal((await ceremony(false, false)).status, 401)
      await cdp.send('WebAuthn.setUserVerified', {
        authenticatorId: modern.authenticatorId,
        isUserVerified: true,
      })
      const signedIn = await ceremony(false)
      assert.equal(signedIn.status, 200)
      const manageToken = signedIn.body.token
      const manage = await request('/mfa/reauth', { password }, manageToken)
      assert.equal(manage.body.mfa_required, true)
      const keyOptions = await request(
        '/auth/mfa/key/options',
        { ticket: manage.body.ticket },
        manageToken,
      )
      const proof = await page.evaluate(async (options) => {
        const decode = (value) =>
          Uint8Array.from(
            atob(value.replace(/-/g, '+').replace(/_/g, '/')),
            (c) => c.charCodeAt(0),
          )
        return (
          await navigator.credentials.get({
            publicKey: {
              ...options,
              challenge: decode(options.challenge),
              allowCredentials: options.allowCredentials.map((key) => ({
                ...key,
                id: decode(key.id),
              })),
            },
          })
        ).toJSON()
      }, keyOptions.body)
      assert.equal(
        (
          await request(
            '/auth/mfa/key/verify',
            { ...proof, ticket: manage.body.ticket },
            manageToken,
            keyOptions.challenge,
          )
        ).status,
        200,
      )
      assert.equal((await request('/mfa/disable', {}, manageToken)).status, 400)
      const removed = await context.request.delete(
        `http://127.0.0.1:${port}/v1/passkeys/${proof.id}`,
        {
          headers: { Authorization: `Bearer ${manageToken}`, Origin: origin },
          data: { sign_out_others: false },
        },
      )
      assert.equal(removed.status(), 204)
      const cleared = await context.request.get(
        `http://127.0.0.1:${port}/v1/mfa`,
        {
          headers: { Authorization: `Bearer ${manageToken}` },
        },
      )
      const clearedInfo = await cleared.json()
      assert.equal(clearedInfo.enabled, false)
      assert.equal(clearedInfo.keys, 0)
      assert.equal(clearedInfo.recovery_codes, 0)
      await cdp.send('WebAuthn.removeVirtualAuthenticator', {
        authenticatorId: modern.authenticatorId,
      })
      await cdp.send('WebAuthn.addVirtualAuthenticator', {
        options: {
          protocol: 'u2f',
          transport: 'usb',
          hasResidentKey: false,
          hasUserVerification: false,
          automaticPresenceSimulation: true,
        },
      })
      const legacy = await request('/auth/signup', {
        email: `legacy-${Date.now()}@example.invalid`,
        password,
      })
      assert.equal(legacy.status, 201)
      token = legacy.body.token
      assert.equal(
        (await request('/mfa/reauth', { password }, token)).status,
        200,
      )
      const factorKey = await ceremony(true)
      assert.equal(factorKey.status, 201, factorKey.body.error)
      assert.equal(factorKey.body.purpose, 'factor')
      assert.equal(factorKey.body.recovery_codes.length, 10)
      const legacyEnabled = await context.request.get(
        `http://127.0.0.1:${port}/v1/mfa`,
        { headers: { Authorization: `Bearer ${token}` } },
      )
      assert.equal((await legacyEnabled.json()).enabled, true)
      console.log(
        'Standalone auto registration accepts and classifies modern passkeys and legacy U2F keys; passwordless sign-in still requires verified user verification.',
      )
    } finally {
      server.kill('SIGINT')
      await new Promise((resolve) =>
        server.exitCode !== null ? resolve() : server.once('exit', resolve),
      )
      await rm(directory, { recursive: true, force: true })
    }
  },
)
