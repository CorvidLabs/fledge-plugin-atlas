<script>
(function(){
  var dataEl = document.getElementById('atlas-data');
  var body = document.getElementById('delta-body');
  if(!dataEl || !body) return;
  var model = {};
  try { model = JSON.parse(dataEl.textContent); } catch(e){ return; }

  var KEY = 'atlas-lastvisit:' + (document.title || model.project || '');
  var now = Math.floor(Date.now() / 1000);
  var specs = model.specs || [];

  var last = null;
  try {
    var raw = localStorage.getItem(KEY);
    if(raw !== null){ var v = parseInt(raw, 10); if(!isNaN(v)) last = v; }
  } catch(e){}

  function esc(s){
    return String(s).replace(/[&<>]/g, function(c){
      return {'&':'&amp;','<':'&lt;','>':'&gt;'}[c];
    });
  }
  function rel(sec){
    sec = Math.max(0, sec);
    if(sec < 3600) return Math.floor(sec / 60) + 'm';
    if(sec < 86400) return Math.floor(sec / 3600) + 'h';
    if(sec < 2592000) return Math.floor(sec / 86400) + 'd';
    if(sec < 31536000) return Math.floor(sec / 2592000) + 'mo';
    return Math.floor(sec / 31536000) + 'y';
  }

  var html;
  if(last === null){
    // First visit: nothing to diff against yet.
    html = '<p class="delta-first">First visit recorded. From now on this lists the specs that '
      + 'have changed since you were last here. ' + esc(String(specs.length))
      + ' spec' + (specs.length === 1 ? '' : 's') + ' are being tracked.</p>';
  } else {
    var changed = specs.filter(function(s){ return s.updated_ts && s.updated_ts > last; });
    changed.sort(function(a, b){ return (b.updated_ts || 0) - (a.updated_ts || 0); });
    var since = rel(now - last);
    if(changed.length === 0){
      html = '<p class="delta-empty">Nothing has changed since your last visit ('
        + esc(since) + ' ago). All ' + esc(String(specs.length)) + ' specs are as you left them.</p>';
    } else {
      html = '<p class="delta-lead">' + changed.length + ' spec'
        + (changed.length === 1 ? '' : 's') + ' changed since your last visit ('
        + esc(since) + ' ago):</p><ul class="delta-list">';
      changed.forEach(function(s){
        var meta = [];
        if(s.updated) meta.push(esc(s.updated));
        if(s.commits != null) meta.push(esc(String(s.commits)) + ' commits');
        html += '<li><span class="dl-mod">' + esc(s.module) + '</span>'
          + '<span class="dl-meta">' + meta.join(' &middot; ') + '</span></li>';
      });
      html += '</ul>';
    }
  }
  body.innerHTML = html;

  // Record this visit only AFTER the delta above was computed against the old one.
  try { localStorage.setItem(KEY, String(now)); } catch(e){}
})();
</script>
