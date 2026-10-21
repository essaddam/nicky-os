<script>
  import { hub } from '../lib/hub.svelte.js';
  import { t, locale } from '../lib/i18n.svelte.js';
  import { minuteText, weeklyPayload, previewMatchesShown } from '../maison/lib/weekly.js';
  import HumanSession from './HumanSession.svelte';

  let { id } = $props();
  let open = $state(false);
  let busy = $state(false);
  let error = $state('');
  let view = $state(null);
  let timezone = $state('UTC');
  let rows = $state([]);
  let saved = $state('');
  let preview = $state(null);
  const dirty = $derived(JSON.stringify({timezone, rows}) !== saved);
  const path = $derived(`/api/dependencies/schedule/${encodeURIComponent(id)}`);
  const days = $derived(Array.from({length: 7}, (_,i) => new Intl.DateTimeFormat(locale(), {weekday:'long', timeZone:'UTC'}).format(new Date(Date.UTC(2026, 0, 5 + i)))));

  async function call(method, suffix = '', body) {
    const response = await fetch(path + suffix, {method, headers: {'content-type':'application/json', 'x-moli-origin':'ui'}, body: body === undefined ? undefined : JSON.stringify(body)});
    const data = await response.json();
    if (!response.ok) throw new Error(data.error || t('systeme.planning.refus'));
    return data;
  }
  function show(data) {
    view = data;
    const plan = data.versions.draft.definition.value;
    timezone = plan.timezone;
    rows = plan.slots.map((s) => ({day:s.day, start:minuteText(s.start), end:minuteText(s.end)}));
    saved = JSON.stringify({timezone, rows});
    preview = null;
  }
  async function work(action) {
    busy = true; error = '';
    try { await action(); } catch (e) { error = e.message || t('systeme.planning.refus'); }
    finally { busy = false; }
  }
  async function load() { await work(async () => show(await call('GET'))); }
  async function toggle() { open = !open; if (open) await load(); }
  async function save() {
    const definition = weeklyPayload(timezone, rows);
    if (!definition) { error = t('systeme.planning.invalide'); return; }
    await work(async () => show(await call('PUT', '', {expected:view.versions.draft.version, definition})));
  }
  async function inspect() {
    await work(async () => {
      const candidate = await call('GET', '/preview');
      if (!previewMatchesShown(view, candidate)) {
        preview = null;
        throw new Error(t('systeme.planning.modifie'));
      }
      preview = candidate;
    });
  }
  async function approve() {
    const shown = preview;
    if (dirty || !previewMatchesShown(view, shown)) { error = t('systeme.planning.modifie'); return; }
    await work(async () => show(await call('POST', '/approve', shown)));
  }
  $effect(() => { if (dirty) preview = null; });
</script>

<div class="planning">
  <button class="edit" aria-expanded={open} onclick={toggle}>{t('systeme.planning.editer')}</button>
  {#if open}
    <div class="editor" aria-busy={busy}>
      {#if view}
        <p>{t('systeme.planning.versions', {draft:view.versions.draft.version, current:view.versions.current?.version ?? '—'})}</p>
        <p class="hint">{t('systeme.planning.brouillon')}</p>
        {#if view.versions.current}
          <details><summary>{t('systeme.planning.courant')}</summary>
            <p>{view.versions.current.definition.value.timezone}</p>
            <ul>{#each view.versions.current.definition.value.slots as slot}<li>{days[slot.day-1]} · {minuteText(slot.start)}–{minuteText(slot.end)}</li>{/each}</ul>
            {#if !view.versions.current.definition.value.slots.length}<p>{t('systeme.planning.vide')}</p>{/if}
          </details>
        {/if}
        <fieldset disabled={busy}>
          <label>{t('systeme.planning.fuseau')}<input bind:value={timezone} maxlength="128" placeholder="UTC" /></label>
          <p class="hint">{t('systeme.planning.horaires')}</p>
          {#each rows as row, i}
            <div class="slot">
              <label>{t('systeme.planning.jour')}<select bind:value={row.day}>{#each days as day, index}<option value={index+1}>{day}</option>{/each}</select></label>
              <label>{t('systeme.planning.debut')}<input type="time" bind:value={row.start} /></label>
              <label>{t('systeme.planning.fin')}<input bind:value={row.end} maxlength="5" placeholder="24:00" inputmode="numeric" /></label>
              <button type="button" aria-label={t('systeme.planning.retirer')} onclick={() => rows = rows.filter((_,j) => j !== i)}>×</button>
            </div>
          {/each}
          <div class="actions">
            <button disabled={rows.length >= 256} onclick={() => rows.push({day:1,start:'09:00',end:'17:00'})}>{t('systeme.planning.ajouter')}</button>
            <button disabled={!dirty} onclick={save}>{t('systeme.planning.enregistrer')}</button>
            <button disabled={dirty} onclick={inspect}>{t('systeme.planning.examiner')}</button>
          </div>
        </fieldset>
        {#if preview && !dirty}
          <section class="confirmation" aria-label={t('systeme.planning.portee')}>
            <h3>{t('systeme.planning.portee')}</h3>
            <p>{t('systeme.planning.effet')}</p>
            {#if preview.consumers.length}
              <ul>{#each preview.consumers as graph (graph.id)}<li><strong>{graph.name}</strong> <small>{graph.id}</small></li>{/each}</ul>
            {:else}<p>{t('systeme.planning.aucun_graphe')}</p>{/if}
            {#if !hub.session.human}<p>{t('systeme.planning.pin')}</p><HumanSession />{/if}
            <button disabled={busy || !hub.session.human} onclick={approve}>{t('systeme.planning.valider', {version:preview.versions.draft})}</button>
          </section>
        {/if}
      {/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <button disabled={busy} onclick={load}>{t('systeme.planning.recharger')}</button>
    </div>
  {/if}
</div>

<style>
  .planning { width:100%; }
  button, input, select { font:inherit; color:var(--ink); background:var(--card); border:var(--stroke, 1px) solid var(--line); border-radius:var(--radius, 6px); padding:8px 10px; max-width:100%; }
  button { cursor:pointer; min-height:40px; }
  button:disabled { opacity:.5; cursor:default; }
  .edit { width:100%; text-align:left; }
  .editor { display:grid; gap:12px; padding-top:12px; }
  p, h3 { margin:0; }
  .hint, small { color:var(--ink-2); font-size:13px; }
  fieldset { border:0; padding:0; margin:0; display:grid; gap:12px; min-width:0; }
  label { display:grid; gap:4px; font-size:13px; min-width:0; }
  .slot { display:grid; grid-template-columns:minmax(100px, 1fr) 95px 95px 40px; gap:8px; align-items:end; }
  .actions { display:flex; flex-wrap:wrap; gap:8px; }
  .confirmation { display:grid; gap:12px; padding:12px; background:var(--bg); border:var(--stroke, 1px) solid var(--ink); border-radius:var(--radius, 6px); }
  .confirmation button { background:var(--sun); font-weight:700; }
  .error { color:var(--alarm); }
  ul { margin:0; padding-left:20px; }
  @media (max-width:520px) { .slot { grid-template-columns:minmax(80px,1fr) minmax(60px,1fr); } .slot button { justify-self:start; } }
</style>
