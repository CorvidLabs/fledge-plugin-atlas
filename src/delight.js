<script>
(function(){
  const dataEl = document.getElementById('atlas-data');
  if(!dataEl) return;
  let data;
  try { data = JSON.parse(dataEl.textContent); } catch(e){ return; }
  const NS = 'http://www.w3.org/2000/svg';
  const mk = (t, cls) => { const e = document.createElementNS(NS, t); if(cls) e.setAttribute('class', cls); return e; };
  const clamp = (v,a,b)=>Math.max(a,Math.min(b,v));
  const esc = s => String(s).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const files = Array.isArray(data.files) ? data.files : [];
  const specs = Array.isArray(data.specs) ? data.specs : [];
  const stats = data.stats || {};
  const hasCov = stats.test_coverage_pct != null;
  const NOSPEC = 'var(--surface-strong)';

  // Shade chart-4 (green) toward bad (clay) by coverage percentage.
  const covFill = pct => `color-mix(in srgb, var(--chart-4) ${Math.round(clamp(pct,0,100))}%, var(--bad))`;

  // A source file's treemap / covered colour.
  function fileFill(f){
    if(f.orphan) return NOSPEC;
    if(hasCov && f.test_pct != null) return covFill(f.test_pct);
    return 'var(--chart-1)';
  }

  const note = (id, text) => {
    const el = document.getElementById(id);
    if(el) el.innerHTML = '<p class="empty">' + text + '</p>';
  };

  // Tooltip wiring shared by all three visuals.
  function bindTip(host, tip, target, html){
    target.addEventListener('mouseenter', ()=>{ tip.innerHTML = html(); tip.style.opacity = 1; });
    target.addEventListener('mousemove', e=>{
      const r = host.getBoundingClientRect();
      tip.style.left = (e.clientX - r.left + 14) + 'px';
      tip.style.top = (e.clientY - r.top + 14) + 'px';
    });
    target.addEventListener('mouseleave', ()=>{ tip.style.opacity = 0; });
  }

  // ---- (1) squarified treemap ---------------------------------------------
  function squarify(items, X, Y, W, H){
    // items: [{value, ref}] -> [{ref, x, y, w, h}]
    const live = items.filter(d=>d.value > 0);
    const total = live.reduce((a,d)=>a+d.value, 0);
    if(total <= 0 || W <= 0 || H <= 0) return [];
    const scale = (W*H)/total;
    const boxes = live.map(d=>({ref:d.ref, area:d.value*scale})).sort((a,b)=>b.area-a.area);
    const out = [];
    let x=X, y=Y, w=W, h=H, i=0, row=[];
    const worst = (arr, len)=>{
      if(!arr.length || len <= 0) return Infinity;
      const s = arr.reduce((a,r)=>a+r.area, 0);
      let mx=-Infinity, mn=Infinity;
      arr.forEach(r=>{ if(r.area>mx) mx=r.area; if(r.area<mn) mn=r.area; });
      const l2=len*len, s2=s*s;
      return Math.max(l2*mx/s2, s2/(l2*mn));
    };
    const flush = ()=>{
      const s = row.reduce((a,r)=>a+r.area, 0);
      if(s <= 0){ row=[]; return; }
      if(w >= h){
        const dw = s/h; let cy=y;
        row.forEach(r=>{ const rh=r.area/dw; out.push({ref:r.ref, x, y:cy, w:dw, h:rh}); cy+=rh; });
        x += dw; w -= dw;
      } else {
        const dh = s/w; let cx=x;
        row.forEach(r=>{ const rw=r.area/dh; out.push({ref:r.ref, x:cx, y, w:rw, h:dh}); cx+=rw; });
        y += dh; h -= dh;
      }
      row = [];
    };
    while(i < boxes.length){
      const len = Math.min(w, h);
      const cur = boxes[i];
      if(row.length === 0 || worst(row.concat([cur]), len) <= worst(row, len)){
        row.push(cur); i++;
      } else {
        flush();
      }
    }
    if(row.length) flush();
    return out;
  }

  function drawTreemap(){
    const host = document.getElementById('tm-wrap');
    const svg = document.getElementById('tm-svg');
    const tip = document.getElementById('tm-tip');
    if(!host || !svg) return;
    if(!files.length){ note('tm-wrap', 'No source files to map yet.'); return; }
    const W = 1180, H = 620;
    svg.setAttribute('viewBox', `0 0 ${W} ${H}`);
    const rects = squarify(files.map(f=>({value:Math.max(f.loc, 1), ref:f})), 0, 0, W, H);
    if(!rects.length){ note('tm-wrap', 'No source files to map yet.'); return; }
    rects.forEach(r=>{
      const f = r.ref;
      const cell = mk('g', 'tm-cell');
      const rc = mk('rect');
      rc.setAttribute('x', r.x); rc.setAttribute('y', r.y);
      rc.setAttribute('width', Math.max(0, r.w)); rc.setAttribute('height', Math.max(0, r.h));
      rc.style.fill = fileFill(f);
      cell.appendChild(rc);
      // Only label roomy cells; keep it clean.
      if(r.w > 46 && r.h > 20){
        const name = f.path.split('/').pop();
        const t = mk('text', 'tm-label');
        t.setAttribute('x', r.x + 5); t.setAttribute('y', r.y + 15);
        t.textContent = name.length > Math.floor(r.w/7) ? name.slice(0, Math.max(1, Math.floor(r.w/7)-1)) + '…' : name;
        cell.appendChild(t);
      }
      const state = f.orphan ? 'no spec' : (f.overlap ? 'shared by 2+ specs' : 'has a spec');
      const tc = f.test_pct != null ? ' · ' + Math.round(f.test_pct) + '% tested' : '';
      bindTip(host, tip, cell, ()=>`<b>${esc(f.path.split('/').pop())}</b><span class="sub">${f.loc} LOC · ${esc(f.lang)}</span><span class="sub">${esc(state)}${tc}</span>`);
      svg.appendChild(cell);
    });
  }

  // ---- (2) coverage sunburst ----------------------------------------------
  function arcPath(cx, cy, r0, r1, a0, a1){
    const p = (r,a)=>[cx + r*Math.cos(a), cy + r*Math.sin(a)];
    const large = (a1 - a0) > Math.PI ? 1 : 0;
    const [x0,y0]=p(r1,a0), [x1,y1]=p(r1,a1), [x2,y2]=p(r0,a1), [x3,y3]=p(r0,a0);
    return `M${x0} ${y0}A${r1} ${r1} 0 ${large} 1 ${x1} ${y1}L${x2} ${y2}A${r0} ${r0} 0 ${large} 0 ${x3} ${y3}Z`;
  }

  function drawSunburst(){
    const host = document.getElementById('sb-wrap');
    const svg = document.getElementById('sb-svg');
    const tip = document.getElementById('sb-tip');
    if(!host || !svg) return;
    if(!specs.length && !files.some(f=>f.orphan)){ note('sb-wrap', 'No specs to chart yet.'); return; }
    const W = 720, H = 620, cx = W/2, cy = H/2;
    svg.setAttribute('viewBox', `0 0 ${W} ${H}`);
    const rIn = 92, rMid = 190, rOut = 262;

    // Inner segments: one per spec (sized by loc, fallback share_pct), plus an
    // orphan segment at the end.
    const segs = specs.map(s=>({
      kind:'spec', label:s.module, color:s.color,
      value:Math.max(s.loc || 0, s.share_pct || 0, 0.001),
      files: files.filter(f=>Array.isArray(f.specs) && f.specs.indexOf(s.index) !== -1)
    }));
    const orphanFiles = files.filter(f=>f.orphan);
    if(orphanFiles.length){
      segs.push({ kind:'orphan', label:'no spec', color:NOSPEC,
        value:Math.max(orphanFiles.reduce((a,f)=>a+Math.max(f.loc,1), 0), 0.001),
        files:orphanFiles });
    }
    const total = segs.reduce((a,s)=>a+s.value, 0);
    if(total <= 0){ note('sb-wrap', 'No specs to chart yet.'); return; }

    let a = -Math.PI/2;
    segs.forEach(seg=>{
      const span = seg.value/total * Math.PI*2;
      const a0 = a, a1 = a + span;
      // inner arc (the spec / orphan band)
      const inner = mk('path', 'sb-arc');
      inner.setAttribute('d', arcPath(cx, cy, rIn, rMid, a0, a1));
      inner.style.fill = seg.color;
      inner.style.fillOpacity = seg.kind === 'orphan' ? 1 : 0.9;
      const scov = seg.kind === 'orphan' ? 'no spec' : 'spec';
      bindTip(host, tip, inner, ()=>`<b>${esc(seg.label)}</b><span class="sub">${scov} · ${Math.round(seg.value)} LOC</span><span class="sub">${seg.files.length} file(s)</span>`);
      svg.appendChild(inner);

      // outer arcs (this segment's files, sub-divided by file loc)
      const fTot = seg.files.reduce((a,f)=>a+Math.max(f.loc,1), 0);
      let fa = a0;
      seg.files.forEach(f=>{
        const fspan = fTot > 0 ? Math.max(f.loc,1)/fTot * span : 0;
        if(fspan <= 0) return;
        const o = mk('path', 'sb-arc');
        o.setAttribute('d', arcPath(cx, cy, rMid+3, rOut, fa, fa+fspan));
        o.style.fill = f.orphan ? NOSPEC : (hasCov && f.test_pct != null ? covFill(f.test_pct) : 'var(--chart-1)');
        const tc = f.test_pct != null ? ' · ' + Math.round(f.test_pct) + '% tested' : '';
        bindTip(host, tip, o, ()=>`<b>${esc(f.path.split('/').pop())}</b><span class="sub">${f.loc} LOC${tc}</span><span class="sub">${f.orphan ? 'no spec' : esc(seg.label)}</span>`);
        svg.appendChild(o);
        fa += fspan;
      });
      a = a1;
    });

    // centre coverage label
    const covPct = hasCov ? stats.test_coverage_pct : stats.coverage_pct;
    const covLbl = hasCov ? 'test coverage' : 'spec coverage';
    const big = mk('text', 'sb-center');
    big.setAttribute('x', cx); big.setAttribute('y', cy - 2);
    big.textContent = (covPct != null ? Math.round(covPct) : 0) + '%';
    svg.appendChild(big);
    const sub = mk('text', 'sb-center-sub');
    sub.setAttribute('x', cx); sub.setAttribute('y', cy + 20);
    sub.textContent = covLbl;
    svg.appendChild(sub);
  }

  // ---- (3) churn vs coverage quadrant -------------------------------------
  function drawQuadrant(){
    const host = document.getElementById('qd-wrap');
    const svg = document.getElementById('qd-svg');
    const tip = document.getElementById('qd-tip');
    if(!host || !svg) return;
    if(!specs.length){ note('qd-wrap', 'No specs to plot yet.'); return; }

    // X = churn. Prefer commit counts; fall back to recency of last change.
    const commitsKnown = specs.some(s=>s.commits != null);
    const tss = specs.map(s=>s.updated_ts).filter(t=>t!=null);
    const tmin = tss.length ? Math.min(...tss) : 0;
    const tspan = Math.max(1, (tss.length ? Math.max(...tss) : 1) - tmin);
    let maxC = 1;
    specs.forEach(s=>{ if(s.commits != null && s.commits > maxC) maxC = s.commits; });
    const churnOf = s => {
      if(commitsKnown) return s.commits != null ? s.commits/maxC : 0;
      if(s.updated_ts == null) return 0;
      return (s.updated_ts - tmin)/tspan; // recency: 1 = freshest = most churn
    };
    // Y = coverage: test_pct if known, else share of codebase.
    const covOf = s => s.test_pct != null ? clamp(s.test_pct,0,100) : clamp(s.share_pct||0,0,100);

    const W = 1180, H = 620;
    const px = 46, pxr = 20, pyt = 20, pyb = 46;
    const x0 = px, x1 = W - pxr, y0 = pyt, y1 = H - pyb;
    svg.setAttribute('viewBox', `0 0 ${W} ${H}`);

    // "watch" corner: high churn (right), low coverage (bottom).
    const shade = mk('rect');
    shade.setAttribute('x', (x0+x1)/2); shade.setAttribute('y', (y0+y1)/2);
    shade.setAttribute('width', (x1-x0)/2); shade.setAttribute('height', (y1-y0)/2);
    shade.style.fill = 'color-mix(in srgb, var(--bad) 12%, transparent)';
    svg.appendChild(shade);
    const wl = mk('text', 'qd-watch');
    wl.setAttribute('x', x1 - 10); wl.setAttribute('y', y1 - 10);
    wl.textContent = 'watch';
    svg.appendChild(wl);

    // axes
    const ax = mk('line', 'qd-axis');
    ax.setAttribute('x1', x0); ax.setAttribute('y1', y1); ax.setAttribute('x2', x1); ax.setAttribute('y2', y1);
    svg.appendChild(ax);
    const ay = mk('line', 'qd-axis');
    ay.setAttribute('x1', x0); ay.setAttribute('y1', y0); ay.setAttribute('x2', x0); ay.setAttribute('y2', y1);
    svg.appendChild(ay);
    const xlab = mk('text', 'qd-axlabel');
    xlab.setAttribute('x', (x0+x1)/2); xlab.setAttribute('y', H - 12); xlab.setAttribute('text-anchor', 'middle');
    xlab.textContent = commitsKnown ? 'change activity (commits) →' : 'recency of change →';
    svg.appendChild(xlab);
    const ylab = mk('text', 'qd-axlabel');
    ylab.setAttribute('x', 16); ylab.setAttribute('y', (y0+y1)/2); ylab.setAttribute('text-anchor', 'middle');
    ylab.setAttribute('transform', `rotate(-90 16 ${(y0+y1)/2})`);
    ylab.textContent = (specs.some(s=>s.test_pct!=null) ? 'test coverage' : 'share of codebase') + ' →';
    svg.appendChild(ylab);

    const sx = v => x0 + clamp(v,0,1)*(x1-x0);
    const sy = v => y1 - clamp(v,0,100)/100*(y1-y0);
    specs.forEach(s=>{
      const cxp = sx(churnOf(s)), cyp = sy(covOf(s));
      const g = mk('g', 'qd-pt');
      const dot = mk('circle');
      dot.setAttribute('cx', cxp); dot.setAttribute('cy', cyp);
      dot.setAttribute('r', 6);
      dot.style.fill = s.color; dot.style.stroke = 'var(--bg)';
      g.appendChild(dot);
      const t = mk('text', 'qd-dotlabel');
      t.setAttribute('x', cxp + 9); t.setAttribute('y', cyp + 4);
      t.textContent = s.module;
      t.style.fill = s.color;
      g.appendChild(t);
      const churnTxt = commitsKnown ? ((s.commits!=null?s.commits:0) + ' commits') : (s.updated || 'unknown');
      const covTxt = s.test_pct != null ? Math.round(s.test_pct) + '% tested' : Math.round(s.share_pct||0) + '% of code';
      bindTip(host, tip, g, ()=>`<b>${esc(s.module)}</b><span class="sub">${churnTxt}</span><span class="sub">${covTxt}</span>`);
      svg.appendChild(g);
    });
  }

  try { drawTreemap(); } catch(e){ note('tm-wrap', 'Treemap unavailable.'); }
  try { drawSunburst(); } catch(e){ note('sb-wrap', 'Sunburst unavailable.'); }
  try { drawQuadrant(); } catch(e){ note('qd-wrap', 'Quadrant unavailable.'); }
})();
</script>
