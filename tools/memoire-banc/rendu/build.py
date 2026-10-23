#!/usr/bin/env python3
"""Construit la page HTML d'une proposition depuis son Markdown (pandoc), dans la charte des
propositions existantes, avec en plus :
- les schémas : un marqueur `<!-- FIGURE:nom -->` est remplacé par `fig/nom.svg.html` ;
- les blocs techniques : « Bloc technique — titre : » suivi d'un bloc de code ;
- les annexes repliées : « Code complet … : » et « Sortie brute … : » suivis d'un bloc.
Usage : build.py <source.md> <sortie.html> <gabarit.html>"""
import hashlib
import html
import pathlib
import re
import subprocess
import sys

src, out, gabarit = sys.argv[1], sys.argv[2], sys.argv[3]
ICI = pathlib.Path(__file__).resolve().parent
md = open(src, encoding="utf-8").read()
sha = hashlib.sha256(md.encode("utf-8")).hexdigest()
name = src.rsplit("/", 1)[-1]

body = subprocess.run(
    ["pandoc", "-f", "gfm", "-t", "html5", "--section-divs", "--wrap=none", "--no-highlight"],
    input=md, capture_output=True, text=True, check=True,
).stdout

# Le titre et l'en-tête du Markdown deviennent la couverture.
body = re.sub(r'^<section id="[^"]*" class="level1">\s*<h1>.*?</h1>\s*', "", body, count=1, flags=re.S)
body = re.sub(r"</section>\s*$", "", body.strip())
first_h2 = body.find('<section id="1-')
intro, body = body[:first_h2], body[first_h2:]


# Schémas : chaque marqueur devient la figure SVG correspondante.
def figure(m):
    f = ICI / "fig" / f"{m.group(1)}.svg.html"
    if not f.exists():
        sys.exit(f"figure absente : {f}")
    return f.read_text(encoding="utf-8")


body = re.sub(r"<!--\s*FIGURE:([a-z0-9-]+)\s*-->", figure, body)
manquants = re.findall(r"FIGURE:[a-z0-9-]+", body)
if manquants:
    sys.exit(f"marqueurs non remplacés : {manquants}")
body = re.sub(r"<!--.*?-->", "", body, flags=re.S)

# Tableaux défilants, badges de gravité.
body = body.replace("<table>", '<div class="table-wrap"><table>').replace("</table>", "</table></div>")
SEV = {"Élevé": "high", "Moyen": "mid", "Faible": "low", "Maîtrisé": "ok"}
for word, cls in SEV.items():
    body = body.replace(f"<strong>{word}</strong>", f'<span class="sev sev-{cls}">{word}</span>')
    body = re.sub(rf"(<h3>[^<]*?) — {word}</h3>", rf'\1 <span class="sev sev-{cls}">{word}</span></h3>', body)
CLS = {"Élevé": "high", "Moyen": "mid", "Faible": "low", "Maîtrisé": "ok", "Critique": "crit"}


def badge_cells(table):
    # Seulement les tableaux de gravité : ailleurs « Faible » ou « Moyen » désignent un coût.
    if "Gravité" not in table.split("</thead>")[0]:
        return table
    return re.sub(r"<td>(Élevé|Moyen|Faible|Maîtrisé|Critique)( [^<]*)?</td>",
                  lambda m: f'<td><span class="sev sev-{CLS[m.group(1)]}">{m.group(1)}</span>{m.group(2) or ""}</td>', table)


body = re.sub(r"<table>.*?</table>", lambda m: badge_cells(m.group(0)), body, flags=re.S)

# Blocs techniques : un cartouche titré autour du bloc de code.
body = re.sub(
    r"<p>Bloc technique — (.*?)\s*:</p>\s*(<pre[^>]*><code>.*?</code></pre>)",
    r'<figure class="tech"><figcaption><span class="tech-tag">Bloc technique</span>\1</figcaption>\2</figure>',
    body, flags=re.S)

# Le code complet des preuves et les sorties brutes se replient.
body = re.sub(
    r"<p>((?:Code complet|Sortie brute).*?)\s*:</p>\s*(<pre[^>]*><code>.*?</code></pre>)",
    r'<details class="code"><summary>\1</summary>\2</details>',
    body, flags=re.S)

toc = "\n".join(
    f'<li><a href="#{sid}">{html.unescape(re.sub("<[^>]+>", "", title))}</a></li>'
    for sid, title in re.findall(r'<section id="([^"]+)" class="level2">\s*<h2>(.*?)</h2>', body, flags=re.S)
)

page = open(gabarit, encoding="utf-8").read()
# Les schémas du gabarit (couverture) d'abord, le corps ensuite (déjà résolu).
page = re.sub(r"<!--\s*FIGURE:([a-z0-9-]+)\s*-->", figure, page)
page = (page.replace("{{SHA}}", sha).replace("{{SOURCE}}", name)
        .replace("{{TOC}}", toc).replace("{{BODY}}", body).replace("{{INTRO}}", intro))
open(out, "w", encoding="utf-8").write(page)
schemas = page.count('<figure class="diagram')
blocs = body.count('<figure class="tech')
replis = body.count("<details")
print(f"ok {out} ({len(page)} octets), sha256 md {sha[:12]}, {schemas} schémas, {blocs} blocs techniques, {replis} replis")
