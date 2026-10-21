<script>
  import { value, reachable, nameOf, act } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';
  let { id } = $props();
  const online = $derived(reachable(id));
  const state = $derived(value(id, 'media_state'));
  const title = $derived(value(id, 'media_title'));
  const app = $derived(value(id, 'media_app'));
  const volume = $derived(value(id, 'volume'));
  const mute = $derived(value(id, 'mute'));
  const playing = $derived(state === 'playing' || state === 'buffering');
  const session = $derived(online && ['playing', 'paused', 'buffering', 'idle'].includes(state));
  let dragging = $state(null);
  let busy = $state(false);
  async function command(key, target, label) {
    if (busy) return;
    busy = true;
    try { await act(`${id}/${key}`, target, label); }
    finally { busy = false; }
  }
</script>

<section class="card cast-player">
  <div class="card-head"><h2><Icon name="tv" size={18} />{nameOf(id)}</h2><span class="chip">{online ? (app ?? t('salon.cast.ready')) : t('salon.cast.offline')}</span></div>
  <p class="track">{title ?? (online ? t('salon.cast.empty') : t('salon.cast.offline_help'))}</p>
  <div class="playback" role="group" aria-label={t('salon.cast.playback')}>
    <button disabled={!session || busy} onclick={() => command('media_control', playing ? 'pause' : 'play', playing ? t('salon.pause') : t('salon.lecture'))}><Icon name={playing ? 'pause' : 'play'} size={20} />{playing ? t('salon.pause') : t('salon.lecture')}</button>
    <button disabled={!session || busy} onclick={() => command('media_control', 'stop', t('salon.cast.stop'))}>{t('salon.cast.stop')}</button>
  </div>
  <div class="sound">
    <label><span>{t('salon.volume')}</span><input type="range" min="0" max="100" step="1" value={dragging ?? volume ?? 0} disabled={!online || typeof volume !== 'number' || busy} oninput={(event) => dragging = Number(event.currentTarget.value)} onchange={(event) => { dragging = null; command('volume', Number(event.currentTarget.value), t('salon.volume')); }} /></label>
    <output>{typeof volume === 'number' ? `${dragging ?? volume} %` : '—'}</output>
    <button disabled={!online || typeof mute !== 'boolean' || busy} aria-pressed={mute === true} onclick={() => command('mute', !mute, t('salon.sourdine'))}><Icon name={mute ? 'volume-off' : 'volume-high'} size={20} />{t('salon.sourdine')}</button>
  </div>
</section>

<style>
  .cast-player { padding: 22px; }
  .card-head { flex-wrap: wrap; gap: 10px; }
  .track { margin: 18px 0; overflow-wrap: anywhere; }
  .playback, .sound { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .sound { margin-top: 18px; }
  label { display: grid; gap: 6px; flex: 1; min-width: 120px; font-size: 13px; }
  input { width: 100%; accent-color: var(--ink); }
  output { min-width: 45px; font-variant-numeric: tabular-nums; }
  button { display: inline-flex; align-items: center; justify-content: center; gap: 8px; min-height: 44px; font: inherit; background: var(--surface-2); color: var(--ink); border: 0; border-radius: 999px; padding: 10px 14px; cursor: pointer; }
  button[aria-pressed='true'] { color: var(--warm-ink); }
  button:disabled, input:disabled { opacity: 0.5; cursor: default; }
</style>
