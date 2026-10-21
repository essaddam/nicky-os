/** Shared versioned dependency transport; human authority stays in the server session. */
export async function dependencyRequest(kind, id, method, suffix = '', body, t) {
  const fallback = () => t('systeme.dependance.refus');
  if (!['schedule', 'computed'].includes(kind)) throw new Error(fallback());
  let response;
  try {
    response = await fetch(`/api/dependencies/${kind}/${encodeURIComponent(id)}${suffix}`, {
      method,
      headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch { throw new Error(t('systeme.dependance.connexion')); }
  const data = await response.json().catch(() => null);
  if (!response.ok || data == null) {
    const error = new Error(data?.error || fallback());
    error.status = response.status;
    throw error;
  }
  return data;
}

/** Never approve a server revision different from the definition on screen. */
export function previewMatchesShown(view, preview) {
  return !!view?.reference && !!preview?.reference
    && view.reference.kind === preview.reference.kind && view.reference.id === preview.reference.id
    && view.versions?.draft?.version === preview.versions?.draft
    && (view.versions?.current?.version ?? null) === preview.versions?.current;
}
