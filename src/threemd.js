<script>
(function(){
  const dataEl = document.getElementById('atlas-data');
  if(!dataEl) return;
  let model = {};
  try { model = JSON.parse(dataEl.textContent); } catch(e){ return; }
  const docs = model.threemd || [];

  // ---- minimal, safe Markdown -> HTML (headings, lists, bold, code, links,
  //      and 3md [[z=N|label]] cross-links) ----
  const esc = s => s.replace(/[&<>]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;'}[c]));
  function inline(s){
    s = esc(s);
    s = s.replace(/`([^`]+)`/g, '<code>$1</code>');
    s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    s = s.replace(/\[\[z=([^\|\]]+)\|([^\]]+)\]\]/g, '<a class="xlink" data-z="$1">$2</a>');
    s = s.replace(/\[([^\]]+)\]\(([^)]+)\)/g, (m, text, url) => {
      const u = url.trim();
      const scheme = (u.match(/^([a-z][a-z0-9+.\-]*):/i) || [])[1];
      const safe = !scheme || /^(https?|mailto)$/i.test(scheme);
      const href = (safe ? u : '#').replace(/"/g, '%22');
      return '<a href="' + href + '" target="_blank" rel="noopener">' + text + '</a>';
    });
    return s;
  }
  function mdToHtml(md){
    const out = []; let list = false;
    const closeList = () => { if(list){ out.push('</ul>'); list = false; } };
    for(const raw of (md||'').split('\n')){
      const line = raw.replace(/\s+$/,'');
      const h = line.match(/^(#{1,6})\s+(.*)$/);
      if(h){ closeList(); out.push('<h'+h[1].length+'>'+inline(h[2])+'</h'+h[1].length+'>'); continue; }
      const li = line.match(/^\s*[-*]\s+(.*)$/);
      if(li){ if(!list){ out.push('<ul>'); list = true; } out.push('<li>'+inline(li[1])+'</li>'); continue; }
      if(!line.trim()){ closeList(); continue; }
      closeList(); out.push('<p>'+inline(line)+'</p>');
    }
    closeList();
    return out.join('');
  }

  document.querySelectorAll('.tmd').forEach(el=>{
    const doc = docs[+el.dataset.doc]; if(!doc) return;
    const planes = doc.planes || []; if(!planes.length) return;
    const stage = el.querySelector('.tmd-plane');
    const label = el.querySelector('.tmd-label');
    const slider = el.querySelector('.tmd-slider');
    let i = 0;
    function show(n){
      i = Math.max(0, Math.min(planes.length-1, n));
      const p = planes[i];
      stage.innerHTML = mdToHtml(p.md);
      label.textContent = (p.z!=='-'? 'z='+p.z+' · ' : '') + (p.label||'') + '  (' + (i+1) + '/' + planes.length + ')';
      slider.value = i;
    }
    el.querySelector('.tmd-prev').addEventListener('click', ()=>show(i-1));
    el.querySelector('.tmd-next').addEventListener('click', ()=>show(i+1));
    slider.addEventListener('input', ()=>show(+slider.value));
    stage.addEventListener('click', e=>{
      const x = e.target.closest('.xlink'); if(!x) return;
      const z = x.dataset.z;
      const idx = planes.findIndex(p=>p.z===z);
      if(idx>=0) show(idx);
    });
    show(0);
  });

  // ---- call-to-action buttons ----
  const note = document.getElementById('act-note');
  function flash(msg){ if(!note) return; note.textContent = msg; note.classList.add('show'); setTimeout(()=>note.classList.remove('show'), 1800); }
  async function copy(text, msg){
    try { await navigator.clipboard.writeText(text); flash(msg); }
    catch(e){
      const ta = document.createElement('textarea'); ta.value = text; document.body.appendChild(ta); ta.select();
      try { document.execCommand('copy'); flash(msg); } catch(_){ flash('copy failed'); }
      ta.remove();
    }
  }
  document.querySelectorAll('.actions .btn[data-act]').forEach(b=>{
    b.addEventListener('click', ()=>{
      const act = b.dataset.act;
      if(act==='copy-json'){ copy(dataEl.textContent, 'model JSON copied'); }
      else if(act==='copy-verdict'){ copy(model.verdict||'', 'verdict copied'); }
      else if(act==='copy-review'){
        const rows = (model.specs||[]).filter(s=>s.needs_review).map(s=>`- ${s.module}: ${s.review_reason||'review'} (${s.path})`);
        copy(rows.join('\n'), rows.length+' specs copied');
      }
      else if(act==='copy-orphans'){
        const rows = (model.files||[]).filter(f=>f.orphan).sort((a,b)=>b.loc-a.loc).map(f=>`${f.path} (${f.loc} LOC)`);
        copy(rows.join('\n'), rows.length+' orphan paths copied');
      }
      else if(act==='go-3md'){
        const t = document.getElementById('c-3md');
        if(t){ const chip = document.querySelector('.cbtoggle[data-target=c-3md]'); if(chip && t.style.display==='none') chip.click(); t.scrollIntoView({behavior:'smooth'}); }
      }
    });
  });
})();
</script>
