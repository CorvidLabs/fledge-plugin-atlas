<script>
(function(){
  const dataEl = document.getElementById('atlas-data');
  const svgEl = document.getElementById('deps-svg');
  const tip = document.getElementById('deps-tip');
  const note = document.getElementById('deps-note');
  if(!dataEl || !svgEl) return;
  let data;
  try { data = JSON.parse(dataEl.textContent); } catch(e){ return; }
  const specs = data.specs || [];
  const NS = 'http://www.w3.org/2000/svg';
  const mk = (t, cls) => { const e = document.createElementNS(NS, t); if(cls) e.setAttribute('class', cls); return e; };
  const clamp = (v,a,b)=>Math.max(a,Math.min(b,v));
  const esc = s => String(s).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));

  // ---- resolve spec->spec edges from depends_on ----
  const modIdx = {};
  specs.forEach(s => { modIdx[s.module] = s.index; });
  const byIndex = {};
  specs.forEach(s => { byIndex[s.index] = s; });

  const outAdj = {};   // index -> [target indices it depends on]
  const inAdj = {};    // index -> [indices that depend on it]
  specs.forEach(s => { outAdj[s.index] = []; inAdj[s.index] = []; });
  const edges = [];
  specs.forEach(s => (s.depends_on || []).forEach(m => {
    const t = modIdx[m];
    if(t == null || t === s.index) return;
    if(outAdj[s.index].indexOf(t) !== -1) return;
    outAdj[s.index].push(t);
    inAdj[t].push(s.index);
    edges.push({ from: s.index, to: t });
  }));

  // Participants: any spec on either end of an edge.
  const part = new Set();
  edges.forEach(e => { part.add(e.from); part.add(e.to); });

  // ---- graceful degrade: nothing declares depends_on ----
  const wrap = svgEl.parentNode;
  if(!edges.length){
    if(wrap) wrap.style.display = 'none';
    if(note){
      note.classList.add('shown');
      note.textContent = specs.length
        ? 'No spec declares depends_on, so there is no dependency graph to draw yet. Add a depends_on: list to a spec to map how your modules relate.'
        : 'No specs found to map.';
    }
    return;
  }

  // ---- layering: level = longest chain of dependencies below a node ----
  // Leaves (depend on nothing) sit at level 0; orchestrators rise to the top.
  const level = {}, state = {};
  const cycleEdges = new Set(), cycleNodes = new Set();
  function depth(i){
    if(state[i] === 2) return level[i];
    if(state[i] === 1) return 0; // guarded per-edge below
    state[i] = 1;
    let m = 0;
    for(const t of outAdj[i]){
      if(state[t] === 1){ cycleEdges.add(i+'>'+t); cycleNodes.add(i); cycleNodes.add(t); continue; }
      m = Math.max(m, depth(t) + 1);
    }
    level[i] = m; state[i] = 2; return m;
  }
  part.forEach(i => depth(i));

  // ---- hub detection: nodes many others depend on ----
  const inDeg = {}; part.forEach(i => inDeg[i] = inAdj[i].length);
  const maxIn = Math.max(...part.size ? [...part].map(i=>inDeg[i]) : [0]);
  const hubCut = Math.max(3, maxIn);
  const isHub = i => inDeg[i] >= 3 || (maxIn >= 2 && inDeg[i] === maxIn);

  // ---- geometry ----
  const nodes = [...part].map(i => {
    const s = byIndex[i];
    const loc = s.loc || 0;
    return {
      idx: i, module: s.module, color: s.color || 'var(--accent)',
      loc: loc, share: s.share_pct || 0, lvl: level[i] || 0,
      inDeg: inDeg[i], outDeg: outAdj[i].length,
      hub: isHub(i), cyc: cycleNodes.has(i),
      r: clamp(11 + Math.sqrt(Math.max(loc,1)) / 3.2, 13, 40)
    };
  });
  const nodeByIdx = {}; nodes.forEach(n => nodeByIdx[n.idx] = n);

  const levels = {};
  nodes.forEach(n => { (levels[n.lvl] || (levels[n.lvl] = [])).push(n); });
  const lvKeys = Object.keys(levels).map(Number).sort((a,b)=>a-b);
  const maxLvl = lvKeys.length ? lvKeys[lvKeys.length-1] : 0;

  const padX = 90, padY = 54;
  const rowGap = 118;
  const H = padY*2 + Math.max(maxLvl, 1) * rowGap;
  // Width scales with the widest row so crowded layers stay legible.
  const widest = Math.max(1, ...lvKeys.map(k => levels[k].length));
  const W = clamp(padX*2 + widest * 150, 720, 1180);

  lvKeys.forEach(k => {
    const row = levels[k];
    // Bigger, more-depended-on modules toward the centre for a calmer read.
    row.sort((a,b) => (b.inDeg - a.inDeg) || (b.loc - a.loc));
    const n = row.length;
    row.forEach((node, k2) => {
      node.x = W * (k2 + 1) / (n + 1);
      node.y = padY + (maxLvl - node.lvl) * rowGap;
    });
  });

  // Horizontal slack so labels on the edge-most nodes are not clipped; the
  // viewBox scales to the container, so this just reserves room, it doesn't overflow.
  const LM = 70;
  svgEl.setAttribute('viewBox', `${-LM} 0 ${W + LM * 2} ${H}`);
  svgEl.setAttribute('preserveAspectRatio', 'xMidYMid meet');

  const gEdges = mk('g'), gArrows = mk('g'), gNodes = mk('g');
  svgEl.appendChild(gEdges); svgEl.appendChild(gArrows); svgEl.appendChild(gNodes);

  // ---- edges + arrowheads ----
  const edgeEls = [];
  edges.forEach(e => {
    const a = nodeByIdx[e.from], b = nodeByIdx[e.to];
    if(!a || !b) return;
    const dx = b.x - a.x, dy = b.y - a.y;
    const len = Math.hypot(dx, dy) || 1;
    const ux = dx/len, uy = dy/len;
    const sx = a.x + ux * (a.r + 2), sy = a.y + uy * (a.r + 2);
    const ex = b.x - ux * (b.r + 8), ey = b.y - uy * (b.r + 8);
    const cyc = cycleEdges.has(e.from+'>'+e.to);

    const line = mk('line', 'dep-edge' + (cyc ? ' cyc' : ''));
    line.setAttribute('x1', sx); line.setAttribute('y1', sy);
    line.setAttribute('x2', ex); line.setAttribute('y2', ey);
    if(cyc) line.style.stroke = 'var(--bad)';
    gEdges.appendChild(line);

    // manual arrowhead (fill via style so CSS vars resolve, matching graph.js)
    const ah = 8, aw = 4.4;
    const px = -uy, py = ux;
    const p1 = (ex) + ',' + (ey);
    const p2 = (ex - ux*ah + px*aw) + ',' + (ey - uy*ah + py*aw);
    const p3 = (ex - ux*ah - px*aw) + ',' + (ey - uy*ah - py*aw);
    const head = mk('polygon', 'dep-arrow' + (cyc ? ' cyc' : ''));
    head.setAttribute('points', p1 + ' ' + p2 + ' ' + p3);
    head.style.fill = cyc ? 'var(--bad)' : 'var(--muted)';
    gArrows.appendChild(head);

    const rec = { el: line, head: head, from: e.from, to: e.to, cyc: cyc };
    edgeEls.push(rec);
    (a.oEdges || (a.oEdges = [])).push(rec);
    (b.iEdges || (b.iEdges = [])).push(rec);
  });

  // ---- nodes ----
  nodes.forEach(n => {
    const g = mk('g', 'dep-node' + (n.hub ? ' hub' : '') + (n.cyc ? ' cyc' : ''));
    g.setAttribute('transform', `translate(${n.x},${n.y})`);
    if(n.hub){
      const ring = mk('circle', 'dep-ring');
      ring.setAttribute('r', n.r + 4);
      g.appendChild(ring);
    }
    const c = mk('circle', 'dep-disc');
    c.setAttribute('r', n.r);
    c.style.fill = `color-mix(in srgb, ${n.color} 26%, var(--bg))`;
    c.style.stroke = n.color;
    n.disc = c; g.appendChild(c);

    const label = mk('text', 'dep-label');
    label.setAttribute('y', n.r + 14);
    label.setAttribute('text-anchor', 'middle');
    label.textContent = n.module;
    label.style.fill = n.color;
    g.appendChild(label);

    g.addEventListener('mouseenter', () => hover(n));
    g.addEventListener('mousemove', moveTip);
    g.addEventListener('mouseleave', unhover);
    n.g = g;
    gNodes.appendChild(g);
  });

  // ---- hover: trace dependencies and dependents ----
  function hover(n){
    svgEl.classList.add('tracing');
    n.g.classList.add('lit');
    const lit = new Set([n.idx]);
    (n.oEdges||[]).forEach(r => { r.el.classList.add('hot'); r.head.classList.add('hot'); lit.add(r.to); });
    (n.iEdges||[]).forEach(r => { r.el.classList.add('hot'); r.head.classList.add('hot'); lit.add(r.from); });
    lit.forEach(i => { const nn = nodeByIdx[i]; if(nn) nn.g.classList.add('lit'); });
    showTip(n);
  }
  function unhover(){
    svgEl.classList.remove('tracing');
    nodes.forEach(nn => nn.g.classList.remove('lit'));
    edgeEls.forEach(r => { r.el.classList.remove('hot'); r.head.classList.remove('hot'); });
    tip.style.opacity = 0;
  }
  function showTip(n){
    const deps = (outAdj[n.idx]||[]).map(i => byIndex[i] && byIndex[i].module).filter(Boolean);
    const rdeps = (inAdj[n.idx]||[]).map(i => byIndex[i] && byIndex[i].module).filter(Boolean);
    const rows = [];
    rows.push(`<span class="sub">${n.loc} LOC${n.hub ? ' <span class="hubflag">hub</span>' : ''}${n.cyc ? ' <span class="cycflag">in cycle</span>' : ''}</span>`);
    rows.push(`<span class="sub">depends on: ${deps.length ? deps.map(esc).join(', ') : 'nothing'}</span>`);
    rows.push(`<span class="sub">depended on by: ${rdeps.length ? rdeps.map(esc).join(', ') : 'nothing'}</span>`);
    tip.innerHTML = `<b>${esc(n.module)}</b>` + rows.join('');
    tip.style.opacity = 1;
  }
  function moveTip(e){
    const r = svgEl.getBoundingClientRect();
    tip.style.left = (e.clientX - r.left + 14) + 'px';
    tip.style.top = (e.clientY - r.top + 14) + 'px';
  }

  // ---- summary note under the graph ----
  if(note){
    const hubs = nodes.filter(n => n.hub).map(n => n.module);
    const bits = [`${part.size} of ${specs.length} specs form the dependency graph`, `${edges.length} edge${edges.length===1?'':'s'}`];
    if(hubs.length) bits.push(`hub${hubs.length===1?'':'s'}: ${hubs.slice(0,4).map(esc).join(', ')}${hubs.length>4?', ...':''}`);
    if(cycleNodes.size) bits.push(`<span class="cycflag">${cycleEdges.size} cycle edge${cycleEdges.size===1?'':'s'}</span>`);
    note.classList.add('shown');
    note.innerHTML = bits.join(' · ');
  }
})();
</script>
