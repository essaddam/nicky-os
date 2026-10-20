<script>
  import { hub, act, reachable } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import { SERVICES, kindsFor, contentRequest, readFavorites, saveFavorite, removeFavorite } from '../lib/media.js';

  let { tv } = $props();
  let service = $state('netflix');
  let kind = $state('app');
  let target = $state('');
  let label = $state('');
  let favorites = $state([]);
  let busy = $state(false);
  let message = $state('');
  let error = $state('');
  const brands = { netflix: 'Netflix', crunchyroll: 'Crunchyroll', freetv: 'Free TV', primevideo: 'Prime Video', disneyplus: 'Disney+', molotov: 'Molotov', appletv: 'Apple TV' };
  const available = $derived(reachable(tv) && hub.devices[tv]?.points?.some(point => point.key === 'play_media' && point.access?.write));
  $effect(() => {
    const selected = tv;
    try { favorites = readFavorites(localStorage, selected); } catch { favorites = []; }
    message = ''; error = '';
  });
  const current = () => ({ service, kind, target: kind === 'app' ? '' : target });

  async function launch(input = current()) {
    if (!available || busy) return;
    busy = true; error = ''; message = '';
    try {
      const request = contentRequest(input);
      const result = await act(`${tv}/play_media`, request, t('salon.media.launch_label', { service: brands[input.service] }));
      if (result === 'ok') message = t('salon.media.transmitted');
      if (result === 'held') message = t('salon.media.held');
    } catch { error = t('salon.media.invalid'); }
    finally { busy = false; }
  }
  function save() {
    try { favorites = saveFavorite(localStorage, tv, { ...current(), label }); error = ''; }
    catch (failure) { error = t(failure.message === 'favorites_full' ? 'salon.media.full' : failure.message === 'unsupported_version' ? 'salon.media.version' : 'salon.media.save_error'); }
  }
  function remove(item) {
    try { favorites = removeFavorite(localStorage, tv, item); error = ''; }
    catch { error = t('salon.media.save_error'); }
  }
</script>

<section class="card launcher" aria-label={t('salon.media.title')}>
  <h2>{t('salon.media.title')}</h2>
  <p class="muted intro">{t('salon.media.help')}</p>
  <form onsubmit={(event) => { event.preventDefault(); launch(); }}>
    <div class="choices">
      <label>{t('salon.media.service')}<select bind:value={service} onchange={() => { kind = 'app'; target = ''; message = ''; }}>{#each SERVICES as provider}<option value={provider}>{brands[provider]}</option>{/each}</select></label>
      <label>{t('salon.media.kind')}<select bind:value={kind} onchange={() => { target = ''; message = ''; }}>{#each kindsFor(service) as type}<option value={type}>{t(`salon.media.kind.${type}`)}</option>{/each}</select></label>
    </div>
    {#if kind !== 'app'}
      <label>{t('salon.media.target')}<input bind:value={target} required maxlength="2048" spellcheck="false" autocomplete="off" aria-describedby="media-target-help" /></label>
      <small id="media-target-help" class="muted">{t(service === 'freetv' ? `salon.media.freetv.${kind}` : 'salon.media.target_help')}</small>
    {/if}
    {#if service === 'molotov'}<small class="muted">{t('salon.media.molotov')}</small>{/if}
    <div class="actions"><button class="primary" disabled={!available || busy}>{busy ? t('salon.media.sending') : t('salon.media.launch')}</button></div>
  </form>
  <form class="save" onsubmit={(event) => { event.preventDefault(); save(); }}>
    <label>{t('salon.media.favorite_name')}<input bind:value={label} maxlength="80" required autocomplete="off" /></label>
    <button disabled={!label.trim() || busy}>{t('salon.media.save')}</button>
  </form>
  {#if favorites.length}
    <ul aria-label={t('salon.media.favorites')}>
      {#each favorites as favorite (contentRequest(favorite))}
        <li><button class="favorite" disabled={!available || busy} onclick={() => launch(favorite)}><b>{favorite.label}</b><small>{brands[favorite.service]} · {t(`salon.media.kind.${favorite.kind}`)}</small></button><button class="remove" onclick={() => remove(favorite)} aria-label={t('salon.media.remove', { name: favorite.label })}>×</button></li>
      {/each}
    </ul>
  {/if}
  <p class="muted caution">{t('salon.media.qualify')}</p>
  {#if message}<p role="status">{message}</p>{/if}
  {#if error}<p class="failure" role="alert">{error}</p>{/if}
</section>

<style>
  .launcher { display: grid; gap: 14px; padding: 22px; }
  h2, p { margin: 0; }
  h2 { font-size: 20px; }
  .intro { max-width: 54ch; font-size: 14px; }
  form { display: grid; gap: 10px; }
  .choices { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  label { display: grid; gap: 5px; font-size: 13px; color: var(--ink-2); font-weight: 650; min-width: 0; }
  input, select { font: inherit; color: var(--ink); padding: 10px 12px; border-radius: var(--r-sm); border: 1px solid var(--line); background: var(--surface); width: 100%; min-width: 0; box-sizing: border-box; }
  button { border: 0; border-radius: 999px; padding: 10px 16px; background: var(--surface-2); color: var(--ink); font: inherit; font-weight: 650; cursor: pointer; }
  button.primary { background: var(--warm); color: #171c28; }
  button:disabled { opacity: 0.5; cursor: default; }
  .save { grid-template-columns: 1fr auto; align-items: end; padding-top: 8px; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { display: flex; align-items: center; gap: 8px; padding: 5px 0; }
  .favorite { flex: 1; text-align: left; border-radius: var(--r-sm); display: grid; gap: 3px; min-width: 0; overflow-wrap: anywhere; }
  small, .caution { font-size: 12px; font-weight: 400; }
  .remove { font-size: 22px; }
  .failure { color: var(--warm-ink); }
  @media (max-width: 480px) { .choices, .save { grid-template-columns: 1fr; } }
</style>
