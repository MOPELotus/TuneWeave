/* No browser storage: passwords, cookies and challenge receipts stay transient. */
const $ = id => document.getElementById(id);
const names = {qq:'QQ 音乐',netease:'网易云音乐',kugou:'酷狗音乐',soda:'汽水音乐'};
const qrTypes = {qq:[['qq','QQ 扫码'],['mobile','QQ 音乐 App'],['wechat','微信']],netease:[['','网易云 App']],kugou:[['standard','标准版'],['concept','概念版'],['web','Web']],soda:[['','汽水 App']]};
let platform='qq', mode='qr', generation=0, timer=null, autoPoll=false, polling=false, tx=null, verification=null, phase=null, selectedUser=null, busy=false;
const records=[];
const instructionHistory=new Map();let verificationViewKey=null,retryEpoch=0;
const alias=()=>$('alias').value.trim();
const context=()=>({platform,account:alias(),credential_mode:'server'});
const query=extra=>new URLSearchParams({platform,account:alias(),...extra});
function clean(value,key='') {
 if(/password|cookie|musickey|caller.?credential|authorization|ticket|token|principal|phone|response|answer|url|verification_id/i.test(key)) return '[已隐藏]';
 if(Array.isArray(value))return value.map(v=>clean(v));
 if(value&&typeof value==='object')return Object.fromEntries(Object.entries(value).map(([k,v])=>[k,clean(v,k)]));
 return typeof value==='string'?value.replace(/twc1_[A-Za-z0-9_-]+/g,'[已隐藏]').slice(0,1200):value;
}
async function api(label,path,body,method=body?'POST':'GET') {
 const g=generation,p=platform,a=alias(),started=performance.now();let data,status;
 try { const response=await fetch('/api'+path,{method,cache:'no-store',headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined});status=response.status;data=await response.json(); }
 catch {data={ok:false,error:{code:'network_error',message:'无法连接测试服务'}};status=0;}
 const record={time:new Date().toISOString(),platform:p,account:a,stage:label,status,elapsed_ms:Math.round(performance.now()-started),error_code:data.error?.code,request_id:data.meta?.request_id};records.push(record);
 if(g!==generation)return null;
 const item=document.createElement('li');item.className=data.ok?'good':'bad';item.textContent=`${new Date().toLocaleTimeString()} · ${label} · ${data.ok?'成功':data.error?.code||status} · ${record.elapsed_ms} ms`;if(record.request_id)item.title=record.request_id;$('timeline').prepend(item);$('response').textContent=JSON.stringify(clean(data),null,2);
 if(!data.ok)$('summary').textContent=p==='qq'&&data.error?.details?.platform_code===20279?'QQ 音乐登录设备数量已达上限（20279）。请在官方客户端检查并清理不用的登录设备，再重试。':data.error?.message||'请求失败，请保留诊断结果。';
 return data;
}
function stop(){retryEpoch++;autoPoll=false;clearTimeout(timer);timer=null;}
function reset(){stop();generation++;instructionHistory.clear();verificationViewKey=null;$('upSmsHistory').replaceChildren();$('upSmsInstructions').hidden=true;tx=null;verification=null;phase=null;selectedUser=null;$('verification').hidden=true;$('browserFrame').removeAttribute('src');$('sessionState').textContent='未检查';$('qrState').textContent='生成后自动检查状态；切换平台会停止轮询。';$('qrBox').replaceChildren();const el=document.createElement('div');el.className='qrplaceholder';el.textContent='点击生成二维码';$('qrBox').append(el);['passwordValue','cookieValue','answer','browserReply'].forEach(id=>$(id).value='');}
function setMode(next){mode=next;stop();document.querySelectorAll('.pane').forEach(p=>p.hidden=p.id!==next);document.querySelectorAll('#modes button').forEach(b=>b.classList.toggle('active',b.dataset.mode===next));}
async function choose(p){reset();platform=p;$('platformName').textContent=names[p];document.querySelectorAll('#platforms button').forEach(b=>b.classList.toggle('active',b.dataset.platform===p));$('qrType').replaceChildren(...qrTypes[p].map(([v,t])=>new Option(t,v)));setMode('qr');const j=await api('读取平台能力','/v1/capabilities?platform='+p);if(j?.ok){const caps=(Array.isArray(j.data)?j.data.find(v=>v.platform===p)?.capabilities:j.data.capabilities)||[];const list=Array.isArray(caps)?caps:[];for(const [m,c]of [['qr','qr_login'],['password','password_login'],['sms','phone_login'],['cookie','credential_import']]){const b=document.querySelector(`[data-mode="${m}"]`);b.disabled=!list.includes(c);b.title=b.disabled?'平台未声明此登录能力':'';}}}
function showVerification(v){verification=v||null;$('verification').hidden=!v;if(!v)return;renderInstructions(v.up_sms);const viewKey=JSON.stringify([v.method,v.state,v.methods,v.image_data_url,v.image,v.accounts,v.url,v.protocol,v.verification_id,v.message_origin,v.message_type,v.response_field]);$('verifyHint').textContent=[phase==='qr'?'扫码二次验证':v.method||v.state,v.masked_destination,v.message,v.resend_after_secs?`重发等待 ${v.resend_after_secs} 秒`:null].filter(Boolean).join(' · ');if(viewKey===verificationViewKey)return;verificationViewKey=viewKey;selectedUser=null;
 const image=v.image_data_url||v.image?.image_data_url;$('verifyImage').hidden=!image;if(image)$('verifyImage').src=image;
 $('accounts').replaceChildren();for(const account of v.accounts||[]){const label=document.createElement('label'),radio=document.createElement('input');radio.type='radio';radio.name='user';radio.style.width='auto';radio.onchange=()=>selectedUser=account.user_id;label.append(radio,document.createTextNode(' '+(account.nickname||account.user_id)));$('accounts').append(label);}
 $('answer').closest('label').hidden=phase==='qr'&&!v.methods?.includes('sms');const method=v.method||v.state;let actions=phase==='qr'?[...(v.methods?.includes('sms')?['send_sms','submit_sms']:[]),...(v.methods?.includes('up_sms')?['verify_up_sms']:[])]:['submit_code'];if(phase!=='qr'){if(/image/.test(method))actions=['submit_image','refresh_image'];else if(/voice/.test(method))actions=['submit_voice','resend_voice'];else if(/account_selection/.test(method))actions=['select_account'];else if(/browser/.test(method))actions=[];else if(phase==='password')actions=['submit_sms','resend_sms'];}
 const labels={send_sms:'发送二次短信',submit_code:'提交验证码',submit_image:'提交图形答案',refresh_image:'换一张图片',submit_sms:'提交短信验证码',resend_sms:'重发二次短信',submit_voice:'提交语音验证码',resend_voice:'重发语音',select_account:'确认所选账户',verify_up_sms:'已发送上行短信，校验'};
 $('verifyActions').replaceChildren();for(const action of actions){const b=document.createElement('button');b.textContent=labels[action];b.onclick=()=>advance(action);$('verifyActions').append(b);}
 const url=v.url||v.browser?.url;$('browserArea').hidden=!url;$('browserFrame').hidden=true;
 if(url){const u=new URL(url);if(u.protocol!=='https:')return;$('browserLink').href=url;const native=v.protocol==='kugou_native_bridge';$('browserHint').textContent=native?'本次为原生 WebView 桥接验证，普通 iframe 无法提供该桥接。请保留此阶段的诊断；可先测试 Web 后端或扫码。':'请在官方页面手动完成验证；收到合法回执后可继续。';if(!native){$('browserFrame').src=url;$('browserFrame').hidden=false;}}
}
async function progress(j){if(!j?.ok)return;const d=j.data;tx=d.transaction_id||tx;if(d.verification){$('sessionState').textContent='等待额外验证';showVerification({...d,...d.verification,...d.verification.verification,method:d.verification.method||(d.verification.image_data_url?'image':d.verification.url?'browser':undefined)});}else if(phase==='sms'&&tx&&!['confirmed','failed','expired'].includes(d.state))showVerification({...d,method:d.state==='account_selection_required'?'account_selection':'sms'});$('qrState').textContent=d.message||d.state||'已收到平台响应';if(d.state==='confirmed'||d.profile?.authenticated||d.authenticated){stop();$('sessionState').textContent='已登录';$('verification').hidden=true;await checkProfile();}else if(['expired','failed'].includes(d.state)){stop();$('sessionState').textContent=d.state==='expired'?'已过期':'登录失败';}}
function renderInstructions(instruction){
 if(!instruction)return;
 $('upSmsInstructions').hidden=false;
 const latest=LoginLabFlow.rememberInstruction(instructionHistory,instruction);
 for(const [key,value]of instructionHistory){
  let row=[...$('upSmsHistory').children].find(el=>el.dataset.key===key);
  if(!row){row=document.createElement('div');row.className='smsinstruction';row.dataset.key=key;const text=document.createElement('span');text.textContent=`发送 ${value.message} 至 ${value.destination}`;row.append(text,document.createElement('small'));$('upSmsHistory').append(row);}
  row.classList.toggle('latest',key===latest);row.lastElementChild.textContent=key===latest?'最近返回':'此前返回';
 }
}
async function qrRequest(label,path,body){
 const g=generation,e=retryEpoch;let retried=false;
 const result=await LoginLabFlow.requestQrWithRetry(()=>api(label,path,body),{platform,isCurrent:()=>g===generation&&e===retryEpoch,onRetry:({nextAttempt,attempts,delay})=>{retried=true;$('qrState').textContent=`请求暂时失败，${Math.ceil(delay/1000)} 秒后自动重试（第 ${nextAttempt}/${attempts} 次）`;$('summary').textContent='正在重试二维码请求；不会自动发送短信或提交验证码。';}});
 if(retried&&result?.ok)$('summary').textContent='二维码请求重试成功，已恢复。';
 else if(retried&&result&&!result.ok)$('qrState').textContent='自动重试后仍失败，可手动检查或重新生成。';
 return result;
}
async function poll(){if(!tx||phase!=='qr'||polling)return;const g=generation;polling=true;try{const j=await qrRequest('扫码状态','/v1/auth/qr/'+encodeURIComponent(tx));await progress(j);if(g===generation&&autoPoll&&j?.ok&&!['confirmed','expired','failed'].includes(j.data.state))timer=setTimeout(poll,3000);}finally{polling=false;}}
async function startQr(){reset();phase='qr';const body=context();if($('qrType').value)body.login_type=$('qrType').value;const j=await qrRequest('创建扫码事务','/v1/auth/qr',body);if(!j?.ok)return;tx=j.data.transaction_id;const box=$('qrBox');box.replaceChildren();if(j.data.image_data_url){const img=new Image();img.alt='登录二维码';img.src=j.data.image_data_url;box.append(img);}else if(j.data.url){const code=qrcode(0,'M');code.addData(j.data.url);code.make();const img=new Image();img.alt='登录二维码';img.src=code.createDataURL(5,8);box.append(img);} $('summary').textContent='二维码已生成，请用对应平台 App 扫码。';$('qrState').textContent='等待扫码 · '+(j.data.expires_at||'');autoPoll=true;timer=setTimeout(poll,2000);}
async function advance(action){if(!tx)return;let body={action};const code=$('answer').value.trim();if(action==='submit_code')body={code};if(['submit_sms','submit_voice','select_account'].includes(action))body.code=code;if(action==='submit_image'){body.answer=code;if(phase==='password')body.password=$('passwordValue').value;}if(action==='select_account'){if(!selectedUser){$('summary').textContent='请先明确选择一个账户。';return;}body.user_id=selectedUser;}
 const path=phase==='qr'?`/v1/auth/qr/${tx}/verification`:phase==='password'?`/v1/auth/password/challenges/${tx}/verify`:`/v1/auth/challenges/${tx}/verify`;
 const j=await api('继续验证：'+action,path,body);$('answer').value='';$('passwordValue').value='';await progress(j);if(j?.ok&&phase==='qr'&&j.data.state==='scanned'){stop();autoPoll=true;timer=setTimeout(poll,1000);}}
async function checkProfile(){const j=await api('独立核验账户资料','/v1/account/profile?'+query());if(j?.ok){$('sessionState').textContent=j.data.authenticated===false?'未登录':'资料核验成功';$('summary').textContent=[j.data.nickname,j.data.user_id,j.data.authenticated===false?'未认证':'平台已返回账户资料'].filter(Boolean).join(' · ');}return j;}
async function run(button,fn){if(busy)return;busy=true;button.disabled=true;try{await fn();}catch{ $('summary').textContent='页面操作失败，可查看浏览器或导出诊断。';}finally{busy=false;button.disabled=false;}}
for(const [p,n]of Object.entries(names)){const b=document.createElement('button');b.dataset.platform=p;b.textContent=n;b.onclick=()=>choose(p);$('platforms').append(b);}
document.querySelectorAll('#modes button').forEach(b=>b.onclick=()=>setMode(b.dataset.mode));$('alias').onchange=reset;
$('startQr').onclick=()=>run($('startQr'),startQr);$('pollQr').onclick=()=>{stop();poll();};$('stopQr').onclick=()=>{stop();$('qrState').textContent='轮询已停止，可手动检查或重新生成。';};
$('loginPassword').onclick=()=>run($('loginPassword'),async()=>{const value=$('passwordValue').value;reset();phase='password';const j=await api('密码登录','/v1/auth/password',{...context(),backend:$('passwordBackend').value,principal_type:$('principalType').value,principal:$('principal').value,password:value,password_format:'plain',country_code:'86'});await progress(j);});
$('sendSms').onclick=()=>run($('sendSms'),async()=>{reset();phase='sms';await progress(await api('发送登录短信','/v1/auth/challenges',{...context(),method:'sms',backend:$('smsBackend').value,principal:$('phone').value,country_code:$('countryCode').value}));});
$('importCookie').onclick=()=>run($('importCookie'),async()=>{const value=$('cookieValue').value;reset();const j=await api('导入会话','/v1/auth/import',{...context(),credential:{kind:'cookie',value}});if(j?.ok){$('sessionState').textContent='已导入';await checkProfile();}});
$('checkSession').onclick=()=>run($('checkSession'),async()=>{const j=await api('检查会话','/v1/auth/session?'+query());if(j?.ok)$('sessionState').textContent=j.data.authenticated?'会话有效':'未登录';});$('profile').onclick=()=>run($('profile'),checkProfile);
$('refresh').onclick=()=>run($('refresh'),async()=>{await api('刷新登录会话','/v1/auth/session/refresh',context());await checkProfile();});
$('logout').onclick=()=>run($('logout'),async()=>{stop();const j=await api('注销测试会话','/v1/auth/session?'+query(),null,'DELETE');if(j?.ok){reset();$('sessionState').textContent='已注销';}});
$('probe').onclick=()=>run($('probe'),async()=>{const path={qq:'/v1/extensions/qq/sound-power/detail',netease:'/v1/recommendations/styles/selection',soda:'/v1/account/history/tracks',kugou:'/v1/account/membership'}[platform];await api('读取登录后接口',path+'?'+(platform==='qq'?new URLSearchParams({account:alias()}):query()));});
$('export').onclick=()=>{const blob=new Blob([JSON.stringify({version:'1.0.0-beta.2',events:records},null,2)],{type:'application/json'});const a=document.createElement('a');a.href=URL.createObjectURL(blob);a.download='tuneweave-login-diagnostics.json';a.click();setTimeout(()=>URL.revokeObjectURL(a.href),1000);};
$('submitBrowser').onclick=()=>run($('submitBrowser'),async()=>{if(!tx||!verification)return;const body={action:phase==='password'&&verification.method==='sms_browser'?'submit_sms_browser':'submit_browser',verification_id:verification.verification_id,response:$('browserReply').value};if(phase==='password'&&body.action==='submit_browser')body.password=$('passwordValue').value;else body.code=$('answer').value;await progress(await api('提交人工验证回执',phase==='password'?`/v1/auth/password/challenges/${tx}/verify`:`/v1/auth/challenges/${tx}/verify`,body));$('browserReply').value='';$('passwordValue').value='';});
window.addEventListener('message',event=>{const v=verification;if(!v?.message_origin||event.origin!==v.message_origin||event.source!==$('browserFrame').contentWindow)return;let data=event.data;try{if(typeof data==='string')data=JSON.parse(data);}catch{return;}if(data?.type!==v.message_type||typeof data[v.response_field]!=='string')return;$('browserReply').value=data[v.response_field];$('summary').textContent='已收到官方人工验证回执，请提交当前步骤。';});
choose(platform);
