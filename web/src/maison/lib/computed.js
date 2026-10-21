export function computedEditor(definition) {
  return { operation: definition.operation, sources: [...definition.sources], unit: definition.unit ?? '', max_age_s: definition.max_age_s };
}
export function computedPayload(form) {
  if (!['min', 'max', 'mean'].includes(form.operation) || !Number.isInteger(form.max_age_s) || form.max_age_s < 1 || form.max_age_s > 604800) return null;
  if (form.sources.length > 32 || new Set(form.sources).size !== form.sources.length || form.sources.some((p) => typeof p !== 'string' || !p.includes('/') || p.length > 256)) return null;
  if (typeof form.unit !== 'string' || new TextEncoder().encode(form.unit).length > 32) return null;
  return {type: 'aggregate', value: {operation: form.operation, sources: [...form.sources], unit: form.unit || null, max_age_s: form.max_age_s}};
}
