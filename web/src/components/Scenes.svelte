<script>
  import { onMount } from 'svelte';
  import { t } from '../lib/i18n.svelte.js';
  import SceneEditor from './SceneEditor.svelte';
  let items=$state([]),error=$state('');
  async function load(){try{const response=await fetch('/api/dependencies');const data=await response.json();if(!response.ok)throw new Error(data.error);items=(data.dependencies??[]).filter((d)=>d.reference.kind==='scene');}catch(e){error=e.message;}}
  onMount(load);
</script>
{#if items.length || error}<section class="scenes"><h2>{t('scenes.titre')}</h2><p>{t('scenes.intro')}</p>{#each items as item (item.reference.id)}<SceneEditor id={item.reference.id}/>{/each}{#if error}<p role="alert">{error}</p>{/if}</section>{/if}
<style>.scenes{display:grid;gap:12px;}h2,p{margin:0;}p{color:var(--ink-2);}</style>
