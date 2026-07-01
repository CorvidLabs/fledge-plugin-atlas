<script>
(function(){
  const dataEl = document.getElementById('atlas-data');
  const svgEl = document.getElementById('graph-svg');
  const tip = document.getElementById('tip');
  if(!dataEl || !svgEl) return;
  const data = JSON.parse(dataEl.textContent);
  const NS = 'http://www.w3.org/2000/svg';
  const mk = (t, cls) => { const e = document.createElementNS(NS, t); if(cls) e.setAttribute('class', cls); return e; };
  const clamp = (v,a,b)=>Math.max(a,Math.min(b,v));
  const W = 1180, H = 700;
  let vb = { x:0, y:0, w:W, h:H };
  const setVB = () => svgEl.setAttribute('viewBox', `${vb.x} ${vb.y} ${vb.w} ${vb.h}`);
  setVB();
  // Respect the user's motion preference: when reduced, we skip the animated
  // settle everywhere and jump straight to the synchronous prewarmed layout.
  const reduceMotion = !!(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);

  // ---- colour scales: brand tokens only, theme-aware via color-mix ----
  const CHART = ['--chart-1','--chart-2','--chart-3','--chart-4','--chart-5'];
  const LANGC = {};
  [...new Set(data.files.map(f=>f.lang))].forEach((l,i)=>{
    const mix = 100 - (Math.floor(i/CHART.length)%3)*22;
    LANGC[l] = `color-mix(in srgb, var(${CHART[i%CHART.length]}) ${mix}%, var(--bg))`;
  });
  const NOSPEC = 'var(--surface-strong)';
  const covColor = pct => pct==null ? NOSPEC : `color-mix(in srgb, var(--chart-4) ${Math.round(pct)}%, var(--danger))`; // 0 clay -> 100 green
  const tss = [...data.specs.map(s=>s.updated_ts), ...data.files.map(f=>f.updated_ts)].filter(t=>t!=null);
  const tmin = tss.length ? Math.min(...tss) : 0, tspan = Math.max(1, (tss.length ? Math.max(...tss) : 1) - tmin);
  const ageColor = ts => { if(ts==null) return NOSPEC; const t=(ts-tmin)/tspan; return `color-mix(in srgb, var(--chart-3) ${Math.round(t*100)}%, var(--chart-2))`; }; // cold steel -> hot amber

  // ---- nodes ----
  const specByIdx = {};
  const specNodes = data.specs.map(s=>{
    const n = { id:'S'+s.index, kind:'spec', label:s.module, idx:s.index, color:s.color,
      loc:s.loc, fileCount:s.files, updated:s.updated, commits:s.commits, needs:s.needs_review,
      solo:[], members:[] };
    specByIdx[s.index] = n; return n;
  });
  const fileNodes = data.files.map((f,i)=>{
    const single = f.specs.length===1 && !f.orphan;
    return { id:'F'+i, kind:'file', label:f.path, name:f.path.split('/').pop(), lang:f.lang, loc:f.loc,
      orphan:f.orphan, overlap:f.overlap, specs:f.specs, testPct:f.test_pct, single:single,
      specColor: f.orphan ? NOSPEC : (single ? specByIdx[f.specs[0]].color : 'var(--muted)'),
      langColor: LANGC[f.lang], covColor: covColor(f.test_pct), ageColor: ageColor(f.updated_ts),
      fr: clamp(2.6+Math.sqrt(Math.max(f.loc,1))/6, 2.6, 9) };
  });
  const nodes = specNodes.concat(fileNodes);
  if(!nodes.length) return;
  const byId = {}; nodes.forEach(n=>byId[n.id]=n);

  // membership + bubble sizing + deterministic in-bubble offsets for solo files
  fileNodes.forEach(f=>f.specs.forEach(si=>{ const s=specByIdx[si]; if(!s) return; s.members.push(f); if(f.single) s.solo.push(f); }));
  specNodes.forEach(s=>{
    s.R = clamp(22+Math.sqrt(s.members.length+1)*12, 32, 116);
    const M = s.solo.length;
    s.solo.forEach((f,k)=>{ const rf=Math.sqrt((k+0.5)/M), a=k*2.399963; f.ox=Math.cos(a)*rf; f.oy=Math.sin(a)*rf; });
  });

  // links (for neighbour trace + network layout)
  const links = [];
  data.files.forEach((f,i)=>f.specs.forEach(si=>links.push({source:'S'+si, target:'F'+i})));
  const nbr = {}; nodes.forEach(n=>nbr[n.id]=new Set());
  links.forEach(l=>{ nbr[l.source].add(l.target); nbr[l.target].add(l.source); });

  // shared-file spec pairs (springs that pull co-owning bubbles into overlap)
  const pairs=[], pk={};
  fileNodes.forEach(f=>{ if(f.specs.length<2) return;
    for(let a=0;a<f.specs.length;a++)for(let b=a+1;b<f.specs.length;b++){
      const k=Math.min(f.specs[a],f.specs[b])+'-'+Math.max(f.specs[a],f.specs[b]);
      if(!pk[k]){ pk[k]={s:specByIdx[f.specs[a]],t:specByIdx[f.specs[b]],w:0}; pairs.push(pk[k]); }
      pk[k].w++; } });

  function seedPositions(){
    specNodes.forEach((s,i)=>{ const a=i*2.399963, rad=60+(i/Math.max(specNodes.length,1))*260;
      s.x=W/2+Math.cos(a)*rad; s.y=H/2+Math.sin(a)*rad*0.85; s.vx=0;s.vy=0;s.fx=null;s.fy=null; });
    fileNodes.forEach((f,i)=>{ const a=i*2.399963, rad=40+(i/nodes.length)*300;
      f.x=W/2+Math.cos(a)*rad; f.y=H/2+Math.sin(a)*rad*0.8; f.vx=0;f.vy=0;f.fx=null;f.fy=null; });
  }
  seedPositions();

  const denseLabels = specNodes.length > 26; // few specs: label all; many: only big bubbles
  const orphanCount = fileNodes.filter(f=>f.orphan).length;
  let orphan = { x:W/2, y:H, cols:1 };
  let showOrphans = orphanCount <= 140;
  let showLabels = false;
  let colorMode = 'spec';
  let layout = 'grouped';
  let focused = null;
  let activeSpecs = [], activeFiles = [], activeLinks = [];
  // Roving tabindex: exactly one node is in the Tab order at a time; arrows walk the rest.
  let focusables = [], focusIndex = -1;

  const gBub = mk('g'), gLinks = mk('g'), gNodes = mk('g');
  svgEl.appendChild(gBub); svgEl.appendChild(gLinks); svgEl.appendChild(gNodes);

  // Off-screen text summary so AT users get the gist without traversing every node.
  const summaryEl = document.createElement('div');
  summaryEl.id = 'graph-summary';
  summaryEl.className = 'sr-only';
  const plur = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;
  summaryEl.textContent = `Interactive spec and code graph: ${plur(specNodes.length, 'spec')}, ` +
    `${plur(fileNodes.length, 'file')}, ${plur(orphanCount, 'orphan')} with no spec. ` +
    `Press Tab to enter the graph, then use the arrow keys to move between nodes, ` +
    `Enter or Space on a spec to focus its subgraph, and Escape to clear focus.`;
  if (svgEl.parentNode) svgEl.parentNode.insertBefore(summaryEl, svgEl.nextSibling);

  const colorOf = f => colorMode==='cov' ? f.covColor : colorMode==='age' ? f.ageColor : colorMode==='lang' ? f.langColor : f.specColor;

  function recompute(){
    if(focused!=null){
      const keepS = new Set([focused]);
      pairs.forEach(p=>{ if(p.s.idx===focused) keepS.add(p.t.idx); if(p.t.idx===focused) keepS.add(p.s.idx); });
      activeSpecs = specNodes.filter(s=>keepS.has(s.idx));
      activeFiles = fileNodes.filter(f=>!f.orphan && f.specs.some(si=>keepS.has(si)) && (showOrphans||!f.orphan));
    } else {
      activeSpecs = specNodes.slice();
      activeFiles = fileNodes.filter(f=>showOrphans || !f.orphan);
    }
    const sset = new Set(activeSpecs.map(s=>s.id)), fset = new Set(activeFiles.map(f=>f.id));
    activeLinks = links.filter(l=>sset.has(l.source) && fset.has(l.target));
  }

  // ---- render position of a file in grouped layout ----
  function groupedPos(f){
    if(f.orphan) return { x:f.gx||orphan.x, y:f.gy||orphan.y };
    if(f.single){ const s=specByIdx[f.specs[0]]; return { x:s.x+f.ox*(s.R-7), y:s.y+f.oy*(s.R-7) }; }
    let x=0,y=0,n=0; f.specs.forEach(si=>{const s=specByIdx[si]; if(s){x+=s.x;y+=s.y;n++;}});
    const a=(f.name.length*13+f.loc)*0.7; // stable jitter so shared files don't stack
    return n?{ x:x/n+Math.cos(a)*13, y:y/n+Math.sin(a)*13 }:{ x:W/2, y:H/2 };
  }
  function posOf(f){ return layout==='grouped' ? groupedPos(f) : { x:f.x, y:f.y }; }

  function build(){
    gBub.textContent=''; gLinks.textContent=''; gNodes.textContent='';
    recompute();
    if(layout==='network'){
      for(const l of activeLinks){ const ln=mk('line','link'); l.el=ln; gLinks.appendChild(ln); }
    }
    // spec groups
    for(const s of activeSpecs){
      const g=mk('g','node spec'+(s.needs?' needs':''));
      if(layout==='grouped'){
        const c=mk('circle','bubble'); c.setAttribute('r',s.R);
        c.style.fill=s.color; c.style.stroke=s.color; s.bub=c; g.appendChild(c);
      } else {
        const c=mk('circle'); s.nr=clamp(9+Math.sqrt(s.members.length)*2.2,10,26); c.setAttribute('r',s.nr);
        c.style.fill=s.color; s.bub=c; g.appendChild(c);
      }
      const t=mk('text','slabel'); t.setAttribute('text-anchor','middle'); t.textContent=s.label; t.style.fill=s.color;
      // Only label big bubbles by default; small specs reveal on hover/focus/search.
      if(layout==='grouped' && focused==null && denseLabels && s.R < 52) t.style.display='none';
      g.appendChild(t); s.t=t; s.g=g; gNodes.appendChild(g); wire(s,g);
    }
    // file dots
    for(const f of activeFiles){
      const g=mk('g','node file'+(f.overlap?' shared':'')+(f.orphan?' orph':''));
      const c=mk('circle'); c.setAttribute('r',f.fr); c.style.fill=colorOf(f); f.shape=c; g.appendChild(c);
      f.g=g; f.t=null; gNodes.appendChild(g); wire(f,g);
    }
    if(layout==='grouped') layoutOrphans();
    applyLabels(); applySearch();
    collectFocusables();
  }

  // ---- keyboard: roving tabindex across nodes ----
  function collectFocusables(){
    focusables = Array.prototype.slice.call(gNodes.querySelectorAll('.node'));
    focusIndex = focusables.length ? 0 : -1;
    focusables.forEach((g,i)=>g.setAttribute('tabindex', i===0 ? '0' : '-1'));
  }
  function setRovingTo(g){
    const i = focusables.indexOf(g); if(i<0) return;
    focusables.forEach((el,j)=>el.setAttribute('tabindex', j===i ? '0' : '-1'));
    focusIndex = i;
  }
  function focusAt(i){
    if(!focusables.length) return;
    i = (i + focusables.length) % focusables.length;
    focusables.forEach((el,j)=>el.setAttribute('tabindex', j===i ? '0' : '-1'));
    focusIndex = i; focusables[i].focus();
  }
  function moveFocus(delta){ if(focusables.length) focusAt((focusIndex<0?0:focusIndex)+delta); }
  gNodes.addEventListener('keydown',e=>{
    const g = e.target && e.target.closest ? e.target.closest('.node') : null; if(!g) return;
    const n = g.__node;
    switch(e.key){
      case 'ArrowRight': case 'ArrowDown': e.preventDefault(); moveFocus(1); break;
      case 'ArrowLeft': case 'ArrowUp': e.preventDefault(); moveFocus(-1); break;
      case 'Home': e.preventDefault(); focusAt(0); break;
      case 'End': e.preventDefault(); focusAt(focusables.length-1); break;
      case 'Enter': case ' ': case 'Spacebar':
        e.preventDefault();
        if(n && n.kind==='spec'){ focusSpec(n.idx); focusAt(0); }
        break;
      case 'Escape':
        if(focused!=null){ e.preventDefault(); clearFocus(); focusAt(0); }
        break;
    }
  });

  function layoutOrphans(){
    const orphans = activeFiles.filter(f=>f.orphan); if(!orphans.length) return;
    let x0=1e9,x1=-1e9,y1=-1e9;
    (activeSpecs.length?activeSpecs:[{x:W/2,y:H/2,R:0}]).forEach(s=>{ x0=Math.min(x0,s.x-s.R);x1=Math.max(x1,s.x+s.R);y1=Math.max(y1,s.y+s.R); });
    const cols=Math.max(1,Math.ceil(Math.sqrt(orphans.length*2.2))), gap=17;
    const startX=(x0+x1)/2-(cols-1)*gap/2, startY=y1+56;
    orphans.forEach((f,i)=>{ f.gx=startX+(i%cols)*gap; f.gy=startY+Math.floor(i/cols)*gap; });
    orphan.x=(x0+x1)/2; orphan.y=startY-20;
  }

  function applyLabels(){
    for(const f of activeFiles){
      if(showLabels && !f.t){ const t=mk('text','flabel'); t.setAttribute('text-anchor','middle'); t.setAttribute('dy',-f.fr-4); t.textContent=f.name; f.g.appendChild(t); f.t=t; }
      else if(!showLabels && f.t){ f.t.remove(); f.t=null; }
    }
  }

  // ---- physics ----
  function tickGrouped(alpha){
    const S=activeSpecs, N=S.length;
    for(let i=0;i<N;i++){ const a=S[i];
      for(let j=i+1;j<N;j++){ const b=S[j];
        let dx=a.x-b.x, dy=a.y-b.y, d2=dx*dx+dy*dy||0.01, d=Math.sqrt(d2);
        const target=a.R+b.R+56, f=target*target/d2*0.05*alpha;
        const fx=dx/d*f, fy=dy/d*f; a.vx+=fx;a.vy+=fy;b.vx-=fx;b.vy-=fy;
      }
    }
    for(const p of pairs){ const a=p.s,b=p.t; if(!a||!b) continue;
      let dx=b.x-a.x, dy=b.y-a.y, d=Math.sqrt(dx*dx+dy*dy)||0.01;
      const rest=a.R+b.R-Math.min(a.R,b.R)*0.4, f=(d-rest)/d*0.011*Math.min(p.w,6)*alpha;
      const fx=dx*f, fy=dy*f; a.vx+=fx;a.vy+=fy;b.vx-=fx;b.vy-=fy;
    }
    for(const s of S){ s.vx+=(W/2-s.x)*0.004*alpha; s.vy+=(H/2-s.y)*0.004*alpha;
      if(s.fx!=null){ s.x=s.fx; s.y=s.fy; s.vx=0; s.vy=0; } else { s.vx*=0.85; s.vy*=0.85; s.x+=s.vx; s.y+=s.vy; } }
  }
  function tickNetwork(alpha){
    const A=activeSpecs.concat(activeFiles), N=A.length;
    for(let i=0;i<N;i++){ const a=A[i];
      for(let j=i+1;j<N;j++){ const b=A[j];
        let dx=a.x-b.x, dy=a.y-b.y, d2=dx*dx+dy*dy||0.01; if(d2>150000) continue;
        const d=Math.sqrt(d2), f=(a.kind==='spec'||b.kind==='spec'?4200:1300)/d2*alpha;
        const fx=dx/d*f, fy=dy/d*f; a.vx+=fx;a.vy+=fy;b.vx-=fx;b.vy-=fy;
      }
    }
    for(const l of activeLinks){ const a=byId[l.source], b=byId[l.target];
      let dx=b.x-a.x, dy=b.y-a.y, d=Math.sqrt(dx*dx+dy*dy)||0.01;
      const f=(d-104)/d*0.05*alpha, fx=dx*f, fy=dy*f; a.vx+=fx;a.vy+=fy;b.vx-=fx;b.vy-=fy;
    }
    for(const n of A){ n.vx+=(W/2-n.x)*0.0024*alpha; n.vy+=(H/2-n.y)*0.0024*alpha;
      if(n.fx!=null){ n.x=n.fx; n.y=n.fy; n.vx=0; n.vy=0; } else { n.vx*=0.86; n.vy*=0.86; n.x+=n.vx; n.y+=n.vy; } }
  }
  const tick = a => layout==='grouped' ? tickGrouped(a) : tickNetwork(a);

  function draw(){
    if(layout==='grouped'){
      for(const s of activeSpecs){ s.bub.setAttribute('cx',s.x); s.bub.setAttribute('cy',s.y);
        s.t.setAttribute('x',s.x); s.t.setAttribute('y',s.y-s.R-7); }
      for(const f of activeFiles){ const p=posOf(f); f.g.setAttribute('transform',`translate(${p.x},${p.y})`); }
    } else {
      for(const l of activeLinks){ const a=byId[l.source], b=byId[l.target];
        l.el.setAttribute('x1',a.x); l.el.setAttribute('y1',a.y); l.el.setAttribute('x2',b.x); l.el.setAttribute('y2',b.y); }
      for(const s of activeSpecs){ s.g.setAttribute('transform',`translate(${s.x},${s.y})`); s.t.setAttribute('x',0); s.t.setAttribute('y',-s.nr-6); }
      for(const f of activeFiles){ f.g.setAttribute('transform',`translate(${f.x},${f.y})`); }
    }
  }
  function prewarm(){ const it = layout==='grouped'?200:260; for(let i=0;i<it;i++){ tick(Math.max(1-i/it,0.05)); } if(layout==='grouped') layoutOrphans(); draw(); }
  let alpha=0, raf=null;
  function loop(){ alpha*=0.985; tick(Math.max(alpha,0.02)); if(layout==='grouped') layoutOrphans(); draw(); if(alpha>0.05) raf=requestAnimationFrame(loop); else raf=null; }
  // With reduced motion we skip the animated settle and re-settle synchronously.
  function reheat(a=0.5){ if(reduceMotion){ prewarm(); return; } alpha=a; if(!raf) raf=requestAnimationFrame(loop); }

  function toSvg(e){ const r=svgEl.getBoundingClientRect(); return { x: vb.x+(e.clientX-r.left)/r.width*vb.w, y: vb.y+(e.clientY-r.top)/r.height*vb.h }; }

  // ---- hover/focus trace + drag/click ----
  // Shared highlight logic so keyboard focus and mouse hover behave identically.
  function enter(n,g){
    svgEl.classList.add('trace'); g.classList.add('lit');
    if(n.kind==='spec' && n.t) n.t.style.display=''; // reveal a hidden small-bubble label
    nbr[n.id].forEach(id=>{ const m=byId[id]; if(m&&m.g) m.g.classList.add('lit'); });
    if(layout==='network') for(const l of activeLinks){ if(l.source===n.id||l.target===n.id) l.el.classList.add('hot'); }
    showTip(n);
  }
  function leave(){
    svgEl.classList.remove('trace');
    [...activeSpecs,...activeFiles].forEach(m=>m.g&&m.g.classList.remove('lit'));
    activeLinks.forEach(l=>l.el&&l.el.classList.remove('hot')); tip.style.opacity=0;
  }
  // Position the tooltip next to a node centre (used on keyboard focus, no pointer).
  function tipAtNode(g){
    const gr=svgEl.getBoundingClientRect(), b=g.getBoundingClientRect();
    tip.style.left=(b.left-gr.left+b.width/2+14)+'px';
    tip.style.top=(b.top-gr.top+b.height/2+14)+'px';
  }
  function wire(n,g){
    g.__node = n;
    g.setAttribute('tabindex','-1');
    if(n.kind==='spec') g.setAttribute('role','button');
    g.setAttribute('aria-label', describe(n)); // screen-reader announces LOC/%tested/owning specs
    g.addEventListener('mouseenter',()=>enter(n,g));
    g.addEventListener('mousemove',moveTip);
    g.addEventListener('mouseleave',leave);
    g.addEventListener('focus',()=>{ setRovingTo(g); enter(n,g); tipAtNode(g); });
    g.addEventListener('blur',leave);
    g.addEventListener('pointerdown',(e)=>{
      e.stopPropagation(); e.preventDefault(); g.setPointerCapture(e.pointerId);
      let moved=0; const start=toSvg(e);
      const move=(ev)=>{ const p=toSvg(ev); moved+=Math.abs(p.x-start.x)+Math.abs(p.y-start.y); n.fx=p.x; n.fy=p.y; reheat(0.3); };
      const up=()=>{ n.fx=null; n.fy=null; g.removeEventListener('pointermove',move); g.removeEventListener('pointerup',up);
        if(moved<4 && n.kind==='spec'){ focusSpec(n.idx); } };
      g.addEventListener('pointermove',move); g.addEventListener('pointerup',up);
    });
  }
  const esc=s=>String(s).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  // Plain-text equivalent of the tooltip, used as the node's accessible name.
  function describe(n){
    if(n.kind==='spec'){
      const bits=[plur(n.fileCount,'file'), n.loc+' LOC'];
      if(n.updated) bits.push('updated '+n.updated);
      if(n.commits!=null) bits.push(plur(n.commits,'commit'));
      return `Spec ${n.label}${n.needs?', needs review':''}. ${bits.join(', ')}. Activate to focus this spec.`;
    }
    const rel = n.orphan ? 'no owning spec' : 'owned by '+n.specs.map(si=>data.specs[si].module).join(' and ');
    const tc = n.testPct==null ? '' : `, ${Math.round(n.testPct)} percent tested`;
    return `File ${n.name}, ${n.loc} LOC, ${n.lang}${tc}, ${rel}.`;
  }
  function showTip(n){
    if(n.kind==='spec'){
      const bits=[`${n.fileCount} files`,`${n.loc} LOC`]; if(n.updated) bits.push('updated '+esc(n.updated)); if(n.commits!=null) bits.push(n.commits+' commits');
      tip.innerHTML=`<b>${esc(n.label)}</b> spec${n.needs?' <span class="warnflag">needs review</span>':''}<span class="sub">${bits.join(' · ')}</span><span class="sub">click to focus</span>`;
    } else {
      const rel = n.orphan ? 'no spec' : n.specs.map(si=>esc(data.specs[si].module)).join(' + ');
      const tc = n.testPct==null ? '' : ` · ${Math.round(n.testPct)}% tested`;
      tip.innerHTML=`<b>${esc(n.name)}</b><span class="sub">${n.loc} LOC · ${esc(n.lang)}${tc}</span><span class="sub">${rel}</span>`;
    }
    tip.style.opacity=1;
  }
  function moveTip(e){ const r=svgEl.getBoundingClientRect(); tip.style.left=(e.clientX-r.left+14)+'px'; tip.style.top=(e.clientY-r.top+14)+'px'; }

  // ---- pan ----
  let panning=null;
  svgEl.addEventListener('pointerdown',(e)=>{ if(e.target.closest('.node')) return;
    panning={x:e.clientX,y:e.clientY,vbx:vb.x,vby:vb.y}; svgEl.style.cursor='grabbing'; });
  window.addEventListener('pointermove',(e)=>{ if(!panning) return; const r=svgEl.getBoundingClientRect();
    vb.x=panning.vbx-(e.clientX-panning.x)/r.width*vb.w; vb.y=panning.vby-(e.clientY-panning.y)/r.height*vb.h; setVB(); });
  window.addEventListener('pointerup',()=>{ panning=null; svgEl.style.cursor='grab'; });

  // ---- zoom / fit ----
  function zoomAt(px,py,k){ const nw=clamp(vb.w*k,W*0.1,W*4), nh=nw*(H/W);
    vb.x=px-(px-vb.x)*(nw/vb.w); vb.y=py-(py-vb.y)*(nh/vb.h); vb.w=nw; vb.h=nh; setVB(); }
  svgEl.addEventListener('wheel',(e)=>{ e.preventDefault(); const p=toSvg(e); zoomAt(p.x,p.y,e.deltaY>0?1.12:0.89); },{passive:false});
  function fit(){
    let x0=1e9,y0=1e9,x1=-1e9,y1=-1e9, any=false;
    for(const s of activeSpecs){ const r=(layout==='grouped'?s.R:s.nr||16)+20; any=true; x0=Math.min(x0,s.x-r);y0=Math.min(y0,s.y-r);x1=Math.max(x1,s.x+r);y1=Math.max(y1,s.y+r); }
    for(const f of activeFiles){ const p=posOf(f); any=true; x0=Math.min(x0,p.x-8);y0=Math.min(y0,p.y-8);x1=Math.max(x1,p.x+8);y1=Math.max(y1,p.y+8); }
    if(!any) return;
    const w=Math.max(x1-x0,80), h=Math.max(y1-y0,80), pad=1.08;
    let vw=Math.max(w,h*(W/H))*pad, vh=vw*(H/W);
    vb={ x:(x0+x1)/2-vw/2, y:(y0+y1)/2-vh/2, w:vw, h:vh }; setVB();
  }

  // ---- search ----
  let query='';
  function applySearch(){
    if(!query){ svgEl.classList.remove('searching'); [...activeSpecs,...activeFiles].forEach(n=>n.g&&n.g.classList.remove('match'));
      activeSpecs.forEach(s=>{ if(s.t && layout==='grouped' && focused==null && denseLabels && s.R<52) s.t.style.display='none'; });
      const c=document.getElementById('g-count'); if(c)c.textContent=''; return; }
    svgEl.classList.add('searching'); let count=0;
    for(const n of [...activeSpecs,...activeFiles]){ const hit=n.label.toLowerCase().includes(query); n.g.classList.toggle('match',hit); if(hit){ count++; if(n.kind==='spec'&&n.t) n.t.style.display=''; } }
    const c=document.getElementById('g-count'); if(c) c.textContent=count?`${count} match${count>1?'es':''}`:'no matches';
  }
  function fitMatches(){ const m=[...activeSpecs,...activeFiles].filter(n=>n.label.toLowerCase().includes(query)); if(!m.length) return;
    let x0=1e9,y0=1e9,x1=-1e9,y1=-1e9; for(const n of m){ const p=n.kind==='spec'?{x:n.x,y:n.y}:posOf(n); x0=Math.min(x0,p.x);y0=Math.min(y0,p.y);x1=Math.max(x1,p.x);y1=Math.max(y1,p.y); }
    const vw=Math.max(x1-x0,200)*1.6, vh=vw*(H/W); vb={x:(x0+x1)/2-vw/2,y:(y0+y1)/2-vh/2,w:vw,h:vh}; setVB();
  }

  // ---- focus ----
  function focusSpec(idx){ focused=idx; build(); prewarm(); fit(); updateFocusChip(); }
  function clearFocus(){ focused=null; build(); prewarm(); fit(); updateFocusChip(); }
  function updateFocusChip(){ const chip=document.getElementById('g-focus'); if(!chip) return;
    if(focused!=null){ chip.style.display='inline-flex'; chip.querySelector('span').textContent=data.specs[focused].module; } else chip.style.display='none'; }

  // ---- controls ----
  const $=id=>document.getElementById(id);
  // Keep aria-pressed of a toggle group in sync with its `.on` class.
  const syncPressed=sel=>document.querySelectorAll(sel).forEach(x=>x.setAttribute('aria-pressed', x.classList.contains('on')?'true':'false'));
  const ob=$('t-orphans'); if(ob){ ob.checked=showOrphans; ob.addEventListener('change',()=>{ showOrphans=ob.checked; build(); prewarm(); fit(); }); }
  const lb=$('t-labels'); if(lb){ lb.addEventListener('change',()=>{ showLabels=lb.checked; applyLabels(); }); }
  document.querySelectorAll('.cmode button').forEach(b=>b.addEventListener('click',()=>{
    document.querySelectorAll('.cmode button').forEach(x=>x.classList.remove('on')); b.classList.add('on'); colorMode=b.dataset.mode;
    syncPressed('.cmode button');
    activeFiles.forEach(f=>{ f.shape.style.fill=colorOf(f); });
  }));
  document.querySelectorAll('.lmode button').forEach(b=>b.addEventListener('click',()=>{
    if(b.dataset.layout===layout) return;
    document.querySelectorAll('.lmode button').forEach(x=>x.classList.remove('on')); b.classList.add('on');
    syncPressed('.lmode button');
    layout=b.dataset.layout; if(layout==='network'){ seedPositions(); } build(); prewarm(); fit();
  }));
  const search=$('g-search'); if(search){ search.addEventListener('input',()=>{ query=search.value.trim().toLowerCase(); applySearch(); });
    search.addEventListener('keydown',e=>{ if(e.key==='Enter') fitMatches(); }); }
  if($('g-zin')) $('g-zin').addEventListener('click',()=>zoomAt(vb.x+vb.w/2,vb.y+vb.h/2,0.8));
  if($('g-zout')) $('g-zout').addEventListener('click',()=>zoomAt(vb.x+vb.w/2,vb.y+vb.h/2,1.25));
  if($('g-fit')) $('g-fit').addEventListener('click',fit);
  if($('g-reset')) $('g-reset').addEventListener('click',()=>{ focused=null; query=''; if(search)search.value=''; layout='grouped';
    document.querySelectorAll('.lmode button').forEach(x=>x.classList.toggle('on',x.dataset.layout==='grouped'));
    syncPressed('.lmode button');
    seedPositions(); build(); prewarm(); fit(); updateFocusChip(); });
  if($('g-focus')) $('g-focus').addEventListener('click',clearFocus);

  const hash=(location.hash||'').replace('#','');
  if(['spec','lang','cov','age'].includes(hash)){ const b=document.querySelector(`.cmode button[data-mode="${hash}"]`);
    if(b){ document.querySelectorAll('.cmode button').forEach(x=>x.classList.remove('on')); b.classList.add('on'); colorMode=hash; } }

  syncPressed('.lmode button'); syncPressed('.cmode button');
  build(); prewarm(); fit();
})();
</script>
