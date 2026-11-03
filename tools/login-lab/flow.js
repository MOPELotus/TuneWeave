/* Shared login UI state helpers; no credentials or browser storage. */
(function(root){
 'use strict';
 async function waitForRetry(ms,isCurrent){
  const end=Date.now()+ms;
  while(isCurrent()&&Date.now()<end)await new Promise(resolve=>setTimeout(resolve,Math.min(100,end-Date.now())));
 }
 async function requestQrWithRetry(request,{platform,isCurrent,onRetry=()=>{},wait=waitForRetry,attempts=3}){
  for(let attempt=1;attempt<=attempts&&isCurrent();attempt++){
   const result=await request();
   if(!isCurrent())return null;
   const error=result?.error;
   const retryable=error&&(error.retryable===true||['network_error','backend_unavailable'].includes(error.code));
   if(platform!=='soda'||result?.ok||!retryable||attempt===attempts)return result;
   const cooldown=Number(error.details?.retry_after_secs);
   const delay=error.code==='rate_limited'?Math.max(60000,Number.isFinite(cooldown)?cooldown*1000:0):2000*attempt;
   onRetry({attempt,nextAttempt:attempt+1,attempts,delay});
   await wait(delay,isCurrent);
  }
  return null;
 }
 function rememberInstruction(history,instruction){
  const key=JSON.stringify([instruction.destination,instruction.message]);
  if(!history.has(key))history.set(key,{destination:instruction.destination,message:instruction.message});
  return key;
 }
 const api={requestQrWithRetry,rememberInstruction};
 if(typeof module!=='undefined'&&module.exports)module.exports=api;
 else root.LoginLabFlow=api;
})(globalThis);
