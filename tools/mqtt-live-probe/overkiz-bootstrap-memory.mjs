// One-shot encrypted bootstrap: the transport private key and token never enter files or logs.
import {generateKeyPairSync,privateDecrypt,constants} from 'node:crypto';
import {createConnection} from 'node:net';
import {lstatSync} from 'node:fs';

const host=process.argv[2],port=Number(process.argv[3]||8443);
if(!host||host.length>253||!/^[a-zA-Z0-9.:-]+$/.test(host)||!Number.isInteger(port)||port<1||port>65535)throw Error('invalid_endpoint');
const {publicKey,privateKey}=generateKeyPairSync('rsa',{modulusLength:4096,publicKeyEncoding:{type:'spki',format:'pem'}});
const path=`/tmp/moli-poc-${process.getuid()}/1414.sock`,stat=lstatSync(path);
if(!stat.isSocket()||stat.uid!==process.getuid()||(stat.mode&0o077)!==0)throw Error('invalid_private_socket');
console.log('PUBLIC_KEY '+Buffer.from(publicKey).toString('base64'));
let input='',used=false;
const timer=setTimeout(()=>{console.error(JSON.stringify({error:'bootstrap_timeout'}));process.exit(1)},45000);
process.stdin.setEncoding('utf8');
process.stdin.on('data',chunk=>{
 if(used)return;input+=chunk;
 if(input.length>2048){console.error(JSON.stringify({error:'bootstrap_input_bound'}));process.exit(1)}
 if(!input.includes('\n'))return;used=true;
 try{
  const encoded=input.trim();if(!/^[a-zA-Z0-9+/=]+$/.test(encoded))throw Error('ciphertext');
  const token=privateDecrypt({key:privateKey,padding:constants.RSA_PKCS1_OAEP_PADDING,oaepHash:'sha256'},Buffer.from(encoded,'base64'));
  if(!token.length||token.length>1024)throw Error('token_bound');
  const body=Buffer.from(JSON.stringify({op:'overkiz',setup:{host,port,token:token.toString('utf8')}}));token.fill(0);input='';
  const connection=createConnection({path});let reply='';
  connection.on('connect',()=>{connection.end(body,()=>body.fill(0))});
  connection.on('data',data=>{reply+=data.toString();if(reply.length>4096)connection.destroy(Error('reply_bound'))});
  connection.on('error',()=>{clearTimeout(timer);console.error(JSON.stringify({error:'bootstrap_socket'}));process.exit(1)});
  connection.on('end',()=>{clearTimeout(timer);try{const result=JSON.parse(reply);console.log(JSON.stringify({overkiz:result.overkiz||null,ok:result.overkiz==='started'}));process.exit(result.overkiz==='started'?0:1)}catch{process.exit(1)}});
 }catch{clearTimeout(timer);console.error(JSON.stringify({error:'bootstrap_refused'}));process.exit(1)}
});
