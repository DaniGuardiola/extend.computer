// Uses the isolated test Chromium; never the user's signed-in browser.
import { chromium } from '../../web/node_modules/playwright/index.mjs'
import assert from 'node:assert/strict'
const url = process.env.EXTEND_TEST_URL ?? 'http://localhost:8081'
const browser = await chromium.connectOverCDP(process.env.EXTEND_TEST_CDP)
const context = await browser.newContext()
const page = await context.newPage()
const cdp = await context.newCDPSession(page)
await cdp.send('WebAuthn.enable')
const { authenticatorId } = await cdp.send('WebAuthn.addVirtualAuthenticator', {
  options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true },
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
  const suffix = Date.now()
  const alice = await request('/auth/signup', { email: `passkey-a-${suffix}@example.invalid`, password: 'disposable-browser-password' })
  const bob = await request('/auth/signup', { email: `passkey-b-${suffix}@example.invalid`, password: 'disposable-browser-password' })
  if(alice.status!==201||bob.status!==201)throw Error('Signup failed')
  const options = await request('/passkeys/register/options', {}, alice.data.token)
  const publicKey = options.data
  publicKey.challenge=decode(publicKey.challenge);publicKey.user.id=decode(publicKey.user.id)
  publicKey.excludeCredentials=(publicKey.excludeCredentials??[]).map(c=>({...c,id:decode(c.id)}))
  const registration = serialize(await navigator.credentials.create({publicKey}))
  const registered = await request('/passkeys/register/verify', registration, alice.data.token, options.challenge)
  if(registered.status!==201)throw Error('Registration failed: '+registered.status+' '+JSON.stringify(registered.data))
  const replay = await request('/passkeys/register/verify', registration, alice.data.token, options.challenge)
  const privateList = await request('/passkeys', undefined, bob.data.token)
  const ownList = await request('/passkeys', undefined, alice.data.token)
  const auth = await request('/passkeys/login/options', {})
  auth.data.challenge=decode(auth.data.challenge)
  auth.data.allowCredentials=(auth.data.allowCredentials??[]).map(c=>({...c,id:decode(c.id)}))
  const assertion = serialize(await navigator.credentials.get({publicKey:auth.data}))
  const loggedIn = await request('/passkeys/login/verify', assertion, undefined, auth.challenge)
  const loginReplay = await request('/passkeys/login/verify', assertion, undefined, auth.challenge)
  const credentialId=ownList.data.passkeys[0]?.id
  const crossRemoval=await fetch('/v1/passkeys/'+credentialId,{method:'DELETE',headers:{Authorization:'Bearer '+bob.data.token}})
  const removal=await fetch('/v1/passkeys/'+credentialId,{method:'DELETE',headers:{Authorization:'Bearer '+alice.data.token}})
  const retry=await request('/passkeys/login/options',{})
  retry.data.challenge=decode(retry.data.challenge)
  const removedAssertion=serialize(await navigator.credentials.get({publicKey:retry.data}))
  const removedLogin=await request('/passkeys/login/verify',removedAssertion,undefined,retry.challenge)
  return { registered:registered.status, replay:replay.status, otherKeys:privateList.data.passkeys.length, ownKeys:ownList.data.passkeys.length, login:loggedIn.status, account:loggedIn.data.account?.id===alice.data.account.id, loginReplay:loginReplay.status, crossRemoval:crossRemoval.status, removal:removal.status, removedLogin:removedLogin.status }
})
assert.deepEqual(results,{registered:201,replay:401,otherKeys:0,ownKeys:1,login:200,account:true,loginReplay:401,crossRemoval:404,removal:204,removedLogin:401})
const wrongOrigin = await context.request.post(url+'/v1/passkeys/login/options',{headers:{Origin:'https://attacker.invalid'},data:{}})
assert.equal(wrongOrigin.status(),403)
console.log(JSON.stringify({passkeys:'passed',oneTimeChallenges:true,accountIsolation:true,revocation:true,originBinding:true}))
await cdp.send('WebAuthn.removeVirtualAuthenticator',{authenticatorId})
await context.close();await browser.close()
