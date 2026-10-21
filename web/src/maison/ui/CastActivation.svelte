<script>
  import { onMount } from 'svelte';
  import { home } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';

  let { instance } = $props();
  let view = $state(null);
  let exclusive = $state(false);
  let busy = $state(false);
  let error = $state('');
  let alive = false;
  let loading = false;
  const path = $derived(`/api/integrations/cast/${encodeURIComponent(instance)}`);
  const pending = $derived(busy || ['discovering', 'persisting'].includes(view?.phase));
  let revision = 0;

  async function load() {
    if (loading || busy) return;
    loading = true;
    const started = revision;
    try {
      const response = await fetch(path);
      const body = await response.json();
      if (!response.ok) throw new Error(body.error ?? t('systeme.cast.network'));
      if (alive && started === revision) view = body;
    } catch (failure) { if (alive && started === revision) error = failure.message; }
    finally { loading = false; }
  }
  onMount(() => {
    alive = true;
    load();
    const timer = setInterval(load, 3000);
    return () => { alive = false; revision += 1; clearInterval(timer); };
  });

  async function write(method, suffix, payload) {
    if (!alive || busy) return;
    busy = true;
    revision += 1;
    error = '';
    try {
      const response = await fetch(`${path}/${suffix}`, {
        method,
        headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
        body: JSON.stringify(payload),
      });
      const body = await response.json().catch(() => ({}));
      if (response.status === 403) {
        home.held = {
          label: t('systeme.cast.title'), reason: t('systeme.cast.human'),
          // Keep the exact chosen generation across the human PIN flow.
          custom: () => write(method, suffix, payload),
        };
        return;
      }
      if (!response.ok) throw new Error(body.error ?? t('systeme.cast.network'));
      if (alive) { view = body; exclusive = false; }
    } catch (failure) { if (alive) error = failure.message; }
    finally { busy = false; }
  }
</script>

<section class="cast-activation" aria-label={t('systeme.cast.title')}>
  <header><div><h3>{t('systeme.cast.title')}</h3><p class="muted">{t('systeme.cast.help')}</p></div><code>{instance}</code></header>
  {#if view}
    <p role="status">{t(`systeme.cast.phase.${view.phase}`)}</p>
    {#if view.message}<p class="failure" role="alert">{view.message}</p>{/if}
    {#if view.candidates.length}
      {#if view.phase !== 'active'}
        <label class="consent"><input type="checkbox" bind:checked={exclusive} disabled={pending} /><span>{t('systeme.cast.exclusive')}</span></label>
      {/if}
      <ul>
        {#each view.candidates as candidate (candidate.id)}
          <li>
            <div class="identity"><strong>{candidate.name}</strong><small class="muted">{candidate.host} · {candidate.id}</small></div>
            {#if view.phase !== 'active'}
              <button disabled={pending || !exclusive || !candidate.video} onclick={() => write('PUT', 'activation', { generation: view.generation, candidate: candidate.id, exclusive: true })}>{t('systeme.cast.activate')}</button>
            {/if}
            {#if !candidate.video}<small class="muted">{t('systeme.cast.audio')}</small>{/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if view.phase === 'active'}
      <div class="actions"><a href="#/salon">{t('systeme.cast.player')}</a><button class="secondary" disabled={pending} onclick={() => write('DELETE', 'activation', { generation: view.generation })}>{t('systeme.cast.revoke')}</button></div>
    {:else if view.phase !== 'unavailable'}
      <button disabled={pending} onclick={() => write('POST', 'discovery', {})}>{t('systeme.cast.discover')}</button>
    {/if}
  {:else}<p class="muted">{t('systeme.cast.loading')}</p>{/if}
  {#if error}<p class="failure" role="alert">{error}</p>{/if}
</section>

<style>
  .cast-activation { display: grid; gap: 14px; background: var(--surface-2); border-radius: var(--r-md); padding: 20px; }
  header, .actions { display: flex; gap: 16px; align-items: center; justify-content: space-between; flex-wrap: wrap; }
  h3, p { margin: 0; }
  h3 { font-size: 17px; }
  header p { margin-top: 6px; max-width: 65ch; font-size: 14px; }
  code { font-size: 12px; overflow-wrap: anywhere; }
  .consent { display: flex; align-items: start; gap: 10px; font-size: 14px; max-width: 65ch; }
  .consent input { margin-top: 3px; accent-color: var(--ink); }
  ul { list-style: none; padding: 0; margin: 0; display: grid; }
  li { padding: 12px 0; border-top: 1px solid var(--line); display: flex; gap: 12px; align-items: center; flex-wrap: wrap; }
  .identity { flex: 1; min-width: 0; display: grid; gap: 5px; }
  small { overflow-wrap: anywhere; }
  button { font: inherit; font-weight: 650; border: 0; border-radius: 999px; padding: 10px 16px; color: var(--bg); background: var(--ink); cursor: pointer; }
  .secondary { color: var(--ink); background: var(--surface); }
  button:disabled { opacity: 0.5; cursor: default; }
  .failure { color: var(--warm-ink); }
  a { color: var(--ink); text-underline-offset: 4px; }
</style>
