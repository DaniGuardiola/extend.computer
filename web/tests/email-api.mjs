// Local-only seeded link fixtures: no production data or real email delivery.
import assert from 'node:assert/strict'
import { randomBytes, createHash } from 'node:crypto'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { fileURLToPath } from 'node:url'
const url = process.env.EXTEND_TEST_URL ?? 'http://localhost:3001'
assert.ok(['localhost','127.0.0.1'].includes(new URL(url).hostname), 'This fixture test is local-only')
const root = fileURLToPath(new URL('..', import.meta.url))
const command = promisify(execFile)
const sql = async statement => { await command(process.execPath, [root+'node_modules/wrangler/bin/wrangler.js','d1','execute','extend-computer','--local','--command',statement],{cwd:root,env:{...process.env,WRANGLER_LOG_PATH:'/tmp/extend-email-test-db.log'}}) }
const hash = s => createHash('sha256').update(s).digest('hex')
const request=async(path,data,token)=>{
  const response=await fetch(url+'/api/v1'+path,{method:data===undefined?'GET':'POST',headers:{'Content-Type':'application/json',Origin:url,'X-Extend-Client':'desktop',...(token?{Authorization:'Bearer '+token}:{})},body:data===undefined?undefined:JSON.stringify(data)})
  return {status:response.status,data:await response.json().catch(()=>null)}
}
const password='disposable-email-test-password'
const email='email-flow-'+Date.now()+'@example.invalid'
const signup=await request('/auth/signup',{email,password});assert.equal(signup.status,201)
const id=signup.data.account.id;const session=signup.data.token
const device=await request('/devices',{public_key:'11'.repeat(32),name:'Email test',platform:'macos'},session);assert.equal(device.status,200)
const seed=async(kind,expires=1800,version=false)=>{
  const raw=randomBytes(32).toString('hex')
  await sql(`INSERT INTO email_tokens SELECT '${hash(raw)}',id,'${kind}',${version?"'old-password-version'":'password_hash'},unixepoch()+${expires} FROM accounts WHERE id='${id}';`)
  return raw
}
try {
  const expired=await seed('verify',-1);assert.equal((await request('/auth/email/verify',{token:expired})).status,400)
  const old=await seed('verify',1800,true);assert.equal((await request('/auth/email/verify',{token:old})).status,400)
  const verify=await seed('verify');assert.equal((await request('/auth/email/verify',{token:verify})).status,200)
  assert.equal((await request('/account',undefined,session)).data.email_verified,1)
  assert.equal((await request('/auth/email/verify',{token:verify})).status,400)
  await sql(`INSERT INTO passkeys (id,account_id,public_key,counter,transports,name,rp_id,created_at) VALUES ('fixture-passkey','${id}','fixture-public-key',0,'[]','Fixture','localhost',unixepoch());`)
  const reset=await seed('reset');const wrongKind=await seed('verify')
  assert.equal((await request('/auth/recovery/complete',{token:wrongKind,password:'new-disposable-password'})).status,400)
  assert.equal((await request('/auth/recovery/complete',{token:reset,password:'short'})).status,400)
  // A malformed new password must not burn a valid link.
  assert.equal((await request('/auth/recovery/complete',{token:reset,password:'new-disposable-password'})).status,200)
  assert.equal((await request('/auth/recovery/complete',{token:reset,password:'new-disposable-password'})).status,400)
  assert.equal((await request('/account',undefined,session)).status,401)
  assert.equal((await request('/devices/'+device.data.id+'/heartbeat',{},device.data.device_token)).status,401)
  assert.equal((await request('/auth/login',{email,password})).status,401)
  const login=await request('/auth/login',{email,password:'new-disposable-password'});assert.equal(login.status,200)
  assert.equal((await request('/passkeys',undefined,login.data.token)).data.passkeys.length,0)
  assert.equal((await request('/auth/email/verify',{token:wrongKind})).status,400)
  console.log(JSON.stringify({emailFlows:'passed',expiry:true,replay:true,passwordVersion:true,kindBinding:true,sessionAndDeviceRevocation:true,passkeyRevocation:true,oldPasswordRejected:true}))
} finally { await sql(`DELETE FROM accounts WHERE id='${id}';`) }
