/** Draft parsing preserves scalar types; the server validates point metadata. */
export function scenePayload(rows, parallel, timeout, confirmation) {
  if (!rows.length || rows.length > 32 || !Number.isInteger(parallel) || parallel < 1 || parallel > 4
    || !Number.isInteger(timeout) || timeout < 1 || timeout > 120 || !Number.isInteger(confirmation)
    || confirmation < 1 || confirmation > 30 || confirmation > timeout) return null;
  const targets = [];
  for (const row of rows) {
    if (!row.point || !row.point.includes('/') || row.value == null || typeof row.value === 'object'
      || typeof row.value === 'number' && !Number.isFinite(row.value)) return null;
    const prior = targets.find((t) => t.point === row.point);
    if (prior && prior.value !== row.value) return null;
    if (!prior) targets.push({ point: row.point, value: row.value });
  }
  return { type: 'scene', value: { targets, parallel, timeout_s: timeout, confirmation_s: confirmation } };
}
export function scenePreviewMatches(view, preview) {
  return view?.reference.kind === 'scene' && view.reference.id === preview?.id
    && view.versions.current?.version === preview?.version;
}

export function sceneNeedsVersion(view, preview) {
  return Boolean(preview?.scene && view?.scope && view.versions.current
    && view.versions.current.version === view.versions.draft.version
    && JSON.stringify(preview.scene) !== JSON.stringify(view.scope));
}
