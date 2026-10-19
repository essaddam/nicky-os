<script>
  import { onMount } from 'svelte';
  import { home, note } from '../lib/home.svelte.js';
  import { authorizationUrl, readJson } from '../lib/netatmo.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  let { instances = [] } = $props();
  let statuses = $state({});
  let busy = $state(null);
  let unavailable = $state(false);

  const words = {
    needs_credentials: 'netatmo.ui.needs_credentials',
    needs_authorization: 'netatmo.ui.needs_authorization',
    authorizing: 'netatmo.ui.authorizing',
    connected: 'netatmo.ui.connected',
    unavailable: 'netatmo.ui.unavailable',
  };
  const statusWord = (id) => unavailable ? t('netatmo.ui.load_failed')
    : t(words[statuses[id]] ?? 'netatmo.ui.private_status');

  function askPin(action) {
    home.held = { label: t('netatmo.ui.title'), reason: t('netatmo.http.human'), custom: action };
  }

  async function load() {
    try {
      const response = await fetch('/api/integrations/netatmo', {
        cache: 'no-store', signal: AbortSignal.timeout(5000),
      });
      if (response.status === 403) {
        statuses = {};
        unavailable = false;
        return;
      }
      if (!response.ok) throw new Error('Status unavailable');
      const body = await readJson(response);
      if (!Array.isArray(body.instances)) throw new Error('Invalid status');
      statuses = Object.fromEntries(body.instances
        .filter((item) => instances.includes(item.instance) && Object.hasOwn(words, item.status))
        .map((item) => [item.instance, item.status]));
      unavailable = false;
    } catch {
      statuses = {};
      unavailable = true;
    }
  }
  onMount(load);

  async function hasHumanSession() {
    const response = await fetch('/api/session', { cache: 'no-store', signal: AbortSignal.timeout(5000) });
    if (!response.ok) throw new Error('Session unavailable');
    return (await readJson(response)).human === true;
  }

  async function refresh() {
    try {
      if (!(await hasHumanSession())) return askPin(load);
      await load();
    } catch {
      note(t('netatmo.ui.failed'), 'error');
    }
  }

  async function connect(id, afterPin = false) {
    if (busy !== null || !instances.includes(id)) return;
    busy = id;
    try {
      const response = await fetch(`/api/integrations/netatmo/${encodeURIComponent(id)}/authorize`, {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
        cache: 'no-store',
        signal: AbortSignal.timeout(25000),
        body: '{}',
      });
      if (response.status === 403) {
        if (!afterPin && !(await hasHumanSession())) {
          askPin(() => connect(id, true));
          return;
        }
        note(t('netatmo.ui.refused'), 'error');
        return;
      }
      if (!response.ok) {
        note(t(response.status === 429 ? 'netatmo.http.rate_limited' : 'netatmo.ui.failed'), 'error');
        return;
      }
      const body = await readJson(response);
      const target = authorizationUrl(body.url, window.location.origin, id);
      if (!target) throw new Error('Invalid consent destination');
      // Ephemeral URL only: never store consent state or code in browser storage.
      window.location.assign(target);
    } catch {
      note(t('netatmo.ui.failed'), 'error');
    } finally {
      busy = null;
    }
  }
</script>

{#if instances.length}
  <section class="authorization" aria-label={t('netatmo.ui.title')}>
    <div class="heading">
      <span class="icon"><Icon name="lock" size={18} /></span>
      <div class="text">
        <b>{t('netatmo.ui.title')}</b>
        <span class="muted">{t('netatmo.ui.explanation')}</span>
      </div>
      <button class="btn" onclick={refresh} disabled={busy !== null}>{t('netatmo.ui.refresh')}</button>
    </div>
    {#each instances as id (id)}
      <div class="instance">
        <div class="text"><b>{id}</b><span class="muted" role="status">{statusWord(id)}</span></div>
        <button class="btn" onclick={() => connect(id)} disabled={busy !== null}>
          {busy === id ? t('netatmo.ui.opening') : statuses[id] === 'connected' ? t('netatmo.ui.reconnect') : t('netatmo.ui.connect')}
        </button>
      </div>
    {/each}
  </section>
{/if}

<style>
  .authorization { display: grid; gap: 14px; padding: 16px 18px; border-radius: var(--r-md); background: var(--surface-2); }
  .heading, .instance { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .instance { padding-top: 12px; border-top: 1px solid var(--line); }
  .icon { display: grid; place-items: center; width: 36px; height: 36px; border-radius: 50%; background: var(--surface); color: var(--warm-ink); }
  .text { display: grid; flex: 1; gap: 3px; min-width: min(220px, 100%); overflow-wrap: anywhere; }
  .muted { font-size: 13px; }
  .btn { border: 0; border-radius: 999px; padding: 9px 16px; font: inherit; font-weight: 650; background: var(--surface); color: var(--ink); cursor: pointer; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn:focus-visible { outline: 2px solid var(--warm-ink); outline-offset: 3px; }
</style>
