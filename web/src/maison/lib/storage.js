// The « Stockage » lines of a brick (history, energy) on the System page:
// translation keys and their values, formatted by `num` and `when`. A figure
// the server did not give is said unavailable, never shown as zero.

/** [{ key, params, alert?, note? }] for `facts` of brick `id`. */
export function storageLines(id, facts, { num, when }) {
  const f = facts ?? {};
  const lines = [];
  if (f.volume_error != null) {
    lines.push({ key: 'systeme.stockage.indisponible', params: { message: f.volume_error }, alert: true });
  } else if (f.rows_24h != null) {
    lines.push({
      key: 'systeme.stockage.lignes_24h',
      params: { count: f.rows_24h, n: num(f.rows_24h) },
      note: f.query_ms != null ? { key: 'systeme.stockage.cout', params: { ms: num(f.query_ms, 1) } } : null,
    });
  }
  const [ok, failed, words] = id === 'history' ? [f.batches_written, f.batches_failed, 'lots'] : [f.writes_ok, f.writes_failed, 'releves'];
  if (ok != null && failed != null) lines.push({ key: `systeme.stockage.${words}`, params: { ok: num(ok), failed: num(failed) } });
  if (id === 'history') {
    if (f.rows_merged || f.rows_dropped) {
      lines.push({ key: 'systeme.stockage.sature', params: { merged: num(f.rows_merged ?? 0), dropped: num(f.rows_dropped ?? 0) }, alert: true });
    }
    // `last_prune` null: none yet; absent: not known.
    if (f.last_prune) lines.push({ key: 'systeme.stockage.purge', params: { date: when(f.last_prune.at), n: num(f.last_prune.rows), ms: num(f.last_prune.ms) } });
    else if (f.last_prune === null) lines.push({ key: 'systeme.stockage.purge_aucune' });
  }
  if (f.last_error) lines.push({ key: 'systeme.stockage.erreur', params: { date: when(f.last_error[0]), message: f.last_error[1] }, alert: true });
  return lines;
}

const FAMILIES = ['power', 'climate', 'energy', 'states', 'diagnostic', 'other'];

/** What the history keeps (`GET /api/history/retention`), as translation
 *  keys: the general duration, each family's and the point rules, the
 *  points not written, and the reduction waiting for a person. */
export function retentionLines(policies, { num, when }) {
  const p = policies ?? {};
  const c = p.configured;
  if (!c) return [];
  const lines = [{ key: 'systeme.conservation.general', params: { count: c.days, n: num(c.days) } }];
  for (const f of FAMILIES) {
    if (c.families?.[f] != null) {
      lines.push({ key: 'systeme.conservation.famille', params: { family: { key: `systeme.conservation.familles.${f}` }, count: c.families[f], n: num(c.families[f]) } });
    }
  }
  if (c.points?.length) lines.push({ key: 'systeme.conservation.regles', params: { count: c.points.length, n: num(c.points.length) } });
  if (p.filter?.exclude?.length) lines.push({ key: 'systeme.conservation.exclus', params: { list: p.filter.exclude.join(', ') } });
  const r = p.reduction;
  if (r) {
    lines.push(
      r.rows
        ? { key: 'systeme.conservation.reduction', params: { count: r.rows, n: num(r.rows), points: num(r.points), from: when(r.from), to: when(r.to) }, alert: true }
        : { key: 'systeme.conservation.reduction_vide', alert: true },
    );
  }
  return lines;
}
