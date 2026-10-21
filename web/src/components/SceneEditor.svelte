<script>
  import { onDestroy } from 'svelte';
  import { hub } from '../lib/hub.svelte.js';
  import { t, locale } from '../lib/i18n.svelte.js';
  import { previewMatchesShown } from '../maison/lib/weekly.js';
  import { scenePayload, scenePreviewMatches, sceneNeedsVersion } from '../maison/lib/scenes.js';
  import { pointSpec, pointName, valueText } from '../maison/lib/auto.svelte.js';
  import PointPicker from '../maison/auto/PointPicker.svelte';
  import Fields from '../maison/auto/Fields.svelte';
  import HumanSession from './HumanSession.svelte';
  let { id } = $props();
  let open = $state(false), busy = $state(false), error = $state(''), view = $state(null);
  let rows = $state([]), parallel = $state(4), timeout = $state(60), confirmation = $state(5);
  let saved = $state(''), preview = $state(null), mode = $state('publish'), run = $state(null);
  let timer, destroyed = false, polling = false;
  const path = $derived(`/api/dependencies/scene/${encodeURIComponent(id)}`);
  const dirty = $derived(JSON.stringify({rows,parallel,timeout,confirmation}) !== saved);
  const scope = $derived(preview?.scene ?? preview?.scope);
  const needsVersion = $derived(mode === 'publish' && sceneNeedsVersion(view,preview));
  function targetValue(target) {
    if (typeof target.value === 'number') return target.value.toLocaleString(locale(), {useGrouping:false,maximumFractionDigits:20}) + (target.unit ? ` ${target.unit}` : '');
    return valueText(target.point,target.value);
  }
  onDestroy(() => { destroyed = true; clearTimeout(timer); });
  async function call(url, method = 'GET', body) {
    const response = await fetch(url, { method, headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' }, body: body === undefined ? undefined : JSON.stringify(body) });
    const data = await response.json();
    if (!response.ok) throw new Error(data.error || t('scenes.refus'));
    return data;
  }
  async function work(action) { busy = true; error = ''; try { await action(); } catch (e) { error = e.message || t('scenes.refus'); } finally { busy = false; } }
  function show(data) {
    view = data;
    const draft = data.versions.draft.definition.value;
    rows = draft.targets.map((r) => ({...r})); parallel = draft.parallel; timeout = draft.timeout_s; confirmation = draft.confirmation_s;
    saved = JSON.stringify({rows,parallel,timeout,confirmation}); preview = null;
  }
  async function load() { await work(async () => show(await call(path))); }
  async function toggle() { open = !open; if (open) await load(); }
  function pick(i, point) {
    const spec = pointSpec(point);
    rows[i] = {point,value:spec?.kind.type === 'binary' ? true : spec?.kind.type === 'numeric' ? spec.kind.min ?? 0 : spec?.kind.values?.[0] ?? ''};
  }
  async function save() {
    const definition = scenePayload(rows, parallel, timeout, confirmation);
    if (!definition) { error = t('scenes.invalide'); return; }
    await work(async () => show(await call(path, 'PUT', {expected:view.versions.draft.version,definition})));
  }
  async function inspect(kind) {
    await work(async () => {
      const candidate = await call(kind === 'publish' ? path + '/preview' : `/api/scenes/${encodeURIComponent(id)}/preview`);
      if (!(kind === 'publish' ? previewMatchesShown(view,candidate) : scenePreviewMatches(view,candidate))) throw new Error(t('scenes.modifie'));
      preview = candidate; mode = kind;
    });
  }
  async function approve() {
    const shown = preview;
    if (dirty || needsVersion || !previewMatchesShown(view,shown)) { error = t('scenes.modifie'); return; }
    await work(async () => show(await call(path + '/approve','POST',shown)));
  }
  async function poll() {
    clearTimeout(timer);
    if (destroyed || polling || !run || run.status !== 'running') return;
    polling = true;
    const id = run.id;
    try {
      const next = await call(`/api/scene-runs/${encodeURIComponent(id)}`);
      if (!destroyed && run?.id === id) run = next;
    } catch(e) { error = e.message; return; }
    finally { polling = false; }
    if (!destroyed && run?.id === id && run.status === 'running') timer = setTimeout(poll,750);
  }
  async function execute() {
    const shown = preview;
    if (dirty || !scenePreviewMatches(view,shown)) { error = t('scenes.modifie'); return; }
    await work(async () => { run = await call(`/api/scenes/${encodeURIComponent(id)}/run`,'POST',shown); preview = null; timer = setTimeout(poll,200); });
  }
  async function cancel() { await work(async () => { await call(`/api/scene-runs/${encodeURIComponent(run.id)}/cancel`,'POST',{}); await poll(); }); }
  $effect(() => { if (dirty) preview = null; });
</script>
<article class="scene">
  <button class="heading" aria-expanded={open} onclick={toggle}><strong>{id}</strong><span>{t('scenes.ouvrir')}</span></button>
  {#if open}
    <div class="editor" aria-busy={busy}>
      {#if view}
        <p>{t('scenes.versions',{draft:view.versions.draft.version,current:view.versions.current?.version ?? '—'})}</p>
        <p class="muted">{t('scenes.brouillon')}</p>
        {#if view.versions.current}
          <details><summary>{t('scenes.courant')}</summary><ul>{#each view.scope.targets as target}<li>{target.name} · {target.label} · {targetValue(target)} <small>{target.point} · {target.native_name}</small></li>{/each}</ul></details>
        {/if}
        <fieldset disabled={busy}>
          {#each rows as row,i}
            <div class="target"><span class="index">{i+1}</span><div><PointPicker point={row.point} mode="scene" onpick={(p)=>pick(i,p)} />{#if row.point}<Fields kind="value" point={row.point} value={row.value} onchange={(v)=>rows[i].value=v} />{/if}</div><button aria-label={t('scenes.retirer')} onclick={()=>rows=rows.filter((_,j)=>j!==i)}>×</button></div>
          {/each}
          <button disabled={rows.length>=32} onclick={()=>rows.push({point:'',value:true})}>{t('scenes.ajouter')}</button>
          <div class="limits">
            <label>{t('scenes.concurrence')}<input type="number" min="1" max="4" step="1" bind:value={parallel} /></label>
            <label>{t('scenes.delai')}<input type="number" min="1" max="120" step="1" bind:value={timeout} /></label>
            <label>{t('scenes.confirmation')}<input type="number" min="1" max="30" step="1" bind:value={confirmation} /></label>
          </div>
          <div class="actions"><button disabled={!dirty && !needsVersion} onclick={save}>{t('scenes.enregistrer')}</button><button disabled={dirty} onclick={()=>inspect('publish')}>{t('scenes.examiner')}</button><button disabled={dirty || !view.versions.current || run?.status==='running'} onclick={()=>inspect('run')}>{t('scenes.preparer')}</button></div>
        </fieldset>
        {#if preview && !dirty}
          <section class="confirmation">
            <h3>{t(mode==='publish'?'scenes.portee_publication':'scenes.portee_execution')}</h3>
            <p>{t(mode==='publish'?'scenes.effet_publication':'scenes.effet_execution')}</p>{#if needsVersion}<p role="status">{t('scenes.portee_modifiee')}</p>{/if}
            <ul>{#each scope?.targets ?? [] as target}<li><strong>{target.name} · {target.label} · {targetValue(target)}</strong><small>{target.native_name} · {target.point} · {target.devices.join(', ')} · {target.rooms.join(', ')}</small></li>{/each}</ul>
            {#if mode==='publish'}
              <h4>{t('scenes.graphes')}</h4><ul>{#each preview.consumers as graph}<li>{graph.name} <small>{graph.id}</small></li>{:else}<li>{t('scenes.aucun_graphe')}</li>{/each}</ul>
              {#if !hub.session.human}<HumanSession />{/if}
              <button disabled={busy || !hub.session.human || needsVersion} onclick={approve}>{t('scenes.publier',{version:preview.versions.draft})}</button>
            {:else}<button disabled={busy} onclick={execute}>{t('scenes.appliquer',{version:preview.version})}</button>{/if}
          </section>
        {/if}
      {/if}
      {#if run}
        <section aria-live="polite"><h3>{t('scenes.resultat')} · {t('scenes.run_'+run.status)}</h3><ul>{#each run.targets as target}<li>{pointName(target.point)} — {t('scenes.cible_'+target.status)}{#if target.acknowledged}<small>{t('scenes.acquittee')}</small>{/if}{#if target.approval}<small>{t('scenes.approbation',{id:target.approval})}</small>{/if}{#if target.detail}<small>{target.detail}</small>{/if}</li>{/each}</ul>{#if run.status==='running'}<button disabled={busy} onclick={cancel}>{t('scenes.annuler')}</button><button disabled={busy} onclick={poll}>{t('scenes.recharger_resultat')}</button>{/if}</section>
      {/if}
      {#if error}<p role="alert" class="error">{error}</p>{/if}
      <button disabled={busy} onclick={load}>{t('scenes.recharger')}</button>
    </div>
  {/if}
</article>
<style>
  .scene { border:1px solid var(--line); border-radius:var(--radius,16px); background:var(--surface); overflow:hidden; }
  .heading { display:flex; justify-content:space-between; gap:12px; width:100%; border:0; padding:18px; background:transparent; text-align:left; }
  .heading span, .muted, small {color:var(--ink-2);font-size:13px;}
  .editor {display:grid;gap:16px;padding:0 18px 18px;}
  h3,h4,p {margin:0;}
  button,input {font:inherit;color:var(--ink);background:var(--surface-2);border:1px solid var(--line);border-radius:10px;padding:10px;}
  button {cursor:pointer;min-height:44px;} button:disabled {opacity:.5;cursor:default;}
  fieldset {border:0;padding:0;margin:0;display:grid;gap:12px;min-width:0;}
  .target {display:grid;grid-template-columns:24px minmax(0,1fr) 44px;gap:8px;align-items:start;}
  .target>div {display:grid;gap:6px;}.index {padding-top:12px;color:var(--ink-2);}
  .limits {display:flex;flex-wrap:wrap;gap:12px;}label {display:grid;gap:4px;font-size:13px;min-width:0;}input {width:90px;}
  .actions {display:flex;gap:8px;flex-wrap:wrap;}.confirmation {display:grid;gap:12px;border:1px solid var(--ink);padding:16px;border-radius:12px;}
  .confirmation>button {background:var(--sun);font-weight:700;}ul {margin:0;padding-left:20px;}li {margin:6px 0;overflow-wrap:anywhere;}small {display:block;}.error {color:var(--alarm);}
</style>
