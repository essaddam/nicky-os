<script>
  import TemporalInput from './TemporalInput.svelte';
  import { command, hub } from '../lib/hub.svelte.js';
  import { t } from '../lib/i18n.svelte.js';
  let { point, spec, value = '', disabled = false, label = '', onheld } = $props();
  let draft = $state('');
  let waiting = $state(false);
  let error = $state('');
  $effect(() => { draft = value ?? ''; });
  const approvals = $derived(hub.approvals.filter((r) => r.point === point));
  async function submit(event) {
    event.preventDefault(); waiting = true; error = '';
    const result = await command(point, draft);
    waiting = false;
    if (result?.error) error = result.error;
  }
</script>
<form onsubmit={submit}>
  <TemporalInput {spec} value={draft} {label} disabled={disabled || waiting} onchange={(v) => (draft = v)} />
  <button type="submit" disabled={disabled || waiting || !draft}>{t('systeme.date.appliquer')}</button>
  {#if error}<span role="alert">{error}</span>{/if}
  {#if spec.format === 'datetime' && value}<small>{t('systeme.date.utc', { value })}</small>{/if}
  {#if onheld}{#each approvals as request (request.id)}<button type="button" onclick={() => onheld(request)}>{t('systeme.date.valider')}</button>{/each}{/if}
</form>
<style>
  form { display: grid; gap: 8px; }
  button { justify-self: start; font: inherit; color: inherit; background: var(--card); border: 1px solid var(--ink); padding: 7px 12px; border-radius: 6px; cursor: pointer; }
  button:focus-visible { outline: 2px solid var(--ink); outline-offset: 2px; }
  small { color: var(--ink-2); }
</style>
