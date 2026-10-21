export function durationEditor(spec) {
  return {duration:true, source:spec.source, source_unit:spec.source_unit ?? null, value:spec.value, window_s:spec.window_s, max_age_s:spec.max_age_s};
}
export function durationPayload(form) {
  if (!form.duration || typeof form.source !== 'string' || !form.source.includes('/') || form.source.length > 256) return null;
  if ([form.window_s,form.max_age_s].some((n)=>!Number.isInteger(n)||n<1||n>604800)) return null;
  if (!['boolean','number','string'].includes(typeof form.value) || (typeof form.value==='number' && !Number.isFinite(form.value)) || (typeof form.value==='string' && new TextEncoder().encode(form.value).length>2000)) return null;
  return {type:'duration',value:{source:form.source,source_unit:form.source_unit ?? null,value:form.value,window_s:form.window_s,max_age_s:form.max_age_s}};
}
