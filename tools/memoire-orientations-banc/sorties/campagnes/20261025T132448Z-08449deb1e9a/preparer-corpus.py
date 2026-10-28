#!/usr/bin/env python3
"""Enregistre seulement couverture et empreintes ; n'extrait aucun texte de chat."""
import hashlib,json,pathlib,re,sys,zipfile
p=pathlib.Path(sys.argv[1]);out=pathlib.Path(__file__).resolve().parents[2]/'docs/proposition/analyse/orientations-v4/corpus.json'
with zipfile.ZipFile(p) as z:
 index=z.read('Sarah_Home_Export/INDEX_ET_EXPORT_COMPLET.md').decode()
 items=[]
 for name in sorted(z.namelist()):
  if '/conversations/' not in name:continue
  raw=z.read(name);text=raw.decode();headers=re.findall(r'^### Message (\d+) — (.*)$',text,re.M)
  items.append(dict(code='C'+name.rsplit('/',1)[1][:2],title=text.splitlines()[0].lstrip('# '),filename=name.rsplit('/',1)[1],lines=len(text.splitlines()),messages=len(headers),user_messages=sum('(Utilisateur)' in who for _,who in headers),assistant_messages=sum('(Assistant)' in who for _,who in headers),sha256=hashlib.sha256(raw).hexdigest()))
 assert len(items)==5
 assert '5 sur 6' in index
 result=dict(archive_sha256=hashlib.sha256(p.read_bytes()).hexdigest(),conversations=items,total_messages=sum(x['messages'] for x in items),total_user_messages=sum(x['user_messages'] for x in items),coverage='5 conversations fournies ; une sixième annoncée triviale et exclue ; début de C04 annoncé manquant',index_is_not_an_extra_conversation=True,raw_transcripts_copied=False)
 out.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n');print(json.dumps(dict(conversations=5,messages=result['total_messages'],user_messages=result['total_user_messages'],raw_transcripts_copied=False)))
