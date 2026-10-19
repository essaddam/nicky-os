// Static same-origin return. No URL, authorization value or response body is logged.
(() => {
  const query = new URLSearchParams(window.location.search);
  window.history.replaceState(null, '', window.location.pathname);
  const status = document.getElementById('status');
  const messages = document.body.dataset;
  const code = query.get('code');
  const state = query.get('state');
  if (query.has('error')) {
    status.textContent = messages.denied;
    return;
  }
  if (!state || !/^[0-9a-fA-F]{64}$/.test(state)
      || !code || !/^[\x21-\x7e]{1,2048}$/.test(code)) {
    status.textContent = messages.failed;
    return;
  }
  fetch(window.location.pathname, {
    method: 'POST',
    credentials: 'same-origin',
    cache: 'no-store',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ state, code }),
  }).then(response => {
    status.textContent = response.ok ? messages.success
      : response.status === 403 ? messages.human : messages.failed;
  }).catch(() => {
    status.textContent = messages.failed;
  });
})();
