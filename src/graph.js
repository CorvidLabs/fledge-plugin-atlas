<script>
(function(){
  const dataEl = document.getElementById('atlas-data');
  const svgEl = document.getElementById('graph-svg');
  const tip = document.getElementById('tip');
  if(!dataEl || !svgEl) return;
  const data = JSON.parse(dataEl.textContent);
  const NS = 'http://www.w3.org/2000/svg';
  const W = 1180, H = 640;
  let vb = {x:0, y:0, w:W, h:H};
  svgEl.setAttribute('viewBox', `0 0 ${W} ${H}`);

  // Language palette (for the "by language" color mode).
  const LANGC = {};
  [...new Set(data.files.map(f=>f.lang))].forEach((l,i)=>{ LANGC[l] = `hsl(${(i*57+25)%360},45%,60%)`; });

  const specNodes = data.specs.map(s=>({
    id:'S'+s.index, kind:'spec', label:s.module, specColor:s.color, langColor:s.color,
    covColor: s.test_pct==null ? s.color : `hsl(${Math.round(s.test_pct*1.2)},60%,52%)`,
    r: Math.max(11, Math.min(26, 8+Math.sqrt(Math.max(s.loc,1))/7)),
    loc:s.loc, files:s.files
  }));
  const covColor = pct => pct==null ? '#3a424a' : `hsl(${Math.round(pct*1.2)},60%,52%)`; // red→green
  const fileNodes = data.files.map((f,i)=>({
    id:'F'+i, kind:'file', label:f.path, lang:f.lang, loc:f.loc,
    orphan:f.orphan, overlap:f.overlap, specs:f.specs, testPct:f.test_pct,
    specColor: f.orphan ? '#3a424a' : (f.specs.length===1 ? data.specs[f.specs[0]].color : '#e7ecef'),
    langColor: LANGC[f.lang], covColor: covColor(f.test_pct),
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
      const a = i*2.399963, rad = 40 + (i/nodes.length)*270;
      n.x = W/2 + Math.cos(a)*rad; n.y = H/2 + Math.sin(a)*rad*0.8;
      n.vx=0; n.vy=0; n.fx=null; n.fy=null;
    });
  }
  seedPositions();

  const orphanCount = fileNodes.filter(f=>f.orphan).length;
  let showOrphans = orphanCount <= 120;   // hide by default when there are many
  let showLabels = false;
  let colorMode = 'spec';
  let active = [], activeLinks = [];

  const gLinks = document.createElementNS(NS,'g');
  const gNodes = document.createElementNS(NS,'g');
  svgEl.appendChild(gLinks); svgEl.appendChild(gNodes);

  const colorOf = n => colorMode==='cov' ? n.covColor : (colorMode==='lang' ? n.langColor : n.specColor);

  function recompute(){
    active = nodes.filter(n => n.kind==='spec' || showOrphans || !n.orphan);
    const set = new Set(active.map(n=>n.id));
    activeLinks = links.filter(l=>set.has(l.source) && set.has(l.target));
  }

  function build(){
    gLinks.textContent=''; gNodes.textContent='';
    recompute();
    for(const l of activeLinks){
      const ln = document.createElementNS(NS,'line');
      ln.setAttribute('class','link'); l.el = ln; gLinks.appendChild(ln);
    }
    for(const n of active){
      const g = document.createElementNS(NS,'g');
      g.setAttribute('class','node '+n.kind);
      const c = document.createElementNS(NS,'circle');
      c.setAttribute('r', n.r); c.setAttribute('fill', colorOf(n));
      g.appendChild(c);
      if(n.kind==='spec'){
        const t = document.createElementNS(NS,'text');
        t.setAttribute('text-anchor','middle'); t.setAttribute('dy', -n.r-6);
        t.textContent = n.label; g.appendChild(t);
      }
      n.g=g; n.c=c; n.t=null; gNodes.appendChild(g);
      wire(n,g);
    }
    applyLabels();
  }

  function applyLabels(){
    for(const n of active){
      if(n.kind!=='file') continue;
      if(showLabels && !n.t){
        const t=document.createElementNS(NS,'text');
        t.setAttribute('text-anchor','middle'); t.setAttribute('dy', -n.r-4);
        t.textContent = n.label.split('/').pop();
        n.g.appendChild(t); n.t=t;
      } else if(!showLabels && n.t){ n.t.remove(); n.t=null; }
    }
  }

  function tick(alpha){
    const N = active.length;
    for(let i=0;i<N;i++){
      const a=active[i];
      for(let j=i+1;j<N;j++){
        const b=active[j];
        let dx=a.x-b.x, dy=a.y-b.y, d2=dx*dx+dy*dy || 0.01;
        if(d2>160000) continue;
        const d=Math.sqrt(d2);
        const span=a.r+b.r+150;
        const f=span*span/d2*0.032*alpha;
        const fx=dx/d*f, fy=dy/d*f;
        a.vx+=fx; a.vy+=fy; b.vx-=fx; b.vy-=fy;
      }
    }
    for(const l of activeLinks){
      const a=byId[l.source], b=byId[l.target];
      let dx=b.x-a.x, dy=b.y-a.y; const d=Math.sqrt(dx*dx+dy*dy)||0.01;
      const f=(d-115)/d*0.055*alpha, fx=dx*f, fy=dy*f;
      a.vx+=fx; a.vy+=fy; b.vx-=fx; b.vy-=fy;
    }
    for(const n of active){
      n.vx += (W/2-n.x)*0.0022*alpha; n.vy += (H/2-n.y)*0.0022*alpha;
      if(n.fx!=null){ n.x=n.fx; n.y=n.fy; n.vx=0; n.vy=0; }
      else {
        n.vx*=0.86; n.vy*=0.86; n.x+=n.vx; n.y+=n.vy;
        n.x=Math.max(n.r+2,Math.min(W-n.r-2,n.x));
        n.y=Math.max(n.r+2,Math.min(H-n.r-2,n.y));
      }
    }
  }

  function draw(){
    for(const l of activeLinks){
      const a=byId[l.source], b=byId[l.target];
      l.el.setAttribute('x1',a.x); l.el.setAttribute('y1',a.y);
      l.el.setAttribute('x2',b.x); l.el.setAttribute('y2',b.y);
    }
    for(const n of active){ n.g.setAttribute('transform',`translate(${n.x},${n.y})`); }
  }

  // Synchronous pre-warm so the layout is settled on first paint (and in a
  // headless screenshot) with no visible wobble.
  function prewarm(){ for(let i=0;i<230;i++){ tick(Math.max(1-i/230, 0.05)); } draw(); }

  let alpha=0, raf=null;
  function loop(){ alpha*=0.985; tick(Math.max(alpha,0.02)); draw(); if(alpha>0.04) raf=requestAnimationFrame(loop); else raf=null; }
  function reheat(a=0.6){ alpha=a; if(!raf) raf=requestAnimationFrame(loop); }

  function toSvg(e){
    const r=svgEl.getBoundingClientRect();
    return { x: vb.x + (e.clientX-r.left)/r.width*vb.w, y: vb.y + (e.clientY-r.top)/r.height*vb.h };
  }

  function wire(n,g){
    g.addEventListener('mouseenter',()=>{
      svgEl.classList.add('dim'); n.g.classList.add('lit');
      nbr[n.id].forEach(id=>{ const m=byId[id]; if(m&&m.g) m.g.classList.add('lit'); });
      for(const l of activeLinks){ if(l.source===n.id||l.target===n.id) l.el.classList.add('hot'); }
      showTip(n);
    });
    g.addEventListener('mousemove',moveTip);
    g.addEventListener('mouseleave',()=>{
      svgEl.classList.remove('dim');
      active.forEach(m=>m.g.classList.remove('lit'));
      activeLinks.forEach(l=>l.el.classList.remove('hot'));
      tip.style.opacity=0;
    });
    g.addEventListener('pointerdown',(e)=>{
      e.preventDefault(); g.setPointerCapture(e.pointerId);
      const move=(ev)=>{ const p=toSvg(ev); n.fx=p.x; n.fy=p.y; reheat(0.3); };
      const up=()=>{ n.fx=null; n.fy=null; g.removeEventListener('pointermove',move); g.removeEventListener('pointerup',up); };
      g.addEventListener('pointermove',move); g.addEventListener('pointerup',up);
    });
  }

  function showTip(n){
    if(n.kind==='spec'){
      tip.innerHTML=`<b>${n.label}</b> spec<span class="sub">${n.files} files · ${n.loc} LOC</span>`;
    } else {
      const rel = n.orphan ? 'orphan (no spec)' : n.specs.map(si=>data.specs[si].module).join(' + ');
      const tc = n.testPct==null ? '' : ` · ${Math.round(n.testPct)}% tested`;
      tip.innerHTML=`<b>${n.label}</b><span class="sub">${n.loc} LOC · ${n.lang}${tc} · ${rel}</span>`;
    }
    tip.style.opacity=1;
  }
  function moveTip(e){ const r=svgEl.getBoundingClientRect(); tip.style.left=(e.clientX-r.left+14)+'px'; tip.style.top=(e.clientY-r.top+14)+'px'; }

  // Controls
  const orphanBox=document.getElementById('t-orphans');
  if(orphanBox){ orphanBox.checked=showOrphans; orphanBox.addEventListener('change',()=>{ showOrphans=orphanBox.checked; build(); prewarm(); reheat(0.5); }); }
  const labelBox=document.getElementById('t-labels');
  if(labelBox){ labelBox.addEventListener('change',()=>{ showLabels=labelBox.checked; applyLabels(); }); }
  document.querySelectorAll('.cmode button').forEach(b=>b.addEventListener('click',()=>{
    document.querySelectorAll('.cmode button').forEach(x=>x.classList.remove('on'));
    b.classList.add('on'); colorMode=b.dataset.mode;
    active.forEach(n=>n.c.setAttribute('fill',colorOf(n)));
  }));
  const resetBtn=document.getElementById('g-reset');
  if(resetBtn){ resetBtn.addEventListener('click',()=>{ seedPositions(); vb={x:0,y:0,w:W,h:H}; svgEl.setAttribute('viewBox',`0 0 ${W} ${H}`); prewarm(); reheat(0.6); }); }

  svgEl.addEventListener('wheel',(e)=>{
    e.preventDefault(); const p=toSvg(e); const k=e.deltaY>0?1.1:0.9;
    const nw=Math.max(W*0.3,Math.min(W*2.5,vb.w*k)), nh=nw*(H/W);
    vb.x = p.x-(p.x-vb.x)*(nw/vb.w); vb.y = p.y-(p.y-vb.y)*(nh/vb.h); vb.w=nw; vb.h=nh;
    svgEl.setAttribute('viewBox',`${vb.x} ${vb.y} ${vb.w} ${vb.h}`);
  },{passive:false});

  // Deep-linkable color mode: #cov / #lang / #spec selects it on load.
  const hash=(location.hash||'').replace('#','');
  if(['spec','lang','cov'].includes(hash)){
    const b=document.querySelector(`.cmode button[data-mode="${hash}"]`);
    if(b){ document.querySelectorAll('.cmode button').forEach(x=>x.classList.remove('on')); b.classList.add('on'); colorMode=hash; }
  }

  build(); prewarm();
})();
</script>
