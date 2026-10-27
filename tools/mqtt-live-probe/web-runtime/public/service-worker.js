'use strict';
// Bump this version whenever the HTML shell or a bundled asset changes.
// Only generic static assets enter this cache: never API responses, MQTT,
// credentials, retained descriptions, or live device states.
const CACHE_PREFIX='moli-poc-shell-';
const CACHE_VERSION='v2-20261027-enums-app-label';
const CACHE_NAME=CACHE_PREFIX+CACHE_VERSION;
const BASE=new URL('./',self.registration.scope);
const ASSETS=new Map([
 ['', 'text/html'],
 ['mqtt.min.js','javascript'],
 ['manifest.webmanifest','json'],
 ['icons/moli.svg','image/svg+xml'],
 ['icons/moli-192.png','image/png'],
 ['icons/moli-512.png','image/png'],
 ['icons/apple-touch-icon.png','image/png'],
].map(([path,type])=>[new URL(path,BASE).href,type]));
function allowedRequest(request){
 const url=new URL(request.url);
 return request.method==='GET'&&url.origin===BASE.origin&&!url.search&&!url.username&&!url.password
  &&!request.headers.has('authorization')&&!request.headers.has('proxy-authorization')
  &&ASSETS.has(url.href);
}
async function cacheable(response,url){
 if(!response.ok||response.redirected||response.type==='opaque')return false;
 const type=response.headers.get('content-type')||'';
 if(!type.includes(ASSETS.get(url)))return false;
 // An authentication page can be HTML too. Persist only our generic shell.
 if(url===BASE.href&&!((await response.clone().text()).includes('<meta name="moli-poc-shell" content="1">')))return false;
 return true;
}
self.addEventListener('install',event=>{
 event.waitUntil((async()=>{
  const cache=await caches.open(CACHE_NAME);
  const responses=await Promise.all([...ASSETS.keys()].map(async url=>{
   const response=await fetch(new Request(url,{cache:'reload',credentials:'same-origin'}));
   if(!await cacheable(response,url))throw new Error('static shell unavailable');
   return [url,response];
  }));
  // Do not persist caller headers or URLs containing credentials/query data.
  for(const [url,response]of responses)await cache.put(url,response);
  await self.skipWaiting();
 })());
});
self.addEventListener('activate',event=>{
 event.waitUntil((async()=>{
  for(const name of await caches.keys())if(name.startsWith(CACHE_PREFIX)&&name!==CACHE_NAME)await caches.delete(name);
  await self.clients.claim();
 })());
});
self.addEventListener('fetch',event=>{
 if(!allowedRequest(event.request))return;
 const url=event.request.url;
 event.respondWith((async()=>{
  const cache=await caches.open(CACHE_NAME);
  // Keep the HTML current online, with a generic offline shell as fallback.
  if(url===BASE.href){
   try{
    const response=await fetch(event.request);
    if(await cacheable(response,url))await cache.put(url,response.clone());
    return response;
   }catch(error){const cached=await cache.match(url);if(cached)return cached;throw error}
  }
  const cached=await cache.match(url);if(cached)return cached;
  const response=await fetch(event.request);
  if(await cacheable(response,url))await cache.put(url,response.clone());
  return response;
 })());
});
