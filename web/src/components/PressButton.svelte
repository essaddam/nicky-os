<script>
  import { buttonRequest, sendButton } from '../lib/button.js';
  import { t } from '../lib/i18n.svelte.js';
  import { hub } from '../lib/hub.svelte.js';

  let { point, label, disabled = false, onheld, action = 'button', caption = null } = $props();
  const approvals = $derived(hub.approvals.filter((request) => request.point === point));
  let waiting = $state(0);
  let failures = $state([]);
  let message = $state('');
  // Serial transport preserves the order of rapid appuis without merging them.
  let tail = Promise.resolve();

  function enqueue(request, target = point) {
    waiting += 1;
    tail = tail.then(async () => {
      try {
        request ??= await buttonRequest(target);
        const result = await sendButton(request);
        if (!result.ok && result.status !== 202) {
          message = result.error || t(action === 'button' ? 'systeme.bouton.refus' : 'systeme.compteur.refus');
          // An ambiguous/expired identity must never become a fresh press.
          if (result.status >= 500 || result.retryable) failures.push(request);
        } else {
          message = result.status === 202
            ? t('systeme.hub.retenu', { reason: result.reason })
            : t(action === 'button' ? 'systeme.bouton.envoye' : 'systeme.compteur.envoye');
        }
      } catch {
        if (request) failures.push(request);
        message = t(request ? (action === 'button' ? 'systeme.bouton.incertain' : 'systeme.compteur.incertain') : (action === 'button' ? 'systeme.bouton.identite_indisponible' : 'systeme.compteur.identite_indisponible'));
      } finally {
        waiting -= 1;
      }
    });
  }

  function press() {
    if (waiting + failures.length + approvals.length >= 32) {
      message = t(action === 'button' ? 'systeme.bouton.attendre' : 'systeme.compteur.attendre');
      return;
    }
    enqueue(null);
  }

  function retry() {
    const request = failures.shift();
    if (request) enqueue(request);
  }
</script>

<div class="press">
  <button class="press-action" onclick={press} {disabled} aria-label={label}>
    {caption ?? t('systeme.bouton.appuyer')}{#if waiting}<span aria-hidden="true"> · {waiting}</span>{/if}
  </button>
  {#if failures.length}<button class="retry" onclick={retry}>{t('systeme.bouton.reessayer')}</button>{/if}
  {#if onheld}
    {#each approvals as request (request.id)}
      <button onclick={() => onheld(request)}>{t(action === 'button' ? 'systeme.bouton.valider' : 'systeme.compteur.valider')}</button>
    {/each}
  {/if}
  <span class="message" role="status">{message}</span>
</div>

<style>
  .press { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  button { font: inherit; cursor: pointer; border: var(--stroke, 1px) solid var(--ink); border-radius: 6px; padding: 8px 12px; background: var(--card); color: var(--ink); }
  button:focus-visible { outline: 2px solid var(--ink); outline-offset: 3px; }
  button:disabled { opacity: 0.6; cursor: default; }
  .message { flex-basis: 100%; font-size: 12px; color: var(--ink-2); }
</style>
