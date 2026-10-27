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
