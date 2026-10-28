#!/usr/bin/env python3
"""Rend la proposition « ordres des appareils » : chaque ⟦expression⟧ du gabarit est calculée depuis la
série du banc (tools/ordres-banc/sorties/linux). Aucun chiffre n'est recopié à la main.
Usage : python3 -I rendre.py <gabarit.md> <sortie.md> <dossier des sorties> [--mutations=f] [--v2=dossier]
(v3 : `v2(sec, e, clé)` lit la série de la v2, pour comparer ; `fautes('D1')` compte les fautes
injectées détectées d'un constat ; NB_MUT, NB_MUT_DET, NB_MUT_V3.)"""
import glob
import pathlib
import re
import sys

tpl, out, sorties = sys.argv[1], sys.argv[2], pathlib.Path(sys.argv[3])
MUT = next((pathlib.Path(x.split("=", 1)[1]) for x in sys.argv[4:] if x.startswith("--mutations=")), None)
V2DIR = next((pathlib.Path(x.split("=", 1)[1]) for x in sys.argv[4:] if x.startswith("--v2=")), None)


def lire(dossier):
    res = {}
    for f in sorted(glob.glob(str(dossier / "*.txt"))):
        for l in open(f, encoding="utf-8"):
            if l.startswith("RESULTAT|"):
                _, sec, e, cle, val, _u = l.strip().split("|")
                res[(sec, int(e), cle)] = float(val)
    return res


RES = lire(sorties)
RES2 = lire(V2DIR) if V2DIR else {}


def v2(sec, e, cle):
    k = (sec, e, cle)
    if k not in RES2:
        sys.exit(f"résultat v2 absent : {k}")
    return RES2[k]


def v(sec, e, cle):
    k = (sec, e, cle)
    if k not in RES:
        sys.exit(f"résultat absent : {k}")
    return RES[k]


def grouper(ent):
    s = str(ent)
    parts = []
    while len(s) > 3:
        parts.insert(0, s[-3:])
        s = s[:-3]
    parts.insert(0, s)
    return " ".join(parts)


def f(x, d=1):
    neg = x < 0
    x = abs(x)
    s = f"{x:.{d}f}"
    ent, _, dec = s.partition(".")
    r = (grouper(int(ent)) if int(ent) >= 1000 else ent) + ("," + dec if dec else "")
    return ("−" if neg else "") + r


def n(x):
    return f(round(x), 0)


def ko(x):
    k = x / 1024
    return f(k, 0) if k >= 100 else f(k, 1)


def kb(x):
    """Ko décimaux : ce qui voyage (messages, transferts)."""
    k = x / 1000
    return f(k, 0) if k >= 100 else f(k, 1)


def mio(x):
    return f(x / 1048576, 1)


def mo(x):
    m = x / 1e6
    return f(m, 1) if m < 100 else f(m, 0)


def pc(x):
    return f(x, 1)


def oui(x):
    return "oui" if x >= 0.5 else "non"


tests = open(sorties / "tests.txt", encoding="utf-8").read()
m = re.search(r"test result: ok\. (\d+) passed", tests)
if not m:
    sys.exit("tests.txt : aucune ligne « test result: ok »")
TESTS = int(m.group(1))

bruts = []
for fic in sorted(glob.glob(str(sorties / "*.txt"))):
    nom = pathlib.Path(fic).name
    lignes = [l.rstrip("\n") for l in open(fic, encoding="utf-8")]
    if nom == "tests.txt":
        lignes = [l for l in lignes if l.startswith("test ") or "test result" in l or l.startswith("rustc")]
    bruts.append(f"Sortie brute — `{nom}` :\n\n```text\n" + "\n".join(lignes) + "\n```\n")
BRUTS = "\n".join(bruts)
MUTATIONS = ""
NB_MUT = NB_MUT_DET = NB_MUT_V3 = 0
DETECTEES = []
if MUT:
    lignes = open(MUT, encoding="utf-8").read().splitlines()
    rangs = []
    for k, l in enumerate(lignes):
        if l.startswith("détectée"):
            nom = l[len("détectée"):].strip()
            suite = lignes[k + 1].strip() if k + 1 < len(lignes) else ""
            nb_ = re.search(r"→ (\d+) test", suite)
            noms = suite.split(":", 1)[1].strip() if ":" in suite else ""
            rangs.append(f"| {nom} | {nb_.group(1) if nb_ else '?'} : {', '.join('`' + x.strip() + '`' for x in noms.split(',') if x.strip())} |")
        elif l.startswith(("PAS DÉTECTÉE", "NE COMPILE PAS")):
            rangs.append(f"| {l[15:].strip()} | **pas détectée** |")
    fin = [l for l in lignes if "mutations détectées sur" in l]
    if not fin:
        sys.exit("mutations.txt : aucune ligne de bilan")
    bilan = re.search(r"(\d+) mutations détectées sur (\d+)", fin[-1])
    NB_MUT_DET, NB_MUT = int(bilan.group(1)), int(bilan.group(2))
    DETECTEES = [l[len("détectée"):].strip() for l in lignes if l.startswith("détectée")]
    NB_MUT_V3 = sum(1 for x in DETECTEES if re.match(r"[DMW]\d+ ·", x))
    MUTATIONS = "| Faute injectée | Tests en échec |\n|---|---|\n" + "\n".join(rangs) + "\n\nBilan : " + fin[-1].strip() + "."
def fautes(code):
    return sum(1 for x in DETECTEES if x.startswith(code + " ·"))


ns = dict(v=v, v2=v2, fautes=fautes, NB_MUT=NB_MUT, NB_MUT_DET=NB_MUT_DET, NB_MUT_V3=NB_MUT_V3, f=f, n=n, ko=ko, kb=kb, mio=mio, mo=mo, pc=pc, oui=oui, TESTS=TESTS, BRUTS=BRUTS, MUTATIONS=MUTATIONS, len=len, round=round, max=max, min=min, abs=abs)
texte = open(tpl, encoding="utf-8").read()


def calc(m):
    r = eval(m.group(1), {"__builtins__": {}}, ns)
    return str(r)


rendu = re.sub(r"⟦(.*?)⟧", calc, texte, flags=re.S)
reste = re.findall(r"⟦.*?⟧", rendu)
if reste:
    sys.exit(f"jetons non rendus : {reste[:3]}")

UNITES = {"ms", "µs", "ns", "s", "o", "Ko", "Kio", "Mio", "Mo", "Go", "Mbit/s", "h", "min", "%", "octets", "fois", "e", "fsync"}


def atomes(texte):
    """Les mots, recollés là où une coupure de ligne serait fautive : un nombre et ses milliers, un
    nombre et son unité, « § » et son numéro, les guillemets et la ponctuation haute."""
    out = []
    for m in texte.split(" "):
        if not m:
            continue
        if out:
            prev = out[-1]
            dernier = prev.split(" ")[-1]
            nu = m.rstrip(".,;:)»")
            if ((re.fullmatch(r"[−-]?\d{1,3}", dernier) and re.fullmatch(r"\d{3}([,.]\d+)?[.,;:)]*", m))
                    or (re.search(r"\d$", prev) and nu in UNITES)
                    or dernier.endswith(("§", "«", "n°"))
                    or m in (":", ";", "?", "!", "»") or m.startswith("»")):
                out[-1] = prev + " " + m
                continue
        out.append(m)
    return out


def envelopper(prefixe, suite, texte, largeur=100):
    lignes, cur = [], prefixe
    for a in atomes(texte):
        vide = cur in (prefixe, suite) and (cur == prefixe or not lignes or cur == suite)
        essai = cur + a if cur in (prefixe, suite) else cur + " " + a
        if len(essai) > largeur and cur not in (prefixe, suite):
            lignes.append(cur)
            cur = suite + a
        else:
            cur = essai
    lignes.append(cur)
    return lignes


def reflow(md):
    sortie, bloc, pref, suite, code = [], [], None, None, False
    def vider():
        nonlocal bloc, pref, suite
        if bloc:
            texte = " ".join(l.strip() for l in bloc)
            sortie.extend(envelopper(pref, suite, texte[len(pref.strip()):].strip() if pref.strip() else texte))
        bloc, pref, suite = [], None, None
    for l in md.split("\n"):
        if l.startswith("```"):
            vider()
            code = not code
            sortie.append(l)
            continue
        if code or not l.strip() or l.lstrip().startswith(("|", "#", "<!--", "![")):
            vider()
            sortie.append(l)
            continue
        m = re.match(r"^(\s*)([-*]|\d+\.)\s+", l)
        if m:
            vider()
            pref = m.group(0)
            suite = " " * len(pref)
            bloc = [l]
            continue
        if bloc and pref and pref.strip() and l.startswith(suite):
            bloc.append(l)
            continue
        if bloc and not (pref and pref.strip()):
            bloc.append(l)
            continue
        vider()
        ind = re.match(r"^\s*", l).group(0)
        pref, suite, bloc = ind, ind, [l]
    vider()
    return "\n".join(sortie)


if out.endswith(".md"):
    rendu = reflow(rendu)
open(out, "w", encoding="utf-8").write(rendu)
print(f"ok {out} : {len(rendu.splitlines())} lignes, {len(RES)} résultats, {TESTS} tests")
