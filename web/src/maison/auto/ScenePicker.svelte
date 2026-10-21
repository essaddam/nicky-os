<script>
  import { onMount } from 'svelte';
  import { t } from '../../lib/i18n.svelte.js';
  let { value='', onchange }=$props();
  let scenes=$state([]), error=$state('');
  onMount(async()=>{
    try {
      const response=await fetch('/api/dependencies');const data=await response.json();
      if (!response.ok) throw new Error(data.error || t('scenes.refus'));
      scenes=(data.dependencies ?? []).filter((d)=>d.reference.kind==='scene');
    } catch(e){error=e.message;}
  });
</script>
<label>{t('scenes.identite')}
  <select {value} onchange={(e)=>onchange?.(e.currentTarget.value)}>
    <option value="">{t('scenes.choisir')}</option>
    {#if value && !scenes.some((s)=>s.reference.id===value)}<option value={value}>{value}</option>{/if}
    {#each scenes as scene (scene.reference.id)}<option value={scene.reference.id}>{scene.reference.id} · {scene.versions.current ? t('scenes.option_version',{version:scene.versions.current.version}) : t('scenes.option_brouillon')}</option>{/each}
  </select>
</label>
{#if error}<p role="alert">{error}</p>{/if}
<style>label{display:grid;gap:6px;}select{font:inherit;color:var(--ink);background:var(--surface-2);border:1px solid var(--line);border-radius:12px;padding:10px;max-width:100%;}p{color:var(--alarm);}</style>
