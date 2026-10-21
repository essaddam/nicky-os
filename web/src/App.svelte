<script>
  import { onMount } from 'svelte';
  import Maison from './maison/Maison.svelte';
  import Atelier from './Atelier.svelte';
  import Guest from './maison/Guest.svelte';
  import Login from './maison/ui/Login.svelte';
  import { hub } from './lib/hub.svelte.js';
  import { consumeInvitation } from './maison/lib/invitation.js';

  let invitation = $state(consumeInvitation(location, history));

  // Two faces, one hub: « Maison » for the family (default), « Atelier »
  // for tinkering with every device. Routes live in the hash.
  const read = () => location.hash.replace(/^#\/?/, '');
  let route = $state(read());

  onMount(() => {
    const update = () => {
      invitation = consumeInvitation(location, history);
      route = read();
    };
    addEventListener('hashchange', update);
    return () => removeEventListener('hashchange', update);
  });
</script>

{#if invitation}
  {#key invitation}<div class="maison"><Login initialCode={invitation} /></div>{/key}
{:else if hub.session?.person?.role === 'guest'}
  <Guest />
{:else if route === 'atelier' || route.startsWith('atelier/')}
  <Atelier />
{:else}
  <Maison {route} />
{/if}
