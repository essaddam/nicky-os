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

  async function validate() {
    const r = policies?.reduction;
    if (!r) return;
    busy = true;
    try {
      const res = await fetch('/api/history/retention/validate', {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
        body: JSON.stringify({ at: r.at, fingerprint: r.fingerprint }),
      });
      if (res.status === 403) {
        // A person's gesture: the code first, then the same request again.
        home.held = { label: t('systeme.conservation.titre'), reason: 'ton code', custom: validate };
        return;
      }
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body.error ?? res.statusText);
      note(t('systeme.conservation.supprimees', { count: body.removed, n: num(body.removed) }), 'info');
    } catch (e) {
      note(t('systeme.conservation.erreur', { message: e.message }), 'error');
    } finally {
      busy = false;
      await load();
    }
  }
</script>

{#if lines.length}
  <div class="retention">
    <div class="head"><b>{t('systeme.conservation.titre')}</b><span class="muted">{t('systeme.conservation.aide')}</span></div>
    {#each lines as l, i (i)}
      <p class:alert={l.alert}>{say(l)}</p>
    {/each}
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
        <button class="btn primary" disabled={busy} onclick={validate}>{busy ? '…' : t('systeme.conservation.valider')}</button>
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
