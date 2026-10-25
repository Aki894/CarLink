'use strict';
const $=id=>document.getElementById(id);
async function send(v){try{const r=await fetch('/api/control',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(v)});const j=await r.json();$('message').textContent=j.ok?'操作已接受':j.error;await refresh();}catch(e){$('message').textContent='连接暂时中断，请稍后刷新。';}}
function button(label,action,parent){const b=document.createElement('button');b.textContent=label;b.onclick=action;parent.append(b);}
async function refresh(){try{const [s,d]=await Promise.all(['/api/status','/api/devices'].map(p=>fetch(p).then(r=>r.json())));if(!s.ok||!d.ok)return;const t=s.status;if(t.last_error)$('message').textContent=t.last_error;$('status').textContent=(t.target?t.target.name+' · ':'')+(t.protocol||'等待选择')+' · '+(t.active?'已连接':t.pairing?'等待手机配对':'等待连接')+(t.paused?' · 自动连接已暂停':'');$('devices').replaceChildren();for(const device of d.devices){const row=document.createElement('section'),name=document.createElement('h2');name.textContent=device.name+' · '+device.protocol;row.append(name);const target={protocol:device.protocol,address:device.address};button('连接',()=>send({op:'connect',...target}),row);button('设为首选',()=>send({op:'policy',mode:'favorite',...target}),row);button('忘记',()=>{if(confirm('忘记 '+device.name+'？下次使用需重新添加。'))send({op:'forget',confirm:true,...target});},row);$('devices').append(row);}}catch(e){$('status').textContent='连接暂时中断，正在重试…';}}
for(const [id,op] of [['pause','pause'],['resume','resume'],['disconnect','disconnect']])$(id).onclick=()=>send({op});
for(const [id,protocol] of [['apple','carplay'],['huawei','hicar']])$(id).onclick=()=>{if(confirm('停止当前投屏并添加新手机？'))send({op:'pair',protocol});};
$('last').onclick=()=>send({op:'policy',mode:'last'});$('manual').onclick=()=>send({op:'policy',mode:'manual'});
$('menu').onclick=()=>send({op:'menu',visible:true});$('return').onclick=()=>send({op:'menu',visible:false});
refresh();setInterval(refresh,2500);
