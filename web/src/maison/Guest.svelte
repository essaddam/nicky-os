<script>
  import '@fontsource-variable/figtree';
  import './maison.css';
  import { onMount } from 'svelte';
  import { home, hub, start, roomList } from './lib/home.svelte.js';
  import { refreshSession } from '../lib/hub.svelte.js';
  import { signOut, setPassword } from './lib/people.svelte.js';
  import { guestRoomList } from './lib/invitation.js';
  import { t } from '../lib/i18n.svelte.js';
  import Icon from './ui/Icon.svelte';
  import RoomCard from './ui/RoomCard.svelte';

  onMount(start);
  const person = $derived(hub.session?.person);
  const rooms = $derived(guestRoomList(roomList(), person));
  let password = $state('');
  let busy = $state(false);
  let message = $state('');
  let error = $state('');
  let wasConnected = false;
  $effect(() => {
    const online = hub.connected;
    if (!online && wasConnected) refreshSession();
    wasConnected = online;
  });
  const date = (value) => new Date(value).toLocaleString(undefined, { day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit', timeZoneName: 'short' });

  async function changePassword(event) {
    event.preventDefault();
    if (busy || password.length < 10) return;
    busy = true; message = ''; error = '';
    try { await setPassword(person.id, password); password = ''; message = t('personnes.carte.mot_de_passe_change'); }
    catch (failure) { error = failure.message; }
    finally { busy = false; }
  }
</script>

<div class="maison guest" data-theme="day">
  <main>
    <header>
      <div>
        <span class="eyebrow"><Icon name="home" size={20} />{t('personnes.invite.titre')}</span>
        <h1>{t('personnes.invite.bonjour', { name: person?.name ?? '' })}</h1>
        <p>{t('personnes.invite.aide')}</p>
        {#if person?.starts != null}<small>{t('personnes.invite.debut', { date: date(person.starts) })}</small>{/if}
        {#if person?.expires != null}<small>{t('personnes.invite.fin', { date: date(person.expires) })}</small>{/if}
      </div>
      <button class="ghost" onclick={signOut}><Icon name="logout" size={18} />{t('personnes.carte.deconnexion')}</button>
    </header>

    {#if !hub.connected}<p class="status" role="status">{t('personnes.invite.liaison')}</p>{/if}
    <section class="rooms" aria-label={t('personnes.invite.pieces')} inert={!hub.connected}>
      {#each rooms as room (room.name)}<RoomCard {room} every />{:else}<p>{t('personnes.invite.vide')}</p>{/each}
    </section>
    {#if home.held}
      <div class="status" role="status">
        <p>{t('personnes.invite.attente')}</p>
        <button class="ghost" onclick={() => (home.held = null)}>{t('commun.fermer')}</button>
      </div>
    {/if}

    <div class="notes" role="status" aria-live="polite">
      {#each home.notes as note (note.id)}<p class:error={note.tone === 'error'}>{note.text}</p>{/each}
    </div>

    <form class="account" onsubmit={changePassword}>
      <h2>{t('personnes.invite.compte')}</h2>
      <label><span>{t('personnes.carte.mon_mot_de_passe')}</span><input bind:value={password} type="password" autocomplete="new-password" placeholder={t('personnes.connexion.min', { min: 10 })} /></label>
      <button class="ghost" type="submit" disabled={busy || password.length < 10}>{t('personnes.carte.changer')}</button>
      {#if message}<p role="status">{message}</p>{/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
    </form>
  </main>
</div>

<style>
  .guest { display: block; min-height: 100svh; background: var(--bg); color: var(--ink); }
  main { max-width: 1120px; margin: 0 auto; padding: 32px 20px 56px; }
  header { display: flex; justify-content: space-between; align-items: flex-start; gap: 20px; margin-bottom: 28px; }
  .eyebrow { display: flex; gap: 8px; align-items: center; color: var(--ink-2); font-weight: 650; }
  h1 { margin: 12px 0 6px; font-size: clamp(26px, 4vw, 38px); }
  h2 { margin: 0; font-size: 18px; }
  header p { margin: 0 0 12px; color: var(--ink-2); }
  small { display: block; color: var(--ink-2); margin-top: 4px; }
  .rooms { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 320px), 1fr)); gap: 18px; align-items: start; }
  .ghost { display: inline-flex; align-items: center; justify-content: center; gap: 8px; min-height: 42px; padding: 8px 16px; border: 1px solid var(--surface-3); border-radius: 999px; background: var(--surface); color: inherit; font: inherit; cursor: pointer; }
  .status, .account { margin-top: 24px; padding: 20px; background: var(--surface); border: 1px solid var(--surface-3); border-radius: var(--r-xl); }
  .status { color: var(--ink-2); }
  .account { display: grid; gap: 12px; max-width: 480px; }
  label { display: grid; gap: 8px; font-weight: 600; }
  input { min-height: 44px; border: 1px solid var(--surface-3); border-radius: 12px; padding: 0 12px; background: var(--surface-2); color: inherit; font: inherit; }
  .error { color: var(--alert); }
  button:disabled { opacity: 0.5; cursor: default; }
  @media (max-width: 560px) { main { padding: 22px 14px 40px; } header { flex-direction: column; gap: 12px; } }
</style>
