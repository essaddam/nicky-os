<script>
  import { onMount } from 'svelte';
  import { home } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';

  let { instance } = $props();
  let state = $state(null);
  let code = $state('');
  let busy = $state(false);
  let cancelling = $state(false);
  let error = $state('');
  let alive = true;
  const path = $derived(`/api/integrations/androidtv/${encodeURIComponent(instance)}/pairing`);
  const active = $derived(['preparing', 'awaiting_code', 'verifying', 'persisting'].includes(state?.phase));
  const remaining = $derived(state?.expires_at ? Math.max(0, Math.ceil((state.expires_at - Date.now()) / 1000)) : null);

  async function load() {
    try {
      const response = await fetch(path);
      if (response.ok && alive) state = await response.json();
    } catch { if (alive) error = t('systeme.androidtv.network'); }
  }
  onMount(() => {
    alive = true;
    load();
    const timer = setInterval(load, 1000);
    return () => { alive = false; code = ''; clearInterval(timer); };
  });

  async function write(method, payload) {
    if (method === 'DELETE') cancelling = true;
    else busy = true;
    error = '';
    try {
      const response = await fetch(path, {
        method,
        headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
        body: JSON.stringify(payload),
      });
      if (response.status === 403) {
        // The human PIN flow never holds the TV code in global UI state.
        home.held = {
          label: t('systeme.androidtv.title'), reason: t('systeme.androidtv.human'),
          custom: method === 'PUT' ? load : () => write(method, payload),
        };
        if (method === 'PUT') error = t('systeme.androidtv.reenter');
        return;
      }
      const body = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(body.error ?? t('systeme.androidtv.network'));
      await load();
    } catch (failure) { if (alive) error = failure.message; }
    finally { code = ''; if (method === 'DELETE') cancelling = false; else busy = false; }
  }
</script>

<section class="pairing" aria-label={t('systeme.androidtv.title')}>
  <div class="heading">
    <div><h4>{t('systeme.androidtv.title')}</h4><p class="muted">{t('systeme.androidtv.help')}</p></div>
    {#if state?.phase === 'paired'}<span class="paired">{t('systeme.androidtv.paired')}</span>{/if}
  </div>
  {#if state}
    <p role="status">{t(`systeme.androidtv.phase.${state.phase}`)}{#if active && remaining != null} · {t('systeme.androidtv.remaining', { seconds: remaining })}{/if}</p>
    {#if state.message}<p class="failure" role="alert">{state.message}</p>{/if}
    {#if state.phase === 'awaiting_code'}
      <form onsubmit={(event) => { event.preventDefault(); write('PUT', { generation: state.generation, code: code.trim() }); }} autocomplete="off">
        <label>{t('systeme.androidtv.code')}<input bind:value={code} maxlength="6" pattern={'[0-9a-fA-F]{6}'} required spellcheck="false" autocomplete="off" autocapitalize="characters" aria-describedby="tv-pairing-code-help" /></label>
        <button disabled={busy || cancelling || !/^[0-9a-fA-F]{6}$/.test(code.trim())}>{t('systeme.androidtv.verify')}</button>
        <small id="tv-pairing-code-help" class="muted">{t('systeme.androidtv.code_help')}</small>
      </form>
    {/if}
    <div class="actions">
      {#if !active && state.phase !== 'paired' && state.phase !== 'unavailable'}
        <button disabled={busy || cancelling} onclick={() => write('POST', {})}>{t('systeme.androidtv.start')}</button>
      {/if}
      {#if active && state.phase !== 'persisting'}
        <button class="secondary" disabled={cancelling} onclick={() => write('DELETE', { generation: state.generation })}>{t('systeme.androidtv.cancel')}</button>
      {/if}
    </div>
  {:else}<p class="muted">{t('systeme.androidtv.loading')}</p>{/if}
  {#if error}<p class="failure" role="alert">{error}</p>{/if}
</section>

<style>
  .pairing { padding: 18px; background: var(--surface-2); border-radius: var(--r-md); display: grid; gap: 10px; }
  .heading { display: flex; align-items: start; gap: 16px; justify-content: space-between; }
  h4, p { margin: 0; }
  h4 { font-size: 16px; }
  .heading p { margin-top: 5px; font-size: 13px; max-width: 52ch; }
  .paired { color: var(--ink-2); font-weight: 650; }
  form { display: flex; flex-wrap: wrap; align-items: end; gap: 10px; }
  label { display: grid; gap: 5px; font-size: 13px; }
  input { width: 10ch; font: inherit; font-size: 20px; letter-spacing: 0.12em; text-transform: uppercase; color: var(--ink); background: var(--surface); border: 1px solid var(--line); border-radius: var(--r-sm); padding: 9px 12px; }
  small { flex-basis: 100%; }
  .actions { display: flex; gap: 10px; }
  button { font: inherit; font-weight: 650; border: 0; border-radius: 999px; padding: 10px 16px; background: var(--warm); color: #171c28; cursor: pointer; }
  button.secondary { color: var(--ink); background: var(--surface); }
  button:disabled { opacity: 0.5; cursor: default; }
  .failure { color: var(--warm-ink); }
</style>
