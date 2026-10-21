import { invitationLink } from './invitation.js';

// Only this small encoder is loaded on demand; scanning is a development test.
export async function invitationQr(link) {
  if (typeof link !== 'string' || link.length > 1024) throw new Error('invalid invitation link');
  const url = new URL(link);
  const parameters = new URLSearchParams(url.hash.slice(1));
  if ([...parameters].length !== 1 || !parameters.has('invite') || invitationLink(url.origin, parameters.get('invite')) !== url.href) throw new Error('invalid invitation link');
  const { default: qrcode } = await import('qrcode-generator');
  const code = qrcode(0, 'M');
  code.addData(url.href, 'Byte');
  code.make();
  const count = code.getModuleCount();
  let path = '';
  for (let y = 0; y < count; y++) for (let x = 0; x < count; x++) {
    if (code.isDark(y, x)) path += `M${x + 4} ${y + 4}h1v1h-1z`;
  }
  return { size: count + 8, path };
}
