<script>
(function(){
  var bar = document.getElementById('compbar');
  if(!bar) return;
  var KEY = 'atlas-hidden:' + (document.title || '');
  var hidden = {};
  try { hidden = JSON.parse(localStorage.getItem(KEY) || '{}') || {}; } catch(e){}
  function set(id, show){
    var el = document.getElementById(id); if(el) el.style.display = show ? '' : 'none';
    var btn = bar.querySelector('[data-target="' + id + '"]'); if(btn) btn.classList.toggle('on', show);
  }
  bar.querySelectorAll('.cbtoggle').forEach(function(btn){
    var id = btn.dataset.target;
    set(id, !hidden[id]);
    btn.addEventListener('click', function(){
      var el = document.getElementById(id); if(!el) return;
      var showing = el.style.display !== 'none';
      set(id, !showing);
      if(showing) hidden[id] = 1; else delete hidden[id];
      try { localStorage.setItem(KEY, JSON.stringify(hidden)); } catch(e){}
    });
  });
})();
</script>
