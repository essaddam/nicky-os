// A request identity is independent of both the value and the wall clock.
// IndexedDB serializes the counter across tabs, including on plain local HTTP.
// No wall clock, per-reload namespace, or localStorage read/write race.
let database;

function openDatabase() {
  database ??= new Promise((resolve, reject) => {
    const request = indexedDB.open('moli-button-requests', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('client');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error('request identity storage blocked'));
  });
  return database;
}

export function nextButtonReference(previous) {
  let { client, sequence } = previous ?? {};
  if (!/^[0-9a-f]{32}$/.test(client ?? '') || !Number.isSafeInteger(sequence)
      || sequence < 0 || sequence >= Number.MAX_SAFE_INTEGER) {
    const bytes = crypto.getRandomValues(new Uint8Array(16));
    client = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
    sequence = 0;
  }
  return { client, sequence: sequence + 1 };
}

async function reserveReference() {
  const db = await openDatabase();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction('client', 'readwrite', { durability: 'strict' });
    const store = transaction.objectStore('client');
    const read = store.get('current');
    let reference;
    read.onsuccess = () => {
      reference = nextButtonReference(read.result);
      store.put(reference, 'current');
    };
    transaction.oncomplete = () => resolve(reference);
    transaction.onabort = () => reject(transaction.error);
    transaction.onerror = () => reject(transaction.error);
  });
}

export async function buttonRequest(point, reserve = reserveReference) {
  const { client, sequence } = await reserve();
  return Object.freeze({ point, value: null, request_id: `${client}:${sequence}` });
}

export async function sendButton(request, send = fetch) {
  const response = await send('/api/command', {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-moli-origin': 'ui' },
    body: JSON.stringify(request),
  });
  const body = await response.json();
  return { ok: response.ok, status: response.status, ...body };
}
