const {test}=require('node:test');
const assert=require('node:assert/strict');
const {requestQrWithRetry,rememberInstruction}=require('./flow.js');
const temporary={ok:false,error:{code:'upstream_error',retryable:true}};
const options={platform:'soda',isCurrent:()=>true,wait:async()=>{}};
test('Soda retries transient QR failures with bounded backoff',async()=>{
 let calls=0;const delays=[];
 const result=await requestQrWithRetry(async()=>++calls<3?temporary:{ok:true},{...options,onRetry:v=>delays.push(v.delay)});
 assert.equal(result.ok,true);assert.equal(calls,3);assert.deepEqual(delays,[2000,4000]);
 calls=0;await requestQrWithRetry(async()=>{calls++;return temporary;},options);assert.equal(calls,3);
});
test('never retries QQ or permanent authentication failures',async()=>{
 for(const [platform,error] of [['qq',temporary],['soda',{ok:false,error:{code:'authentication_required',retryable:false}}]]){
  let calls=0;await requestQrWithRetry(async()=>{calls++;return error;},{...options,platform});assert.equal(calls,1);
 }
});
test('honors upstream cooldown and cancellation during backoff',async()=>{
 let current=true,calls=0,delay;
 const result=await requestQrWithRetry(async()=>{calls++;return {ok:false,error:{code:'rate_limited',retryable:true,details:{retry_after_secs:75}}};},{...options,isCurrent:()=>current,wait:async ms=>{delay=ms;current=false;}});
 assert.equal(delay,75000);assert.equal(calls,1);assert.equal(result,null);
});
test('instruction history deduplicates pairs and preserves first-seen order',()=>{
 const history=new Map();const first={destination:'950000',message:'YZ'},second={destination:'10690000',message:'YZ'};
 rememberInstruction(history,first);rememberInstruction(history,second);const latest=rememberInstruction(history,first);
 assert.deepEqual([...history.values()],[first,second]);assert.equal(latest,[...history.keys()][0]);
 rememberInstruction(history,{...first,message:'different'});assert.equal(history.size,3);
});
