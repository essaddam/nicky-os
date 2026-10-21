<script>
  import { hub } from '../lib/hub.svelte.js';
  import { t } from '../lib/i18n.svelte.js';
  import { dependencyRequest, previewMatchesShown } from '../lib/dependency.js';
  import { computedEditor, computedPayload } from '../maison/lib/computed.js';
  import {durationEditor,durationPayload} from '../maison/lib/durations.js';
  import {pointSpec,valueText} from '../maison/lib/auto.svelte.js';
  import Fields from '../maison/auto/Fields.svelte';
  import DurationStatus from './DurationStatus.svelte';
  import PointPicker from '../maison/auto/PointPicker.svelte';
  import HumanSession from './HumanSession.svelte';

  let { id } = $props();
  let open = $state(false), busy = $state(false), error = $state('');
  let view = $state(null), form = $state(null), saved = $state(''), preview = $state(null);
  const dirty = $derived(form != null && JSON.stringify(form) !== saved);
  const sample = $derived(hub.devices[id]?.state?.value);
  const call = (method, suffix = '', body) => dependencyRequest('computed', id, method, suffix, body, t);
  function show(data) {
    view = data; form = data.versions.draft.definition.type==='duration' ? durationEditor(data.versions.draft.definition.value) : computedEditor(data.versions.draft.definition.value);
    saved = JSON.stringify(form); preview = null;
  }
  async function work(action) {
    busy = true; error = '';
    try { await action(); } catch (e) { error = e.message || t('systeme.dependance.refus'); }
    finally { busy = false; }
  }
  const load = () => work(async () => show(await call('GET')));
  async function toggle() { open = !open; if (open) await load(); }
  async function save() {
    const definition = form.duration ? durationPayload(form) : computedPayload(form);
    if (!definition) { error = t('systeme.calcul.invalide'); return; }
    await work(async () => show(await call('PUT', '', {expected:view.versions.draft.version,definition})));
  }
  async function inspect() {
    await work(async () => {
      const candidate = await call('GET', '/preview');
      if (!previewMatchesShown(view,candidate)) throw new Error(t('systeme.calcul.modifie'));
      preview = candidate;
    });
  }
  async function approve() {
    if (dirty || !previewMatchesShown(view,preview)) {error=t('systeme.calcul.modifie');return;}
    const shown = preview;
    await work(async () => show(await call('POST','/approve',shown)));
  }
  $effect(() => {if (dirty) preview = null;});
</script>

<div class="calculated">
  <button class="edit" aria-expanded={open} onclick={toggle}>{t('systeme.calcul.editer')}</button>
  {#if open}
    <div class="editor" aria-busy={busy}>
      {#if view && form}
        <p>{t('systeme.planning.versions',{draft:view.versions.draft.version,current:view.versions.current?.version ?? '—'})}</p>
        <p class="hint">{t('systeme.calcul.brouillon')}</p>
        {#if !view.versions.current}<p>{t('systeme.calcul.validation_necessaire')}</p>{:else if sample?.value == null}<p>{t('systeme.calcul.indisponible')}</p>{/if}
        {#if view.versions.current}
          {@const current=view.versions.current.definition.value}
          <details><summary>{t('systeme.calcul.courant')}</summary>
            {#if view.versions.current.definition.type==='duration'}
              <p>{current.source} · {valueText(current.source,current.value)} · {t('systeme.duree.fenetre',{seconds:current.window_s})}</p>
              <DurationStatus {id} />
            {:else}
            <p>{t(`systeme.calcul.operation.${current.operation}`)} · {current.unit || t('systeme.calcul.sans_unite')} · {t('systeme.calcul.age',{seconds:current.max_age_s})}</p>
            <ul>{#each current.sources as source (source)}<li class="identity">{source}</li>{/each}</ul>
            {/if}
          </details>
        {/if}
        <fieldset disabled={busy}>
          {#if form.duration}
            <p class="hint">{t('systeme.duree.aide')}</p>
            <PointPicker point={form.source} mode="duration" onpick={(point)=>{const spec=pointSpec(point);form.source=point;form.source_unit=spec?.unit ?? null;form.value=['binary','schedule'].includes(spec?.kind.type)?true:['numeric','computed','counter'].includes(spec?.kind.type)?0:spec?.kind.values?.[0] ?? '';}} />
            <Fields kind="value" point={form.source} value={form.value} onchange={(v)=>form.value=v} />
            <label>{t('systeme.duree.fenetre_label')}<input type="number" min="1" max="604800" step="1" bind:value={form.window_s} /></label>
            <label>{t('systeme.calcul.fraicheur')}<input type="number" min="1" max="604800" step="1" bind:value={form.max_age_s} /></label>
          {:else}
          <label>{t('systeme.calcul.operation')}<select bind:value={form.operation}>{#each ['min','max','mean'] as operation}<option value={operation}>{t(`systeme.calcul.operation.${operation}`)}</option>{/each}</select></label>
          <div class="pair">
            <label>{t('systeme.calcul.unite')}<input bind:value={form.unit} maxlength="32" list="units-{id}" placeholder={t('systeme.calcul.sans_unite')} /></label>
            <datalist id="units-{id}">{#each ['W','Wh','kWh','°C','%','V','mV','A','s','min','h'] as unit}<option value={unit}></option>{/each}</datalist>
            <label>{t('systeme.calcul.fraicheur')}<input type="number" min="1" max="604800" step="1" bind:value={form.max_age_s} /></label>
          </div>
          <p class="hint">{t('systeme.calcul.sources_aide')}</p>
          {#each form.sources as source, index}
            <div class="source"><PointPicker point={source} mode="number" onpick={(point) => {form.sources[index]=point;}} /><button type="button" aria-label={t('systeme.calcul.retirer')} onclick={() => form.sources=form.sources.filter((_,i)=>i!==index)}>×</button></div>
          {/each}
          {/if}
          <div class="actions">
            {#if !form.duration}<button disabled={form.sources.length>=32} onclick={() => form.sources.push('')}>{t('systeme.calcul.ajouter')}</button>{/if}
            <button disabled={!dirty} onclick={save}>{t('systeme.planning.enregistrer')}</button>
            <button disabled={dirty} onclick={inspect}>{t('systeme.planning.examiner')}</button>
          </div>
        </fieldset>
        {#if preview && !dirty}
          <section class="confirmation" aria-label={t('systeme.calcul.portee')}>
            <h3>{t('systeme.calcul.portee')}</h3><p>{t('systeme.calcul.effet')}</p>
            {#if preview.related?.length}
              <ul>{#each preview.related as related}<li><span class="identity">{related.reference.id}</span> · {t(`systeme.calcul.role.${related.role}`)} · {t('systeme.calcul.version',{version:related.current ?? '—'})}</li>{/each}</ul>
            {/if}
            {#if preview.consumers.length}<ul>{#each preview.consumers as graph (graph.id)}<li><strong>{graph.name}</strong> <small>{graph.id}</small></li>{/each}</ul>{:else}<p>{t('systeme.planning.aucun_graphe')}</p>{/if}
            {#if !hub.session.human}<p>{t('systeme.planning.pin')}</p><HumanSession />{/if}
            <button disabled={busy || !hub.session.human} onclick={approve}>{t('systeme.planning.valider',{version:preview.versions.draft})}</button>
          </section>
        {/if}
      {/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <button disabled={busy} onclick={load}>{t('systeme.planning.recharger')}</button>
    </div>
  {/if}
</div>

<style>
  .calculated {width:100%;}
  button, input, select {font:inherit;color:var(--ink);background:var(--card);border:var(--stroke,1px) solid var(--line);border-radius:var(--radius,6px);padding:8px 10px;max-width:100%;}
  button {cursor:pointer;min-height:40px;} button:disabled {opacity:.5;cursor:default;}
  .edit {width:100%;text-align:left;}
  .editor, fieldset {display:grid;gap:12px;min-width:0;} .editor {padding-top:12px;}
  fieldset {border:0;padding:0;margin:0;} p,h3 {margin:0;} label {display:grid;gap:4px;font-size:13px;min-width:0;}
  .hint,small {color:var(--ink-2);font-size:13px;}
  .pair {display:grid;grid-template-columns:1fr 1fr;gap:8px;}
  .source {display:grid;grid-template-columns:minmax(0,1fr) 40px;gap:8px;align-items:start;}
  .actions {display:flex;flex-wrap:wrap;gap:8px;}
  .confirmation {display:grid;gap:12px;padding:12px;background:var(--bg);border:var(--stroke,1px) solid var(--ink);border-radius:var(--radius,6px);}
  .confirmation button {background:var(--sun);font-weight:700;}
  .error {color:var(--alarm);} ul {margin:0;padding-left:20px;}.identity {overflow-wrap:anywhere;}
  @media (max-width:520px) {.pair {grid-template-columns:1fr;}}
</style>
