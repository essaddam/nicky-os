<script>
  import { onMount } from 'svelte';
  import { home, note } from '../lib/home.svelte.js';
  import { t, locale } from '../../lib/i18n.svelte.js';
  import { retentionLines } from '../lib/storage.js';

  /** What the history keeps, and a shorter duration waiting for a person:
   *  the volume and the period it would delete are shown first, then
   *  validated with the code (an agent never validates). */
  let policies = $state(null);
  let busy = $state(false);
  // The proposal clicked is no longer the one waiting: a new look, a new click.
  let changed = $state(false);

  async function load() {
    try {
      const res = await fetch('/api/history/retention');
      if (res.ok) policies = await res.json();
    } catch {
      /* the card stays hidden */
    }
  }
  onMount(load);

  const num = (n) => Number(n).toLocaleString(locale());
  const when = (ms) => (ms == null ? '—' : new Date(ms).toLocaleDateString(locale(), { dateStyle: 'medium' }));
  const lines = $derived(retentionLines(policies, { num, when }));
  // A parameter may itself be a key (a family's name).
  const say = (l) => t(l.key, Object.fromEntries(Object.entries(l.params ?? {}).map(([k, v]) => [k, v?.key ? t(v.key) : v])));

  /** Validates `shown`, the proposal on screen when it was clicked: after
   *  the code, that same one, never a newer count. */
  async function validate(shown) {
    if (!shown) return;
    busy = true;
    changed = false;
    let reload = true;
    try {
      const res = await fetch('/api/history/retention/validate', {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
        body: JSON.stringify({ at: shown.at, fingerprint: shown.fingerprint }),
      });
      if (res.status === 403) {
        // A person's gesture: the code first, then this proposal again.
        reload = false;
        home.held = { label: t('systeme.conservation.titre'), reason: 'ton code', custom: () => validate(shown) };
        return;
      }
      const body = await res.json().catch(() => ({}));
      // Changed or too old: the new count is shown, and asks for a new click.
      changed = res.status === 409 || res.status === 410;
      if (changed) return;
      if (!res.ok) throw new Error(body.error ?? res.statusText);
      note(t('systeme.conservation.supprimees', { count: body.removed, n: num(body.removed) }), 'info');
    } catch (e) {
      note(t('systeme.conservation.erreur', { message: e.message }), 'error');
    } finally {
      busy = false;
      if (reload) await load();
    }
  }
</script>

{#if lines.length}
  <div class="retention">
    <div class="head"><b>{t('systeme.conservation.titre')}</b><span class="muted">{t('systeme.conservation.aide')}</span></div>
    {#each lines as l, i (i)}
      <p class:alert={l.alert}>{say(l)}</p>
    {/each}
    {#if changed}<p class="alert">{t('systeme.conservation.changee')}</p>{/if}
    {#if policies.reduction}
      {#if policies.reduction.top.length}
        <ol>
          {#each policies.reduction.top as p (p.point)}
            <li title={p.point}>
              <span>{p.point}</span>
              <span class="muted">{t('systeme.conservation.jours', { from: num(p.from_days), to: num(p.to_days) })} · {num(p.rows)}</span>
            </li>
          {/each}
        </ol>
      {/if}
      <div class="act">
        <button class="btn primary" disabled={busy} onclick={() => validate(policies.reduction)}>{busy ? '…' : t('systeme.conservation.valider')}</button>
        <small class="muted">{t('systeme.conservation.code')}</small>
      </div>
    {/if}
  </div>
{/if}

<style>
  .retention {
    display: grid;
    gap: 8px;
  }

  .head {
    display: flex;
    gap: 12px;
    align-items: baseline;
    flex-wrap: wrap;
  }

  p {
    margin: 0;
  }

  .alert {
    color: var(--warm-ink);
    font-weight: 600;
  }

  ol {
    margin: 0;
    padding-left: 20px;
  }

  li span:first-child {
    overflow-wrap: anywhere;
    margin-right: 8px;
  }

  .act {
    display: flex;
    gap: 12px;
    align-items: center;
    flex-wrap: wrap;
  }

  .btn {
    border: 0;
    border-radius: 999px;
    padding: 9px 16px;
    font: inherit;
    font-weight: 650;
    background: var(--warm);
    color: #fff;
    cursor: pointer;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
