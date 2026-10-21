<script>
  import {onDestroy} from 'svelte';
  import {t,locale} from '../lib/i18n.svelte.js';
  let {id}=$props();
  let stats=$state(null),error=$state('');
  let timer, destroyed=false;
  const seconds=(ms)=>(ms/1000).toLocaleString(locale(),{maximumFractionDigits:3});
  const date=(ms)=>new Date(ms).toLocaleString(locale());
  async function refresh() {
    clearTimeout(timer);
    try {
      const response=await fetch(`/api/durations/${encodeURIComponent(id)}`);
      const data=await response.json();
      if (!response.ok) throw new Error(data.error || t('systeme.duree.indisponible'));
      if (!destroyed) {stats=data;error='';}
    } catch(e) {if (!destroyed) {stats=null;error=e.message;}}
    if (!destroyed) timer=setTimeout(refresh,3000);
  }
  $effect(()=>{id;refresh();});
  onDestroy(()=>{destroyed=true;clearTimeout(timer);});
</script>
<section aria-label={t('systeme.duree.bilan')}>
  {#if stats}
    <p>{t('systeme.duree.acquis',{seconds:seconds(stats.acquired_ms)})}</p>
    <p>{t('systeme.duree.couverture',{known:seconds(stats.known_ms),unknown:seconds(stats.unknown_ms)})}</p>
    <p>{t('systeme.duree.fenetre_connue',{start:date(stats.window_start_ms),end:date(stats.as_of_ms)})}</p>
    <p>{stats.observing?t('systeme.duree.observation'):t('systeme.duree.inconnu')}</p>
    {#if stats.entered_ms!=null}<p>{t('systeme.duree.entree',{date:date(stats.entered_ms)})}</p>{/if}
    {#if stats.limited}<p role="status">{t('systeme.duree.limite')}</p>{/if}
  {/if}
  {#if error}<p role="status">{error}</p>{/if}
</section>
<style>section {display:grid;gap:6px;padding:12px;border:1px solid var(--line);border-radius:var(--radius,6px);}p {margin:0;font-size:13px;}</style>
