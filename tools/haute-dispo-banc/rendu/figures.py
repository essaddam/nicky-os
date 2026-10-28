#!/usr/bin/env python3
"""Schémas et graphiques de la proposition haute disponibilité (docs/proposition/haute-disponibilite.md),
dans la charte des propositions. Les graphiques lisent la série du banc (../sorties/linux/*.txt) :
aucun chiffre recopié. Écrit les figures dans rendu/fig/ ; build.py les insère dans la page.

Usage : python3 -I tools/haute-dispo-banc/rendu/figures.py [dossier des sorties] [dossier des figures]"""
import glob
import pathlib
import sys

ICI = pathlib.Path(__file__).resolve().parent
SORTIES = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else ICI.parent / "sorties/linux"
F = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else ICI / "fig"
F.mkdir(exist_ok=True, parents=True)
# Les mêmes schémas en fichiers SVG autonomes, que le Markdown affiche comme des images.
MD = pathlib.Path(sys.argv[3]) if len(sys.argv) > 3 else ICI.parents[2] / "docs/proposition/haute-disponibilite"
MD.mkdir(exist_ok=True, parents=True)
PARTIES = {}
VERT, VERT2, AMBRE, AMBRE2, ROUGE, ENCRE, GRIS, LIGNE, FOND, DOUX = (
    "#195744", "#7fa892", "#b07d12", "#e3c77f", "#8b3b22", "#1d302a", "#52615a", "#d4ddd4", "#fffdf8", "#f1f4ee")
FONT = 'font-family="Avenir Next, Avenir, Segoe UI, sans-serif"'
MONO = 'font-family="SFMono-Regular, Consolas, monospace"'

RES = {}
for f in sorted(glob.glob(str(SORTIES / "*.txt"))):
    for l in open(f, encoding="utf-8"):
        if l.startswith("RESULTAT|"):
            _, sec, e, cle, v, _u = l.strip().split("|")
            RES[(sec, int(e), cle)] = float(v)


def r(sec, e, cle):
    k = (sec, e, cle)
    if k not in RES:
        raise SystemExit(f"résultat absent : {k}")
    return RES[k]


def esc(s):
    return str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def fr(x, d=1):
    return f"{x:.{d}f}".replace(".", ",")


def nb(v):
    return f"{v:,.0f}".replace(",", " ")


def court(v, unite=""):
    if v >= 100:
        s = nb(v)
    elif v >= 10:
        s = fr(v, 0)
    elif v >= 1:
        s = fr(v, 1)
    else:
        s = fr(v, 2)
    return s + ((" " + unite) if unite else "")


def figure(id_, titre, desc, corps, h, legende, fleches=True):
    defs = ""
    if fleches:
        defs = "\n".join(
            f'            <marker id="{id_}-{k}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="{c}"/></marker>'
            for k, c in (("v", VERT), ("a", AMBRE), ("g", GRIS), ("r", ROUGE)))
        defs = f"          <defs>\n{defs}\n          </defs>\n"
    PARTIES[id_] = (titre, desc, defs, corps, h, legende)
    return f'''      <figure class="diagram">
        <svg viewBox="0 0 1000 {h}" role="img" aria-labelledby="{id_}-t {id_}-d" {FONT}>
          <title id="{id_}-t">{esc(titre)}</title>
          <desc id="{id_}-d">{esc(desc)}</desc>
{defs}{corps}
        </svg>
        <figcaption>{legende}</figcaption>
      </figure>
'''


class Dessin:
    def __init__(self, id_):
        self.id, self.c = id_, []

    def t(self, x, y, s, taille=12, gras=False, coul=ENCRE, ancre="start", mono=False, halo=False, italique=False):
        a = [f'x="{x:.1f}"', f'y="{y:.1f}"', f'font-size="{taille}"', f'fill="{coul}"']
        if gras:
            a.append('font-weight="700"')
        if ancre != "start":
            a.append(f'text-anchor="{ancre}"')
        if mono:
            a.append(MONO)
        if italique:
            a.append('font-style="italic"')
        if halo:
            a.append(f'paint-order="stroke" stroke="{FOND}" stroke-width="4"')
        self.c.append(f'          <text {" ".join(a)}>{esc(s)}</text>')

    def rect(self, x, y, w, h, fond=DOUX, trait=LIGNE, rx=6, epais=1, tirets=None):
        d = f' stroke-dasharray="{tirets}"' if tirets else ""
        self.c.append(f'          <rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="{rx}" fill="{fond}" stroke="{trait}" stroke-width="{epais}"{d}/>')

    def ligne(self, x1, y1, x2, y2, coul=GRIS, epais=1.5, tirets=None, fleche=None):
        d = f' stroke-dasharray="{tirets}"' if tirets else ""
        m = f' marker-end="url(#{self.id}-{fleche})"' if fleche else ""
        self.c.append(f'          <line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" stroke="{coul}" stroke-width="{epais}"{d}{m}/>')

    def chemin(self, d, coul=GRIS, epais=1.5, tirets=None, fleche=None, fond="none"):
        t = f' stroke-dasharray="{tirets}"' if tirets else ""
        m = f' marker-end="url(#{self.id}-{fleche})"' if fleche else ""
        self.c.append(f'          <path d="{d}" fill="{fond}" stroke="{coul}" stroke-width="{epais}"{t}{m}/>')

    def titre(self, x, y, s, coul=GRIS):
        self.t(x, y, s.upper(), 11.5, True, coul)

    def boite(self, x, y, w, h, titre, lignes=(), fond="#ffffff", trait=LIGNE, coul=ENCRE, taille=11.5):
        self.rect(x, y, w, h, fond, trait)
        self.t(x + 12, y + 19, titre, taille, True, coul)
        for k, l in enumerate(lignes):
            self.t(x + 12, y + 36 + k * 15, l, 10, coul=GRIS)

    def texte(self):
        return "\n".join(self.c)


def couper(texte, largeur):
    lignes, cur = [], ""
    for m in texte.split():
        if cur and len(cur) + 1 + len(m) > largeur:
            lignes.append(cur)
            cur = m
        else:
            cur = (cur + " " + m).strip()
    return lignes + ([cur] if cur else [])


def ecrire(nom, html):
    (F / f"{nom}.svg.html").write_text(html, encoding="utf-8")
    titre, desc, defs, corps, h, legende = PARTIES[nom]
    # Le SVG autonome porte sa légende en bas, et un fond : il se lit seul, dans un éditeur Markdown.
    lignes = couper(legende, 150)
    hh = h + 16 + 17 * len(lignes)
    leg = "\n".join(f'  <text x="16" y="{h + 12 + 17 * (k + 1)}" font-size="12" fill="{GRIS}" font-style="italic">{esc(l)}</text>' for k, l in enumerate(lignes))
    (MD / f"{nom}.svg").write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 {hh}" width="1000" height="{hh}" role="img" aria-labelledby="{nom}-t {nom}-d" {FONT}>\n'
        f'  <title id="{nom}-t">{esc(titre)}</title>\n  <desc id="{nom}-d">{esc(desc)}</desc>\n'
        f'  <rect width="1000" height="{hh}" fill="{FOND}"/>\n{defs}{corps}\n{leg}\n</svg>\n', encoding="utf-8")


# ================================================================ 1. Le principe
d = Dessin("principe")
d.titre(16, 26, "Actif : il agit, il date, il numérote")
d.titre(684, 26, "Secours : il applique, dans l'ordre")
rangs = [
    ("Hub : valeurs, santé, numéros", "sorties val/ vu/ avail/ meta/ evt/ · rang s", VERT, "le pont"),
    ("Moteur, coffre, fichiers métier", "journal de réplication · rang j, dépend d'un s", AMBRE, "le même pont, repl/"),
    ("Programme, index, réseau, niveaux", "échéances monotones : à chaque machine", GRIS, "rien ne voyage"),
]
for k, (a, b, c, via) in enumerate(rangs):
    y = 44 + k * 78
    d.boite(16, y, 300, 58, a, [b], trait=c)
    if c == GRIS:
        d.boite(684, y, 300, 58, "se recalcule sur place", ["recompilé, reconstruit, refait"], trait=c)
        d.ligne(330, y + 29, 670, y + 29, GRIS, 1.2, "4 5")
        d.t(500, y + 22, via, 11, True, GRIS, "middle", halo=True)
    else:
        lab = "copie exacte des valeurs, instants de l'actif" if c == VERT else "exécutions, attentes, horaires, coffre…"
        d.boite(684, y, 300, 58, "copie du journal → mémoire", [lab], trait=c)
        d.ligne(330, y + 29, 670, y + 29, c, 2.4, fleche="v" if c == VERT else "a")
        d.t(500, y + 22, via, 11, True, c, "middle", halo=True)
d.rect(16, 284, 968, 72, DOUX, LIGNE)
for k, s in enumerate(["1. Recopier les sorties de l'actif, jamais ses entrées : le secours ne date rien, ne numérote rien, ne décide rien.",
                       "2. Tout ce qui quitte une machine est en heure murale ; chaque machine refait ses échéances monotones.",
                       "3. Le porteur de l'époque est seul à agir : ordres, numéros, jetons, export, notifications."]):
    d.t(30, 304 + k * 19, s, 11, coul=ENCRE)
ecrire("principe", figure("principe", "Trois sortes d'état, trois chemins",
    "À gauche l'actif, à droite le secours. Les sorties du hub passent par le pont ; ce qui n'est pas sur MQTT passe par le journal de réplication, dans le même pont ; ce qui se recalcule ne voyage pas. Trois règles en dessous.",
    d.texte(), 372,
    "Ce qui se recalcule ne voyage pas ; ce que le hub publie passe par le pont ; le reste passe par le journal de réplication. Le secours n'est jamais une seconde source de vérité."))

# ================================================================ 2. Copier par les sorties ou par les entrées
d = Dessin("copie")
att = r("copie", 1, "attentes")
ta, tb = r("copie", 1, "a_attentes_tenues"), r("copie", 1, "b_attentes_tenues")
d.titre(16, 26, "Une heure de la maison, pont coupé 30 s, horloges à 23 ms")
cols = [(300, "A · par les sorties", VERT), (650, "B · par les entrées", AMBRE)]
for x, lab, c in cols:
    d.t(x, 56, lab, 13, True, c)
rows = [
    ("Messages perdus pendant la coupure", f"{nb(r('copie', 1, 'a_perdues_au_pont'))}, vus au rang s, comblés", f"{nb(r('copie', 1, 'b_perdues'))}, que rien ne signale"),
    ("Fiches dont la valeur diffère", "0", nb(r("copie", 1, "b_valeur_differente"))),
    ("Fiches dont « changé » diffère", "0", f"{nb(r('copie', 1, 'b_change_different'))} (écart médian {nb(r('copie', 1, 'b_ecart_change_ms_median'))} ms)"),
    ("Mémoire identique (empreinte)", "oui", "non"),
]
for k, (lib, a, b) in enumerate(rows):
    y = 84 + k * 30
    d.ligne(16, y + 9, 984, y + 9, LIGNE, 1)
    d.t(16, y, lib, 11.5, True)
    d.t(300, y, a, 11.5, coul=VERT)
    d.t(650, y, b, 11.5, coul=AMBRE)
y = 84 + 4 * 30 + 6
d.t(16, y + 14, "Attentes « depuis 10 min » armées sur l'actif,", 11.5, True)
d.t(16, y + 30, "qui se revérifient sur le secours après une bascule", 11.5, True)
for x, v, c in ((300, ta, VERT), (650, tb, AMBRE)):
    d.rect(x, y, 300, 20, "#ffffff", LIGNE, 4)
    if v > 0:
        d.rect(x, y, 300 * v / att, 20, c, c, 4)
    d.t(x + 6, y + 36, f"{nb(v)} sur {nb(att)}", 12, True, c)
ecrire("copie", figure("copie", "Copier la mémoire par les sorties ou par les entrées",
    "Comparaison sur une heure de la maison. Par les sorties de l'actif, la mémoire du secours est identique et toutes les attentes se revérifient. Par les entrées, les valeurs perdues pendant la coupure restent fausses, les instants « changé » sont ceux du secours, et aucune attente ne se revérifie.",
    d.texte(), y + 52,
    "Le hub est la seule horloge de la maison. Un secours qui date les valeurs lui-même a d'autres instants « changé » : les attentes armées sur l'actif ne se revérifient plus après la bascule (mémoire v2 § 8.5)."))

# ================================================================ 3. Le pont à reprise
d = Dessin("pont")
d.titre(16, 26, "Actif")
d.titre(370, 26, "Le lien")
d.titre(690, 26, "Secours")
d.boite(16, 40, 280, 52, "Hub", ["publie val/ vu/ avail/ meta/ evt/ (rang s)"], trait=VERT)
d.boite(16, 110, 280, 52, "Courtier de l'actif (rumqttd)", ["un journal par filtre : l'ordre entre filtres se perd"])
d.boite(16, 180, 280, 66, "Journal (disque, 7 jours)", ["répond aux demandes de tronçons", "fenêtres de 32 × 64 sorties (≈ 94 Kio)"], trait=AMBRE)
d.ligne(156, 92, 156, 108, VERT, 2, fleche="v")
d.ligne(156, 180, 156, 164, AMBRE, 1.6, fleche="a")
d.rect(330, 40, 330, 206, DOUX, LIGNE, 8, 1, "5 4")
d.t(495, 62, "réseau local, ou NetBird (§ 10)", 11, True, GRIS, "middle")
fl = [(92, "sorties, en direct", VERT, True, None), (124, "demande de tronçon, à la reconnexion", AMBRE, False, None),
      (156, "ou quand un rang manque après 200 ms", AMBRE, False, None), (190, "tronçons, en binaire (46 o par sortie)", AMBRE, True, "6 4"),
      (222, "accusé de rang, toutes les 500 ms", GRIS, False, None)]
for y, lab, c, aller, tir in fl:
    if aller:
        d.ligne(300, y, 686, y, c, 2.2 if c == VERT else 1.6, tir, fleche="v" if c == VERT else "a")
    elif lab.startswith("ou"):
        d.t(495, y - 6, lab, 10, coul=c, ancre="middle", halo=True)
        continue
    else:
        d.ligne(686, y, 300, y, c, 1.4, tir, fleche="a" if c == AMBRE else "g")
    d.t(495, y - 6, lab, 10.5, True, c, "middle", halo=True)
d.boite(690, 40, 294, 66, "Pont à reprise", ["remet l'ordre de s ; comble par le journal", "une boucle MQTT jamais interrompue"], trait=VERT)
d.boite(690, 124, 294, 52, "Copie du journal", ["écrite avant de publier ; journal du suivant"], trait=AMBRE)
d.boite(690, 194, 294, 52, "Courtier du secours", ["les abonnés locaux lisent ici"])
d.ligne(837, 106, 837, 122, VERT, 2, fleche="v")
d.ligne(837, 176, 837, 192, VERT, 2, fleche="v")
d.boite(690, 266, 294, 66, "Hub de secours · base · écrans", ["comblent leurs trous par la copie du journal", "répondent à Unschedule"])
d.ligne(837, 246, 837, 264, VERT, 2, fleche="v")
d.chemin("M 690 300 C 640 300 640 150 688 150", AMBRE, 1.4, "5 4", fleche="a")
d.t(16, 290, "Jamais in/, req/, cmd/, res/ : l'ACL du compte « secours » le tient.", 11, coul=GRIS)
d.t(16, 308, "Le pont va toujours du porteur de l'époque vers les autres : aucune boucle.", 11, coul=GRIS)
ecrire("pont", figure("pont", "Le pont à reprise",
    "L'actif publie ses sorties ; le pont du secours les reçoit en direct, remet l'ordre par le rang s, demande les tronçons manquants au journal de l'actif, écrit chaque sortie dans la copie du journal, puis la publie sur le courtier du secours, où lisent les abonnés locaux. Les accusés de rang remontent vers l'actif.",
    d.texte(), 344,
    "Le tampon est le journal de l'actif, sur disque : le pont redemande exactement ce qui lui manque, à chaque reconnexion et à chaque rang qui tarde."))

# ================================================================ 4. Ce que le banc mesure
d = Dessin("mesures")
d.titre(16, 26, "Messages perdus sous deux coupures")
tot = r("pont", 1, "rumqttd_messages")
for k, (lab, v, c) in enumerate([("pont de rumqttd", r("pont", 1, "rumqttd_perdus"), ROUGE), ("pont à reprise", r("pont", 1, "coupures_perdus"), VERT)]):
    y = 46 + k * 44
    d.t(16, y + 14, lab, 11.5, True)
    d.rect(140, y, 160, 20, "#ffffff", LIGNE, 4)
    if v > 0:
        d.rect(140, y, 160 * v / tot, 20, c, c, 4)
    d.t(140, y + 36, f"{nb(v)} sur {nb(tot)}", 11, True, c)
d.titre(350, 26, "Rattrapage après la coupure")
rat = [("5 s coupé", r("pont", 1, "coupures_rattrapage_1")), ("10 s coupé", r("pont", 1, "coupures_rattrapage_2"))]
mx = max(v for _, v in rat) * 1.25
for k, (lab, v) in enumerate(rat):
    y = 46 + k * 44
    d.t(350, y + 14, lab, 11.5, True)
    d.rect(440, y, max(3, 170 * v / mx), 20, VERT, VERT, 4)
    d.t(440 + max(3, 170 * v / mx) + 6, y + 15, court(v, "ms"), 11, True, VERT)
d.titre(670, 26, "Retour après une journée")
jours = [("réseau local", r("rattrapage", 1, "jour_local_rattrapage_1") / 1000), ("10 Mbit/s", r("rattrapage", 1, "jour_10mbit_rattrapage_1") / 1000),
         ("1 Mbit/s", r("rattrapage", 1, "jour_1mbit_rattrapage_1") / 1000)]
mx = max(v for _, v in jours) * 1.2
for k, (lab, v) in enumerate(jours):
    y = 46 + k * 34
    d.t(670, y + 14, lab, 11.5, True)
    d.rect(770, y, max(3, 150 * v / mx), 20, AMBRE, AMBRE, 4)
    d.t(770 + max(3, 150 * v / mx) + 6, y + 15, court(v, "s"), 11, True, AMBRE)
d.t(16, 160, f"Régime, 200 messages par seconde : délai de la copie {court(r('pont', 1, 'regime_lat_p50'), 'ms')} en médiane, {court(r('pont', 1, 'regime_lat_p99'), 'ms')} au 99e centile.", 11.5, coul=ENCRE)
d.t(16, 180, f"Une journée de la maison : {nb(r('rattrapage', 1, 'jour_local_messages'))} sorties, {fr(r('rattrapage', 1, 'jour_1mbit_octets_retour') / 1e6, 1)} Mo en tronçons. Mémoire identique à la fin, dans tous les cas.", 11.5, coul=ENCRE)
ecrire("mesures", figure("mesures", "Le pont, mesuré",
    "Trois graphiques. Sous deux coupures, le pont de rumqttd perd des milliers de messages, le pont à reprise aucun. Après une coupure, le rattrapage prend une centaine de millisecondes. Une journée entière se rattrape en moins d'une seconde en local, en quelques secondes à 10 Mbit/s, en un peu plus d'une minute à 1 Mbit/s.",
    d.texte(), 196,
    "Deux courtiers rumqttd 0.20 et un relais qui imite le lien, en Linux musl (2 CPU, 128 Mio), médiane de trois essais.", fleches=False))

# ================================================================ 5. Le rang d'export
d = Dessin("stockage")
d.titre(16, 26, "Les opérations de moli.db (D13), dans l'ordre de leur rang seq")
k0 = int(r("stockage", 1, "premier_rang_different"))
lignes = [
    ("Actif", ["relevé", "relevé", "relevé", "import", "import", "heure close"], VERT),
    ("Secours qui dérive", ["relevé", "relevé", "relevé", "relevé", "heure close", "relevé"], ROUGE),
    ("Secours qui reçoit", ["relevé", "relevé", "relevé", "import", "import", "heure close"], VERT),
]
for j, (lab, ops, c) in enumerate(lignes):
    y = 46 + j * 56
    d.t(16, y + 22, lab, 11.5, True, c if j else ENCRE)
    for k, o in enumerate(ops):
        x = 190 + k * 128
        diff = j == 1 and k >= 3
        d.rect(x, y, 118, 34, "#ffffff", ROUGE if diff else LIGNE, 5, 1.5 if diff else 1)
        d.t(x + 59, y + 15, o, 10.5, True, ROUGE if diff else ENCRE, "middle")
        d.t(x + 59, y + 28, f"seq {nb(k0 - 3 + k)}", 9.5, coul=GRIS, ancre="middle", mono=True)
d.t(16, 222, f"L'archive a reçu les rangs jusqu'à K ; elle exige pour la suite exactement les mêmes octets (sinon 409). Le secours qui dérive diverge dès le {nb(k0)}e rang :", 11, coul=GRIS)
d.t(16, 240, "il ne peut pas reprendre l'export. Le secours qui reçoit les opérations de l'actif, avec leur rang, le peut.", 11, coul=GRIS)
ecrire("stockage", figure("stockage", "Le rang d'export de l'historique",
    "Trois rangées d'opérations. L'actif et le secours qui reçoit ses opérations ont les mêmes rangs. Le secours qui dérive son historique du flux numérote autrement à partir d'un import fait sur l'actif et d'une heure close à sa propre horloge.",
    d.texte(), 256,
    "Un import, une correction, une heure close à l'horloge de chaque machine : deux bases dérivées du même flux ne donnent pas les mêmes rangs.", fleches=False))

# ================================================================ 6. Les jetons qui tournent
d = Dessin("jetons")
for x0, titre in ((16, "Deux renouveleurs : révoqués"), (516, "Un renouveleur, copie en direct")):
    d.titre(x0, 26, titre, ROUGE if x0 == 16 else VERT)
    for k, lab in enumerate(["Actif", "Fournisseur", "Secours"]):
        x = x0 + 50 + k * 170
        d.rect(x - 45, 40, 90, 24, "#ffffff", LIGNE, 5)
        d.t(x, 56, lab, 11, True, ENCRE, "middle")
        d.ligne(x, 64, x, 300, LIGNE, 1, "3 3")
A, P, S = 66, 236, 406
d.ligne(A, 90, P - 4, 90, GRIS, 1.4, fleche="g"); d.t((A + P) / 2, 84, "renouveler (jeton 1)", 10, coul=GRIS, ancre="middle")
d.ligne(P, 112, A + 4, 112, VERT, 1.4, fleche="v"); d.t((A + P) / 2, 106, "jeton 2 ; le 1 est invalidé", 10, coul=VERT, ancre="middle")
d.ligne(S, 150, P + 4, 150, GRIS, 1.4, fleche="g"); d.t((S + P) / 2, 144, "renouveler (jeton 1)", 10, coul=GRIS, ancre="middle")
d.ligne(P, 172, S - 4, 172, ROUGE, 1.6, fleche="r"); d.t((S + P) / 2, 166, "déjà utilisé : révocation", 10, True, ROUGE, "middle")
d.ligne(A, 210, P - 4, 210, GRIS, 1.4, fleche="g"); d.t((A + P) / 2, 204, "renouveler (jeton 2)", 10, coul=GRIS, ancre="middle")
d.ligne(P, 232, A + 4, 232, ROUGE, 1.6, fleche="r"); d.t((A + P) / 2, 226, "révoqué aussi", 10, True, ROUGE, "middle")
d.t(16, 272, f"{nb(r('jetons', 1, 'a_deux_renouveleurs_revoques'))} fois sur 1 000, en 6 h au plus.", 11, True, ROUGE)
A, P, S = 566, 736, 906
d.ligne(A, 90, P - 4, 90, GRIS, 1.4, fleche="g"); d.t((A + P) / 2, 84, "renouveler (jeton 1)", 10, coul=GRIS, ancre="middle")
d.ligne(P, 112, A + 4, 112, VERT, 1.4, fleche="v"); d.t((A + P) / 2, 106, "jeton 2", 10, coul=VERT, ancre="middle")
d.rect(A - 40, 122, 80, 22, AMBRE2, AMBRE, 4); d.t(A, 137, "coffre, fsync", 9.5, True, ENCRE, "middle")
d.chemin(f"M {A} 152 C {A + 120} 152 {S - 120} 166 {S - 4} 166", AMBRE, 1.6, fleche="a"); d.t((A + S) / 2, 152, "copie scellée (jeton 2)", 10, True, AMBRE, "middle", halo=True)
d.t(S, 196, "bascule", 10.5, True, ENCRE, "middle")
d.ligne(S, 210, P + 4, 210, GRIS, 1.4, fleche="g"); d.t((S + P) / 2, 204, "renouveler (jeton 2)", 10, coul=GRIS, ancre="middle")
d.ligne(P, 232, S - 4, 232, VERT, 1.6, fleche="v"); d.t((S + P) / 2, 226, "jeton 3 : la maison continue", 10, True, VERT, "middle")
d.t(516, 272, f"{nb(r('jetons', 1, 'b_local_echecs'))} échec sur 10 000 bascules (secours local).", 11, True, VERT)
ecrire("jetons", figure("jetons", "Un jeton qui tourne, deux machines",
    "À gauche, deux machines renouvellent chacune le même jeton : le fournisseur voit revenir un jeton déjà utilisé et révoque tout. À droite, l'actif seul renouvelle, range le nouveau jeton au coffre, le copie scellé au secours, qui le présente après la bascule.",
    d.texte(), 290,
    "RFC 9700 § 4.14.2 : un jeton déjà utilisé qui revient fait révoquer le jeton actif. Netatmo fait tourner ses jetons. Simulation sur 30 jours, un renouvellement toutes les 3 h."))

# ================================================================ 7. L'arrêt figé
d = Dessin("arret")
d.titre(16, 26, "L'arrêt figé : budget visé 2 s, sous les 10 s de Docker")
etapes = [("1", "annonce", "sys/arret ; plus d'ordre accepté", 0.0, 0.05), ("2", "ordres en vol", "1 s au plus, sinon « incertain »", 0.05, 1.05),
          ("3", "pilotes", "arrêt, déconnexion", 1.05, 1.35), ("4", "figer", "s = N, j = M", 1.35, 1.4), ("5", "instantané", "écrit, fsync", 1.4, 1.55),
          ("6", "marque", "« arrêt propre à (N, M) »", 1.55, 1.6), ("7", "relève", "si le secours prend la main", 1.6, 1.85), ("8", "fin", "le processus s'arrête", 1.85, 1.9)]
X0, W, Y0, PAS = 470, 500, 46, 24
def TX(s): return X0 + W * s / 2.0
for s_ in (0, 0.5, 1, 1.5, 2):
    d.ligne(TX(s_), Y0 - 4, TX(s_), Y0 + PAS * len(etapes) + 2, LIGNE, 1)
    d.t(TX(s_), Y0 + PAS * len(etapes) + 18, f"{fr(s_, 1)} s", 10, coul=GRIS, ancre="middle")
for k, (num, lab, note, a_, b_) in enumerate(etapes):
    y = Y0 + k * PAS
    c = AMBRE if num in ("2", "7") else VERT
    d.t(16, y + 13, f"{num}. {lab}", 11, True, ENCRE)
    d.t(150, y + 13, note, 10.5, coul=GRIS)
    d.rect(TX(a_), y + 2, max(5, TX(b_) - TX(a_)), 14, c, c, 3)
ecrire("arret", figure("arret", "L'arrêt figé, pas à pas",
    "Huit étapes, une par ligne, sur une frise de 0 à 2 secondes : annonce, attente des ordres en vol (au plus une seconde), arrêt des pilotes, gel de la mémoire au rang (N, M), écriture de l'instantané, marque d'arrêt propre au journal, relève éventuelle, fin.",
    d.texte(), Y0 + PAS * len(etapes) + 30,
    "Seule l'attente des ordres en vol prend du temps ; figer et écrire l'instantané se compte en millisecondes (§ 9.5).", fleches=False))

# ================================================================ 8. Le secours distant
d = Dessin("distant")
d.titre(16, 26, "La maison")
d.titre(700, 26, "Hors de la maison")
d.rect(16, 36, 380, 250, DOUX, LIGNE, 8)
d.boite(30, 48, 170, 52, "H1 · actif", ["époques maison et cloud"], trait=VERT)
d.boite(214, 48, 170, 52, "H2 · secours local", ["peut tout reprendre"], trait=VERT)
d.boite(30, 116, 170, 46, "Témoin", ["NAS, routeur ou Pi"])
d.boite(214, 116, 170, 46, "Boîtiers radio A et B", ["Zigbee, Thread"])
d.ligne(200, 74, 212, 74, VERT, 2, fleche="v")
d.t(30, 190, "Le pont local : le même que vers le distant.", 10.5, coul=GRIS)
d.t(30, 208, "Le témoin départage H1 et H2 ; il n'arbitre pas", 10.5, coul=GRIS)
d.t(30, 224, "le distant : sans Internet, la maison doit vivre.", 10.5, coul=GRIS)
d.ligne(396, 70, 694, 70, VERT, 2.2, fleche="v")
d.t(545, 62, "le pont, par NetBird", 11, True, VERT, "middle", halo=True)
d.ligne(396, 104, 694, 104, AMBRE, 1.4, "5 4", fleche="a")
d.t(545, 96, "battement direct en HTTPS : la seconde veille", 10, True, AMBRE, "middle", halo=True)
d.rect(410, 124, 270, 122, "#ffffff", LIGNE, 8, 1, "5 4")
d.t(545, 146, "NetBird (WireGuard maillé)", 11.5, True, ENCRE, "middle")
d.t(545, 166, "pair à pair : ICE, STUN", 10.5, coul=GRIS, ancre="middle")
d.t(545, 184, "sinon un relais : QUIC, ou WebSocket", 10.5, coul=GRIS, ancre="middle")
d.t(545, 200, "sur le port 443 si l'UDP est bloqué", 10.5, coul=GRIS, ancre="middle")
d.t(545, 220, "chiffré de bout en bout", 10.5, coul=GRIS, ancre="middle")
d.boite(700, 48, 284, 112, "Secours distant", ["copie : mémoire, journal, historique", "alerte : par son propre chemin", "cloud : sur geste humain (époque cloud)", "peut héberger l'archive (D13)", "jamais les radios ni le réseau local"], trait=AMBRE)
d.boite(700, 190, 284, 58, "Clouds : Netatmo, Voltalis, Ariston…", ["un seul porteur de l'époque cloud les appelle"])
d.ligne(842, 160, 842, 188, AMBRE, 1.6, fleche="a")
d.t(16, 306, "Secrets : le distant ne reçoit que ceux du domaine cloud, scellés avec sa clé ; la clé maître de la maison ne sort jamais.", 10.5, coul=GRIS)
ecrire("distant", figure("distant", "Le secours distant, par NetBird",
    "À gauche, la maison : l'actif, le secours local, le témoin et les boîtiers radio. Le pont part vers le secours distant par NetBird, pair à pair ou par un relais, en QUIC ou en WebSocket sur le port 443 ; un battement HTTPS direct sert de seconde veille. À droite, le secours distant : copie, alerte, cloud sur geste humain, archive.",
    d.texte(), 318,
    "Le même pont qu'en local, sur l'adresse NetBird de l'actif. Le WebSocket, NetBird le fait déjà, en dessous, quand l'UDP est bloqué."))

# ================================================================ 9. La bascule
d = Dessin("bascule")
d.titre(16, 26, "Une bascule de H1 vers H2, et ce qui est incertain")
X0, W = 60, 880
def TB(s): return X0 + W * s / 8.0
for s in range(0, 9):
    d.ligne(TB(s), 46, TB(s), 196, LIGNE, 1)
    d.t(TB(s), 212, f"{s} s", 10, coul=GRIS, ancre="middle")
d.t(16, 66, "H1", 11.5, True)
for s in (0, 1, 2):
    d.c.append(f'          <circle cx="{TB(s):.1f}" cy="62" r="4" fill="{VERT}"/>')
d.t(TB(0), 82, "bail renouvelé chaque seconde", 10, coul=GRIS)
d.rect(TB(2.3), 54, TB(4.3) - TB(2.3), 16, ROUGE, ROUGE, 3)
d.t(TB(2.3), 50, "gel de H1", 10, True, ROUGE)
d.t(16, 116, "Témoin", 11.5, True)
d.rect(TB(2), 104, TB(5) - TB(2), 16, AMBRE2, AMBRE, 3)
d.t(TB(2) + 6, 116, "le bail N court encore (3 s)", 10, True, ENCRE)
d.t(16, 166, "H2", 11.5, True)
d.rect(TB(2.3), 150, TB(5) - TB(2.3), 22, "#fbe9e2", ROUGE, 3, 1, "4 3")
d.t(TB(2.3) + 6, 165, "incertain : « peut-être déjà parti », jamais rejoué", 10, True, ROUGE)
d.rect(TB(5), 150, TB(8) - TB(5), 22, VERT2, VERT, 3)
d.t(TB(5) + 6, 165, "époque N + 1 : H2 agit ; rattrapage normal (2 min)", 10, True, ENCRE)
d.ligne(TB(3.3), 124, TB(3.3), 146, GRIS, 1.2, fleche="g")
d.t(TB(3.3) + 4, 140, "silence ≥ 1 s : H2 demande N + 1", 9.5, coul=GRIS, halo=True)
d.t(16, 236, "Domaine maison : le secours local seulement. Domaine cloud : le secours local avec la maison, ou le distant sur geste humain.", 10.5, coul=GRIS)
d.t(16, 254, "Chaque ordre porte l'époque de qui l'émet : un module refuse une époque plus ancienne.", 10.5, coul=GRIS)
ecrire("bascule", figure("bascule", "La bascule et la fenêtre d'incertitude",
    "Une frise de 8 secondes. H1 renouvelle son bail chaque seconde puis gèle ; le témoin laisse courir le bail N trois secondes ; H2 demande l'époque suivante après une seconde de silence et la reçoit à la fin du bail. Entre le dernier enregistrement reçu et la fin du bail, ce qui était dû est incertain ; après, H2 agit et rattrape normalement.",
    d.texte(), 266,
    "La fenêtre d'incertitude dure le bail : quelques secondes. Ce qui y tombe n'est ni rejoué ni perdu en silence."))

# ================================================================ 10. Vue d'ensemble
d = Dessin("ensemble")
d.titre(16, 26, "La maison")
d.titre(700, 26, "Hors de la maison")
d.rect(16, 36, 600, 378, DOUX, LIGNE, 8)
d.boite(32, 52, 250, 112, "H2 · secours local (passif)", ["pont à reprise, copie du journal", "hub et base en copie", "moteur sans aucune action", "pilotes et ponts radio arrêtés"], trait=VERT)
d.boite(350, 52, 250, 112, "H1 · actif (porte les époques)", ["hub, moteur, coffre, base", "courtier, journal (7 jours)", "pilotes, Zigbee2MQTT, Matter", "seul à agir, numéroter, exporter"], trait=VERT)
d.ligne(348, 88, 284, 88, VERT, 2.4, fleche="v")
d.t(316, 80, "pont", 10.5, True, VERT, "middle", halo=True)
d.ligne(284, 128, 348, 128, AMBRE, 1.3, "4 3", fleche="a")
d.t(316, 146, "accusés", 9.5, coul=AMBRE, ancre="middle", halo=True)
d.boite(32, 190, 170, 52, "Témoin", ["vote le bail de l'époque"])
d.ligne(117, 164, 117, 188, GRIS, 1.2, "3 3")
d.chemin("M 400 164 L 400 177 L 186 177 L 186 188", GRIS, 1.2, "3 3")
d.boite(232, 190, 176, 52, "Boîtier radio A", ["Zigbee actif · Thread actif"], trait=VERT)
d.boite(424, 190, 176, 52, "Boîtier radio B", ["Zigbee secours · Thread actif"], trait=AMBRE)
d.rect(32, 262, 568, 50, "#ffffff", LIGNE, 6)
d.t(46, 282, "Appareils : Zigbee, Thread, Wi-Fi, MQTT du réseau local", 11.5, True)
d.t(46, 300, "ils parlent à l'adresse virtuelle de l'actif (VRRP) ; les radios suivent l'époque", 10, coul=GRIS)
d.t(32, 338, "Le témoin départage H1 et H2 ; sans lui, la promotion du secours reste humaine (V7 § 18).", 10.5, coul=GRIS)
d.t(32, 356, "Une liaison Zigbee directe (interrupteur → lampe) marche même quand tout le reste est tombé.", 10.5, coul=GRIS)
d.t(32, 374, "Thread : un routeur de bordure sur chaque boîtier, rien à basculer. Zigbee, Matter : un seul écrivain.", 10.5, coul=GRIS)
d.t(32, 392, "Le secours local peut tout reprendre ; il ne fait rien tant qu'il ne porte pas l'époque.", 10.5, coul=GRIS)
d.boite(700, 52, 260, 98, "Secours distant", ["copie : mémoire, journal, historique", "alerte si la maison se tait", "cloud : sur geste humain", "jamais les radios ni le réseau local"], trait=AMBRE)
d.boite(700, 176, 260, 52, "Archive (Parquet, long terme)", ["reçoit l'export du porteur de l'époque"])
d.boite(700, 254, 260, 52, "Clouds : Netatmo, Voltalis, Ariston…", ["appelés par le seul porteur du cloud"])
d.ligne(602, 80, 698, 80, VERT, 2.4, fleche="v")
d.t(650, 72, "NetBird", 10, True, VERT, "middle", halo=True)
d.chemin("M 602 136 C 640 136 650 202 698 202", GRIS, 1.4, fleche="g")
d.t(648, 172, "export", 10, True, GRIS, "middle", halo=True)
d.chemin("M 602 152 C 630 152 650 280 698 280", AMBRE, 1.4, fleche="a")
d.t(632, 246, "cloud", 10, True, AMBRE, "middle", halo=True)
d.chemin("M 960 100 L 978 100 L 978 280 L 962 280", AMBRE, 1.3, "5 4", fleche="a")
d.t(700, 330, "Le distant n'appelle les clouds que s'il est promu (geste humain).", 10, coul=AMBRE)
ecrire("ensemble", figure("ensemble", "La maison en double, vue d'ensemble",
    "À gauche, la maison : l'actif H1 porte les époques et envoie ses sorties au secours local H2 par le pont ; un témoin vote le bail ; deux boîtiers radio, Thread actif sur les deux, Zigbee actif sur A seulement ; les appareils parlent à l'adresse virtuelle de l'actif. À droite, hors de la maison : le secours distant reçoit le pont par NetBird, l'archive reçoit l'export, les clouds ne sont appelés que par le porteur du cloud.",
    d.texte(), 420,
    "Une seule machine agit à la fois ; les autres copient. Le secours local peut tout reprendre ; le distant, seulement le cloud, et seulement sur un geste humain."))

# ================================================================ 11. Pourquoi les entrées ne suffisent pas
d = Dessin("entrees")
for y, l1, l2, c in ((78, "Actif", "", ENCRE), (170, "Secours A", "recopie les sorties", VERT), (262, "Secours B", "retraite les entrées", AMBRE)):
    d.t(16, y - (6 if l2 else -4), l1.upper(), 11.5, True, c)
    if l2:
        d.t(16, y + 12, l2, 10.5, coul=c)
d.chemin("M 180 106 L 180 262 L 286 262", AMBRE, 1.3, "5 4", fleche="a")
d.t(186, 200, "in/ par le pont", 9.5, coul=AMBRE, halo=True)
d.boite(150, 50, 120, 56, "Module", ["in/ : porte ouverte"])
d.ligne(270, 78, 288, 78, GRIS, 1.4, fleche="g")
d.boite(290, 50, 180, 56, "21:42:03,120", ["hub : changé = 03,120"], trait=VERT)
d.ligne(380, 106, 380, 140, VERT, 2, fleche="v")
d.t(386, 128, "val/ : c = 03,120", 9.5, True, VERT, halo=True)
d.ligne(650, 106, 650, 232, VERT, 1.4, "5 4", fleche="v")
d.boite(290, 142, 180, 56, "changé = 03,120", ["recopié de la sortie"], trait=VERT)
d.boite(290, 234, 180, 56, "changé = 03,145", ["son horloge, après le pont"], trait=AMBRE)
d.boite(490, 50, 180, 56, "attente armée", ["depuis = 03,120"], trait=VERT)
d.ligne(580, 106, 580, 140, VERT, 2, fleche="v")
d.t(586, 128, "journal", 9.5, True, VERT, halo=True)
d.boite(490, 142, 180, 56, "attente reçue", ["depuis = 03,120"], trait=VERT)
d.boite(490, 234, 180, 56, "attente reçue", ["depuis = 03,120"], trait=AMBRE)
d.ligne(690, 36, 690, 300, ROUGE, 1.6, "6 4")
d.t(696, 46, "21:45 : bascule", 10.5, True, ROUGE)
d.boite(710, 56, 274, 50, "H1 gelé", ["H2 porte l'époque suivante"])
d.boite(710, 142, 274, 66, "à 21:52:03,120", ["changé 03,120 = depuis 03,120", "le déclencheur part ✓"], trait=VERT)
d.boite(710, 228, 274, 66, "à 21:52:03,120", ["changé 03,145 ≠ depuis 03,120", "attente abandonnée : perdu, sans un mot ✗"], trait=ROUGE)
d.t(16, 322, "L'instant « changé » sert de version à la valeur (mémoire v2 § 8.5) : il doit être celui de l'actif, à la milliseconde.", 10.5, coul=GRIS)
ecrire("entrees", figure("entrees", "Pourquoi la copie par les entrées casse les attentes",
    "Trois rangées : l'actif, le secours A qui recopie les sorties, le secours B qui retraite les entrées. La porte s'ouvre à 21:42:03,120 ; l'actif note changé = 03,120 ; A recopie 03,120 ; B note 03,145, son horloge après le pont. Une attente « depuis 10 min » est armée sur l'actif avec depuis = 03,120 et part au journal. Après la bascule de 21:45, à l'échéance, A trouve changé égal à depuis et le déclencheur part ; B trouve 03,145 différent de 03,120 : l'attente est abandonnée.",
    d.texte(), 334,
    "Un secours qui date lui-même ses valeurs a d'autres instants « changé » que l'actif : toutes les attentes armées avant la bascule échouent à leur revérification."))

# ================================================================ 12. Comment un trou se comble
d = Dessin("trou")
X = {"j": 110, "c": 330, "p": 570, "s": 850}
for k, lab in (("j", "Journal de l'actif"), ("c", "Courtier de l'actif"), ("p", "Pont à reprise"), ("s", "Copie du journal, secours")):
    d.rect(X[k] - 92, 36, 184, 26, "#ffffff", LIGNE, 5)
    d.t(X[k], 53, lab, 11, True, ENCRE, "middle")
    d.ligne(X[k], 62, X[k], 330, LIGNE, 1, "3 3")
def msg(y, a, b, lab, c=VERT, tir=None, gras=True):
    d.ligne(X[a], y, X[b] + (-4 if X[b] > X[a] else 4), y, c, 1.8 if c == VERT else 1.5, tir, fleche="v" if c == VERT else ("a" if c == AMBRE else ("r" if c == ROUGE else "g")))
    d.t((X[a] + X[b]) / 2, y - 6, lab, 10, gras, c, "middle", halo=True)
msg(88, "c", "p", "sorties 98, 99, 100, en direct")
msg(112, "p", "s", "écrites, puis publiées : 98, 99, 100")
d.rect(X["c"] + 10, 128, X["p"] - X["c"] - 20, 26, "#fbe9e2", ROUGE, 4, 1, "4 3")
d.t((X["c"] + X["p"]) / 2, 145, "coupure : 101 à 250 passent sans lui", 10, True, ROUGE, "middle")
msg(178, "p", "c", "reconnexion, abonnement, demande {de : 101}", AMBRE)
msg(202, "c", "j", "demande", AMBRE)
msg(228, "c", "p", "251, 252 en direct : gardées en tampon", VERT, gras=False)
msg(256, "j", "p", "tronçons 101–164, 165–228, 229–250 (une fenêtre)", AMBRE, "6 4")
msg(284, "p", "s", "101 … 250, puis 251, 252 : dans l'ordre")
d.rect(X["s"] - 120, 296, 240, 30, DOUX, LIGNE, 5)
d.t(X["s"], 315, "les abonnés locaux n'ont vu qu'un retard", 10, coul=GRIS, ancre="middle")
d.t(16, 350, "Le courtier n'a rien à garder : le trou se voit au rang s, le journal de l'actif le comble, le pont remet l'ordre.", 10.5, coul=GRIS)
ecrire("trou", figure("trou", "Comment un trou se comble",
    "Diagramme de séquence entre le journal de l'actif, le courtier de l'actif, le pont à reprise et la copie du journal du secours. Les sorties 98 à 100 passent en direct ; une coupure fait perdre 101 à 250 ; à la reconnexion, le pont demande le journal à partir de 101 ; les sorties 251 et 252 arrivent en direct et attendent dans le tampon ; les tronçons 101 à 250 arrivent ; le pont publie tout dans l'ordre.",
    d.texte(), 362,
    "Une fenêtre de 32 tronçons de 64 sorties à la fois : le courtier ne saute jamais une fenêtre. Le pont écrit dans la copie du journal avant de publier."))

# ================================================================ 13. Remettre l'ordre
d = Dessin("ordre")
d.titre(16, 26, "Deux filtres, deux journaux du courtier")
d.titre(520, 26, "Un enregistrement j attend sa valeur")
def jeton(x, y, lab, c, w=46):
    d.rect(x, y, w, 26, "#ffffff", c, 5, 1.6)
    d.t(x + w / 2, y + 17, lab, 10.5, True, c, "middle")
for k, (lab, y) in enumerate((("publié par le hub", 52), ("reçu par un abonné à val/# et vu/#", 120), ("après le rattrapeur", 188))):
    d.t(16, y - 6, lab, 10, coul=GRIS)
for k, n in enumerate([1, 2, 3, 4, 5]):
    jeton(16 + k * 56, 52, f"s {n}", VERT if n % 2 else AMBRE)
for k, n in enumerate([1, 3, 5, 2, 4]):
    jeton(16 + k * 56, 120, f"s {n}", VERT if n % 2 else AMBRE)
for k, n in enumerate([1, 2, 3, 4, 5]):
    jeton(16 + k * 56, 188, f"s {n}", VERT if n % 2 else AMBRE)
d.t(300, 70, "val/ (vert), vu/ (ambre)", 10, coul=GRIS)
d.t(300, 132, "3 et 5 avant 2 et 4,", 10, coul=GRIS)
d.t(300, 146, "aucun message perdu", 10, coul=GRIS)
d.t(300, 200, "remis dans l'ordre de s ;", 10, coul=GRIS)
d.t(300, 214, "trou demandé après 200 ms", 10, coul=GRIS)
d.ligne(500, 36, 500, 236, LIGNE, 1)
d.t(520, 46, "flux s (valeurs)", 10, coul=GRIS)
jeton(520, 52, "s 99", VERT, 56)
jeton(584, 52, "s 100 : porte ouverte", VERT, 170)
d.t(520, 104, "flux j (journal)", 10, coul=GRIS)
jeton(520, 110, "j 50 : attente armée, dépend de s 100", AMBRE, 270)
d.t(520, 162, "au secours, dans l'ordre d'arrivée puis d'application", 10, coul=GRIS)
jeton(520, 168, "j 50", AMBRE, 50)
d.t(578, 186, "attend…", 10, True, AMBRE)
jeton(640, 168, "s 99", VERT, 50)
jeton(698, 168, "s 100", VERT, 56)
d.ligne(760, 181, 790, 181, GRIS, 1.4, fleche="g")
jeton(796, 168, "j 50 appliqué", VERT, 108)
d.t(520, 222, "Sans cette règle, une bascule entre les deux laisserait", 10, coul=GRIS)
d.t(520, 236, "une attente sans sa valeur : elle ne se revérifierait pas.", 10, coul=GRIS)
ecrire("ordre", figure("ordre", "Remettre l'ordre : le rang s, et la dépendance de j",
    "À gauche : le hub publie s 1 à 5, alternant val/ et vu/ ; un abonné aux deux filtres les reçoit dans l'ordre 1, 3, 5, 2, 4 ; le rattrapeur les remet dans l'ordre et n'appelle le journal qu'après 200 ms. À droite : l'enregistrement j 50, attente armée, dépend de la valeur s 100 ; arrivé avant elle au secours, il attend qu'elle soit appliquée.",
    d.texte(), 250,
    "Le courtier tient un journal par filtre et les livre entrelacés : tout abonné qui lit plusieurs filtres remet l'ordre par s."))

# ================================================================ 14. Plusieurs stockages
d = Dessin("stockages")
cols = [(200, "Actif", VERT), (472, "Secours local", VERT), (744, "Secours distant", AMBRE)]
for x, lab, c in cols:
    d.t(x + 120, 30, lab.upper(), 11.5, True, c, "middle")
lignes = [
    ("État courant", "maison.db", [("écrit depuis ses sorties", VERT), ("écrit depuis les sorties reçues", VERT), ("écrit depuis les sorties reçues", VERT)]),
    ("Récent", "moli.db (D13)", [("opérations, rang seq", VERT), ("copie de reprise : mêmes rangs", VERT), ("copie de lecture : peut dériver", AMBRE)]),
    ("Long terme", "archive Parquet", [("exporte : seul porteur de l'époque", VERT), ("exporte après la bascule", VERT), ("peut héberger l'archive", AMBRE)]),
    ("Qui répond", "aux questions", [("les écrans, les courbes", VERT), ("après la bascule", VERT), ("la maison éteinte : « hier ? »", AMBRE)]),
]
for k, (lab, sous, cases) in enumerate(lignes):
    y = 44 + k * 64
    d.t(16, y + 22, lab, 11.5, True)
    d.t(16, y + 38, sous, 10, coul=GRIS, mono=True)
    for (x, _, _), (txt, c) in zip(cols, cases):
        d.rect(x, y + 6, 240, 40, "#ffffff", c, 6, 1.4)
        d.t(x + 120, y + 31, txt, 10.5, False, ENCRE, "middle")
d.t(16, 312, "L'état courant se refait à l'identique depuis les sorties ; l'historique, non : une copie qui doit reprendre l'export reçoit les opérations de l'actif.", 10.5, coul=GRIS)
ecrire("stockages", figure("stockages", "Plusieurs stockages, et qui fait quoi",
    "Un tableau en trois colonnes, l'actif, le secours local et le secours distant, et quatre rangées : l'état courant, écrit partout depuis les sorties ; le récent, rangé par l'actif avec son rang, copié avec les mêmes rangs par le secours local, dérivé par le distant ; le long terme, exporté par le seul porteur de l'époque ; et qui répond aux questions.",
    d.texte(), 324,
    "Copie de reprise : mêmes opérations, mêmes rangs, elle peut reprendre l'export. Copie de lecture : elle répond aux questions, elle n'exporte jamais."))

# ================================================================ 15. Les familles d'intégrations
d = Dessin("familles")
d.titre(16, 26, "Ce que fait le secours, intégration par intégration")
qs = [
    (48, "Le secret change-t-il à chaque renouvellement ?", "A · un seul renouveleur ; copie scellée au secours", "Netatmo", VERT),
    (104, "Une session par identifiant et mot de passe ?", "B · connexion après la bascule, jamais avant", "Ariston, Voltalis, Landroid, Overkiz cloud, Reolink, Twinkly, routeurs", VERT),
    (160, "Une clé fixe, un appairage, une empreinte ?", "C · copier le coffre ; sinon réappairage humain", "Hue, Samsung, Android TV, Cast, Philips, Nuki, ESPHome, Tuya…", VERT),
    (236, "Un identifiant MQTT fixe ?", "D · rien ne démarre en mode passif", "Bambu, Frigate, Zigbee2MQTT", AMBRE),
    (292, "Des effets visibles (envois, appuis) ?", "E · un seul actif, jamais deux", "Telegram, notifications, récap de 21 h, Toggle", AMBRE),
    (348, "Liée à l'adresse de la machine ?", "F · adresse virtuelle, ou réglage par machine", "Sonos, ESPHome, Cast, Netatmo, présence", AMBRE),
]
for k, (y, q, r_, ex, c) in enumerate(qs):
    d.rect(16, y, 360, 40, "#ffffff", AMBRE2, 18, 1.6)
    d.t(196, y + 25, q, 11, True, ENCRE, "middle")
    d.ligne(378, y + 20, 438, y + 20, c, 1.8, fleche="v" if c == VERT else "a")
    d.t(408, y + 14, "oui", 10, True, c, "middle", halo=True)
    d.rect(442, y, 542, 40, "#ffffff", c, 6, 1.4)
    d.t(456, y + 17, r_, 11, True, ENCRE)
    d.t(456, y + 32, ex, 9.5, coul=GRIS)
    if k in (0, 1):
        d.ligne(196, y + 40, 196, y + 54, GRIS, 1.4, fleche="g")
        d.t(204, y + 52, "non", 9.5, coul=GRIS)
d.t(16, 222, "Et, pour toutes, trois questions de plus :", 11, True, GRIS)
d.rect(16, 404, 968, 30, DOUX, LIGNE, 6)
d.t(30, 424, "G · sinon, rien à faire : météo, Open-Meteo, Tempo, Wyoming, IGD, IPP, profils sans secret.", 10.5, coul=ENCRE)
ecrire("familles", figure("familles", "Les intégrations : que fait le secours ?",
    "Un arbre de questions. Le secret change-t-il à chaque renouvellement ? Oui : un seul renouveleur (Netatmo). Sinon, une session par identifiant ? Oui : connexion après la bascule. Sinon, une clé fixe ou un appairage ? Oui : copier le coffre. Puis trois questions pour toutes : identifiant MQTT fixe, effets visibles, lien à l'adresse de la machine. Sinon, rien à faire.",
    d.texte(), 446,
    "Une seule instance active par intégration, toujours ; le secours est prêt (secrets, matériel de session), sans aucun appel avant la bascule."))

# ================================================================ 16. Le mode passif
d = Dessin("passif")
d.t(470, 30, "ACTIF", 11.5, True, VERT, "middle")
d.t(817, 30, "SECOURS (PASSIF)", 11.5, True, AMBRE, "middle")
rangs = [
    ("Hub : valeurs, santé", "écrit, date, numérote", VERT, "applique les sorties reçues", AMBRE),
    ("Courtier", "modules, écrans, appareils", VERT, "le pont y publie", AMBRE),
    ("Journal", "écrit", VERT, "copie, écrite par le pont", AMBRE),
    ("Base (maison.db, moli.db)", "écrit, exporte", VERT, "copie ; n'exporte pas", AMBRE),
    ("Moteur d'automatismes", "agit", VERT, "applique le journal, n'agit pas", AMBRE),
    ("Moteur du temps", "fait partir les heures", VERT, "tient ses fiches, rien ne part", AMBRE),
    ("Pilotes, ponts radio", "en marche", VERT, "arrêtés", GRIS),
    ("Assistant, notifications", "en marche", VERT, "arrêtés", GRIS),
    ("API, écrans", "lecture et écriture", VERT, "lecture seule, « secours »", AMBRE),
]
for k, (lab, a, ca, b, cb) in enumerate(rangs):
    y = 44 + k * 32
    d.t(16, y + 18, lab, 11, True)
    for x, txt, c in ((320, a, ca), (667, b, cb)):
        fond = {VERT: "#e6eee5", AMBRE: "#fbf0d6", GRIS: "#eef0ec"}[c]
        d.rect(x, y + 2, 300, 24, fond, c, 12, 1.2)
        d.t(x + 150, y + 18, txt, 10.5, c != GRIS, ENCRE if c != GRIS else GRIS, "middle")
y = 44 + len(rangs) * 32 + 6
d.rect(16, y, 968, 34, DOUX, LIGNE, 6)
d.t(30, y + 22, "La seule porte entre les deux : l'époque. Un verrou de fichier empêche deux instances sur une même machine.", 11, True, ENCRE)
ecrire("passif", figure("passif", "Le mode passif, composant par composant",
    "Deux colonnes, actif et secours passif, pour neuf composants : le hub, le courtier, le journal et la base tournent des deux côtés, en copie sur le secours ; le moteur d'automatismes et le moteur du temps tiennent leur état sans rien faire partir ; les pilotes, les ponts radio, l'assistant et les notifications sont arrêtés ; l'API est en lecture seule. L'époque est la seule porte entre les deux modes.",
    d.texte(), y + 46,
    "Le code d'aujourd'hui n'a pas ce mode : toute instance lancée démarre ses pilotes et son moteur. C'est la première marche du plan (§ 15)."))

# ================================================================ 17. Le coffre scellé
d = Dessin("coffre")
d.titre(16, 26, "Un secret écrit sur l'actif, et où il va")
d.rect(16, 50, 300, 178, DOUX, VERT, 8, 1.4)
d.t(30, 72, "Coffre de l'actif (clé maître M)", 11.5, True)
for k, (sec, dom, c) in enumerate((("netatmo/oauth_tokens · v12", "cloud", AMBRE), ("hue/app_key", "maison", VERT))):
    y = 88 + k * 64
    d.rect(30, y, 272, 48, "#ffffff", c, 6, 1.4)
    d.t(42, y + 20, sec, 10.5, True, ENCRE, mono=True)
    d.t(42, y + 37, f"domaine : {dom}", 10, coul=GRIS)
d.t(30, 214, "écrit d'abord ici (fsync)", 10, coul=GRIS)
d.rect(380, 50, 240, 178, "#ffffff", LIGNE, 8, 1, "5 4")
d.t(500, 72, "journal de réplication", 11, True, GRIS, "middle")
d.t(500, 100, "chaque secret, scellé", 10.5, coul=ENCRE, ancre="middle")
d.t(500, 116, "pour chaque secours", 10.5, coul=ENCRE, ancre="middle")
d.t(500, 132, "qui y a droit", 10.5, coul=ENCRE, ancre="middle")
d.t(500, 160, "ChaCha20-Poly1305", 10, coul=GRIS, ancre="middle", mono=True)
d.t(500, 178, f"≈ {fr(r('jetons', 1, 'sceller_2kio'), 1)} µs pour 2 Kio", 10, coul=GRIS, ancre="middle")
d.boite(684, 50, 300, 76, "Secours local", ["reçoit tous les secrets", "les applique en mémoire, puis sur son disque", "clé maître M, installée à l'appairage"], trait=VERT)
d.boite(684, 152, 300, 76, "Secours distant", ["ne reçoit que les secrets cloud", "scellés avec sa clé de liaison", "son coffre, sa propre clé maître"], trait=AMBRE)
d.ligne(316, 112, 378, 112, AMBRE, 1.6, fleche="a")
d.ligne(316, 176, 378, 176, VERT, 1.6, fleche="v")
d.ligne(620, 96, 682, 88, VERT, 1.8, fleche="v")
d.t(651, 84, "les deux", 9.5, True, VERT, "middle", halo=True)
d.ligne(620, 150, 682, 190, AMBRE, 1.8, fleche="a")
d.t(646, 186, "cloud", 9.5, True, AMBRE, "middle", halo=True)
d.t(16, 256, "Un secret « maison » ne quitte jamais la maison ; la clé maître de la maison non plus. Un secret « mémoire seule » passe en direct, jamais sur un disque.", 10.5, coul=GRIS)
d.t(16, 274, "Ordre : coffre de l'actif (fsync) → copie scellée → usage par le pilote. Le secours garde la plus grande version reçue.", 10.5, coul=GRIS)
ecrire("coffre", figure("coffre", "Le coffre répliqué, scellé par destinataire",
    "À gauche, le coffre de l'actif avec deux secrets : les jetons Netatmo, du domaine cloud, et la clé Hue, du domaine maison. Au milieu, le journal de réplication scelle chaque secret pour chaque secours qui y a droit. À droite, le secours local reçoit les deux et partage la clé maître ; le secours distant ne reçoit que le secret cloud, scellé avec sa propre clé.",
    d.texte(), 286,
    "Décidé le 23 octobre : les jetons et les sessions vont au coffre, et en direct au secours ; le distant reçoit les secrets cloud, chiffrés."))

# ================================================================ 18. La reprise après un arrêt
d = Dessin("reprise")
d.rect(400, 28, 200, 30, VERT, VERT, 15)
d.t(500, 48, "Démarrage", 11.5, True, "#ffffff", "middle")
d.ligne(500, 58, 500, 78, GRIS, 1.4, fleche="g")
d.rect(260, 80, 480, 42, "#ffffff", AMBRE, 21, 1.6)
d.t(500, 106, "Le journal finit-il par « arrêt propre à (N, M) » ?", 11.5, True, ENCRE, "middle")
d.ligne(260, 101, 180, 101, VERT, 1.6)
d.ligne(180, 101, 180, 140, VERT, 1.6, fleche="v")
d.t(220, 95, "oui", 10, True, VERT, "middle")
d.ligne(740, 101, 820, 101, AMBRE, 1.6)
d.ligne(820, 101, 820, 140, AMBRE, 1.6, fleche="a")
d.t(780, 95, "non", 10, True, AMBRE, "middle")
d.boite(16, 142, 330, 52, "Charger l'instantané", ["la mémoire au rang (N, M), rien à rejouer"], trait=VERT)
d.boite(654, 142, 330, 52, "Dernier instantané périodique", ["puis rejouer le journal après lui"], trait=AMBRE)
d.chemin("M 180 194 C 180 214 500 204 500 222", GRIS, 1.4, fleche="g")
d.chemin("M 820 194 C 820 214 500 204 500 222", GRIS, 1.4, fleche="g")
etapes = [
    ("Grâce", "chaque appareil reçoit un délai complet : personne n'est muet pour rien"),
    ("Exécutions", "à l'heure : reprises au même nœud ; plus de 2 min de retard : « interrompues »"),
    ("Heures tombées pendant l'arrêt", "2 min de retard au plus : parties ; au-delà : manquées, une seule alerte"),
    ("Rappels d'agenda", "partent tant que le rendez-vous n'a pas commencé"),
    ("Jetons", "encore valables : servis ; expirés : renouvelés, ou une autorisation est demandée"),
]
for k, (a, b) in enumerate(etapes):
    y = 224 + k * 40
    d.rect(150, y, 700, 32, "#ffffff", LIGNE, 6)
    d.t(166, y + 20, a, 11, True)
    d.t(380, y + 20, b, 10.5, coul=GRIS)
    if k < len(etapes) - 1:
        d.ligne(500, y + 32, 500, y + 40, GRIS, 1.2)
ecrire("reprise", figure("reprise", "La reprise après un arrêt, même des jours plus tard",
    "Un organigramme. Au démarrage : si le journal finit par « arrêt propre », on charge l'instantané sans rien rejouer ; sinon, le dernier instantané périodique puis le journal. Ensuite, dans l'ordre : la grâce pour chaque appareil, les exécutions reprises ou interrompues selon la règle des 2 minutes, les heures tombées pendant l'arrêt parties ou manquées, les rappels d'agenda, les jetons.",
    d.texte(), 430,
    "Aucune règle nouvelle : ce sont celles de la mémoire v2 (§ 6.5, § 10.6, § 11). Elles valent pour une minute d'arrêt comme pour une semaine."))

# ================================================================ 19. Le cloud repris par le distant
d = Dessin("cloud")
d.titre(16, 26, "L'époque cloud, d'un geste humain à l'autre")
def etat(x, y, titre, sous, c):
    d.rect(x, y, 300, 70, "#ffffff", c, 14, 1.8)
    d.t(x + 150, y + 30, titre, 12, True, ENCRE, "middle")
    d.t(x + 150, y + 50, sous, 10.5, coul=GRIS, ancre="middle")
etat(16, 50, "La maison porte le cloud", "époque cloud N : le cas normal", VERT)
etat(684, 50, "Le distant porte le cloud", "époque cloud N + 1", AMBRE)
etat(684, 220, "La maison revient", "lit l'époque cloud ; ne relance rien", GRIS)
etat(16, 220, "Rendre la main", "époque cloud N + 2 à la maison", VERT)
d.ligne(318, 78, 682, 78, AMBRE, 2, fleche="a")
d.t(500, 68, "les deux veilles se taisent, puis un geste humain", 10.5, True, AMBRE, "middle", halo=True)
d.ligne(834, 122, 834, 218, GRIS, 1.6, fleche="g")
d.t(842, 176, "la maison redémarre", 10, coul=GRIS, halo=True)
d.ligne(682, 255, 318, 255, VERT, 2, fleche="v")
d.t(500, 245, "geste humain", 10.5, True, VERT, "middle", halo=True)
d.ligne(166, 218, 166, 122, VERT, 1.6, fleche="v")
d.t(16, 150, "une seule veille muette :", 10, coul=GRIS)
d.t(16, 164, "c'est le lien, pas la maison", 10, coul=GRIS)
d.t(16, 178, "→ aucune proposition", 10, coul=GRIS)
d.t(684, 310, "distant injoignable au redémarrage : un humain décide", 10, coul=GRIS)
d.t(16, 316, "Le distant ne touche jamais aux radios ni au réseau local ; il n'appelle un cloud et ne renouvelle un jeton que s'il porte l'époque cloud.", 10.5, coul=GRIS)
ecrire("cloud", figure("cloud", "Le cloud repris par le secours distant",
    "Un automate à quatre états. La maison porte le cloud. Si les deux veilles se taisent, un geste humain donne l'époque cloud suivante au distant. Quand la maison revient, elle lit l'époque cloud chez le distant et ne relance rien. Un geste humain lui rend la main avec l'époque suivante. Une seule veille muette ne déclenche rien.",
    d.texte(), 328,
    "Les deux veilles : le pont par NetBird, et un battement direct en HTTPS. Une seule qui se tait, c'est le lien, pas la maison."))

# ================================================================ 20. L'époque sur chaque ordre
d = Dessin("cloture")
d.titre(16, 22, "Un ordre d'une époque ancienne est refusé par le module lui-même")
X = {"h1": 120, "t": 380, "h2": 640, "m": 890}
for k, lab in (("h1", "H1 (époque N)"), ("t", "Témoin"), ("h2", "H2"), ("m", "Module")):
    d.rect(X[k] - 80, 36, 160, 26, "#ffffff", LIGNE, 5)
    d.t(X[k], 53, lab, 11, True, ENCRE, "middle")
    d.ligne(X[k], 62, X[k], 300, LIGNE, 1, "3 3")
def msg(y, a, b, lab, c=GRIS, gras=False):
    d.ligne(X[a], y, X[b] + (-4 if X[b] > X[a] else 4), y, c, 1.6, fleche={VERT: "v", AMBRE: "a", ROUGE: "r"}.get(c, "g"))
    d.t((X[a] + X[b]) / 2, y - 6, lab, 10, gras, c, "middle", halo=True)
msg(86, "h1", "t", "renouveler le bail (N)")
d.rect(X["h1"] - 7, 100, 14, 112, "#fbe9e2", ROUGE, 3)
d.t(X["h1"] + 12, 160, "gelé", 10, True, ROUGE)
msg(126, "h2", "t", "demander N + 1", AMBRE)
msg(148, "t", "h2", "accordée : le bail N a expiré", VERT, True)
msg(178, "h2", "m", "ordre, époque N + 1", VERT, True)
d.t(X["m"] - 8, 200, "plus grande époque vue : N + 1", 9.5, coul=GRIS, ancre="end")
msg(232, "h1", "m", "ordre, époque N (réveillé)", GRIS)
msg(254, "m", "h1", "refusé : époque ancienne", ROUGE, True)
msg(282, "h1", "t", "renouveler (N) → refusé : H1 redevient secours", GRIS)
ecrire("cloture", figure("cloture", "L'époque sur chaque ordre",
    "Diagramme de séquence. H1, époque N, renouvelle son bail puis gèle. H2 demande l'époque N + 1, que le témoin accorde à l'expiration du bail. H2 envoie un ordre d'époque N + 1 au module. H1 se réveille et envoie un ordre d'époque N : le module le refuse. H1 apprend qu'il a perdu le bail et redevient secours.",
    d.texte(), 312,
    "Le jeton de clôture des systèmes distribués : un module garde la plus grande époque vue et refuse tout ordre plus ancien."))


# ================================================================ 21. Les deux systèmes, en détail
d = Dessin("systemes")
d.titre(16, 26, "Les clients")
GRISF = "#eef0ec"
def systeme(y0, actif):
    c = VERT if actif else AMBRE
    d.rect(236, y0, 748, 352, DOUX, c, 8, 1.6)
    d.t(962, y0 + 20, "H1 · ACTIF : PORTE L'ÉPOQUE" if actif else "H2 · SECOURS LOCAL : PASSIF", 11.5, True, c, "end")
    d.rect(250, y0 + 32, 720, 240, "#ffffff", LIGNE, 8)
    d.t(262, y0 + 50, "Sarah Home : un seul binaire", 10.5, True, GRIS)
    d.t(958, y0 + 50, "liens en mémoire, sans TCP", 10, coul=GRIS, ancre="end")
    rang1 = ([("Hub", ["vérité, garde", "publie val/ (rang s)"]), ("Moteur", ["automatismes,", "moteur du temps"]),
              ("Base", ["maison.db, moli.db", "export (D13)"]), ("WebSocket", ["les écrans"]), ("API, MCP", ["Sarah, outils"]),
              ("Coffre, journal", ["secrets ; journal", "7 jours (s, j)"])] if actif else
             [("Hub", ["applique les sorties", "dans l'ordre de s"]), ("Moteur", ["applique le journal,", "n'agit pas"]),
              ("Base", ["copie de reprise", "n'exporte pas"]), ("WebSocket", ["écrans en lecture", "seule"]), ("API, MCP", ["lecture seule"]),
              ("Coffre, journal", ["copies, écrites", "par le pont"])])
    for k, (t, l) in enumerate(rang1):
        x = 262 + k * 118
        d.ligne(x + 55, y0 + 118, x + 55, y0 + 132, c, 1.6)
        d.boite(x, y0 + 58, 110, 60, t, l, trait=c, taille=11)
    # le courtier, avec ses écouteurs aux deux bouts
    d.rect(262, y0 + 132, 700, 40, "#e6eee5" if actif else "#fbf0d6", c, 6, 1.4)
    d.t(612, y0 + 149, "Courtier MQTT embarqué (rumqttd)" if actif else "Courtier MQTT du secours : seul le pont y publie", 11.5, True, ENCRE, "middle")
    d.t(612, y0 + 164, "in/, req/ → hub ; val/, evt/, avail/, meta/, cmd/ → abonnés" if actif else "les abonnés locaux y lisent, comme sur l'actif", 10, coul=GRIS, ancre="middle")
    if actif:
        d.rect(262, y0 + 132, 94, 40, "#ffffff", VERT, 6, 1.4)
        d.t(309, y0 + 149, "1883 · 8883", 10.5, True, ENCRE, "middle")
        d.t(309, y0 + 164, "local · réseau", 9.5, coul=GRIS, ancre="middle")
        d.rect(868, y0 + 132, 94, 40, "#ffffff", VERT, 6, 1.4)
        d.t(915, y0 + 149, "8885", 10.5, True, ENCRE, "middle")
        d.t(915, y0 + 164, "secours, TLS", 9.5, coul=GRIS, ancre="middle")
        d.boite(360, y0 + 186, 602, 46, "Modules api-to-mqtt du binaire : Hue, Overkiz, Netatmo, Sonos…", ["in/ vers le hub, cmd/ depuis le hub ; ils parlent aux appareils IP et aux clouds"], trait=VERT, taille=11)
        d.t(730, y0 + 254, "seul le hub écrit val/ et cmd/", 10, coul=GRIS)
    else:
        d.rect(262, y0 + 132, 94, 40, GRISF, GRIS, 6, 1, "4 3")
        d.t(309, y0 + 149, "1883 · 8883", 10.5, True, GRIS, "middle")
        d.t(309, y0 + 164, "sans client", 9.5, coul=GRIS, ancre="middle")
        d.rect(868, y0 + 132, 94, 40, "#ffffff", AMBRE, 6, 1.4)
        d.t(915, y0 + 149, "pont", 10.5, True, ENCRE, "middle")
        d.t(915, y0 + 164, "seul éditeur", 9.5, coul=GRIS, ancre="middle")
        d.rect(360, y0 + 186, 402, 46, GRISF, GRIS, 6, 1, "4 3")
        d.t(372, y0 + 205, "Modules api-to-mqtt : arrêtés", 11, True, GRIS)
        d.t(372, y0 + 222, "ils démarrent à la prise de l'époque", 10, coul=GRIS)
        d.boite(774, y0 + 186, 188, 46, "Pont à reprise", ["client de H1:8885, TLS"], trait=VERT, taille=11)
        d.ligne(868, y0 + 186, 868, y0 + 174, VERT, 1.8, fleche="v")
        d.t(360, y0 + 254, "Les abonnés comblent leurs trous par la copie du journal ; rien ne part vers un appareil.", 10, coul=GRIS)
    # sous le binaire : les ponts radio et les modules à part
    for x, w, t, l in ((262, 158, "Zigbee2MQTT", "1883 ; radio A par TCP"), (432, 158, "Modules à part", "clients de 1883"), (602, 198, "Contrôleur Matter", "matter.js, WebSocket")):
        if actif:
            d.boite(x, y0 + 282, w, 52, t, [l], trait=VERT, taille=11)
        else:
            d.rect(x, y0 + 282, w, 52, GRISF, GRIS, 6, 1, "4 3")
            d.t(x + 12, y0 + 301, t, 11, True, GRIS)
            d.t(x + 12, y0 + 318, "arrêté", 10, coul=GRIS)
    d.t(820, y0 + 304, "un seul écrivain par radio :" if actif else "démarrés après la clôture", 10, coul=GRIS)
    d.t(820, y0 + 320, "attachés à l'époque (V7)" if actif else "de l'ancien actif (V7)", 10, coul=GRIS)
    if actif:
        # Zigbee2MQTT et les modules à part vers l'écouteur local ; Matter vers son module
        d.ligne(300, y0 + 282, 300, y0 + 174, VERT, 1.5, fleche="v")
        d.chemin(f"M 511 {y0 + 282} L 511 {y0 + 277} L 330 {y0 + 277} L 330 {y0 + 174}", VERT, 1.5, fleche="v")
        d.ligne(701, y0 + 282, 701, y0 + 234, VERT, 1.5, fleche="v")

systeme(36, True)
systeme(416, False)
# Les clients, à gauche : ils parlent à l'adresse virtuelle, portée par l'actif
d.boite(16, 52, 200, 52, "Écrans", ["navigateur, appli"], trait=VERT)
d.boite(16, 122, 200, 52, "Appareils MQTT du réseau", ["Tasmota, Shelly · MQTT TLS"], trait=VERT)
d.boite(16, 262, 200, 52, "Boîtier radio A", ["Zigbee actif · Thread actif"], trait=VERT)
d.boite(16, 470, 200, 52, "Boîtier radio B", ["Zigbee secours · Thread actif"], trait=AMBRE)
d.boite(16, 590, 200, 52, "Témoin", ["vote le bail de l'époque"])
d.t(16, 200, "Adresse virtuelle (VRRP) :", 10, True, GRIS)
d.t(16, 215, "écrans et appareils parlent", 10, coul=GRIS)
d.t(16, 230, "à celle de l'actif", 10, coul=GRIS)
d.chemin("M 216 78 L 228 78 L 228 62 L 671 62 L 671 92", VERT, 1.5, fleche="v")
d.t(450, 57, "WebSocket, par l'adresse virtuelle", 10, True, VERT, "middle", halo=True)
d.chemin("M 216 148 L 230 148 L 230 188 L 260 188", VERT, 1.5, fleche="v")
d.chemin("M 216 288 L 232 288 L 232 344 L 260 344", VERT, 1.5, fleche="v")
d.chemin("M 216 496 L 232 496 L 232 724 L 260 724", GRIS, 1.3, "4 3", fleche="g")
d.chemin("M 216 606 L 226 606 L 226 380 L 236 380", GRIS, 1.2, "3 3")
d.ligne(216, 626, 236, 626, GRIS, 1.2, "3 3")
# Le pont : de l'écouteur 8885 de l'actif au pont du secours, par l'extérieur des boîtes
d.chemin("M 962 188 L 992 188 L 992 625 L 964 625", VERT, 2.4, fleche="v")
d.t(984, 405, "pont à reprise : sorties et journal (8885, TLS)", 11, True, VERT, "end", halo=True)
d.t(16, 790, "À la bascule, H2 prend l'époque et l'adresse virtuelle, démarre Zigbee2MQTT, Matter et ses modules ; les clients se reconnectent à la même adresse ; le pont s'arrête.", 10.5, coul=GRIS)
ecrire("systemes", figure("systemes", "Les deux systèmes, en détail : courtiers, cœur et clients",
    "Deux machines l'une sous l'autre. H1, actif : dans un seul binaire, le hub, le moteur, la base, le module WebSocket, l'API, le coffre et le journal, reliés en mémoire au courtier MQTT embarqué, qui écoute en 1883 (local), 8883 (réseau, TLS) et 8885 (secours, TLS) ; les modules api-to-mqtt parlent aux appareils et aux clouds ; Zigbee2MQTT et les modules à part sont clients de 1883 ; le contrôleur Matter parle à son module. Les écrans et les appareils MQTT du réseau viennent par l'adresse virtuelle de l'actif ; le boîtier radio A parle à Zigbee2MQTT. H2, passif : les mêmes composants en copie, son courtier sans client, ses modules et ses ponts radio arrêtés ; le pont à reprise, client de H1:8885, est le seul éditeur de son courtier. Le témoin vote le bail des deux.",
    d.texte(), 802,
    "Un seul binaire par machine ; un courtier par machine ; les clients ne parlent qu'à l'actif, par l'adresse virtuelle. Le secours ne reçoit que le pont."))

print(f"ok : {len(list(F.glob('*.svg.html')))} figures dans {F}")
