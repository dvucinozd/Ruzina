const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
class Element {
  constructor() {
    this.children=[]; this.listeners={}; this.value=''; this.dataset={}; this.style={};
    this.classList={add(){},remove(){},toggle(){}}; this.elements=new Map();
  }
  querySelector(s) { if(!this.elements.has(s)) this.elements.set(s,new Element()); return this.elements.get(s); }
  querySelectorAll() { return []; }
  append(...children) {this.children.push(...children);}
  replaceChildren() {this.children=[];}
  addEventListener(type,cb) {this.listeners[type]=cb;}
  click() {this.listeners.click?.();}
}
function harness() {
  const document=new Element(); document.createElement=()=>new Element();
  const waves=[1,2].map(deck=>{const wave=document.querySelector(`.wave[data-deck="${deck}"]`); wave.dataset.deck=String(deck); return wave;});
  document.querySelectorAll=s=>s==='.wave'?waves:[];
  const calls=[]; const timers=[]; let now=0; const uploads=[];
  class XMLHttpRequest {
    constructor() { this.headers={}; this.upload={}; uploads.push(this); }
    open(method,url) { this.method=method; this.url=url; }
    setRequestHeader(name,value) { this.headers[name]=value; }
    send(body) { this.body=body; }
  }
  const context=vm.createContext({document, URLSearchParams, Uint32Array, XMLHttpRequest,
    crypto:{getRandomValues:a=>a}, Date:{now:()=>now},
    setTimeout:(fn,delay)=>{timers.push({fn,delay});return timers.length;},
    fetch:url=>new Promise((resolve,reject)=>calls.push({url,resolve,reject}))});
  vm.runInContext(fs.readFileSync(path.join(__dirname,'../app.js'),'utf8')+
    '\nglobalThis.testApi={pollStatus,renderDeck,dragging,renderSettings,settingParams,passwordProblem,changePassword,firmwareProblem,installFirmware};',context);
  const status=(revision,boot=1)=>({stamp:{boot,sequence:calls.length,library_revision:revision},
    deck1:{state_text:'IDLE',pitch_percent:0},deck2:{state_text:'IDLE',pitch_percent:0},mixer:{}});
  const reply=(call,body)=>call.resolve({ok:true,json:async()=>body,text:async()=>''});
  const flush=async()=>{for(let i=0;i<12;i++) await Promise.resolve();};
  return {document,calls,context,reply,flush,status,setNow:n=>{now=n},uploads};
}
const track=(title='Old')=>({track_key:42,title,artist:'Artist',bpm:120,duration_ms:1000});
test('old row cannot load with a new generation',async()=>{
  const h=harness(); h.reply(h.calls[0],h.status(10)); await h.flush();
  h.reply(h.calls[1],{generation:10,tracks:[track()],next:null}); await h.flush();
  const old=h.document.querySelector('#tracks').children[0].children[2];
  h.context.testApi.pollStatus(); h.reply(h.calls[2],h.status(11)); await h.flush();
  const before=h.calls.length; old.click(); assert.equal(h.calls.length,before);
});
test('late old success or failure cannot replace a newer snapshot',async()=>{
  for(const reject of [false,true]) {
    const h=harness(); h.reply(h.calls[0],h.status(20)); await h.flush(); const old=h.calls[1];
    h.context.testApi.pollStatus(); h.reply(h.calls[2],h.status(21)); await h.flush();
    h.reply(h.calls[3],{generation:21,tracks:[track('New')],next:null}); await h.flush();
    if(reject) old.reject(Error('old')); else h.reply(old,{generation:20,tracks:[track()],next:null});
    await h.flush();
    assert.equal(h.document.querySelector('#tracks').children[0].children[0].children[0].textContent,'New');
    assert.equal(h.document.querySelector('#library-info').textContent,'1 tracks');
  }
});
test('failed current snapshot retries without a revision change',async()=>{
  const h=harness(); h.reply(h.calls[0],h.status(1)); await h.flush();
  h.calls[1].reject(Error('offline')); await h.flush(); h.setNow(1001);
  h.context.testApi.pollStatus(); h.reply(h.calls[2],h.status(1)); await h.flush();
  assert.match(h.calls[3].url,/api\/library/);
});
test('reboot with same revision and media removal invalidate pending responses',async()=>{
  const h=harness(); h.reply(h.calls[0],h.status(5)); await h.flush();
  const old=h.calls[1]; h.context.testApi.pollStatus(); h.reply(h.calls[2],h.status(5,2)); await h.flush();
  assert.match(h.calls[3].url,/api\/library/);
  h.context.testApi.pollStatus(); h.reply(h.calls[4],h.status(null,2)); await h.flush();
  h.reply(old,{generation:5,tracks:[track()],next:null});
  h.reply(h.calls[3],{generation:5,tracks:[track()],next:null}); await h.flush();
  assert.equal(h.document.querySelector('#tracks').children.length,0);
});
test('mismatched page and repeated next offset never publish partial rows',async()=>{
  for(const page of [{generation:2,tracks:[track()],next:null},{generation:1,tracks:[track()],next:0}]){
    const h=harness(); h.reply(h.calls[0],h.status(1)); await h.flush();
    h.reply(h.calls[1],page); await h.flush();
    assert.equal(h.document.querySelector('#tracks').children.length,0);
    assert.equal(h.calls.length,2);
  }
});

test('pitch reflects raw fader, preserves drag and never emits control on render',()=>{
  const h=harness(); const card=h.document.querySelector('.deck[data-deck="1"]');
  const slider=card.querySelector('[data-continuous="pitch"]');
  for(const raw of [0,8192,16383]){
    h.context.testApi.renderDeck(1,{state_text:'IDLE',pitch_percent:6,pitch_raw:raw},{});
    assert.equal(slider.value,raw);
  }
  h.context.testApi.dragging.add(slider);
  h.context.testApi.renderDeck(1,{state_text:'READY',pitch_percent:-5,pitch_raw:100},{});
  assert.equal(slider.value,16383);
  h.context.testApi.dragging.delete(slider);
  h.context.testApi.renderDeck(1,{state_text:'READY',pitch_percent:-5,pitch_raw:100},{});
  assert.equal(slider.value,100); assert.equal(h.calls.length,1);
});
test('a seek carries the load generation of the track it was meant for',async()=>{
  const h=harness(); const status=h.status(3); status.stamp.load_generations=[4,9];
  status.deck2={state_text:'READY',pitch_percent:0,duration_ms:1000,position_ms:0};
  h.reply(h.calls[0],status); await h.flush();
  const bar=h.document.querySelector('.wave[data-deck="2"]').querySelector('[data-field="bar"]');
  bar.getBoundingClientRect=()=>({left:0,width:100});
  const before=h.calls.length; bar.listeners.click({clientX:25});
  const seek=new URL(h.calls[before].url,'http://deck').searchParams;
  assert.equal(seek.get('action'),'seek'); assert.equal(seek.get('deck'),'2');
  assert.equal(seek.get('value'),'250'); assert.equal(seek.get('generation'),'9');
});
test('the page names the deck it drives',async()=>{
  const h=harness(); h.reply(h.calls[0],{...h.status(10),name:'Ruzina-AB2D'}); await h.flush();
  assert.equal(h.document.querySelector('#deck-name').textContent,'Ruzina-AB2D');
  assert.equal(h.document.title,'Ruzina-AB2D · RUZINA');
});
test('a deck shows its tempo to the hundredth and the master clock follows the sync master',async()=>{
  const h=harness(); const status=h.status(null);
  status.deck2={state_text:'PLAYING',playing:true,pitch_percent:-1.59,tempo_x100:12600,sync_master:true};
  h.reply(h.calls[0],status); await h.flush();
  const card=h.document.querySelector('.deck[data-deck="2"]');
  assert.equal(card.querySelector('[data-field="bpm"]').textContent,'126');
  assert.equal(card.querySelector('[data-field="bpm-fraction"]').textContent,'.00');
  assert.equal(card.querySelector('[data-field="tempo"]').textContent,'-1.59%');
  assert.equal(h.document.querySelector('#mclk').textContent,'126.00');
});
test('the overview is fetched once per track and a stale answer is dropped',async()=>{
  const h=harness();
  const fresh=()=>({...h.status(null),deck1:{state_text:'READY',pitch_percent:0,duration_ms:1000,track_key:7}});
  h.reply(h.calls[0],fresh()); await h.flush();
  assert.equal(h.calls[1].url,'/api/wave?deck=1');
  h.reply(h.calls[1],{track_key:6,peaks:'00ff'}); await h.flush();
  // A stale answer waits a second before the page asks again.
  h.context.testApi.pollStatus(); h.reply(h.calls[2],fresh()); await h.flush();
  assert.equal(h.calls.length,3);
  h.setNow(1001); h.context.testApi.pollStatus(); h.reply(h.calls[3],fresh()); await h.flush();
  assert.equal(h.calls[4].url,'/api/wave?deck=1');
  h.reply(h.calls[4],{track_key:7,peaks:'00ff'}); await h.flush();
  h.context.testApi.pollStatus(); h.reply(h.calls[5],fresh()); await h.flush();
  assert.equal(h.calls.length,6);
});
const deckSettings=()=>({load_lock:false,jog:['vinyl','cdj'],brightness:{percent:25,min:10,max:50,step:5},
  trim_db:3,screensaver_min:2,info:{network:'Ruzina-AB2D',firmware:'v1',built:'Nov 25 2026 12:00:00',controller:'',tracks:25,media:'removed',uptime_s:3725,slot:'ota_1'},
  profiles:[{name:'Pioneer DDJ-FLX4',id:'2B73:0045',tested:true,active:true},{name:'Numark DJ2GO2 Touch',id:'15E4:0050',tested:false,active:false}]});
test('the setup page shows the deck\'s settings and information',()=>{
  const h=harness(); h.context.testApi.renderSettings(deckSettings());
  assert.equal(h.document.querySelector('#brightness').textContent,'25 %');
  assert.equal(h.document.querySelector('#info-controller').textContent,'none');
  assert.equal(h.document.querySelector('#info-media').textContent,'REMOVED');
  assert.equal(h.document.querySelector('#info-tracks').textContent,'25');
  assert.equal(h.document.querySelector('#info-uptime').textContent,'1 h 2 min');
});
test('a setting change names one setting, and the brightness steps within its range',()=>{
  const h=harness(); const api=h.context.testApi;
  assert.equal(api.settingParams('brightness','1'),null);
  api.renderSettings(deckSettings());
  assert.equal(String(api.settingParams('brightness','1')),'name=brightness&value=30');
  assert.equal(String(api.settingParams('brightness','-1')),'name=brightness&value=20');
  api.renderSettings({...deckSettings(),brightness:{percent:50,min:10,max:50,step:5}});
  assert.equal(api.settingParams('brightness','1'),null);
  assert.equal(String(api.settingParams('jog','cdj','2')),'name=jog&value=cdj&deck=2');
  assert.equal(String(api.settingParams('load_lock','1')),'name=load_lock&value=1');
});
test('a new network password is checked, then sent in the body only',async()=>{
  const h=harness(); const api=h.context.testApi;
  assert.equal(api.passwordProblem('short'),'Password must be 8 to 63 characters');
  assert.equal(api.passwordProblem('lozinka čaša'),'Use letters, digits, spaces and common symbols only');
  assert.equal(api.passwordProblem('say "hi" now'),'Quotes and backslashes cannot be used');
  assert.equal(api.passwordProblem('club night 42'),null);
  assert.equal(api.passwordProblem('ab'.repeat(32)),null);
  const before=h.calls.length;
  assert.equal(await api.changePassword('short'),false);
  assert.equal(h.calls.length,before);
  const sent=api.changePassword('club night 42');
  const call=h.calls[h.calls.length-1];
  assert.equal(call.url,'/api/network');
  call.resolve({ok:true,text:async()=>'Saved.'});
  assert.equal(await sent,true);
});
test('the setup page names the deck\'s network',()=>{
  const h=harness(); h.context.testApi.renderSettings(deckSettings());
  assert.equal(h.document.querySelector('#info-network').textContent,'Ruzina-AB2D');
});
test('the setup page lists the controller profiles, the attached one marked',()=>{
  const h=harness(); h.context.testApi.renderSettings(deckSettings());
  const rows=h.document.querySelector('#profiles').children;
  assert.equal(rows.length,2);
  assert.equal(rows[0].className,'active');
  assert.equal(rows[0].children.map(c=>c.textContent).join('|'),'Pioneer DDJ-FLX4|2B73:0045|ACTIVE|TESTED');
  assert.equal(rows[1].children.map(c=>c.textContent).join('|'),'Numark DJ2GO2 Touch|15E4:0050');
});
test('the setup page shows the slot the firmware runs from',()=>{
  const h=harness(); h.context.testApi.renderSettings(deckSettings());
  assert.equal(h.document.querySelector('#info-slot').textContent,'ota_1');
});
test('a firmware bundle is checked, then sent whole with the control marker',()=>{
  const h=harness(); const api=h.context.testApi;
  assert.equal(api.firmwareProblem(undefined),'Choose a .ddjota bundle');
  assert.equal(api.firmwareProblem({name:'s210.bin',size:4000000}),'A firmware bundle ends in .ddjota');
  assert.equal(api.firmwareProblem({name:'s210.ddjota',size:100}),'That file is not a firmware bundle');
  assert.equal(api.firmwareProblem({name:'s210.ddjota',size:188+6*1024*1024+1}),'That file is not a firmware bundle');
  assert.equal(api.installFirmware({name:'notes.txt',size:5000}),false);
  assert.equal(h.uploads.length,0);
  const file={name:'s210.ddjota',size:3431308};
  assert.equal(api.installFirmware(file),true);
  const [sent]=h.uploads;
  assert.equal(sent.method,'POST');
  assert.equal(sent.url,'/api/ota/p4');
  assert.equal(sent.headers['X-DDJ-Control'],'1');
  assert.equal(sent.headers['Content-Type'],'application/octet-stream');
  assert.equal(sent.body,file);
  sent.upload.onprogress({lengthComputable:true,loaded:1715654,total:3431308});
  assert.equal(h.document.querySelector('#firmware-progress').value,50);
  sent.status=409; sent.responseText='Conflict'; sent.onload();
  assert.equal(h.document.querySelector('#firmware-progress').hidden,true);
});
