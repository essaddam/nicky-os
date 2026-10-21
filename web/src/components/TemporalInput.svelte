<script>
  import { t } from '../lib/i18n.svelte.js';
  import { temporalEditor, temporalPayload, supportsTimezone } from '../lib/temporal.js';
  let { spec, value = '', onchange, disabled = false, label = '' } = $props();
  let utc = $state(false);
  let local = $state('');
  let offset = $state('');
  const supported = $derived(spec.format !== 'datetime' || supportsTimezone(spec.timezone));
  const effectiveUTC = $derived(utc || !supported);
  $effect(() => { const parsed = temporalEditor(spec, value, effectiveUTC); local = !supported && String(value).length === 19 ? '' : parsed.local; offset = parsed.offset; });
  const inputType = $derived(spec.format === 'datetime' ? 'datetime-local' : spec.format);
  function changed() { onchange?.(temporalPayload(spec.format, local, offset, effectiveUTC)); }
  function mode(next) {
    const parsed = temporalEditor(spec, value, next);
    utc = next; local = parsed.local; offset = parsed.offset; changed();
  }
</script>
<div class="temporal-input">
  <input type={inputType} value={local} step="1" required {disabled}
    aria-label={label || t('systeme.date.valeur')} min={spec.format === 'time' ? undefined : spec.format === 'date' ? '0001-01-01' : '0001-01-01T00:00:00'}
    max={spec.format === 'time' ? undefined : spec.format === 'date' ? '9999-12-31' : '9999-12-31T23:59:59'}
    onchange={(e) => { local = e.currentTarget.value; offset = ''; changed(); }} />
  {#if spec.format === 'datetime'}
    <small>{t('systeme.date.fuseau', { zone: spec.timezone })}</small>
    <label><input type="checkbox" checked={effectiveUTC} disabled={disabled || !supported} onchange={(e) => mode(e.currentTarget.checked)} />{t('systeme.date.mode_utc')}</label>
    {#if !supported}<small role="status">{t('systeme.date.fuseau_navigateur')}</small>{/if}
    {#if !effectiveUTC}
      <label>{t('systeme.date.decalage')}<input value={offset} maxlength="6" pattern="(\+|-)[0-9]{2}:[0-9]{2}" placeholder="+02:00" {disabled}
        onchange={(e) => { offset = e.currentTarget.value; changed(); }} /></label>
      <small>{t('systeme.date.choix')}</small>
    {/if}
  {/if}
</div>
<style>
  .temporal-input { display: grid; gap: 6px; }
  input { font: inherit; color: inherit; background: var(--card); border: 1px solid var(--ink); padding: 6px; border-radius: 6px; min-width: 0; }
  input:focus-visible { outline: 2px solid var(--ink); outline-offset: 2px; }
  label { display: flex; gap: 6px; align-items: center; font-size: 12px; }
  label input:not([type=checkbox]) { width: 7em; }
  small { color: var(--ink-2); max-width: 35em; }
</style>
