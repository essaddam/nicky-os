<script>
  import PressButton from './PressButton.svelte';
  import { t } from '../lib/i18n.svelte.js';

  let { point, spec, label, disabled = false, onheld } = $props();
  let seconds = $state(300);
  let previous = '';
  $effect(() => {
    if (previous !== point) {
      previous = point;
      seconds = spec.default_seconds ?? 300;
    }
  });
  const starts = $derived(spec.operation === 'start');
  const valid = $derived(!starts || (Number.isSafeInteger(seconds) && seconds >= 1 && seconds <= 604800));
</script>

<div class="timer-control">
  {#if starts}
    <label>{t('systeme.minuteur.duree')}
      <input type="number" min="1" max="604800" step="1" bind:value={seconds} {disabled} />
    </label>
  {/if}
  <PressButton {point} {label} {onheld} action="timer" caption={t(starts ? 'systeme.minuteur.demarrer' : 'systeme.minuteur.annuler')}
    value={starts ? seconds : null} disabled={disabled || !valid} />
  {#if starts}<small>{t('systeme.minuteur.portee')}</small>{/if}
</div>

<style>
  .timer-control { display: grid; gap: 8px; }
  label { display: flex; gap: 8px; align-items: center; font-size: 13px; }
  input { width: 8em; padding: 6px; font: inherit; border: 1px solid var(--ink); background: var(--card); color: var(--ink); border-radius: 6px; }
  input:focus-visible { outline: 2px solid var(--ink); outline-offset: 3px; }
  small { color: var(--ink-2); max-width: 34em; }
</style>
