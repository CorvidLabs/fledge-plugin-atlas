<script>
(function(){
  const dataEl = document.getElementById('atlas-data');
  const svgEl = document.getElementById('graph-svg');
  const tip = document.getElementById('tip');
  if(!dataEl || !svgEl) return;
  const data = JSON.parse(dataEl.textContent);
  const NS = 'http://www.w3.org/2000/svg';
  const mk = (t, cls) => { const e = document.createElementNS(NS, t); if(cls) e.setAttribute('class', cls); return e; };
  const W = 1180, H = 700;
  let vb = { x:0, y:0, w:W, h:H };
  const setVB = () => svgEl.setAttribute('viewBox', `${vb.x} ${vb.y} ${vb.w} ${vb.h}`);
  setVB();

  // ---- colour scales ----
  const LANGC = {};
  [...new Set(data.files.map(f=>f.lang))].forEach((l,i)=>{ LANGC[l] = `hsl(${(i*57+25)%360},45%,60%)`; });
  const covColor = pct => pct==null ? '#3a424a' : `hsl(${Math.round(pct*1.2)},60%,52%)`;
  const tss = [...data.specs.map(s=>s.updated_ts), ...data.files.map(f=>f.updated_ts)].filter(t=>t!=null);
  const tmin = tss.length ? Math.min(...tss) : 0, tspan = Math.max(1, (tss.length ? Math.max(...tss) : 1) - tmin);
  const ageColor = ts => { if(ts==null) return '#3a424a'; const t=(ts-tmin)/tspan; return `hsl(${Math.round(210-t*192)},${Math.round(25+t*55)}%,52%)`; };

  // ---- nodes ----
  const specNodes = data.specs.map(s=>({
    id:'S'+s.index, kind:'spec', label:s.module, idx:s.index,
    specColor:s.color, langColor:s.color,
    covColor: s.test_pct==null ? s.color : `hsl(${Math.round(s.test_pct*1.2)},60%,52%)`,
    ageColor: ageColor(s.updated_ts),
    side: Math.max(20, Math.min(46, 15+Math.sqrt(Math.max(s.loc,1))/6)),
    loc:s.loc, files:s.files, updated:s.updated, commits:s.commits
  }));
  specNodes.forEach(n=>{ n.r = n.side*0.62; }); // physics radius
  const fileNodes = data.files.map((f,i)=>({
    id:'F'+i, kind:'file', label:f.path, name:f.path.split('/').pop(), lang:f.lang, loc:f.loc,
    orphan:f.orphan, overlap:f.overlap, specs:f.specs, testPct:f.test_pct,
    specColor: f.orphan ? '#3a424a' : (f.specs.length===1 ? data.specs[f.specs[0]].color : '#e7ecef'),
    langColor: LANGC[f.lang], covColor: covColor(f.test_pct), ageColor: ageColor(f.updated_ts),
    r: Math.max(3.5, Math.min(12, 2.5+Math.sqrt(Math.max(f.loc,1))/5.5))
  }));
  const nodes = specNodes.concat(fileNodes);
  if(!nodes.length){ return; }
  const byId = {}; nodes.forEach(n=>byId[n.id]=n);

  const links = [];
  data.files.forEach((f,i)=>f.specs.forEach(si=>links.push({source:'S'+si, target:'F'+i})));
  const nbr = {}; nodes.forEach(n=>nbr[n.id]=new Set());
  links.forEach(l=>{ nbr[l.source].add(l.target); nbr[l.target].add(l.source); });

  function seedPositions(){
    nodes.forEach((n,i)=>{
      const a = i*2.399963, rad = 40 + (i/nodes.length)*300;
      n.x = W/2 + Math.cos(a)*rad; n.y = H/2 + Math.sin(a)*rad*0.8;
      n.vx=0; n.vy=0; n.fx=null; n.fy=null;
    });
  }
  seedPositions();

  const orphanCount = fileNodes.filter(f=>f.orphan).length;
  let showOrphans = orphanCount <= 120;
  let showLabels = false;
  let colorMode = 'spec';
  let focused = null;            // spec index we've isolated, or null
  let active = [], activeLinks = [];

  const gLinks = mk('g'), gNodes = mk('g');
  svgEl.appendChild(gLinks); svgEl.appendChild(gNodes);

  const colorOf = n => colorMode==='cov' ? n.covColor : colorMode==='age' ? n.ageColor : colorMode==='lang' ? n.langColor : n.specColor;

  function recompute(){
    if(focused!=null){
      const keep = new Set(['S'+focused]);
      links.forEach(l=>{ if(l.source==='S'+focused) keep.add(l.target); });
      links.forEach(l=>{ if(keep.has(l.target)) keep.add(l.source); }); // co-owning specs (overlap)
      active = nodes.filter(n=>keep.has(n.id) && (n.kind==='spec' || showOrphans || !n.orphan));
    } else {
      active = nodes.filter(n => n.kind==='spec' || showOrphans || !n.orphan);
    }
    const set = new Set(active.map(n=>n.id));
    activeLinks = links.filter(l=>set.has(l.source) && set.has(l.target));
  }

  function build(){
    gLinks.textContent=''; gNodes.textContent='';
    recompute();
    for(const l of activeLinks){ const ln = mk('line','link'); l.el = ln; gLinks.appendChild(ln); }
    for(const n of active){
      const g = mk('g','node '+n.kind);
      let shape;
      if(n.kind==='spec'){
        shape = mk('rect'); const s=n.side;
        shape.setAttribute('x',-s/2); shape.setAttribute('y',-s/2);
        shape.setAttribute('width',s); shape.setAttribute('height',s); shape.setAttribute('rx',3);
      } else {
        shape = mk('circle'); shape.setAttribute('r',n.r);
      }
      shape.setAttribute('fill', colorOf(n));
      g.appendChild(shape);
      n.g=g; n.shape=shape; n.t=null;
      if(n.kind==='spec'){
        const t=mk('text'); t.setAttribute('text-anchor','middle'); t.setAttribute('dy',-n.side/2-6);
        t.textContent=n.label; g.appendChild(t); n.t=t;
      }
      gNodes.appendChild(g);
      wire(n,g);
    }
    applyLabels();
    applySearch();
  }

  function applyLabels(){
    for(const n of active){
      if(n.kind!=='file') continue;
      if(showLabels && !n.t){
        const t=mk('text'); t.setAttribute('text-anchor','middle'); t.setAttribute('dy',-n.r-4);
        t.textContent=n.name; n.g.appendChild(t); n.t=t;
      } else if(!showLabels && n.t){ n.t.remove(); n.t=null; }
    }
  }

  // ---- physics ----
  function tick(alpha){
    const N=active.length;
    for(let i=0;i<N;i++){ const a=active[i];
      for(let j=i+1;j<N;j++){ const b=active[j];
        let dx=a.x-b.x, dy=a.y-b.y, d2=dx*dx+dy*dy||0.01;
        if(d2>160000) continue;
        const d=Math.sqrt(d2), span=a.r+b.r+150, f=span*span/d2*0.032*alpha;
        const fx=dx/d*f, fy=dy/d*f; a.vx+=fx;a.vy+=fy;b.vx-=fx;b.vy-=fy;
      }
    }
    for(const l of activeLinks){ const a=byId[l.source], b=byId[l.target];
      let dx=b.x-a.x, dy=b.y-a.y; const d=Math.sqrt(dx*dx+dy*dy)||0.01;
      const f=(d-115)/d*0.055*alpha, fx=dx*f, fy=dy*f; a.vx+=fx;a.vy+=fy;b.vx-=fx;b.vy-=fy;
    }
    for(const n of active){
      n.vx+=(W/2-n.x)*0.0022*alpha; n.vy+=(H/2-n.y)*0.0022*alpha;
      if(n.fx!=null){ n.x=n.fx; n.y=n.fy; n.vx=0; n.vy=0; }
      else { n.vx*=0.86; n.vy*=0.86; n.x+=n.vx; n.y+=n.vy; }
    }
  }
  function draw(){
    for(const l of activeLinks){ const a=byId[l.source], b=byId[l.target];
      l.el.setAttribute('x1',a.x); l.el.setAttribute('y1',a.y); l.el.setAttribute('x2',b.x); l.el.setAttribute('y2',b.y); }
    for(const n of active){ n.g.setAttribute('transform',`translate(${n.x},${n.y})`); }
  }
  function prewarm(){ for(let i=0;i<230;i++){ tick(Math.max(1-i/230,0.05)); } draw(); }
  let alpha=0, raf=null;
  function loop(){ alpha*=0.985; tick(Math.max(alpha,0.02)); draw(); if(alpha>0.04) raf=requestAnimationFrame(loop); else raf=null; }
  function reheat(a=0.6){ alpha=a; if(!raf) raf=requestAnimationFrame(loop); }

  // ---- coordinate helpers ----
  function toSvg(e){ const r=svgEl.getBoundingClientRect(); return { x: vb.x+(e.clientX-r.left)/r.width*vb.w, y: vb.y+(e.clientY-r.top)/r.height*vb.h }; }

  // ---- hover trace ----
  function wire(n,g){
    g.addEventListener('mouseenter',()=>{
      svgEl.classList.add('trace'); n.g.classList.add('lit');
      nbr[n.id].forEach(id=>{ const m=byId[id]; if(m&&m.g) m.g.classList.add('lit'); });
      for(const l of activeLinks){ if(l.source===n.id||l.target===n.id) l.el.classList.add('hot'); }
      showTip(n);
    });
    g.addEventListener('mousemove',moveTip);
    g.addEventListener('mouseleave',()=>{
      svgEl.classList.remove('trace');
      active.forEach(m=>m.g.classList.remove('lit'));
      activeLinks.forEach(l=>l.el.classList.remove('hot'));
      tip.style.opacity=0;
    });
    // drag vs click
    g.addEventListener('pointerdown',(e)=>{
      e.stopPropagation(); e.preventDefault(); g.setPointerCapture(e.pointerId);
      let moved=0; const start=toSvg(e);
      const move=(ev)=>{ const p=toSvg(ev); moved+=Math.abs(p.x-start.x)+Math.abs(p.y-start.y); n.fx=p.x; n.fy=p.y; reheat(0.3); };
      const up=()=>{ n.fx=null; n.fy=null; g.removeEventListener('pointermove',move); g.removeEventListener('pointerup',up);
        if(moved<4 && n.kind==='spec'){ focusSpec(n.idx); } };
      g.addEventListener('pointermove',move); g.addEventListener('pointerup',up);
    });
  }
  function showTip(n){
    if(n.kind==='spec'){
      const bits=[`${n.files} files`,`${n.loc} LOC`]; if(n.updated) bits.push('updated '+n.updated); if(n.commits!=null) bits.push(n.commits+' commits');
      tip.innerHTML=`<b>${n.label}</b> spec<span class="sub">${bits.join(' · ')}</span><span class="sub">click to focus</span>`;
    } else {
      const rel = n.orphan ? 'no spec' : n.specs.map(si=>data.specs[si].module).join(' + ');
      const tc = n.testPct==null ? '' : ` · ${Math.round(n.testPct)}% tested`;
      tip.innerHTML=`<b>${n.label}</b><span class="sub">${n.loc} LOC · ${n.lang}${tc} · ${rel}</span>`;
    }
    tip.style.opacity=1;
  }
  function moveTip(e){ const r=svgEl.getBoundingClientRect(); tip.style.left=(e.clientX-r.left+14)+'px'; tip.style.top=(e.clientY-r.top+14)+'px'; }

  // ---- pan (drag background) ----
  let panning=null;
  svgEl.addEventListener('pointerdown',(e)=>{
    if(e.target.closest('.node')) return;
    panning={x:e.clientX,y:e.clientY,vbx:vb.x,vby:vb.y}; svgEl.style.cursor='grabbing';
  });
  window.addEventListener('pointermove',(e)=>{
    if(!panning) return; const r=svgEl.getBoundingClientRect();
    vb.x=panning.vbx-(e.clientX-panning.x)/r.width*vb.w; vb.y=panning.vby-(e.clientY-panning.y)/r.height*vb.h; setVB();
  });
  window.addEventListener('pointerup',()=>{ panning=null; svgEl.style.cursor='grab'; });

  // ---- zoom ----
  function zoomAt(px,py,k){
    const nw=Math.max(W*0.12,Math.min(W*3,vb.w*k)), nh=nw*(H/W);
    vb.x=px-(px-vb.x)*(nw/vb.w); vb.y=py-(py-vb.y)*(nh/vb.h); vb.w=nw; vb.h=nh; setVB();
  }
  svgEl.addEventListener('wheel',(e)=>{ e.preventDefault(); const p=toSvg(e); zoomAt(p.x,p.y,e.deltaY>0?1.12:0.89); },{passive:false});
  function fit(){
    const ns=active.length?active:nodes; if(!ns.length) return;
    let x0=1e9,y0=1e9,x1=-1e9,y1=-1e9;
    for(const n of ns){ const r=(n.side||n.r*2)/2+18; x0=Math.min(x0,n.x-r);y0=Math.min(y0,n.y-r);x1=Math.max(x1,n.x+r);y1=Math.max(y1,n.y+r); }
    const w=Math.max(x1-x0,60), h=Math.max(y1-y0,60), pad=1.06;
    let vw=Math.max(w,h*(W/H))*pad, vh=vw*(H/W);
    vb={ x:(x0+x1)/2-vw/2, y:(y0+y1)/2-vh/2, w:vw, h:vh }; setVB();
  }

  // ---- search ----
  let query='';
  function applySearch(){
    if(!query){ svgEl.classList.remove('searching'); active.forEach(n=>n.g.classList.remove('match')); return; }
    svgEl.classList.add('searching'); let count=0;
    for(const n of active){ const hit=n.label.toLowerCase().includes(query); n.g.classList.toggle('match',hit); if(hit)count++; }
    const c=document.getElementById('g-count'); if(c) c.textContent=count?`${count} match${count>1?'es':''}`:'no matches';
  }
  function fitMatches(){ const m=active.filter(n=>n.label.toLowerCase().includes(query)); if(!m.length) return;
    let x0=1e9,y0=1e9,x1=-1e9,y1=-1e9; for(const n of m){ x0=Math.min(x0,n.x);y0=Math.min(y0,n.y);x1=Math.max(x1,n.x);y1=Math.max(y1,n.y); }
    const vw=Math.max(x1-x0,200)*1.6, vh=vw*(H/W); vb={x:(x0+x1)/2-vw/2,y:(y0+y1)/2-vh/2,w:vw,h:vh}; setVB();
  }

  // ---- focus a spec ----
  function focusSpec(idx){ focused=idx; build(); prewarm(); fit(); updateFocusChip(); }
  function clearFocus(){ focused=null; build(); prewarm(); fit(); updateFocusChip(); }
  function updateFocusChip(){
    const chip=document.getElementById('g-focus'); if(!chip) return;
    if(focused!=null){ chip.style.display='inline-flex'; chip.querySelector('span').textContent=data.specs[focused].module; }
    else chip.style.display='none';
  }

  // ---- controls ----
  const $=id=>document.getElementById(id);
  const orphanBox=$('t-orphans'); if(orphanBox){ orphanBox.checked=showOrphans; orphanBox.addEventListener('change',()=>{ showOrphans=orphanBox.checked; build(); prewarm(); reheat(0.5); }); }
  const labelBox=$('t-labels'); if(labelBox){ labelBox.addEventListener('change',()=>{ showLabels=labelBox.checked; applyLabels(); }); }
  document.querySelectorAll('.cmode button').forEach(b=>b.addEventListener('click',()=>{
    document.querySelectorAll('.cmode button').forEach(x=>x.classList.remove('on')); b.classList.add('on'); colorMode=b.dataset.mode;
    active.forEach(n=>n.shape.setAttribute('fill',colorOf(n)));
  }));
  const search=$('g-search'); if(search){ search.addEventListener('input',()=>{ query=search.value.trim().toLowerCase(); applySearch(); });
    search.addEventListener('keydown',e=>{ if(e.key==='Enter'){ fitMatches(); } }); }
  if($('g-zin')) $('g-zin').addEventListener('click',()=>zoomAt(vb.x+vb.w/2,vb.y+vb.h/2,0.8));
  if($('g-zout')) $('g-zout').addEventListener('click',()=>zoomAt(vb.x+vb.w/2,vb.y+vb.h/2,1.25));
  if($('g-fit')) $('g-fit').addEventListener('click',fit);
  if($('g-reset')) $('g-reset').addEventListener('click',()=>{ focused=null; query=''; if(search)search.value=''; seedPositions(); build(); prewarm(); fit(); updateFocusChip(); });
  if($('g-focus')) $('g-focus').addEventListener('click',clearFocus);

  // deep-linkable colour mode
  const hash=(location.hash||'').replace('#','');
  if(['spec','lang','cov','age'].includes(hash)){ const b=document.querySelector(`.cmode button[data-mode="${hash}"]`);
    if(b){ document.querySelectorAll('.cmode button').forEach(x=>x.classList.remove('on')); b.classList.add('on'); colorMode=hash; } }

  build(); prewarm(); fit();
})();
</script>
