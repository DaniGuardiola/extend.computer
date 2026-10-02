// Uses the isolated test Chromium; never the user's signed-in browser.
import { chromium } from '../../web/node_modules/playwright/index.mjs'
import assert from 'node:assert/strict'
const url = process.env.EXTEND_TEST_URL ?? 'http://localhost:8081'
const browser = process.env.EXTEND_TEST_CDP ? await chromium.connectOverCDP(process.env.EXTEND_TEST_CDP) : await chromium.launch({ headless: true })
const context = await browser.newContext()
const page = await context.newPage()
const cdp = await context.newCDPSession(page)
await cdp.send('WebAuthn.enable')
const { authenticatorId } = await cdp.send('WebAuthn.addVirtualAuthenticator', {
  options: { protocol: 'ctap2', transport: 'usb', hasResidentKey: false, hasUserVerification: false, isUserVerified: false, automaticPresenceSimulation: true },
})
await page.goto(url + '/healthz')
const results = await page.evaluate(async () => {
  const request = async (path, body, token, challenge) => {
    const response = await fetch('/v1' + path, {
      method: body === undefined ? 'GET' : 'POST',
      headers: { 'Content-Type': 'application/json', ...(token ? { Authorization: 'Bearer ' + token } : {}), ...(challenge ? { 'X-Extend-Challenge': challenge } : {}) },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
    return { status: response.status, data: await response.json().catch(() => null), challenge: response.headers.get('X-Extend-Challenge') }
  }
  const decode = s => Uint8Array.from(atob(s.replace(/-/g,'+').replace(/_/g,'/')), c=>c.charCodeAt(0))
  const encode = buffer => btoa(String.fromCharCode(...new Uint8Array(buffer))).replace(/\+/g,'-').replace(/\//g,'_').replace(/=+$/,'')
  const serialize = credential => ({ id: credential.id, rawId: encode(credential.rawId), type: credential.type, response: Object.fromEntries(['clientDataJSON','attestationObject','authenticatorData','signature','userHandle'].filter(k=>credential.response[k]).map(k=>[k,encode(credential.response[k])])), clientExtensionResults: credential.getClientExtensionResults() })
  const email = `mfa-key-${Date.now()}@example.invalid`
  const password = 'disposable-browser-password'
  const alice = await request('/auth/signup', {email,password})
  if(alice.status!==201)throw Error('Signup failed')
  const token=alice.data.token
  const reauth=await request('/mfa/reauth',{password},token)
  if(reauth.status!==200)throw Error('Reauth failed')
  const options=await request('/passkeys/register/options',{factor:true},token)
  options.data.challenge=decode(options.data.challenge);options.data.user.id=decode(options.data.user.id)
  options.data.excludeCredentials=(options.data.excludeCredentials??[]).map(c=>({...c,id:decode(c.id)}))
  const registration=serialize(await navigator.credentials.create({publicKey:options.data}))
  const registered=await request('/passkeys/register/verify',registration,token,options.challenge)
  if(registered.status!==201)throw Error('Key enrollment failed '+registered.status)
  const enabled=await request('/mfa/enable',{},token)
  if(enabled.status!==200)throw Error('Enable failed '+enabled.status)
  const login=await request('/auth/login',{email,password})
  if(!login.data.mfa_required||login.data.token)throw Error('Password bypassed MFA')
  const keyOptions=await request('/auth/mfa/key/options',{ticket:login.data.ticket})
  keyOptions.data.challenge=decode(keyOptions.data.challenge)
  keyOptions.data.allowCredentials=(keyOptions.data.allowCredentials??[]).map(c=>({...c,id:decode(c.id)}))
  const credential=serialize(await navigator.credentials.get({publicKey:keyOptions.data}))
  const body={ticket:login.data.ticket,...credential}
  const verified=await request('/auth/mfa/key/verify',body,undefined,keyOptions.challenge)
  const replay=await request('/auth/mfa/key/verify',body,undefined,keyOptions.challenge)
  return {registered:registered.status,enabled:enabled.status,factorRequired:login.data.mfa_required,verified:verified.status,token:!!verified.data.token,replay:replay.status,codes:enabled.data.recovery_codes.length}

})
assert.deepEqual(results,{registered:201,enabled:200,factorRequired:true,verified:200,token:true,replay:401,codes:10})
console.log(JSON.stringify({mfaSecurityKey:'passed',touchOnly:true,replayBlocked:true}))
await context.close();await browser.close()
