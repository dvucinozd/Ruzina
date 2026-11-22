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
  const calls=[]; const timers=[]; let now=0;
  const context=vm.createContext({document, URLSearchParams, Uint32Array,
    crypto:{getRandomValues:a=>a}, Date:{now:()=>now},
    setTimeout:(fn,delay)=>{timers.push({fn,delay});return timers.length;},
    fetch:url=>new Promise((resolve,reject)=>calls.push({url,resolve,reject}))});
  vm.runInContext(fs.readFileSync(path.join(__dirname,'../app.js'),'utf8')+
    '\nglobalThis.testApi={pollStatus,renderDeck,dragging};',context);
  const status=(revision,boot=1)=>({stamp:{boot,sequence:calls.length,library_revision:revision},
    deck1:{state_text:'IDLE',pitch_percent:0},deck2:{state_text:'IDLE',pitch_percent:0},mixer:{}});
  const reply=(call,body)=>call.resolve({ok:true,json:async()=>body,text:async()=>''});
  const flush=async()=>{for(let i=0;i<12;i++) await Promise.resolve();};
  return {document,calls,context,reply,flush,status,setNow:n=>{now=n}};
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
