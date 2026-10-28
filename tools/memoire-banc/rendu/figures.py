#!/usr/bin/env python3
"""Schémas et graphiques de l'étude mémoire (docs/proposition/memoire.md), dans la charte de la V7.
Les graphiques lisent les sorties du banc Linux (../sorties/linux/*.txt) : aucun chiffre recopié.
Écrit les figures dans rendu/fig/ ; build.py les insère dans la page.

Usage : python3 -I tools/memoire-banc/rendu/figures.py [dossier des sorties] [dossier des figures]
(par défaut : la série courante ../sorties/linux, et rendu/fig ; la série de la v1 est dans
../sorties/v1/linux)."""
import glob, math, pathlib, re

import sys

ICI = pathlib.Path(__file__).resolve().parent
SORTIES = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else ICI.parent / "sorties/linux"
F = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else ICI / "fig"
F.mkdir(exist_ok=True, parents=True)
VERT, VERT2, AMBRE, AMBRE2, ROUGE, ENCRE, GRIS, LIGNE, FOND, DOUX = (
    "#195744", "#7fa892", "#b07d12", "#e3c77f", "#8b3b22", "#1d302a", "#52615a", "#d4ddd4", "#fffdf8", "#f1f4ee")
FONT = 'font-family="Avenir Next, Avenir, Segoe UI, sans-serif"'
MONO = 'font-family="SFMono-Regular, Consolas, monospace"'

# ---------------------------------------------------------------- résultats du banc
RES = {}
for f in sorted(glob.glob(str(SORTIES / "*.txt"))):
    vitesse = "vitesse" in f
    for l in open(f, encoding="utf-8"):
        if l.startswith("RESULTAT|"):
            _, sec, e, cle, v, _u = l.strip().split("|")
            RES[(sec + ("-vitesse" if vitesse else ""), int(e), cle)] = float(v)


def r(sec, e, cle):
    k = (sec, e, cle)
    if k not in RES:
        raise SystemExit(f"résultat absent : {k}")
    return RES[k]


# ---------------------------------------------------------------- aides
def esc(s):
    return str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def fr(x, d=1):
    return f"{x:.{d}f}".replace(".", ",")


def nb(v):
    return f"{v:,.0f}".replace(",", " ")


def court(v, unite=""):
    """Un nombre lisible : 0,38 · 5,1 · 92 · 1 470."""
    if v >= 100:
        s = nb(v)
    elif v >= 10:
        s = fr(v, 0)
    elif v >= 1:
        s = fr(v, 1)
    else:
        s = fr(v, 2)
    return s + ((" " + unite) if unite else "")


def figure(id_, titre, desc, corps, h, legende, fleches=True):
    defs = ""
    if fleches:
        defs = "\n".join(
            f'            <marker id="{id_}-{k}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="{c}"/></marker>'
            for k, c in (("v", VERT), ("a", AMBRE), ("g", GRIS), ("r", ROUGE)))
        defs = f"          <defs>\n{defs}\n          </defs>\n"
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

    def texte(self):
        return "\n".join(self.c)


def ecrire(nom, html):
    (F / f"{nom}.svg.html").write_text(html, encoding="utf-8")


# ================================================================ 1. Le numéro est l'adresse
d = Dessin("principe")
d.titre(16, 26, "Au bord : l'annuaire")
noms = [("z2m:0x00158d00a1b2c3d4/temperature", 1234), ("z2m:0x00158d00a1b2c3d4/humidity", 1235),
        ("overkiz:io:__0812-4471-0007_1593…/core:ClosureState", 1236)]
for i, (n, k) in enumerate(noms):
    y = 44 + i * 44
    d.rect(16, y, 290, 30, "#ffffff")
    d.t(26, y + 19.5, n, 9.8, mono=True)
    d.ligne(310, y + 15, 344, y + 15, VERT, 1.5, fleche="v")
    d.rect(350, y + 2, 54, 26, VERT, VERT, 13)
    d.t(377, y + 19.5, str(k), 12, True, "#ffffff", "middle")
d.t(16, 192, "Un nom devient un numéro une seule fois, quand le message entre.", 10.5, coul=GRIS)
d.t(16, 207, "Humains, profils, fichiers et sujets MQTT gardent les noms.", 10.5, coul=GRIS)
d.ligne(420, 102, 470, 102, VERT, 2.5, fleche="v")
d.t(445, 90, "4 octets", 10.5, True, VERT, "middle")
d.titre(486, 26, "Dedans : des colonnes, une place par numéro")
cols = [("Def", 16, 52), ("Vif", 40, 150), ("Échéance", 4, 34)]
x = 540
rangs = [1232, 1233, 1234, 1235, 1236]
y0, hr = 44, 30
for nom, oct_, w in cols:
    d.t(x + w / 2, y0 - 4, f"{nom} · {oct_} o", 10.5, True, ENCRE, "middle")
    for i, n in enumerate(rangs):
        y = y0 + 6 + i * hr
        fond = VERT if n == 1234 else "#ffffff"
        d.rect(x, y, w, hr - 4, fond, VERT if n == 1234 else LIGNE, 3)
        if nom == "Vif":
            d.t(x + w / 2, y + 17, "valeur · instants · qualité" if n == 1234 else "…", 9.2,
                coul="#ffffff" if n == 1234 else GRIS, ancre="middle")
    x += w + 14
for i, n in enumerate(rangs):
    d.t(530, y0 + 6 + i * hr + 17, nb(n), 10.5, n == 1234, VERT if n == 1234 else GRIS, "end")
d.t(486, 250, "adresse de la fiche Vif = début + 1234 × 40 = début + 49 360", 11.5, True, VERT)
d.t(486, 268, "Le même numéro ouvre toutes les colonnes : aucune adresse ne voyage, elle se calcule.", 10.5, coul=GRIS)
# appareil
d.rect(850, 44, 134, 150, "#ffffff")
d.t(917, 64, "Santé · 20 o", 10.5, True, ENCRE, "middle")
d.t(917, 80, "par appareil", 9.5, coul=GRIS, ancre="middle")
for i, n in enumerate([270, 271, 272]):
    y = 92 + i * 32
    d.rect(866, y, 102, 26, VERT if n == 271 else DOUX, VERT if n == 271 else LIGNE, 3)
    d.t(917, y + 17, f"appareil {n}", 10, n == 271, "#ffffff" if n == 271 else GRIS, "middle")
d.chemin("M 566,136 L 566,218 L 917,218 L 917,200", AMBRE, 1.5, "5 4", "a")
d.t(742, 232, "Def[1234].appareil = 271 : la santé de son appareil", 9.8, True, AMBRE, "middle", halo=True)
ecrire("principe", figure("principe", "Le numéro est l'adresse",
    "À l'entrée, l'annuaire traduit une fois le nom d'un point en numéro. Dedans, chaque sorte de fiche est une colonne rangée par numéro : la fiche 1234 se trouve par un calcul, début plus 1234 fois la taille de la fiche. La définition du point donne le numéro de son appareil, qui ouvre la colonne de santé.",
    d.texte(), 282,
    "Le principe : le numéro d'un point est sa place dans chaque colonne. L'annuaire ne sert qu'au bord ; à l'intérieur, une adresse se calcule et n'est jamais transmise."))

# ================================================================ 2. Lignes de cache
d = Dessin("lignes")
x0, k = 40, 3.55  # px par octet
d.titre(16, 24, "Quatre lignes de cache de 64 octets")
for i in range(5):
    x = x0 + i * 64 * k
    d.ligne(x, 40, x, 214, GRIS if i in (0, 4) else AMBRE, 1, None if i in (0, 4) else "4 3")
for i in range(4):
    d.t(x0 + (i + 0.5) * 64 * k, 52, f"ligne {i + 1} · 64 o", 10, True, GRIS, "middle")
rangs = [("Vif · 40 o", 40, 74), ("Vif32 · 32 o", 32, 124), ("Chaud · 16 o", 16, 174)]
for lib, t, y in rangs:
    d.t(16, y - 8, lib, 11, True)
    o = 0
    while o + t <= 256:
        a_cheval = (o // 64) != ((o + t - 1) // 64)
        d.rect(x0 + o * k + 1, y, t * k - 2, 26, AMBRE2 if a_cheval else "#e6eee5", AMBRE if a_cheval else VERT2, 3)
        if t >= 32:
            d.t(x0 + (o + t / 2) * k, y + 17, "à cheval" if a_cheval else "fiche", 9.5, a_cheval, ROUGE if a_cheval else VERT, "middle")
        o += t
d.t(16, 234, "À 40 octets, une fiche sur deux est à cheval sur deux lignes : la lire peut coûter deux accès à la mémoire au lieu d'un. À 32 et 16 octets, jamais.", 10.5, coul=GRIS)
d.t(16, 250, "Tant que la colonne tient dans le cache (×1 : 88 Kio), l'effet est nul ; sur un Pi Zero 2 W (512 Kio de cache L2), ×10 (884 Kio) n'y tient plus.", 10.5, coul=GRIS)
ecrire("lignes", figure("lignes", "Les fiches et les lignes de cache",
    "Le processeur lit la mémoire par lignes de 64 octets. Des fiches de 40 octets, rangées bout à bout, chevauchent une frontière de ligne une fois sur deux ; des fiches de 32 ou 16 octets n'en chevauchent jamais.",
    d.texte(), 262,
    "Pourquoi la taille de fiche compte un peu : le processeur lit par lignes de 64 octets. Ambre : une fiche à cheval sur deux lignes.", fleches=False))

# ================================================================ 3. La vie d'un numéro
d = Dessin("cycle")
etapes = [("Un module déclare", "un point (kit : declarer)"), ("Le hub donne", "le numéro suivant, au bout"),
          ("Il l'écrit", "dans l'annuaire, avant usage"), ("Le numéro sert", "partout dans le binaire"),
          ("Point retiré :", "la case reste, condamnée")]
xs = [16, 214, 412, 610, 808]
for i, ((a, b), x) in enumerate(zip(etapes, xs)):
    fond = "#f6e4dc" if i == 4 else ("#e6eee5" if i in (2, 3) else DOUX)
    d.rect(x, 40, 176, 64, fond, ROUGE if i == 4 else (VERT if i in (2, 3) else LIGNE), 10)
    d.t(x + 88, 66, a, 12.5, True, ROUGE if i == 4 else ENCRE, "middle")
    d.t(x + 88, 86, b, 10.5, coul=GRIS, ancre="middle")
    if i < 4:
        d.ligne(x + 179, 72, x + 195, 72, VERT, 1.8, fleche="v")
d.t(102, 124, "« z2m:0x00…/temperature »", 9.8, coul=GRIS, ancre="middle", mono=True)
d.t(302, 124, "n = points déjà connus : 1234", 9.8, coul=GRIS, ancre="middle")
d.t(500, 124, "maison.db, puis fsync", 9.8, coul=GRIS, ancre="middle")
d.t(698, 124, "colonnes, kit, programme, journal", 9.8, coul=GRIS, ancre="middle")
d.t(896, 124, "jamais redonné : pas de génération", 9.8, coul=ROUGE, ancre="middle")
d.chemin("M 500,150 L 500,190 L 698,190 L 698,152", VERT, 1.8, "6 4", "v")
d.rect(520, 176, 160, 28, FOND, LIGNE, 6)
d.t(600, 195, "redémarrage", 11, True, VERT, "middle")
d.t(16, 232, "Au redémarrage, l'annuaire relu remet chaque nom à son numéro : les colonnes se reconstruisent dans le même ordre, et le journal de reprise,", 10.5, coul=GRIS)
d.t(16, 248, "qui cite les points par numéro, se rejoue tel quel. La machine de secours rejoue le même journal : elle a les mêmes numéros.", 10.5, coul=GRIS)
ecrire("cycle", figure("cycle", "La vie d'un numéro",
    "Un module déclare un point ; le hub lui donne le numéro suivant ; il l'écrit dans l'annuaire persistant avant de s'en servir ; le numéro sert partout dans le binaire ; un point retiré garde sa case, et son numéro n'est jamais redonné. Au redémarrage, l'annuaire relu remet chaque nom à son numéro.",
    d.texte(), 262,
    "Un numéro est donné une fois, écrit avant usage, jamais redonné. C'est ce qui permet au journal de reprise et à la machine de secours de parler de « la valeur 1234 » sans ambiguïté."))

# ================================================================ 4. Trouver ce qui a expiré
d = Dessin("echeances")
d.titre(16, 24, "Une colonne d'échéances, un passage")
vals = [("app 0", "1 380"), ("app 1", "jamais"), ("app 2", "1 212"), ("app 3", "1 205"), ("app 4", "1 260"),
        ("app 5", "jamais"), ("app 6", "1 500"), ("app 7", "1 230")]
for i, (a, v) in enumerate(vals):
    y = 40 + i * 30
    expire = v == "1 205"
    proche = v == "1 212"
    d.t(70, y + 18, a, 10.5, coul=GRIS, ancre="end")
    d.rect(80, y, 110, 25, "#f6e4dc" if expire else ("#e6eee5" if proche else "#ffffff"), ROUGE if expire else (VERT if proche else LIGNE), 3)
    d.t(135, y + 17, v, 11, expire or proche, ROUGE if expire else (VERT if proche else ENCRE), "middle")
d.ligne(214, 44, 214, 270, AMBRE, 2.5, fleche="a")
d.t(226, 60, "le passage lit la colonne", 10.5, True, AMBRE)
d.t(226, 76, "de haut en bas : 4 o par case", 10.5, coul=GRIS)
d.t(226, 134, "maintenant = 1 210", 11, True, ENCRE)
d.t(226, 150, "1 205 < 1 210 → app 3 muet", 10.5, True, ROUGE)
d.t(226, 188, "au passage, le plus petit reste :", 10.5, coul=GRIS)
d.t(226, 204, "1 212 → dormir jusque-là", 10.5, True, VERT)
d.t(16, 300, "Une nouvelle de l'appareil : échéance = maintenant + délai.", 10.5, coul=GRIS)
d.t(16, 316, "Une addition, en place ; et si elle tombe avant le réveil, elle l'avance.", 10.5, coul=GRIS)
d.ligne(500, 30, 500, 330, LIGNE, 1)
d.titre(524, 24, "La file de priorité (écartée)")
noeuds = [(740, 60, "1 205 · app 3 · g7", True, False), (640, 130, "1 207 · app 9 · g2", False, True),
          (840, 130, "1 212 · app 2 · g4", False, False), (566, 200, "1 230 · app 7 · g1", False, False),
          (712, 200, "1 248 · app 2 · g3", False, True), (858, 200, "1 260 · app 4 · g5", False, False)]
for (x, y, s, top, perime) in noeuds:
    pass
liens = [((740, 60), (640, 130)), ((740, 60), (840, 130)), ((640, 130), (566, 200)), ((640, 130), (712, 200)), ((840, 130), (858, 200))]
for (a, b) in liens:
    d.ligne(a[0], a[1] + 13, b[0], b[1] - 13, LIGNE, 1.5)
for (x, y, s, top, perime) in noeuds:
    d.rect(x - 62, y - 13, 124, 26, "#f6e4dc" if top else ("#eeeeea" if perime else "#ffffff"), ROUGE if top else LIGNE, 13)
    d.t(x, y + 4, s, 9.6, top, GRIS if perime else (ROUGE if top else ENCRE), "middle")
d.t(524, 250, "Chaque nouvelle empile une entrée ; l'ancienne reste jusqu'à son échéance,", 10.5, coul=GRIS)
d.t(524, 266, "puis la génération (g) dit qu'elle ne vaut plus rien (en gris).", 10.5, coul=GRIS)
d.t(524, 290, "Le prochain réveil est en haut, sans passage ; mais le tas grossit avec", 10.5, coul=GRIS)
d.t(524, 306, "le débit des nouvelles (1 428 entrées pour 496 appareils à ×1), et chaque", 10.5, coul=GRIS)
d.t(524, 322, "nouvelle coûte un tri partiel. À l'échelle d'une maison, rien ne le justifie.", 10.5, coul=GRIS)
ecrire("echeances", figure("echeances", "Trouver ce qui a expiré",
    "À gauche, la colonne d'échéances : le passage la lit de haut en bas, déclare muet l'appareil dont l'échéance est passée, et retient la plus petite échéance restante pour dormir jusque-là. À droite, la file de priorité : la prochaine échéance est en haut, mais chaque nouvelle empile une entrée et les anciennes restent jusqu'à leur terme.",
    d.texte(), 338,
    "Le numéro dit où est une fiche, pas laquelle a expiré. Pour le savoir, on lit la colonne d'échéances (à gauche) ; une file de priorité (à droite) l'éviterait, au prix d'une structure de plus à tenir à jour."))

# ================================================================ 5. Trois mémoires
d = Dessin("trois")
boites = [(16, "Les valeurs", "ce que la maison est", "le hub",
           ["Vif · 40 o par point", "Santé · 20 o par appareil", "échéances · 4 o", "annuaire : nom ↔ numéro"],
           "journal de reprise + instantané", VERT),
          (352, "Le programme", "ce que les workflows sont", "le moteur",
           ["plans · 12 o", "nœuds · 32 o, liens · 8 o", "règles · 16 o, textes", "index par point"],
           "figé ; recompilé à chaque validation", AMBRE),
          (688, "Les exécutions", "ce que les workflows font", "le moteur",
           ["exécutions · 64 o, avec leur cause", "attentes des déclencheurs · 16 o", "vérité des conditions composées", "textes produits, baux « pendant »"],
           "journal du moteur ; reprises si à l'heure", ROUGE)]
for x, t1, t2, qui, items, persiste, coul in boites:
    d.rect(x, 30, 296, 214, "#ffffff", coul, 12, 1.6)
    d.t(x + 16, 56, t1, 15, True, coul)
    d.t(x + 16, 74, t2, 11, coul=GRIS, italique=True)
    d.t(x + 280, 56, qui, 10.5, coul=GRIS, ancre="end")
    for i, it in enumerate(items):
        d.rect(x + 16, 88 + i * 30, 264, 24, DOUX, LIGNE, 4)
        d.t(x + 26, 104 + i * 30, it, 10.8)
    d.t(x + 16, 230, persiste, 10, True, coul)
d.chemin("M 352,140 C 330,140 330,140 314,140", AMBRE, 1.8, None, "a")
d.t(332, 288, "", 1)
d.chemin("M 314,184 C 330,184 336,184 350,184", VERT, 1.8, None, "v")
d.chemin("M 688,140 L 650,140", ROUGE, 1.8, None, "r")
d.chemin("M 836,246 L 836,264 L 168,264 L 168,250", ROUGE, 1.6, "5 4", "r")
d.t(502, 258, "une attente relit l'instant « changé » d'un point", 9.8, True, ROUGE, "middle", halo=True)
d.t(16, 292, "Ambre : une règle lit une valeur par son numéro de point.   Vert : un changement du point n réveille index[n].", 10.5, coul=GRIS)
d.t(16, 308, "Rouge : une exécution pointe un nœud par son numéro (curseur), et relit au besoin l'instant d'une valeur.", 10.5, coul=GRIS)
d.t(16, 338, "Aucune mémoire ne copie une autre : elles se citent par numéro, et chacune a sa durée de vie.", 11.5, True, ENCRE)
ecrire("trois", figure("trois", "Trois mémoires, pas une",
    "La mémoire des valeurs, tenue par le hub ; le programme des workflows, compilé et figé ; les exécutions en cours, avec leurs attentes. Les règles du programme lisent les valeurs par numéro ; un changement de valeur réveille les déclencheurs de son point par l'index ; une exécution pointe un nœud du programme par son numéro.",
    d.texte(), 352,
    "La mémoire des workflows est à part de celle des valeurs, et se divise elle-même en deux : ce que les workflows sont (le programme) et ce qu'ils font (les exécutions)."))

# ================================================================ 6. Un automatisme compilé
d = Dessin("programme")
d.titre(16, 24, "Sur disque : la définition, ≈ 2 500 octets")
d.rect(16, 36, 420, 268, "#ffffff")
js = ['{"id": "couloir-7", "name": "Couloir 7 : lumière…",', ' "graph": {"nodes": [',
      '  {"id": "declencheur", "type": "when_state",', '   "point": "z2m:…/occupancy", "to": true},',
      '  {"id": "condition", "type": "if", "rules": [3 règles]},', '  {"id": "allumer", "type": "set",',
      '   "point": "hue:…/on", "value": true},', '  {"id": "attendre", "type": "wait", "seconds": 300},',
      '  {"id": "eteindre", …}, {"id": "prevenir", …}],', ' "edges": [5 liens par nom]},',
      ' "approved_version": {le même graphe, recopié},', ' "approved": "4f1c…", "activation": "0b7d…"}']
for i, l in enumerate(js):
    retrait = len(l) - len(l.lstrip(" "))
    d.t(28, 58 + i * 20, "\u00a0" * retrait + l.lstrip(" "), 9.6, mono=True, coul=ENCRE)
auj_w1 = (r("workflows", 1, "memoire_definitions_aujourdhui") + r("workflows", 1, "memoire_copie_live_aujourdhui")) / 400
prog_w1 = r("workflows", 1, "memoire_programme")
d.t(16, 324, f"En mémoire aujourd'hui : ≈ {fr(auj_w1 / 1024, 0)} Kio par automatisme", 10.5, True, ROUGE)
d.t(16, 340, "(la liste, plus la copie des actifs ; noms et nœuds en texte).", 10.5, coul=GRIS)
d.titre(470, 24, "En mémoire : des tables de fiches")
rangs = [("1200", "QUAND_ÉTAT", "point 917 · vers 1"), ("1201", "SI", "règles 600–602 · toutes"),
         ("1202", "RÉGLER", "point 233 ← 1"), ("1203", "ATTENDRE", "300 s"), ("1204", "RÉGLER", "point 233 ← 0"),
         ("1205", "PRÉVENIR", "texte 200")]
d.t(470, 50, "nœuds · 32 o", 10.5, True)
for i, (n, op_, arg) in enumerate(rangs):
    y = 58 + i * 26
    d.t(508, y + 16, n, 10, True, AMBRE, "end", mono=True)
    d.rect(514, y, 270, 22, "#fbf0d6" if i == 0 else "#ffffff", AMBRE if i == 0 else LIGNE, 3)
    d.t(522, y + 15, op_, 9.8, True, mono=True)
    d.t(620, y + 15, arg, 9.8, coul=GRIS)
d.t(800, 50, "liens · 8 o", 10.5, True)
for i, s in enumerate(["sortie → 1201", "oui → 1202", "sortie → 1203", "sortie → 1204", "sortie → 1205"]):
    d.rect(800, 58 + i * 26, 184, 22, "#ffffff", LIGNE, 3)
    d.t(808, 73 + i * 26, s, 9.8, mono=True)
d.t(470, 232, "règles · 16 o", 10.5, True)
for i, s in enumerate(["600  point 1107 > 2000 (20,00 °C)", "601  heure 22:00–06:00, lun.–ven.", "602  soleil = nuit"]):
    d.rect(470, 240 + i * 24, 314, 20, "#ffffff", LIGNE, 3)
    d.t(478, 254 + i * 24, s, 9.6, mono=True)
d.t(800, 232, "index par point", 10.5, True)
d.rect(800, 240, 184, 68, "#e6eee5", VERT, 6)
d.t(808, 258, "debut[917] = 44", 9.8, mono=True)
d.t(808, 276, "debut[918] = 46", 9.8, mono=True)
d.t(808, 296, "index[44..46] = 1200, 3412", 9.8, True, VERT, mono=True)
d.t(470, 330, f"≈ {nb(prog_w1 / 400)} o par automatisme, index compris : {nb(prog_w1 / 1024)} Kio pour 400 (au lieu de {fr(auj_w1 * 400 / 2**20)} Mio).", 10.5, True, VERT)
d.t(470, 346, "Aucune chaîne n'est comparée en marche : tout est numéro, et la place se calcule.", 10.5, coul=GRIS)
ecrire("programme", figure("programme", "Un automatisme, de la définition au programme",
    "À gauche, la définition JSON telle qu'on l'édite, environ 2,5 Ko, avec le graphe recopié dans la version validée. À droite, le même automatisme compilé : six nœuds de 32 octets, cinq liens de 8 octets, trois règles de 16 octets, et l'index par point qui dit quels déclencheurs écoutent le point 917.",
    d.texte(), 358,
    "L'automatisme type de la maquette (quand, si, régler, attendre, régler, prévenir), compilé. Les numéros sont fictifs mais la forme est celle du banc."))

# ================================================================ 7. n8n : ce qui voyage, ce qu'on lit
d = Dessin("n8n")
etapes = [(70, "22 h 41 min 00", "déclencheur", "porte → ouverte"), (330, "attendre", "5 min", ""),
          (590, "22 h 46 min 00", "si", "porte ouverte ?"), (850, "", "régler", "lumière")]
for i, (x, h_, n, s) in enumerate(etapes):
    d.rect(x - 70, 40, 140, 54, "#ffffff", AMBRE if i == 2 else LIGNE, 10, 1.6 if i == 2 else 1)
    d.t(x, 62, n, 12.5, True, ENCRE, "middle")
    d.t(x, 80, s, 10.5, coul=GRIS, ancre="middle")
    if h_:
        d.t(x, 30, h_, 10, True, GRIS, "middle")
    if i < 3:
        d.ligne(x + 74, 67, x + 186, 67, GRIS, 1.6, fleche="g")
d.t(460, 128, "La porte s'est refermée à 22 h 44 min 10.", 11, True, ROUGE, "middle")
d.rect(16, 148, 470, 104, "#fbf0d6", AMBRE, 10)
d.t(32, 172, "Décider sur la cause emportée, à la n8n (écarté)", 12.5, True, AMBRE)
d.t(32, 192, "Le « si » relit la valeur du déclenchement : porte = ouverte.", 10.5)
d.t(32, 210, "Il croit la porte encore ouverte.", 10.5)
d.t(32, 236, "→ la lumière s'allume pour une porte fermée.", 11, True, ROUGE)
d.rect(514, 148, 470, 104, "#e6eee5", VERT, 10)
d.t(530, 172, "Décider sur la mémoire des valeurs (retenu)", 12.5, True, VERT)
d.t(530, 192, "Le « si » lit la fiche du point par son numéro, quand il passe :", 10.5)
d.t(530, 210, "porte = fermée depuis 22 h 44 min 10.", 10.5)
d.t(530, 236, "→ il voit la maison telle qu'elle est.", 11, True, VERT)
d.rect(16, 266, 968, 74, "#ffffff", LIGNE, 10)
d.t(32, 290, "Ce que le flux garde, pour toute la suite : sa cause", 12.5, True, ENCRE)
d.t(32, 310, "porte d'entrée · ouverte · avant : fermée · 22 h 41 min 00 : 32 octets dans la fiche d'exécution, lisibles par tout nœud qui suit", 10.5)
d.t(32, 328, "(« régler le point qui a déclenché », un message). Une étape franchie ne se rejoue pas : le flux avance, il ne revient jamais en arrière.", 10.5, coul=GRIS)
ecrire("n8n", figure("n8n", "Ce que le flux porte, ce que la maison dit",
    "Un automatisme se déclenche à 22 h 41 quand la porte s'ouvre, attend 5 minutes, puis teste si la porte est ouverte. La porte s'est refermée à 22 h 44. S'il relit ce que le flux porte, comme n8n, il croit la porte ouverte ; s'il lit la mémoire des valeurs, il la voit fermée.",
    d.texte(), 354,
    "Décidé le 23 octobre : une condition lit la maison au moment où elle passe ; le flux garde l'état qui l'a déclenché pour toute la suite ; une étape franchie ne se rejoue pas."))

# ================================================================ graphiques (mesures)
def barres_log(d, x0, y0, largeur, lignes, series, vmin, vmax, unite, h_barre=11, pas=46, couleurs=(VERT, AMBRE), fmt=None, graduations=None):
    """lignes : [(libellé, note, [valeur par série])] ; échelle logarithmique ; fmt : étiquette d'une valeur ;
    graduations : [(valeur, étiquette)] à la place des puissances de dix."""
    lmin, lmax = math.log10(vmin), math.log10(vmax)
    def X(v):
        return x0 + largeur * (math.log10(max(v, vmin)) - lmin) / (lmax - lmin)
    grad = graduations or [(10 ** e, None) for e in range(int(math.floor(lmin)), int(math.ceil(lmax)) + 1)]
    for v, lab in grad:
        if vmin <= v <= vmax:
            d.ligne(X(v), y0 - 8, X(v), y0 + pas * len(lignes) - 10, LIGNE, 1)
            lab = lab or (fmt(v) if fmt else (court(v) if v < 1000 else nb(v)) + f" {unite}")
            d.t(X(v), y0 - 12, lab, 9.5, coul=GRIS, ancre="middle")
    for i, (lib, note, vals) in enumerate(lignes):
        y = y0 + i * pas
        d.t(16, y + 10, lib, 11.5, True)
        if note:
            d.t(16, y + 25, note, 9.6, coul=GRIS)
        for k, v in enumerate(vals):
            if v is None:
                continue
            yy = y + k * (h_barre + 3)
            w = max(2, X(v) - x0)
            d.rect(x0, yy, w, h_barre, couleurs[k % len(couleurs)], couleurs[k % len(couleurs)], 2)
            d.t(x0 + w + 6, yy + h_barre - 1.5, fmt(v) if fmt else court(v, unite), 10, coul=ENCRE)
    return X


# ---------------- 8. Lire une valeur
d = Dessin("lecture")
L = [
    ("Hub::state aujourd'hui", "alloue un DeviceId, verrou, deux recherches, copie", "lire_hub_state_aujourdhui"),
    ("Miroir du moteur aujourd'hui", "HashMap<String, Value> : hacher le nom", "lire_miroir_moteur_aujourdhui"),
    ("Annuaire : nom → numéro", "une fois, quand le message entre", "annuaire_nom_vers_numero"),
    ("Par numéro, derrière un verrou", "un lecteur d'un autre fil", "lire_par_numero_derriere_verrou"),
    ("Par numéro + qualité de l'appareil", "fiche, définition, santé : trois lectures", "lire_par_numero_qualite_effective"),
    ("Écrire une nouvelle", "valeur, instants, santé, échéances, en place", "ecrire_une_nouvelle"),
    ("Par numéro, fiche de 40 o", "début + n × 40", "lire_par_numero_vif40"),
    ("Par numéro, colonne de 16 o", "début + n × 16", "lire_par_numero_chaud16"),
]
lignes = [(a, b, [r("valeurs", 1, c), r("valeurs", 10, c)]) for a, b, c in L]
barres_log(d, 340, 46, 560, lignes, 2, 0.1, 1000, "ns")
y = 46 + 46 * len(L) + 2
d.rect(340, y, 12, 12, VERT, VERT, 2); d.t(358, y + 10, "×1 : les deux maisons (2 264 points)", 10.5, coul=GRIS)
d.rect(600, y, 12, 12, AMBRE, AMBRE, 2); d.t(618, y + 10, "×10 (22 640 points)", 10.5, coul=GRIS)
ecrire("lecture", figure("lecture", "Lire une valeur : par nom, par numéro",
    "Temps pour lire une valeur, en nanosecondes, échelle logarithmique. Aujourd'hui, Hub::state coûte environ 90 ns et le miroir du moteur 35 à 45 ns. Par numéro, une lecture coûte moins d'une nanoseconde ; avec la qualité de l'appareil, moins de 2 ns ; derrière un verrou, environ 3,5 ns.",
    d.texte(), y + 30,
    "Lire une valeur, mesuré en Linux musl (conteneur de 2 CPU et 128 Mio, Apple M4), lectures dans un ordre tiré au hasard. Échelle logarithmique : chaque trait vaut dix fois le précédent.", fleches=False))

# ---------------- 9. Le coût d'un passage selon la taille
d = Dessin("passage")
xs = [2264, 22640, 226400, 2264000]
x0, y0, w, h = 120, 40, 760, 260
lx0, lx1 = math.log10(1000), math.log10(4e6)
ly0, ly1 = math.log10(0.05), math.log10(20000)
def PX(v): return x0 + w * (math.log10(v) - lx0) / (lx1 - lx0)
def PY(v): return y0 + h - h * (math.log10(v) - ly0) / (ly1 - ly0)
for e in range(-1, 5):
    v = 10 ** e
    d.ligne(x0, PY(v), x0 + w, PY(v), LIGNE, 1)
    d.t(x0 - 8, PY(v) + 4, court(v) + " µs" if v < 1000 else nb(v) + " µs", 9.5, coul=GRIS, ancre="end")
for v, lab in zip(xs, ["2 264 · ×1", "22 640 · ×10", "226 400 · ×100", "2,26 millions"]):
    d.ligne(PX(v), y0, PX(v), y0 + h, LIGNE, 1, "3 3")
    d.t(PX(v), y0 + h + 18, lab, 10, True, GRIS, "middle")
d.t(x0 + w / 2, y0 + h + 36, "nombre d'échéances lues par le passage", 10.5, coul=GRIS, ancre="middle")
d.ligne(x0, PY(10000), x0 + w, PY(10000), ROUGE, 1.2, "6 4")
d.t(x0 + w - 4, PY(10000) - 6, "1 % d'un cœur, pour un passage chaque seconde", 10, True, ROUGE, "end")
for sec, coul, lib in (("passage", VERT, "réglage de production (opt-level « s »)"), ("passage-vitesse", AMBRE, "réglé pour la vitesse (opt-level 3)")):
    pts = [(PX(n), PY(r(sec, n, "passage_blocs"))) for n in xs]
    d.chemin("M " + " L ".join(f"{a:.1f},{b:.1f}" for a, b in pts), coul, 2.2)
    for (a, b), n in zip(pts, xs):
        d.c.append(f'          <circle cx="{a:.1f}" cy="{b:.1f}" r="4" fill="{coul}"/>')
        v = r(sec, n, "passage_blocs")
        d.t(a + 8, b + (14 if sec == "passage-vitesse" else -8), court(v, "µs"), 10, True, coul, halo=True)
d.rect(x0 + w - 330, y0 + h - 52, 12, 12, VERT, VERT, 2); d.t(x0 + w - 312, y0 + h - 42, "réglage de production (opt-level « s »)", 10.5, coul=GRIS)
d.rect(x0 + w - 330, y0 + h - 32, 12, 12, AMBRE, AMBRE, 2); d.t(x0 + w - 312, y0 + h - 22, "le même code réglé pour la vitesse (opt-level 3)", 10.5, coul=GRIS)
ecrire("passage", figure("passage", "Le coût d'un passage selon la taille",
    "Temps d'un passage sur la colonne d'échéances, en microsecondes, selon le nombre d'échéances : environ 0,75 µs pour les 2 264 points de la maison, et moins d'une milliseconde pour 2,26 millions. Le même code réglé pour la vitesse est environ cinq fois plus rapide. Tout reste très loin de 1 % d'un cœur.",
    d.texte(), y0 + h + 48,
    "Un passage complet sur une colonne d'échéances de 4 octets, sans rien d'expiré (le cas de chaque seconde), en Linux musl. Échelles logarithmiques. La ligne rouge : ce qu'un passage par seconde devrait coûter pour prendre 1 % d'un cœur.", fleches=False))

# ---------------- 10. Une heure d'échéances, quatre méthodes
d = Dessin("heure")
meth = [("A · calcul au passage", "prototype de la V7 : santé de 20 o, 3 600 passages", "A"),
        ("B · colonne d'échéances", "4 o par appareil, 3 600 passages", "B"),
        ("R · colonne + réveil au plus proche", "on ne passe que si une échéance peut tomber", "R"),
        ("C · file de priorité", "un tas, une entrée par nouvelle", "C")]
lignes = [(a, b, [r("echeances", e, f"{k}_heure_totale") for e in (1, 10, 100)]) for a, b, k in meth]
barres_log(d, 340, 46, 540, lignes, 3, 0.01, 1000, "ms", h_barre=9, pas=50, couleurs=(VERT, VERT2, AMBRE))
y = 46 + 50 * 4 + 4
for k, (lib, coul) in enumerate([("×1", VERT), ("×10", VERT2), ("×100 (49 600 appareils)", AMBRE)]):
    d.rect(340 + k * 150, y, 12, 12, coul, coul, 2); d.t(358 + k * 150, y + 10, lib, 10.5, coul=GRIS)
pa = [int(r("echeances", e, "R_passages_par_heure")) for e in (1, 10, 100)]
d.t(16, y + 36, f"Passages par heure avec le réveil au plus proche : {pa[0]} à ×1, {pa[1]} à ×10, {pa[2]} à ×100, au lieu de 3 600.", 10.5, True, VERT)
ta = [r("echeances", e, "C_memoire_tas") / 1024 for e in (1, 10, 100)]
co = [r("echeances", e, "B_memoire_colonne") / 1024 for e in (1, 10, 100)]
d.t(16, y + 54, f"Mémoire : colonnes {fr(co[0])} / {fr(co[1], 0)} / {fr(co[2], 0)} Kio ; tas {fr(ta[0])} / {fr(ta[1], 0)} / {nb(ta[2])} Kio (×1 / ×10 / ×100). Une heure dure 3 600 000 ms.", 10.5, coul=GRIS)
ecrire("heure", figure("heure", "Une heure d'échéances, quatre méthodes",
    "Temps processeur total pour une heure simulée de la maison, nouvelles et passages compris, en millisecondes, échelle logarithmique, à ×1, ×10 et ×100. Les quatre méthodes déclarent exactement les mêmes appareils muets. La colonne avec réveil au plus proche est la moins chère, et la plus simple après la colonne seule.",
    d.texte(), y + 68,
    "Une heure de la maison rejouée : chaque appareil émet à son rythme, 2 % se taisent à la 20e minute ; les quatre méthodes déclarent les mêmes appareils muets (vérifié à chaque échelle). Même à ×100, la plus chère prend 0,01 % d'un cœur.", fleches=False))

# ---------------- 11. Un événement, aujourd'hui et par l'index
d = Dessin("evenement")
L = [("Aujourd'hui, point déclencheur", "tous les automatismes, tous leurs déclencheurs, chaînes", "evenement_declencheur_aujourdhui"),
     ("Aujourd'hui, point quelconque", "même balayage, rien ne se déclenche", "evenement_quelconque_aujourdhui"),
     ("Index, point déclencheur", "index[n] : ses déclencheurs seulement", "evenement_declencheur_index"),
     ("Index, point quelconque", "index[n] vide : deux lectures", "evenement_quelconque_index")]
lignes = [(a, b, [r("workflows", e, c) for e in (1, 3, 10)]) for a, b, c in L]
barres_log(d, 340, 46, 540, lignes, 3, 0.3, 30000, "ns", h_barre=9, pas=50, couleurs=(VERT, VERT2, AMBRE))
y = 46 + 50 * 4 + 4
for k, (lib, coul) in enumerate([("×1 : 400 automatismes", VERT), ("×3 : 1 200", VERT2), ("×10 : 4 000", AMBRE)]):
    d.rect(340 + k * 170, y, 12, 12, coul, coul, 2); d.t(358 + k * 170, y + 10, lib, 10.5, coul=GRIS)
ecrire("evenement", figure("evenement", "Un changement de valeur : qui se réveille ?",
    "Temps pour traiter un changement de valeur côté moteur, en nanosecondes, échelle logarithmique. Aujourd'hui le moteur parcourt tous les déclencheurs de tous les automatismes : de 0,8 à 14 µs selon l'échelle. Par l'index, seuls les déclencheurs du point se réveillent : 1 à 6 ns, quelle que soit l'échelle.",
    d.texte(), y + 30,
    "Côté moteur, un changement de valeur : le balayage d'aujourd'hui (engine.rs, on_state) grandit avec le nombre d'automatismes ; l'index par point ne dépend que des déclencheurs du point.", fleches=False))

# ---------------- 12. Cinq façons de calculer les conditions, ramenées à la seconde
d = Dessin("conditions")
pointe = 239
L = [("Balayage à chaque événement", "aujourd'hui : tous les déclencheurs, chaînes", r("workflows", 1, "evenement_quelconque_aujourdhui") * pointe / 1000),
     ("« Attendre que » aujourd'hui, 400 attentes", "chaque attente réveillée à chaque événement", r("conditions", 1, "attendre_que_aujourdhui_400_attentes_par_evenement") * pointe),
     ("Cycle d'automate, 10 tours par seconde", "toutes les conditions, à chaque tour", r("conditions", 1, "cycle_automate_un_tour") * 10),
     ("Réseau incrémental", "les feuilles du point, puis leurs « si »", r("conditions", 1, "reseau_changement_quelconque") * pointe / 1000),
     ("Index + calcul à la demande (proposé)", "les déclencheurs du point ; un « si » à son passage", r("workflows", 1, "evenement_quelconque_index") * pointe / 1000)]
lignes = [(a, b, [v]) for a, b, v in L]
barres_log(d, 380, 46, 500, lignes, 1, 0.1, 20000, "µs", h_barre=14, pas=48, couleurs=(VERT,))
y = 46 + 48 * len(L) + 2
d.t(16, y + 6, f"CPU par seconde, à la pointe de {pointe} changements par seconde, ×1. 10 000 µs par seconde = 1 % d'un cœur.", 10.5, True, GRIS)
ecrire("conditions", figure("conditions", "Cinq façons de calculer les conditions, ramenées à la seconde",
    "Temps processeur par seconde, à la pointe de 239 changements par seconde, pour cinq façons de calculer les conditions : le balayage d'aujourd'hui, les « attendre que » d'aujourd'hui avec 400 attentes, un cycle d'automate de 10 tours par seconde, un réseau incrémental, et l'index avec calcul à la demande.",
    d.texte(), y + 22,
    "Les coûts mesurés par opération, ramenés à une seconde de pointe (239 changements). Seul « attendre que » d'aujourd'hui, avec beaucoup d'attentes, approche le pour cent d'un cœur ; tout le reste est négligeable : le choix se fait sur d'autres critères (§ 9.3).", fleches=False))

# ---------------- 13. La mémoire, aujourd'hui et proposée
d = Dessin("memoire")
groupes = []
for e in (1, 3, 10):
    auj_v = r("valeurs", e, "memoire_states_aujourdhui") + r("valeurs", e, "memoire_miroir_moteur_aujourdhui")
    pro_v = r("valeurs", e, "memoire_etat_propose")
    auj_w = r("workflows", e, "memoire_definitions_aujourdhui") + r("workflows", e, "memoire_copie_live_aujourdhui")
    pro_w = (r("workflows", e, "memoire_programme") + r("decisions", e, "memoire_executions64")
             + r("workflows", e, "memoire_attentes") + r("decisions", e, "reseau_memoire_totale"))
    groupes.append((e, auj_v, pro_v, auj_w, pro_w))
vmax = max(g[3] for g in groupes) / 2**20
x0, w = 250, 640
def W(v): return w * (v / 2**20) / (vmax * 1.08)
d.titre(16, 24, "Mio demandés à l'allocateur")
y = 40
for e, av, pv, aw, pw in groupes:
    d.t(16, y + 30, f"×{e}", 15, True)
    for k, (lib, v, coul) in enumerate([("valeurs aujourd'hui", av, ROUGE), ("valeurs proposées", pv, VERT),
                                        ("workflows aujourd'hui", aw, ROUGE), ("workflows proposés", pw, VERT)]):
        yy = y + k * 17
        d.t(x0 - 8, yy + 11, lib, 10, coul=GRIS, ancre="end")
        ww = max(2, W(v))
        d.rect(x0, yy, ww, 12, coul if "aujourd" in lib else VERT, coul, 2)
        d.t(x0 + ww + 6, yy + 10.5, court(v / 2**20, "Mio"), 10, coul=ENCRE)
    y += 86
d.t(16, y + 4, "Valeurs : states du hub et miroir du moteur, contre colonnes, santé, échéances et annuaire interné.", 10.5, coul=GRIS)
d.t(16, y + 20, "Workflows : définitions et copie des actifs, contre programme, exécutions de 64 o (une par automatisme), attentes et réseau des", 10.5, coul=GRIS)
d.t(16, y + 36, "conditions composées (au pire : un déclencheur composé de trois feuilles par automatisme).", 10.5, coul=GRIS)
y += 16
ecrire("memoire", figure("memoire", "La mémoire, aujourd'hui et proposée",
    "Octets demandés, en Mio, pour les valeurs et pour les workflows, à ×1, ×3 et ×10. Les valeurs proposées pèsent un peu moins que les structures d'aujourd'hui en portant bien plus d'informations ; les workflows proposés pèsent environ vingt fois moins.",
    d.texte(), y + 32,
    "Ce que pèsent les valeurs et les workflows, aujourd'hui (rouge) et proposés (vert), octets demandés à l'allocateur, mesurés. Les valeurs proposées portent en plus la précédente, trois instants, la qualité, la santé et les échéances ; les workflows proposés, la cause de chaque exécution et les déclencheurs composés.", fleches=False))
# ================================================================ 14. Un déclencheur composé
d = Dessin("compose")
d.titre(16, 24, "Quand toutes les fenêtres sont fermées depuis 10 min")
feuilles = [("fenêtre cuisine", "301", "fermée", "21 h 02", True), ("fenêtre salon", "302", "fermée", "20 h 15", True),
            ("fenêtre chambre", "303", "fermée", "22 h 30", True), ("fenêtre bureau", "304", "ouverte → fermée", "22 h 41", False)]
for i, (nom, n, v, h_, deja) in enumerate(feuilles):
    y = 44 + i * 50
    d.rect(16, y, 250, 40, "#ffffff", AMBRE if not deja else LIGNE, 8, 1.6 if not deja else 1)
    d.t(28, y + 17, f"{nom} · point {n}", 11, True)
    d.t(28, y + 32, f"{v} · changé {h_}", 10, coul=AMBRE if not deja else GRIS)
    d.rect(282, y + 8, 64, 24, "#e6eee5", VERT, 12)
    d.t(314, y + 25, "vraie", 10.5, True, VERT, "middle")
    d.ligne(348, y + 20, 418, 140, VERT2, 1.4)
d.rect(420, 106, 170, 70, "#e6eee5", VERT, 12, 1.6)
d.t(505, 132, "ET : 4 / 4 vraies", 12.5, True, VERT, "middle")
d.t(505, 152, "un compteur de 2 octets", 10, coul=GRIS, ancre="middle")
d.t(505, 167, "par condition", 10, coul=GRIS, ancre="middle")
d.ligne(594, 141, 640, 141, VERT, 1.8, fleche="v")
d.rect(644, 96, 340, 90, "#ffffff", VERT, 12)
d.t(660, 120, "La condition bascule : fausse → vraie", 12, True, ENCRE)
d.t(660, 140, "vraie depuis 22 h 41 : le plus grand des « changé »", 10.5)
d.t(660, 157, "de ses feuilles ; se recalcule, rien à ranger", 10.5, coul=GRIS)
d.t(660, 174, "attente : échéance 22 h 51, depuis = 22 h 41", 10.5, True, AMBRE)
d.ligne(814, 190, 814, 222, VERT, 1.8, fleche="v")
d.rect(560, 226, 424, 70, "#e6eee5", VERT, 12)
d.t(576, 250, "22 h 51 : toujours vraie depuis 22 h 41 ? oui → exécution", 12, True, VERT)
d.t(576, 270, "cause gardée : fenêtre bureau (304) · fermée · avant : ouverte · 22 h 41", 10.5)
d.t(576, 286, "c'est la feuille qui a fait basculer la condition", 10, coul=GRIS)
d.t(16, 266, "Le programme fige la structure : feuilles, conditions, index par point.", 10.5, coul=GRIS)
d.t(16, 282, "La vérité vit à part : un octet par feuille, un compteur par condition.", 10.5, coul=GRIS)
d.t(16, 312, f"Reconstruite d'un coup au démarrage, après une recompilation ou une perte de messages : {court(r('decisions', 1, 'reseau_reconstruire'), 'µs')} à ×1, {court(r('decisions', 10, 'reseau_reconstruire'), 'µs')} à ×10.", 10.5, True, VERT)
d.t(16, 330, "Les changements d'un même message sont appliqués ensemble, puis on lit les bascules : pas de fausse alerte entre deux valeurs.", 10.5, coul=GRIS)
d.t(16, 346, "Une feuille sur une valeur mauvaise (appareil muet) est fausse : un automatisme ne déclenche jamais sur une valeur mauvaise.", 10.5, coul=GRIS)
ecrire("compose", figure("compose", "Un déclencheur sur une condition composée",
    "Quatre fenêtres sont des feuilles ; chacune garde sa vérité. Quand la fenêtre du bureau se ferme à 22 h 41, le compteur de la condition « et » passe à 4 sur 4 : la condition bascule, vraie depuis le plus grand des instants « changé » de ses feuilles. Une attente de 10 minutes s'arme ; à 22 h 51, si elle est toujours vraie depuis 22 h 41, une exécution part, avec la fenêtre du bureau pour cause.",
    d.texte(), 362,
    "Décidé le 23 octobre : les déclencheurs composés entrent dès la V7, par un réseau incrémental. Un changement ne recalcule que les feuilles de son point ; la durée composée se calcule sur les instants « changé », comme pour un point seul."))

# ================================================================ 15. La soirée rejouée (v2 : F, G, H, le thermomètre)
d = Dessin("soiree")
def X(m):
    """Minutes depuis 18:00 → abscisse : 19:00–21:40 resserré, 21:40–23:15 étiré."""
    if m <= 220:
        return 150 + (m - 60) * (212 / 160)
    return 362 + (m - 220) * (622 / 95)
def mn(h, m, s=0):
    return (h - 18) * 60 + m + s / 60
lanes = [("cond", "Entrée de A : vraie ?", 84), ("A", "A · Coucher (23 nœuds)", 120), ("B", "B · Couloir (relance)", 160),
         ("C", "C · Porte ouverte", 200), ("D", "D · Extinction auto", 240), ("E", "E · Alerte", 280),
         ("F", "F · Ronde de 22:30:00", 320), ("G", "G · Tâches de nuit", 360), ("H", "H · Rendez-vous −10 min", 400),
         ("T", "Thermomètre du salon", 440)]
Y = {k: y for k, _, y in lanes}
BAS = 466
OZ = 160  # le zoom et la légende descendent sous les nouveaux couloirs
# axe et grille
for (h, m) in [(19, 0), (19, 30), (20, 0), (20, 30), (21, 0), (21, 40), (22, 0), (22, 20), (22, 40), (23, 0), (23, 15)]:
    x = X(mn(h, m))
    d.ligne(x, 66, x, BAS, LIGNE, 1, "2 3")
    d.t(x, 58, f"{h}:{m:02d}", 10, True, GRIS, "middle")
for k, lib, y in lanes:
    d.t(16, y + 12, lib, 10.8, True)
    d.ligne(150, y + 22, 984, y + 22, "#e7ece6", 1)
# la panne (22:20–22:25), la modification de B (22:31)
x1, x2 = X(mn(22, 20)), X(mn(22, 25))
d.rect(x1, 70, x2 - x1, BAS - 70, "#e8e6e1", "#c9c5bb", 0)
d.t((x1 + x2) / 2, BAS + 12, "panne", 10, True, GRIS, "middle")
xm = X(mn(22, 31))
d.ligne(xm, 70, xm, BAS, ROUGE, 1.2, "4 3")
d.t(xm + 4, BAS + 12, "B modifié", 10, True, ROUGE)
# vérité de la condition d'entrée de A, et le capteur muet
for a, b in [(mn(21, 45), mn(21, 50)), (mn(21, 51), mn(22, 17)), (mn(22, 50), mn(22, 55, 1)), (mn(23, 5), mn(23, 15))]:
    d.rect(X(a), Y["cond"] + 4, X(b) - X(a), 12, VERT2, VERT2, 2)
a, b = mn(22, 55, 1), mn(23, 5)
d.rect(X(a), Y["cond"] + 4, X(b) - X(a), 12, AMBRE2, AMBRE, 2)
d.t((X(a) + X(b)) / 2, Y["cond"] + 1, "capteur muet", 9, True, AMBRE, "middle", halo=True)
COUL = {"duree": VERT, "que": AMBRE, "issue": "#a9b5ad"}
def barre(k, segs):
    for a, b, e in segs:
        x, w = X(a), max(1.5, X(b) - X(a))
        d.rect(x, Y[k] + 3, w, 12, COUL[e], COUL[e], 1)
def attente(k, a, b):
    d.rect(X(a), Y[k] + 16, X(b) - X(a), 5, "none", GRIS, 1, 0.9, "3 2")
def ordre(k, m):
    x = X(m)
    d.chemin(f"M {x-3.5},{Y[k]+1} L {x+3.5},{Y[k]+1} L {x},{Y[k]-5} Z", ENCRE, 1, fond=ENCRE)
def avis(k, m, dy=0):
    d.c.append(f'          <circle cx="{X(m):.1f}" cy="{Y[k]+9+dy}" r="4.2" fill="{ROUGE}" stroke="{FOND}" stroke-width="1.2"/>')
def croix(k, m):
    x, y = X(m), Y[k] + 9
    d.chemin(f"M {x-4},{y-4} L {x+4},{y+4} M {x+4},{y-4} L {x-4},{y+4}", ROUGE, 2)
# A
attente("A", mn(21, 45), mn(21, 51)); attente("A", mn(21, 51), mn(22, 1)); attente("A", mn(22, 50), mn(23, 0))
barre("A", [(mn(22, 1), mn(22, 1, 1), "issue"), (mn(22, 1, 1), mn(22, 6, 1), "duree"), (mn(22, 6, 1), mn(22, 6, 48), "que"),
            (mn(22, 6, 48), mn(22, 36, 48), "duree"), (mn(22, 36, 48), mn(22, 37, 49), "que")])
avis("A", mn(20, 30)); croix("A", mn(23, 0))
for m in [mn(22, 1), mn(22, 6, 1), mn(22, 36, 48)]:
    ordre("A", m)
avis("A", mn(22, 37, 49)); avis("A", mn(23, 5))
d.t(X(mn(20, 30)), Y["A"] - 4, "bouton : trop tôt", 9, coul=GRIS, ancre="middle")
d.t(X(mn(22, 21, 30)), Y["A"] - 4, "fiche gardée pendant la panne", 9, coul=VERT, ancre="middle", halo=True)
# B
barre("B", [(mn(21, 0), mn(21, 0, 5), "issue"), (mn(21, 0, 5), mn(21, 1, 30), "duree"), (mn(21, 1, 30), mn(21, 1, 31), "issue"),
            (mn(21, 1, 31), mn(21, 3, 31), "duree"), (mn(22, 19), mn(22, 19, 1), "issue"), (mn(22, 19, 1), mn(22, 20), "duree"),
            (mn(22, 30), mn(22, 30, 1), "issue"), (mn(22, 30, 1), mn(22, 31), "duree"), (mn(23, 8), mn(23, 8, 1), "issue"),
            (mn(23, 8, 1), mn(23, 10, 1), "duree")])
for m in [mn(21, 0), mn(21, 1, 30), mn(21, 3, 31), mn(22, 19), mn(22, 30), mn(23, 8), mn(23, 10, 1)]:
    ordre("B", m)
croix("B", mn(22, 25)); croix("B", mn(22, 31)); avis("B", mn(23, 10, 2))
d.t(X(mn(21, 3, 31)) + 8, Y["B"] + 12, "relancé à 21:01:30", 9, coul=GRIS)
# C
attente("C", mn(22, 17), mn(22, 22)); avis("C", mn(22, 25))
# D
attente("D", mn(19, 0), mn(19, 10)); ordre("D", mn(19, 10))
attente("D", mn(19, 30), mn(19, 40)); croix("D", mn(19, 40))
attente("D", mn(19, 45), mn(19, 55)); ordre("D", mn(19, 55))
# E
attente("E", mn(22, 44), mn(22, 45)); croix("E", mn(22, 45))
attente("E", mn(23, 11), mn(23, 12)); avis("E", mn(23, 12))
# F : une heure, à la seconde
avis("F", mn(22, 30))
d.t(X(mn(22, 30)) + 8, Y["F"] + 13, "22:30:00, à la seconde (la porte est ouverte)", 9, coul=GRIS)
# G : deux heures tombent dans la panne ; le retard décide au redémarrage
croix("G", mn(22, 21, 30))
d.c.append(f'          <circle cx="{X(mn(22, 24)):.1f}" cy="{Y["G"]+9}" r="3.6" fill="{FOND}" stroke="{VERT}" stroke-width="1.6"/>')
avis("G", mn(22, 25))
d.t(X(mn(22, 20)) - 6, Y["G"] + 13, "22:21:30 manquée (3 min 30 de retard)", 9, coul=ROUGE, ancre="end")
d.t(X(mn(22, 25)) + 8, Y["G"] + 13, "22:24:00 rattrapée à 22:25 (60 s de retard)", 9, coul=VERT)
# H : les rendez-vous de l'agenda (barres) et les rappels 10 min avant (avis)
for a, b in [(mn(21, 30), mn(22, 0)), (mn(22, 31), mn(22, 45)), (mn(23, 0), mn(23, 15)), (mn(23, 14), mn(23, 15))]:
    d.rect(X(a), Y["H"] + 16, X(b) - X(a), 6, "#e7ece6", GRIS, 1, 0.8)
for m in [mn(21, 20), mn(22, 25), mn(22, 56)]:
    avis("H", m)
avis("H", mn(23, 4), -3); avis("H", mn(23, 4), 3)
d.t(X(mn(21, 20)) - 7, Y["H"] + 13, "à l'heure", 9, coul=GRIS, ancre="end")
d.t(X(mn(22, 25)) + 7, Y["H"] + 6, "rappel de 22:21 (panne)", 9, coul=GRIS)
d.t(X(mn(22, 56)) - 7, Y["H"] + 6, "ajouté à 22:56", 9, coul=GRIS, ancre="end")
d.t(X(mn(23, 4)) + 7, Y["H"] + 6, "×2", 9, True, ROUGE)
# Le thermomètre : la valeur devient trop vieille (fraîcheur 15 min), puis l'appareil muet
a, b, c = mn(22, 52, 31), mn(23, 7, 31), mn(23, 15)
d.rect(X(a), Y["T"] + 4, X(b) - X(a), 12, AMBRE2, AMBRE, 2)
d.rect(X(b), Y["T"] + 4, X(c) - X(b), 12, "#c9c5bb", GRIS, 2)
d.t((X(a) + X(b)) / 2, Y["T"] + 14, "trop vieille", 9, True, ENCRE, "middle")
d.t((X(b) + X(c)) / 2, Y["T"] + 14, "muet", 9, True, ENCRE, "middle")
d.t(X(a) - 7, Y["T"] + 14, "dernière nouvelle 22:37:31 · fraîcheur exigée 15 min", 9, coul=GRIS, ancre="end")
# zoom : 22:06:00 → 22:07:00
zx = lambda s: 150 + s * (800 / 60)
d.rect(16, 334 + OZ, 968, 118, "#ffffff", LIGNE, 10)
d.t(28, 354 + OZ, "Zoom : 22:06:00 → 22:07:00, les vérifications de A", 11, True)
for s_ in range(0, 61, 10):
    d.ligne(zx(s_), 362 + OZ, zx(s_), 424 + OZ, LIGNE, 1, "2 3")
    d.t(zx(s_), 438 + OZ, f"22:{6 + s_ // 60:02d}:{s_ % 60:02d}", 9.5, coul=GRIS, ancre="middle")
for a, b, e in [(1, 2, "issue"), (2, 45, "que"), (45, 46, "issue"), (46, 47, "que"), (47, 48, "issue"), (48, 60, "duree")]:
    d.rect(zx(a), 384 + OZ, zx(b) - zx(a), 14, COUL[e], COUL[e], 1)
for s_, lib, ancre in [(1, "fermer le volet", "start"), (45, "armer", "end"), (47, "mode nuit", "start")]:
    x = zx(s_)
    d.chemin(f"M {x-3.5},{382 + OZ} L {x+3.5},{382 + OZ} L {x},{376 + OZ} Z", ENCRE, 1, fond=ENCRE)
    d.t(x + (5 if ancre == "start" else -5), 374 + OZ, lib, 9.5, True, ancre=ancre)
d.t(zx(23.5), 395 + OZ, "attend : volet = 0", 9.5, True, "#ffffff", "middle")
for s_, lib, ok in [(20, "volet 50 : pas encore", False), (45, "volet 0 : vérifié", True), (47, "alarme 2 : vérifiée", True)]:
    x = zx(s_)
    d.ligne(x, 400 + OZ, x, 410 + OZ, VERT if ok else ROUGE, 1.5)
    d.t(x + (0 if s_ < 40 else (-4 if s_ == 45 else 4)), 421 + OZ, lib, 9.5, True, VERT if ok else ROUGE, "middle" if s_ < 40 else ("end" if s_ == 45 else "start"))
d.t(zx(54), 395 + OZ, "attendre 30 min →", 9.5, True, "#ffffff", "middle")
# légende
yl = 468 + OZ
for i, (lib, coul) in enumerate([("attend une durée", VERT), ("attend une condition (vérification)", AMBRE), ("attend une issue", "#a9b5ad")]):
    d.rect(16 + i * 190, yl, 12, 10, coul, coul, 2); d.t(32 + i * 190, yl + 9, lib, 10, coul=GRIS)
d.rect(616, yl + 2, 18, 5, "none", GRIS, 1, 0.9, "3 2"); d.t(640, yl + 9, "attente d'un déclencheur", 10, coul=GRIS)
yl += 18
d.chemin(f"M 19,{yl+9} L 26,{yl+9} L 22.5,{yl+3} Z", ENCRE, 1, fond=ENCRE); d.t(32, yl + 9, "ordre", 10, coul=GRIS)
d.c.append(f'          <circle cx="210" cy="{yl+6}" r="4.2" fill="{ROUGE}"/>'); d.t(220, yl + 9, "avis", 10, coul=GRIS)
d.chemin(f"M 402,{yl+2} L 410,{yl+10} M 410,{yl+2} L 402,{yl+10}", ROUGE, 2); d.t(416, yl + 9, "abandon, interruption, arrêt, manquée", 10, coul=GRIS)
d.rect(616, yl + 1, 18, 9, VERT2, VERT2, 2); d.t(640, yl + 9, "condition d'entrée de A vraie", 10, coul=GRIS)
yl += 18
d.rect(16, yl + 2, 18, 6, "#e7ece6", GRIS, 1, 0.8); d.t(40, yl + 9, "rendez-vous de l'agenda", 10, coul=GRIS)
d.c.append(f'          <circle cx="210" cy="{yl+6}" r="3.6" fill="{FOND}" stroke="{VERT}" stroke-width="1.6"/>'); d.t(220, yl + 9, "occurrence tombée pendant la panne, rattrapée", 10, coul=GRIS)
d.rect(616, yl + 1, 18, 9, AMBRE2, AMBRE, 2); d.t(640, yl + 9, "valeur trop vieille ou capteur muet", 10, coul=GRIS)
ecrire("soiree", figure("soiree", "La soirée rejouée, automatisme par automatisme",
    "Une frise de 19 h à 23 h 15. A, le coucher, attend sa condition composée, part à 22:01, passe ses vérifications (volet, alarme), attend 30 min à travers la panne de 22:20 à 22:25, et finit à 22:37:49 sur une consigne non confirmée. B est relancé à 21:01:30, interrompu par la panne, arrêté par sa modification à 22:31, puis tourne dans sa version modifiée. C part au redémarrage. D éteint la lumière deux fois et abandonne une attente. E abandonne une attente à 22:45 et part à 23:12. F part à 22:30:00. G manque l'occurrence de 22:21:30 et rattrape celle de 22:24:00. H prévient 10 min avant chaque rendez-vous : à l'heure à 21:20, en retard à 22:25 et à 22:56, deux fois à 23:04. Le thermomètre se tait : sa valeur devient trop vieille à 22:52:31, l'appareil muet à 23:07:31. Un zoom montre les vérifications de A entre 22:06:00 et 22:06:50.",
    d.texte(), 528 + OZ,
    "La soirée que rejouent les tests, telle que le moteur de référence l'a vécue. Les heures sont celles des assertions des tests. F, G et H sont les déclencheurs horaires de la v2 ; le dernier couloir montre la fraîcheur exigée, tenue en v2. Le zoom montre que le volet à 50 ne suffit pas, et que A n'avance qu'à la confirmation de chaque nouvelle valeur.", fleches=False))
# (la frise reprend les heures vérifiées par les tests de src/scenario.rs)


# ================================================================ v2 : les mémoires (agendas à part)
d = Dessin("memoires")
boites = [(16, "Les valeurs", "ce que la maison est", "hub",
           ["Vif 40 o par point", "Santé 20 o par appareil", "échéances 4 o", "annuaire nom ↔ numéro", "points du hub : nuit, plages,", "en rendez-vous, prochain RDV"],
           "journal + instantané", VERT),
          (262, "Les agendas", "ce qui va arriver", "hub, à part",
           ["événement 24 o : début, fin,", "agenda, numéro ; sans titre", "7 jours glissants + en cours", "remplacés à chaque lecture", "projetés en valeurs", "(niveaux et dates)"],
           "instantané du hub", AMBRE),
          (508, "Le programme", "ce que les workflows sont", "moteur",
           ["plans 12 o, nœuds 32 o", "liens 8 o, règles 16 o", "fiches horaires 24 o", "index par point", "réseau figé (composées)", "textes bout à bout"],
           "recompilé, jamais écrit", ENCRE),
          (754, "Les exécutions", "ce que les workflows font", "moteur",
           ["exécution 64 o, avec sa cause", "attente de déclencheur 24 o", "état des fiches horaires 25 o", "vérité des composées", "minuteries 24 o", "textes produits"],
           "journal du moteur", ROUGE)]
for x, t1, t2, qui, items, persiste, coul in boites:
    d.rect(x, 30, 230, 262, "#ffffff", coul, 12, 1.6)
    d.t(x + 12, 54, t1, 14, True, coul)
    d.t(x + 218, 54, qui, 9.5, coul=GRIS, ancre="end")
    d.t(x + 12, 72, t2, 10.5, coul=GRIS, italique=True)
    for i, it in enumerate(items):
        d.rect(x + 12, 84 + i * 28, 206, 23, DOUX, LIGNE, 4)
        d.t(x + 20, 100 + i * 28, it, 10)
    d.t(x + 12, 280, persiste, 10, True, coul)
d.t(16, 318, "Les agendas sont à part des valeurs : un événement n'est pas l'état d'un point, c'est un intervalle à venir. Le hub en tire des valeurs", 10.5, coul=GRIS)
d.t(16, 334, "(« en rendez-vous », « prochain rendez-vous ») ; les automatismes lisent ces valeurs, et le moteur du temps lit la table pour ses rappels.", 10.5, coul=GRIS)
d.t(16, 360, "Aucune mémoire ne copie une autre : elles se citent par numéro, et chacune a sa durée de vie.", 11.5, True, ENCRE)
ecrire("memoires", figure("memoires", "Quatre mémoires",
    "Les valeurs (hub), les agendas (hub, à part, projetés en valeurs), le programme des workflows (figé) et les exécutions (avec les échéances horaires et les minuteries).",
    d.texte(), 372,
    "v2 : les agendas rejoignent les mémoires, à part des valeurs ; le temps (fiches horaires, minuteries) se range comme le reste, en fiches de taille fixe."))

# ================================================================ v2 : l'échéancier aux deux horloges
d = Dessin("temps")
d.titre(16, 24, "Horloge monotone : les durées")
cols = [("santé des appareils", "4 o"), ("fraîcheur des valeurs", "4 o"), ("attentes des déclencheurs", "4 o"),
        ("réveils des exécutions", "4 o"), ("minuteries", "4 o")]
for i, (lib, o) in enumerate(cols):
    y = 40 + i * 30
    d.rect(16, y, 300, 24, "#e6eee5", VERT2, 5)
    d.t(26, y + 16, lib, 10.5)
    d.t(306, y + 16, o, 10, True, VERT, "end")
d.t(16, 210, "Aucun réglage de l'heure ne les touche. Elles repartent de zéro", 10, coul=GRIS)
d.t(16, 224, "au démarrage : l'instantané les range en heure murale.", 10, coul=GRIS)
d.titre(684, 24, "Horloge murale : le calendrier")
cols = [("heures, soleil, périodes", "8 o"), ("rappels d'agenda", "8 o"), ("plages, nuit, en rendez-vous", "8 o"), ("dernière occurrence partie", "8 o")]
for i, (lib, o) in enumerate(cols):
    y = 40 + i * 30
    d.rect(684, y, 300, 24, "#fbf0d6", AMBRE, 5)
    d.t(694, y + 16, lib, 10.5)
    d.t(974, y + 16, o, 10, True, AMBRE, "end")
d.t(684, 180, "Échéances en ms : la seconde et la milliseconde", 10, coul=GRIS)
d.t(684, 194, "sont tenues sans coût de plus.", 10, coul=GRIS)
d.rect(380, 80, 240, 100, "#ffffff", ENCRE, 12, 1.6)
d.t(500, 104, "Le réveil au plus proche", 12.5, True, ENCRE, "middle")
d.t(500, 124, "min(durées, calendrier converti)", 10.5, coul=GRIS, ancre="middle")
d.t(500, 142, "on dort jusque-là : aucun", 10.5, coul=GRIS, ancre="middle")
d.t(500, 158, "balayage à la seconde", 10.5, coul=GRIS, ancre="middle")
d.ligne(320, 130, 376, 130, VERT, 1.8, fleche="v")
d.ligne(680, 130, 624, 130, AMBRE, 1.8, fleche="a")
d.rect(380, 200, 240, 54, "#f6e4dc", ROUGE, 10)
d.t(500, 222, "Horloge pas sûre (Pi sans horloge à pile,", 10.5, True, ROUGE, "middle")
d.t(500, 238, "pas encore NTP) : rien de calendaire", 10.5, True, ROUGE, "middle")
d.t(16, 280, "Saut d'horloge en avant : les occurrences enjambées partent si elles ont 2 min de retard au plus, sinon elles sont manquées (une seule alerte).", 10.5, coul=GRIS)
d.t(16, 296, "Saut en arrière : rien ne repart, la dernière occurrence partie est gardée. Les durées, elles, ne voient aucun des deux.", 10.5, coul=GRIS)
ecrire("temps", figure("temps", "L'échéancier aux deux horloges",
    "À gauche, les échéances en horloge monotone : santé, fraîcheur, attentes, exécutions, minuteries. À droite, les échéances calendaires en heure murale : heures, soleil, périodes, rappels d'agenda, niveaux. Au centre, le réveil au plus proche prend la plus petite des deux familles. Une horloge pas sûre bloque le calendrier.",
    d.texte(), 310,
    "Le temps dans la mémoire : deux familles de colonnes d'échéances, un seul réveil. Les durées ne dépendent jamais de l'heure murale ; le calendrier ne part jamais sur une heure qui n'est pas sûre."))

# ================================================================ v2 : les changements d'heure
d = Dessin("dst")
def panneau(x0, titre, heures, lignes):
    d.titre(x0 - 50, 24, titre)
    w = 360
    pas = w / (len(heures) - 1)
    for i, (u, l) in enumerate(heures):
        x = x0 + i * pas
        d.ligne(x, 68, x, 236, LIGNE, 1, "2 3")
        d.t(x, 46, u, 9.5, True, GRIS, "middle")
        d.t(x, 60, l, 9.5, coul=AMBRE, ancre="middle")
    d.t(x0 - 26, 46, "UTC", 9, True, GRIS, "end")
    d.t(x0 - 26, 60, "local", 9, True, AMBRE, "end")
    for j, (lib, auj, v2) in enumerate(lignes):
        y = 82 + j * 52
        d.t(x0 - 50, y, lib, 10.5, True)
        for k, (marques, coul) in enumerate([(auj, ROUGE), (v2, VERT)]):
            yy = y + 10 + k * 16
            d.t(x0 - 14, yy + 8, "auj." if k == 0 else "v2", 9, True, coul, "end")
            for m in marques:
                if isinstance(m, tuple):
                    a, b = m
                    d.rect(x0 + a * pas, yy, (b - a) * pas, 10, coul, coul, 2)
                else:
                    xm = x0 + m * pas
                    d.c.append(f'          <circle cx="{xm:.1f}" cy="{yy + 5}" r="3.6" fill="{coul}"/>')
# retour à l'heure d'hiver : 25/10/2026, UTC 00:00 → 02:00 = local 02:00 CEST → 03:00 CET ; pas de 15 min
heures = [("00:00", "02:00"), ("00:30", "02:30"), ("01:00", "02:00"), ("01:30", "02:30"), ("02:00", "03:00")]
quart = [i * 0.5 for i in range(9)]
panneau(90, "Retour à l'heure d'hiver, 25 octobre 2026", heures, [
    ("« à 02:30 »", [1], [1]),
    ("« toutes les 15 min »", [0, 0.5, 1, 1.5, 4], quart),
    ("plage « 02:30 → 02:45 »", [(1, 1.5), (3, 3.5)], [(1, 1.5), (3, 3.5)]),
])
heures = [("00:00", "01:00"), ("00:30", "01:30"), ("01:00", "03:00"), ("01:30", "03:30"), ("02:00", "04:00")]
panneau(600, "Passage à l'heure d'été, 28 mars 2027", heures, [
    ("« à 02:30 »", [], [2]),
    ("« toutes les 15 min »", quart, quart),
    ("plage « 02:30 → 03:30 »", [(2, 3)], [(2, 3)]),
])
d.t(16, 258, "Retour à l'heure d'hiver : 02:30 existe deux fois. Une occurrence part une fois, à la première ; un niveau vaut pour les deux copies (comme les plannings d'aujourd'hui).", 10, coul=GRIS)
d.t(16, 273, "Aujourd'hui, « toutes les 15 min » saute les quatre occurrences de l'heure répétée : la minute locale porte le même tampon. En v2, une période compte le temps réel.", 10, coul=GRIS)
d.t(16, 288, "Passage à l'heure d'été : 02:30 n'existe pas. Aujourd'hui, l'occurrence est perdue ; en v2, elle a lieu au saut (03:00). Une plage qui enjambe le saut s'ouvre au saut.", 10, coul=GRIS)
ecrire("dst", figure("dst", "Les changements d'heure, aujourd'hui et en v2",
    "Deux frises : le 25 octobre 2026 (retour à l'heure d'hiver) et le 28 mars 2027 (passage à l'heure d'été), en UTC et en heure locale. Pour une occurrence à 02:30, une période de 15 min et une plage, les points rouges montrent le comportement d'aujourd'hui, les verts celui de la v2.",
    d.texte(), 300,
    "Ce que font les déclencheurs horaires aux changements d'heure. Rouge : le moteur d'aujourd'hui (engine.rs, on_clock). Vert : la v2, vérifiée par les tests du moteur du temps."))

# ================================================================ v2 : rattrapage, sauts, horloge pas sûre
d = Dessin("rattrapage")
def frise(y, titre, debut, fin, bande, marques, note):
    x0, w = 170, 560
    X = lambda m: x0 + (m - debut) * w / (fin - debut)
    d.t(16, y + 14, titre, 11, True)
    d.ligne(x0, y + 12, x0 + w, y + 12, LIGNE, 2)
    for m in range(int(debut), int(fin) + 1, 2):
        d.t(X(m), y + 30, f"7:{m:02d}" if m >= 0 else f"6:{60 + m:02d}", 9, coul=GRIS, ancre="middle")
    if bande:
        a, b, lib, coul = bande
        d.rect(X(a), y + 2, X(b) - X(a), 20, coul, coul, 3)
        d.t((X(a) + X(b)) / 2, y + 16, lib, 9.5, True, "#ffffff", "middle")
    for m, lib, ok in marques:
        x = X(m)
        coul = VERT if ok else ROUGE
        d.c.append(f'          <circle cx="{x:.1f}" cy="{y + 12}" r="5" fill="{coul}" stroke="{FOND}" stroke-width="1.5"/>')
        d.t(x, y - 2, lib, 9.5, True, coul, "middle")
    d.t(750, y + 16, note, 10, coul=GRIS)
frise(40, "Panne du hub", -4, 8, (-2, 4, "panne", "#9aa59d"), [(0, "7:00 manquée", False), (3, "7:03 rattrapée", True)], "au redémarrage (7:04) : 4 min → manquée,")
d.t(750, 72, "1 min → part, avec son retard", 10, coul=GRIS)
frise(110, "Saut en avant (NTP)", -4, 8, (-2, 6, "l'heure saute", AMBRE), [(0, "7:00 manquée", False), (5, "7:05 part", True)], "comme une panne : le retard décide")
frise(180, "Saut en arrière", -4, 8, None, [(0, "7:00 partie", True)], "l'horloge recule à 6:52 : 7:00 ne")
d.t(750, 212, "repart pas (la dernière est gardée)", 10, coul=GRIS)
frise(250, "Horloge pas sûre", -4, 8, (-4, 2, "pas sûre", ROUGE), [(4, "7:04 part", True)], "rien ne part ni n'est compté manqué")
d.t(750, 282, "avant que l'heure soit sûre", 10, coul=GRIS)
ecrire("rattrapage", figure("rattrapage", "Rattraper, manquer, ne jamais partir deux fois",
    "Quatre situations : une panne, un saut d'horloge en avant, un saut en arrière, une horloge pas encore sûre. Une occurrence en retard de 2 min au plus part avec son retard ; au-delà, elle est manquée ; une occurrence partie ne repart jamais ; rien de calendaire ne compte tant que l'heure n'est pas sûre.",
    d.texte(), 300,
    "La règle du 23 octobre (2 min) appliquée aux déclencheurs horaires, et ce qui protège d'un double départ. Chaque cas est un test du moteur du temps."))

# ================================================================ v2 : l'agenda
d = Dessin("agenda")
d.titre(16, 24, "La table des événements (hub, à part des valeurs)")
rangs = [("1", "21:30", "22:00"), ("2", "22:31", "22:45"), ("3", "23:00", "23:30"), ("4", "23:14", "23:20"), ("5", "23:14", "23:45")]
d.t(16, 50, "n°", 10, True, GRIS)
d.t(60, 50, "début", 10, True, GRIS)
d.t(130, 50, "fin", 10, True, GRIS)
d.t(200, 50, "24 o chacun, sans titre", 10, coul=GRIS)
for i, (n, a, b) in enumerate(rangs):
    y = 58 + i * 26
    d.rect(16, y, 340, 22, "#ffffff", LIGNE, 3)
    d.t(26, y + 15, n, 10, True, mono=True)
    d.t(60, y + 15, a, 10, mono=True)
    d.t(130, y + 15, b, 10, mono=True)
d.t(16, 208, "Relue à chaque lecture de l'agenda (5 min par défaut) :", 10, coul=GRIS)
d.t(16, 222, "remplacée, et les rappels concernés se réarment.", 10, coul=GRIS)
d.titre(400, 24, "Les rappels « 10 min avant » de la soirée")
ev = [("21:20", "événement 1", "à l'heure", VERT), ("22:25", "événement 2 (rappel de 22:21)", "tardif : la panne, le RDV n'a pas commencé", AMBRE),
      ("22:56", "événement 3 (ajouté à 22:56)", "tardif : ajouté au dernier moment", AMBRE), ("23:04", "événements 4 et 5", "même heure : deux rappels", VERT)]
for i, (h, quoi, note, coul) in enumerate(ev):
    y = 50 + i * 40
    d.c.append(f'          <circle cx="414" cy="{y + 8}" r="6" fill="{coul}"/>')
    d.t(428, y + 12, f"{h}  {quoi}", 11, True)
    d.t(428, y + 28, note, 10, coul=GRIS)
d.titre(400, 228, "Ce que l'agenda donne aux valeurs")
d.t(400, 248, "« en rendez-vous » (niveau, dès le décalage) et « prochain rendez-vous » (une date) :", 10.5, coul=GRIS)
d.t(400, 264, "des points du hub, que les règles lisent par numéro comme les autres.", 10.5, coul=GRIS)
d.t(16, 290, "Horizon décidé : 7 jours glissants et les événements en cours, au moins le plus grand décalage des rappels.", 10.5, True, ENCRE)
d.t(16, 307, f"24 o par événement : une semaine chargée, 150 événements, 3,5 Kio ({nb(r('precision', 1, 'agenda_semaine_memoire') / 1024)} Kio réservés) ; la relire et réarmer 10 rappels : {court(r('precision', 1, 'agenda_semaine_relire_et_rearmer_10_rappels'), 'µs')}.", 10.5, coul=GRIS)
ecrire("agenda", figure("agenda", "L'agenda : une table à part, des rappels, des valeurs projetées",
    "À gauche, la table des événements de l'agenda : début, fin, numéro, sans titre. À droite, les rappels de la soirée : à l'heure, tardifs (panne, ajout au dernier moment), et deux événements à la même heure. En bas, ce que l'agenda donne aux valeurs.",
    d.texte(), 318,
    "Un événement n'est pas une valeur : c'est un intervalle à venir, rangé dans sa propre table. Les automatismes y réagissent par des fiches du moteur du temps, et lisent ses projections comme des valeurs."))

# ================================================================ v2 : le retard d'un réveil
d = Dessin("precision")
x0, largeur, y0 = 300, 600, 70
lmin, lmax = 1.0, 6.0  # 10 µs → 1 s
def XP(us):
    return x0 + largeur * (math.log10(max(us, 10)) - lmin) / (lmax - lmin)
for e, lab in [(1, "10 µs"), (2, "100 µs"), (3, "1 ms"), (4, "10 ms"), (5, "100 ms"), (6, "1 s")]:
    x = XP(10 ** e)
    rep_ = e in (3, 6)
    d.ligne(x, y0 - 10, x, y0 + 8 * 34 - 6, ROUGE if rep_ else LIGNE, 1.2 if rep_ else 1, "5 4" if rep_ else None)
    d.t(x, y0 - 16, lab, 9.5, rep_, ROUGE if rep_ else GRIS, "middle")
d.t(x0 + largeur / 2, 24, "retard du réveil, en plus du délai demandé (échelle logarithmique)", 10.5, coul=GRIS, ancre="middle")
rangs = []
for dm, lib in [(1, "1 ms"), (10, "10 ms"), (100, "100 ms"), (1000, "1 s")]:
    for ex, lex in [("thread", "un fil qui dort"), ("tokio", "Tokio")]:
        rangs.append((f"{lib} demandée", lex, dm, ex))
for i, (lib, lex, dm, ex) in enumerate(rangs):
    y = y0 + i * 34
    if i % 2 == 0:
        d.t(16, y + 12, lib, 11, True)
    d.t(150, y + 12, lex, 10, coul=GRIS)
    for k, (charge, coul) in enumerate([("repos", VERT), ("charge", AMBRE)]):
        yy = y + 6 + k * 12
        p50, p99, mx = (r("precision", 1, f"retard_{ex}_{dm}ms_{charge}_{q}") for q in ("p50", "p99", "max"))
        d.ligne(XP(p50), yy, XP(mx), yy, coul, 1.2)
        d.ligne(XP(p99), yy - 4, XP(p99), yy + 4, coul, 2)
        d.c.append(f'          <circle cx="{XP(p50):.1f}" cy="{yy}" r="3.8" fill="{coul}"/>')
    if i % 2 == 1:
        d.ligne(16, y + 28, 984, y + 28, "#e7ece6", 1)
yl = y0 + 8 * 34 + 8
d.c.append(f'          <circle cx="22" cy="{yl + 6}" r="3.8" fill="{VERT}"/>'); d.t(32, yl + 10, "médiane, au repos", 10, coul=GRIS)
d.c.append(f'          <circle cx="182" cy="{yl + 6}" r="3.8" fill="{AMBRE}"/>'); d.t(192, yl + 10, "médiane, deux fils occupent les deux CPU", 10, coul=GRIS)
d.ligne(470, yl + 2, 470, yl + 10, ENCRE, 2); d.t(478, yl + 10, "99e centile", 10, coul=GRIS)
d.ligne(570, yl + 6, 610, yl + 6, ENCRE, 1.2); d.t(616, yl + 10, "jusqu'au pire mesuré", 10, coul=GRIS)
pire = {ch: max(r("precision", 1, f"retard_{ex}_{dm}ms_{ch}_max") for ex in ("thread", "tokio") for dm in (1, 10, 100, 1000)) for ch in ("repos", "charge")}
med = [r("precision", 1, f"retard_{ex}_{dm}ms_repos_p50") for ex in ("thread", "tokio") for dm in (1, 10, 100, 1000)]
d.t(16, yl + 32, f"Au repos, un réveil arrive avec {court(min(med) / 1000, 'ms')} à {court(max(med) / 1000, 'ms')} de retard (médianes) ; Tokio compte en millisecondes entières et ajoute environ 1 ms. Sous charge, la médiane", 10, coul=GRIS)
d.t(16, yl + 47, f"baisse (le processeur ne s'endort plus). Le pire de trois essais : {court(pire['repos'] / 1000, 'ms')} au repos, {court(pire['charge'] / 1000, 'ms')} sous charge. La seconde est tenue avec une large marge ; la milliseconde, jamais garantie.", 10, coul=GRIS)
ecrire("precision", figure("precision", "Le retard d'un réveil",
    "Pour des attentes de 1 ms, 10 ms, 100 ms et 1 s, par un fil qui dort ou par Tokio, au repos et sous charge : la médiane, le 99e centile et le pire retard mesurés en Linux. D'ordinaire, quelques millisecondes ; au pire mesuré, quelques dizaines. La seconde est toujours tenue, la milliseconde jamais garantie.",
    d.texte(), yl + 60,
    "Mesuré dans le conteneur Linux musl (2 CPU, 128 Mio) d'une VM Colima sur Apple M4 : 5 à 200 réveils par cas, trois essais ; médiane et 99e centile : la médiane des essais ; pire : le pire des essais. Le retard d'une VM n'est pas celui d'un Pi ; l'ordre de grandeur, si.", fleches=False))

# ================================================================ v2 : le coût d'un « cron » selon sa précision
d = Dessin("cron")
def duree(ms):
    if ms < 1:
        return fr(ms, 2) + " ms"
    if ms < 1_000:
        return court(ms) + " ms"
    if ms < 60_000:
        return fr(ms / 1_000, 1 if ms < 10_000 else 0) + " s"
    if ms < 3_600_000:
        return fr(ms / 60_000, 1) + " min"
    return fr(ms / 3_600_000, 1) + " h"
tour1, tour10 = r("precision", 1, "balayage_aujourdhui_un_tour"), r("precision", 10, "balayage_aujourdhui_un_tour")
def jour(tour_us, par_s):
    """ms de processeur par jour."""
    return tour_us * par_s * 86_400 / 1_000
lignes = [
    ("Balayage, chaque seconde", "aujourd'hui : l'heure relue en texte, le soleil recalculé", [jour(tour1, 1), jour(tour10, 1)]),
    ("Balayage, 10 fois par seconde", "pour viser le dixième (extrapolé du tour mesuré)", [jour(tour1, 10), jour(tour10, 10)]),
    ("Balayage, 1 000 fois par seconde", "pour viser la milliseconde (extrapolé)", [jour(tour1, 1_000), jour(tour10, 1_000)]),
    ("Moteur du temps proposé", "réveil au plus proche, seconde ou milliseconde", [r("precision", 1, "journee_cpu_moteur_du_temps"), r("precision", 10, "journee_cpu_moteur_du_temps")]),
]
XC = barres_log(d, 330, 50, 560, lignes, 2, 0.1, 1e8, "", fmt=duree,
                graduations=[(0.1, "0,1 ms"), (1, "1 ms"), (10, "10 ms"), (100, "100 ms"), (1_000, "1 s"), (10_000, "10 s"),
                             (60_000, "1 min"), (600_000, "10 min"), (3_600_000, "1 h"), (36_000_000, "10 h")])
xc = XC(864_000)
d.ligne(xc, 40, xc, 50 + 46 * 4 - 6, ROUGE, 1.2, "6 4")
d.t(xc - 4, 50 + 46 * 4 + 6, "1 % d'un cœur", 10, True, ROUGE, "end")
y = 50 + 46 * 4 + 18
d.rect(330, y, 12, 12, VERT, VERT, 2); d.t(348, y + 10, "×1 : 400 déclencheurs horaires", 10.5, coul=GRIS)
d.rect(600, y, 12, 12, AMBRE, AMBRE, 2); d.t(618, y + 10, "×10 : 4 000", 10.5, coul=GRIS)
d.t(16, y + 36, f"Le moteur du temps se réveille {nb(r('precision', 1, 'journee_reveils'))} fois par jour pour {nb(r('precision', 1, 'journee_occurrences'))} départs ; son coût suit le nombre d'occurrences, pas la précision.", 10.5, coul=GRIS)
d.t(16, y + 52, "Un balayage coûte en proportion de sa fréquence : viser la milliseconde en balayant coûterait mille fois la seconde.", 10.5, coul=GRIS)
ecrire("cron", figure("cron", "Le coût d'un « cron » selon sa précision",
    "Processeur par jour, échelle logarithmique. Le balayage d'aujourd'hui, une fois par seconde, coûte environ une seconde par jour pour 400 déclencheurs ; dix fois par seconde, dix fois plus ; mille fois par seconde, plus d'un quart d'heure par jour. Le moteur du temps proposé coûte moins d'une milliseconde par jour, à la seconde comme à la milliseconde.",
    d.texte(), y + 64,
    "Processeur par jour pour tenir les déclencheurs horaires, mesuré en Linux musl (un tour du balayage, une journée du moteur du temps) ; les fréquences de 10 et 1 000 tours par seconde sont extrapolées du tour mesuré. La ligne rouge : 1 % d'un cœur.", fleches=False))

# ================================================================ Critique des workflows
# docs/proposition/critique-workflow.md : quatre schémas dessinés à la main, sans chiffre du banc
# (le moteur et ses répondeurs ; l'origine qui suit les données jusqu'aux actions ; un dossier de bout
# en bout ; un module de chaque côté d'un lien qui se coupe).
ecrire("demande", r'''      <figure class="diagram">
        <svg viewBox="0 0 1000 500" role="img" aria-labelledby="demande-t demande-d" font-family="Avenir Next, Avenir, Segoe UI, sans-serif">
          <title id="demande-t">Une demande, un produit, une origine</title>
          <desc id="demande-d">Le moteur de workflow, déterministe, lit les valeurs, attend les échéances et adresse des demandes par le courtier à quatre sortes de répondeurs : un appareil, Sarah, un service, une personne. Les issues reviennent par le même chemin ; ce qu'elles rapportent est rangé dans le dossier de la démarche, pris au départ dans la zone de travail, à la taille que le compilateur a calculée, avec son origine. Le dossier descend au journal du moteur : avant chaque engagement, en attendant le disque, et par lots pour le reste. Un réflexe n'a pas de dossier.</desc>
          <defs>
            <marker id="demande-g" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#52615a"/></marker>
          </defs>
          <rect x="16" y="30" width="330" height="236" rx="12" fill="#ffffff" stroke="#1d302a" stroke-width="2.2"/>
          <text x="28" y="54" font-size="14" fill="#1d302a" font-weight="700">Le moteur de workflow</text>
          <text x="334" y="54" font-size="9.5" fill="#52615a" text-anchor="end">déterministe</text>
          <text x="28" y="72" font-size="10.5" fill="#52615a" font-style="italic">il lit, il attend, il demande</text>
          <rect x="28" y="84" width="306" height="23" rx="4" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="36" y="100" font-size="10" fill="#1d302a">lit les valeurs par numéro (son miroir)</text>
          <rect x="28" y="112" width="306" height="23" rx="4" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="36" y="128" font-size="10" fill="#1d302a">attend les échéances (le moteur du temps)</text>
          <rect x="28" y="140" width="306" height="23" rx="4" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="36" y="156" font-size="10" fill="#1d302a">adresse une demande, attend son issue</text>
          <rect x="28" y="168" width="306" height="23" rx="4" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="36" y="184" font-size="10" fill="#1d302a">Si, Selon : sur la maison, la cause, un produit</text>
          <rect x="28" y="196" width="306" height="23" rx="4" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="36" y="212" font-size="10" fill="#1d302a">fiche d'exécution : 64 o, inchangée</text>
          <text x="28" y="250" font-size="10" fill="#1d302a" font-weight="700">un réflexe s'arrête là : il n'a pas de dossier</text>
          <text x="520" y="20" font-size="10" fill="#52615a" font-weight="700" text-anchor="middle">le courtier</text>
          <line x1="520" y1="30" x2="520" y2="251" stroke="#52615a" stroke-width="1.5"/>
          <line x1="346" y1="142" x2="516" y2="142" stroke="#52615a" stroke-width="1.5" marker-end="url(#demande-g)"/>
          <text x="433" y="135" font-size="10" fill="#52615a" text-anchor="middle">une demande (req/)</text>
          <line x1="518" y1="162" x2="350" y2="162" stroke="#52615a" stroke-width="1.5" marker-end="url(#demande-g)"/>
          <text x="433" y="179" font-size="10" fill="#52615a" text-anchor="middle">une issue et son produit (res/)</text>
          <line x1="520" y1="57" x2="596" y2="57" stroke="#52615a" stroke-width="1.5" marker-end="url(#demande-g)"/>
          <line x1="520" y1="121" x2="596" y2="121" stroke="#52615a" stroke-width="1.5" marker-end="url(#demande-g)"/>
          <line x1="520" y1="185" x2="596" y2="185" stroke="#52615a" stroke-width="1.5" marker-end="url(#demande-g)"/>
          <line x1="520" y1="249" x2="596" y2="249" stroke="#52615a" stroke-width="1.5" marker-end="url(#demande-g)"/>
          <rect x="600" y="30" width="384" height="54" rx="10" fill="#ffffff" stroke="#195744" stroke-width="1.6"/>
          <text x="612" y="51" font-size="13" fill="#195744" font-weight="700">Un appareil</text>
          <text x="972" y="51" font-size="9.5" fill="#52615a" text-anchor="end">par le hub et la garde</text>
          <text x="612" y="71" font-size="10" fill="#1d302a">ordre ou scène · fait, refus, trop long · jamais rejoué</text>
          <rect x="600" y="94" width="384" height="54" rx="10" fill="#ffffff" stroke="#b07d12" stroke-width="1.6"/>
          <text x="612" y="115" font-size="13" fill="#b07d12" font-weight="700">Sarah</text>
          <text x="972" y="115" font-size="9.5" fill="#52615a" text-anchor="end">un module, ailleurs que le cœur</text>
          <text x="612" y="135" font-size="10" fill="#1d302a">écrire pour des humains, remplir une fiche · ne sait pas</text>
          <rect x="600" y="158" width="384" height="54" rx="10" fill="#ffffff" stroke="#52615a" stroke-width="1.6"/>
          <text x="612" y="179" font-size="13" fill="#52615a" font-weight="700">Un service</text>
          <text x="972" y="179" font-size="9.5" fill="#52615a" text-anchor="end">météo, agenda, courriel, école</text>
          <text x="612" y="199" font-size="10" fill="#1d302a">lire · écrire avec une clé, sans effet si renvoyé</text>
          <rect x="600" y="222" width="384" height="54" rx="10" fill="#ffffff" stroke="#1d302a" stroke-width="1.6"/>
          <text x="612" y="243" font-size="13" fill="#1d302a" font-weight="700">Une personne</text>
          <text x="972" y="243" font-size="9.5" fill="#52615a" text-anchor="end">le nœud « Demander »</text>
          <text x="612" y="263" font-size="10" fill="#1d302a">un délai, une réponse par défaut, revérifier à la réponse</text>
          <line x1="520" y1="252" x2="520" y2="298" stroke="#52615a" stroke-width="1.5" marker-end="url(#demande-g)"/>
          <text x="530" y="283" font-size="10" fill="#52615a">ce qui revient va au dossier, avant la suite</text>
          <rect x="16" y="302" width="968" height="82" rx="12" fill="#ffffff" stroke="#b07d12" stroke-width="1.6"/>
          <text x="28" y="326" font-size="13" fill="#b07d12" font-weight="700">La zone de travail · le dossier de la démarche</text>
          <text x="972" y="326" font-size="9.5" fill="#52615a" text-anchor="end">à part des fiches · prise au départ, à la taille calculée · une case par produit</text>
          <rect x="28" y="342" width="150" height="26" rx="5" fill="#f1f4ee" stroke="#b07d12" stroke-width="1.2"/>
          <text x="40" y="359" font-size="10" fill="#b07d12" font-weight="700">texte · Sarah</text>
          <rect x="190" y="342" width="220" height="26" rx="5" fill="#f1f4ee" stroke="#8b3b22" stroke-width="1.2"/>
          <text x="202" y="359" font-size="10" fill="#8b3b22" font-weight="700">fiche.date · Sarah + Dehors</text>
          <rect x="422" y="342" width="170" height="26" rx="5" fill="#f1f4ee" stroke="#52615a" stroke-width="1.2"/>
          <text x="434" y="359" font-size="10" fill="#52615a" font-weight="700">prévision · Service</text>
          <rect x="604" y="342" width="170" height="26" rx="5" fill="#f1f4ee" stroke="#1d302a" stroke-width="1.2"/>
          <text x="616" y="359" font-size="10" fill="#1d302a" font-weight="700">réponse · Personne</text>
          <rect x="786" y="342" width="186" height="26" rx="5" fill="#f1f4ee" stroke="#195744" stroke-width="1.2"/>
          <text x="798" y="359" font-size="10" fill="#195744" font-weight="700">issue de l'ordre · Maison</text>
          <line x1="500" y1="384" x2="500" y2="410" stroke="#52615a" stroke-width="1.5" marker-end="url(#demande-g)"/>
          <text x="510" y="401" font-size="10" fill="#52615a">descend : avant chaque engagement, en attendant le disque ; par lots pour le reste</text>
          <rect x="16" y="414" width="968" height="34" rx="10" fill="#f1f4ee" stroke="#1d302a" stroke-width="1.4"/>
          <text x="28" y="436" font-size="12" fill="#1d302a" font-weight="700">Le journal du moteur</text>
          <text x="972" y="436" font-size="9.5" fill="#52615a" text-anchor="end">la base : fait foi · relu au redémarrage · reçu par la secours · gardé 30 jours après la fin, puis les codes</text>
          <text x="16" y="474" font-size="10.5" fill="#52615a">Un réflexe ne fait que lire, attendre et ordonner. Une démarche attend aussi Sarah, un service ou une personne : leurs demandes ont la même</text>
          <text x="16" y="490" font-size="10.5" fill="#52615a">mécanique que l'ordre à un appareil (une issue, un délai, une génération), et ce qu'elles rapportent va au dossier, puis au journal.</text>
        </svg>
      </figure>
''')

ecrire("origine", r'''      <figure class="diagram">
        <svg viewBox="0 0 1000 340" role="img" aria-labelledby="origine-t origine-d" font-family="Avenir Next, Avenir, Segoe UI, sans-serif">
          <title id="origine-t">L'origine suit les données jusqu'aux actions</title>
          <desc id="origine-d">Un courriel du collège, d'origine Dehors, est lu par Sarah, qui remplit une fiche : catégorie École, date, enfant. La fiche hérite des origines Sarah et Dehors. Un nœud Selon aiguille sur la catégorie. Noter à l'agenda et prévenir la famille sont permis. Ouvrir le portail est refusé par le compilateur : un texte du dehors n'ouvre jamais une porte, c'est le plancher. Régler le radiateur dépend du réglage de la famille : en « seule », Sarah choisit, dans des bornes écrites ; en « me proposer », la question est posée aux parents, et la réponse a l'origine Personne.</desc>
          <defs>
            <marker id="origine-g" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#52615a"/></marker>
            <marker id="origine-r" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#8b3b22"/></marker>
          </defs>
          <rect x="16" y="30" width="168" height="94" rx="10" fill="#ffffff" stroke="#8b3b22" stroke-width="1.6"/>
          <text x="28" y="52" font-size="12.5" fill="#8b3b22" font-weight="700">Courriel reçu</text>
          <text x="28" y="70" font-size="10" fill="#1d302a">du collège, à 12 h 05</text>
          <rect x="28" y="88" width="66" height="22" rx="5" fill="#f1f4ee" stroke="#8b3b22" stroke-width="1.2"/>
          <text x="36" y="103" font-size="10" fill="#8b3b22" font-weight="700">Dehors</text>
          <rect x="210" y="30" width="232" height="94" rx="10" fill="#ffffff" stroke="#b07d12" stroke-width="1.6"/>
          <text x="222" y="52" font-size="12.5" fill="#b07d12" font-weight="700">Sarah remplit</text>
          <text x="222" y="70" font-size="10" fill="#1d302a">catégorie : École</text>
          <text x="222" y="84" font-size="10" fill="#1d302a">date : vendredi 10 h · enfant : la cadette</text>
          <rect x="222" y="92" width="112" height="22" rx="5" fill="#f1f4ee" stroke="#8b3b22" stroke-width="1.2"/>
          <text x="230" y="107" font-size="10" fill="#8b3b22" font-weight="700">Sarah + Dehors</text>
          <rect x="468" y="30" width="150" height="94" rx="10" fill="#ffffff" stroke="#1d302a" stroke-width="1.6"/>
          <text x="480" y="52" font-size="12.5" fill="#1d302a" font-weight="700">Selon catégorie</text>
          <text x="480" y="70" font-size="10" fill="#1d302a">« École »</text>
          <rect x="480" y="88" width="112" height="22" rx="5" fill="#f1f4ee" stroke="#8b3b22" stroke-width="1.2"/>
          <text x="488" y="103" font-size="10" fill="#8b3b22" font-weight="700">Sarah + Dehors</text>
          <rect x="644" y="30" width="170" height="94" rx="10" fill="#ffffff" stroke="#195744" stroke-width="1.6"/>
          <text x="656" y="52" font-size="12.5" fill="#195744" font-weight="700">Noter à l'agenda</text>
          <text x="656" y="70" font-size="10" fill="#1d302a">clé : exécution, rang</text>
          <text x="656" y="84" font-size="10" fill="#1d302a">accord donné à la validation</text>
          <text x="656" y="110" font-size="10" fill="#195744" font-weight="700">✓ permis</text>
          <rect x="840" y="30" width="144" height="94" rx="10" fill="#ffffff" stroke="#195744" stroke-width="1.6"/>
          <text x="852" y="52" font-size="12.5" fill="#195744" font-weight="700">Prévenir</text>
          <text x="852" y="70" font-size="10" fill="#1d302a">« d'après le courriel</text>
          <text x="852" y="84" font-size="10" fill="#1d302a">du collège de 12 h 05 »</text>
          <text x="852" y="110" font-size="10" fill="#195744" font-weight="700">✓ permis</text>
          <line x1="184" y1="77" x2="206" y2="77" stroke="#52615a" stroke-width="1.5" marker-end="url(#origine-g)"/>
          <line x1="442" y1="77" x2="464" y2="77" stroke="#52615a" stroke-width="1.5" marker-end="url(#origine-g)"/>
          <line x1="618" y1="77" x2="640" y2="77" stroke="#52615a" stroke-width="1.5" marker-end="url(#origine-g)"/>
          <line x1="814" y1="77" x2="836" y2="77" stroke="#52615a" stroke-width="1.5" marker-end="url(#origine-g)"/>
          <line x1="505" y1="124" x2="334" y2="182" stroke="#8b3b22" stroke-width="1.5" stroke-dasharray="5 4" marker-end="url(#origine-r)"/>
          <line x1="575" y1="124" x2="604" y2="176" stroke="#52615a" stroke-width="1.5" marker-end="url(#origine-g)"/>
          <rect x="150" y="186" width="300" height="84" rx="10" fill="#ffffff" stroke="#8b3b22" stroke-width="1.6" stroke-dasharray="6 4"/>
          <text x="162" y="208" font-size="12.5" fill="#8b3b22" font-weight="700">Ouvrir le portail</text>
          <text x="162" y="226" font-size="10" fill="#8b3b22" font-weight="700">✗ jamais sans une personne :</text>
          <text x="162" y="242" font-size="10" fill="#1d302a">un texte du dehors n'ouvre pas une porte</text>
          <text x="162" y="258" font-size="10" fill="#52615a">le plancher, vérifié par le compilateur</text>
          <rect x="480" y="180" width="270" height="96" rx="10" fill="#ffffff" stroke="#b07d12" stroke-width="1.6"/>
          <text x="492" y="202" font-size="12.5" fill="#b07d12" font-weight="700">Régler le radiateur</text>
          <text x="492" y="220" font-size="10" fill="#1d302a">votre réglage décide :</text>
          <text x="492" y="236" font-size="10" fill="#195744" font-weight="700">« seule » ✓ Sarah choisit, de 17 à 21 °C</text>
          <text x="492" y="252" font-size="10" fill="#1d302a">« me proposer » : la question, à droite</text>
          <text x="492" y="268" font-size="9.5" fill="#52615a">à la validation : « un courriel l'oriente »</text>
          <line x1="750" y1="228" x2="776" y2="228" stroke="#52615a" stroke-width="1.5" marker-end="url(#origine-g)"/>
          <rect x="780" y="180" width="204" height="96" rx="10" fill="#ffffff" stroke="#1d302a" stroke-width="1.6"/>
          <text x="792" y="202" font-size="12.5" fill="#1d302a" font-weight="700">Demander aux parents</text>
          <text x="792" y="220" font-size="10" fill="#1d302a">« Je baisse le chauffage ? »</text>
          <rect x="792" y="228" width="74" height="20" rx="5" fill="#f1f4ee" stroke="#1d302a" stroke-width="1.2"/>
          <text x="800" y="242" font-size="10" fill="#1d302a" font-weight="700">Personne</text>
          <text x="792" y="266" font-size="10" fill="#195744" font-weight="700">✓ permis, après la réponse</text>
          <text x="16" y="306" font-size="10.5" fill="#52615a">Le masque d'origine suit les données et les conditions jusqu'aux actions. Le compilateur tient le plancher : ouvrir, déverrouiller, désarmer, une pièce</text>
          <text x="16" y="322" font-size="10.5" fill="#52615a">protégée, agir auprès d'un tiers. Au-dessus, le réglage de la famille décide si Sarah agit seule ; la garde reçoit le même masque, en défense de profondeur.</text>
        </svg>
      </figure>
''')

ecrire("zone", r'''      <figure class="diagram">
        <svg viewBox="0 0 1000 500" role="img" aria-labelledby="zone-t zone-d" font-family="Avenir Next, Avenir, Segoe UI, sans-serif">
          <title id="zone-t">Le dossier d'une démarche, de bout en bout</title>
          <desc id="zone-d">Le courriel trié, de 12:05:00 à 12:05:06, sur trois lignes. La fiche de 64 octets, inchangée. La zone de travail, où le dossier reçoit le courriel (produit 0, Dehors), la fiche remplie par Sarah (produits 1 à 4, Sarah et Dehors) et l'identifiant du rendez-vous (produit 5, Service). Le journal du moteur, où les produits et les demandes descendent par lots, sans attendre, et où les intentions de noter à l'agenda et de prévenir sont attendues sur le disque avant l'envoi. Le dossier a pris au départ la place que le compilateur a calculée ; à la fin, elle est rendue à la zone, et le dossier reste 30 jours au journal, puis il est réduit à ses codes.</desc>
          <defs>
            <marker id="zone-g" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#52615a"/></marker>
          </defs>
          <rect x="160" y="24" width="112" height="36" rx="6" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="216" y="38" font-size="9.5" fill="#52615a" text-anchor="middle">12:05:00</text>
          <text x="216" y="53" font-size="10.5" fill="#1d302a" font-weight="700" text-anchor="middle">courriel reçu</text>
          <rect x="278" y="24" width="112" height="36" rx="6" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="334" y="38" font-size="9.5" fill="#52615a" text-anchor="middle">12:05:00</text>
          <text x="334" y="53" font-size="10.5" fill="#1d302a" font-weight="700" text-anchor="middle">Sarah remplit</text>
          <rect x="396" y="24" width="112" height="36" rx="6" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="452" y="38" font-size="9.5" fill="#52615a" text-anchor="middle">12:05:04</text>
          <text x="452" y="53" font-size="10.5" fill="#1d302a" font-weight="700" text-anchor="middle">fiche remplie</text>
          <rect x="514" y="24" width="112" height="36" rx="6" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="570" y="38" font-size="9.5" fill="#52615a" text-anchor="middle">12:05:04</text>
          <text x="570" y="53" font-size="10.5" fill="#1d302a" font-weight="700" text-anchor="middle">Selon : École</text>
          <rect x="632" y="24" width="112" height="36" rx="6" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="688" y="38" font-size="9.5" fill="#52615a" text-anchor="middle">12:05:04</text>
          <text x="688" y="53" font-size="10.5" fill="#1d302a" font-weight="700" text-anchor="middle">Noter à l'agenda</text>
          <rect x="750" y="24" width="112" height="36" rx="6" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="806" y="38" font-size="9.5" fill="#52615a" text-anchor="middle">12:05:05</text>
          <text x="806" y="53" font-size="10.5" fill="#1d302a" font-weight="700" text-anchor="middle">Prévenir</text>
          <rect x="868" y="24" width="112" height="36" rx="6" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="924" y="38" font-size="9.5" fill="#52615a" text-anchor="middle">12:05:06</text>
          <text x="924" y="53" font-size="10.5" fill="#1d302a" font-weight="700" text-anchor="middle">fin</text>
          <rect x="16" y="70" width="134" height="40" rx="8" fill="#ffffff" stroke="#1d302a" stroke-width="1.6"/>
          <text x="28" y="86" font-size="11.5" fill="#1d302a" font-weight="700">La fiche · 64 o</text>
          <text x="28" y="101" font-size="9.5" fill="#52615a" font-style="italic">où elle en est</text>
          <rect x="160" y="70" width="820" height="40" rx="6" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="172" y="94" font-size="10" fill="#1d302a">curseur, réveil, demande en vol, génération, et la cause (le courriel : un code) : inchangée, comme pour un réflexe</text>
          <rect x="16" y="124" width="134" height="126" rx="8" fill="#ffffff" stroke="#b07d12" stroke-width="1.6"/>
          <text x="28" y="146" font-size="11.5" fill="#b07d12" font-weight="700">La zone</text>
          <text x="28" y="161" font-size="11.5" fill="#b07d12" font-weight="700">de travail</text>
          <text x="28" y="180" font-size="9.5" fill="#52615a">en mémoire,</text>
          <text x="28" y="194" font-size="9.5" fill="#52615a">à part des fiches :</text>
          <text x="28" y="212" font-size="9.5" fill="#52615a">un dossier, pris</text>
          <text x="28" y="226" font-size="9.5" fill="#52615a">au départ, rendu</text>
          <text x="28" y="240" font-size="9.5" fill="#52615a">à la fin</text>
          <rect x="160" y="124" width="820" height="126" rx="6" fill="#ffffff" stroke="#b07d12" stroke-width="1" stroke-opacity="0.6"/>
          <rect x="163" y="132" width="106" height="82" rx="6" fill="#f1f4ee" stroke="#8b3b22" stroke-width="1.2"/>
          <text x="171" y="148" font-size="10" fill="#1d302a" font-weight="700">0 · le courriel</text>
          <text x="171" y="163" font-size="9" fill="#1d302a">sa référence</text>
          <text x="171" y="176" font-size="9" fill="#1d302a">expéditeur classé</text>
          <rect x="171" y="188" width="50" height="16" rx="4" fill="#ffffff" stroke="#8b3b22" stroke-width="1"/>
          <text x="178" y="200" font-size="9" fill="#8b3b22" font-weight="700">Dehors</text>
          <rect x="399" y="132" width="106" height="110" rx="6" fill="#f1f4ee" stroke="#8b3b22" stroke-width="1.2"/>
          <text x="407" y="148" font-size="10" fill="#1d302a" font-weight="700">1 à 4 · la fiche</text>
          <text x="407" y="163" font-size="9" fill="#1d302a">École</text>
          <text x="407" y="176" font-size="9" fill="#1d302a">vendredi 10 h</text>
          <text x="407" y="189" font-size="9" fill="#1d302a">la cadette</text>
          <text x="407" y="202" font-size="9" fill="#1d302a">l'objet, court</text>
          <rect x="407" y="212" width="86" height="16" rx="4" fill="#ffffff" stroke="#8b3b22" stroke-width="1"/>
          <text x="413" y="224" font-size="9" fill="#8b3b22" font-weight="700">Sarah + Dehors</text>
          <text x="570" y="178" font-size="9.5" fill="#52615a" text-anchor="middle">le « Selon »</text>
          <text x="570" y="192" font-size="9.5" fill="#52615a" text-anchor="middle">lit la case 1</text>
          <rect x="635" y="132" width="106" height="82" rx="6" fill="#f1f4ee" stroke="#52615a" stroke-width="1.2"/>
          <text x="643" y="148" font-size="10" fill="#1d302a" font-weight="700">5 · rendez-vous</text>
          <text x="643" y="163" font-size="9" fill="#1d302a">son identifiant,</text>
          <text x="643" y="176" font-size="9" fill="#1d302a">rendu par l'agenda</text>
          <rect x="643" y="188" width="52" height="16" rx="4" fill="#ffffff" stroke="#52615a" stroke-width="1"/>
          <text x="649" y="200" font-size="9" fill="#52615a" font-weight="700">Service</text>
          <text x="806" y="170" font-size="9.5" fill="#52615a" text-anchor="middle">le message :</text>
          <text x="806" y="184" font-size="9.5" fill="#52615a" text-anchor="middle">gabarit validé</text>
          <text x="806" y="198" font-size="9.5" fill="#52615a" text-anchor="middle">et cases 2, 3</text>
          <text x="924" y="172" font-size="9.5" fill="#b07d12" font-weight="700" text-anchor="middle">la place</text>
          <text x="924" y="186" font-size="9.5" fill="#52615a" text-anchor="middle">est rendue</text>
          <text x="924" y="200" font-size="9.5" fill="#52615a" text-anchor="middle">à la zone</text>
          <rect x="16" y="262" width="134" height="132" rx="8" fill="#ffffff" stroke="#1d302a" stroke-width="1.6"/>
          <text x="28" y="284" font-size="11.5" fill="#1d302a" font-weight="700">Le journal</text>
          <text x="28" y="299" font-size="11.5" fill="#1d302a" font-weight="700">du moteur</text>
          <text x="28" y="318" font-size="9.5" fill="#52615a">la base : fait foi,</text>
          <text x="28" y="332" font-size="9.5" fill="#52615a">relu au redémarrage,</text>
          <text x="28" y="346" font-size="9.5" fill="#52615a">reçu par la secours</text>
          <rect x="160" y="262" width="820" height="132" rx="6" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <rect x="163" y="274" width="106" height="28" rx="5" fill="#ffffff" stroke="#52615a" stroke-width="1.2" stroke-dasharray="4 3"/>
          <text x="216" y="292" font-size="9.5" fill="#1d302a" text-anchor="middle">départ, produit 0</text>
          <rect x="281" y="274" width="106" height="28" rx="5" fill="#ffffff" stroke="#52615a" stroke-width="1.2" stroke-dasharray="4 3"/>
          <text x="334" y="292" font-size="9.5" fill="#1d302a" text-anchor="middle">demande partie</text>
          <rect x="399" y="274" width="106" height="28" rx="5" fill="#ffffff" stroke="#52615a" stroke-width="1.2" stroke-dasharray="4 3"/>
          <text x="452" y="292" font-size="9.5" fill="#1d302a" text-anchor="middle">produits 1 à 4</text>
          <text x="570" y="292" font-size="9.5" fill="#52615a" font-style="italic" text-anchor="middle">rien à écrire</text>
          <rect x="635" y="270" width="106" height="40" rx="5" fill="#ffffff" stroke="#1d302a" stroke-width="2.4"/>
          <text x="688" y="286" font-size="9.5" fill="#1d302a" font-weight="700" text-anchor="middle">intention + clé</text>
          <text x="688" y="300" font-size="9" fill="#1d302a" font-style="italic" text-anchor="middle">attendue</text>
          <rect x="635" y="318" width="106" height="28" rx="5" fill="#ffffff" stroke="#52615a" stroke-width="1.2" stroke-dasharray="4 3"/>
          <text x="688" y="336" font-size="9.5" fill="#1d302a" text-anchor="middle">reçu : produit 5</text>
          <rect x="753" y="270" width="106" height="40" rx="5" fill="#ffffff" stroke="#1d302a" stroke-width="2.4"/>
          <text x="806" y="286" font-size="9.5" fill="#1d302a" font-weight="700" text-anchor="middle">intention</text>
          <text x="806" y="300" font-size="9" fill="#1d302a" font-style="italic" text-anchor="middle">attendue</text>
          <rect x="753" y="318" width="106" height="28" rx="5" fill="#ffffff" stroke="#52615a" stroke-width="1.2" stroke-dasharray="4 3"/>
          <text x="806" y="336" font-size="9.5" fill="#1d302a" text-anchor="middle">reçu</text>
          <rect x="871" y="274" width="106" height="28" rx="5" fill="#ffffff" stroke="#52615a" stroke-width="1.2" stroke-dasharray="4 3"/>
          <text x="924" y="292" font-size="9.5" fill="#1d302a" text-anchor="middle">fin</text>
          <text x="924" y="330" font-size="9" fill="#52615a" text-anchor="middle">gardé 30 jours,</text>
          <text x="924" y="344" font-size="9" fill="#52615a" text-anchor="middle">puis réduit</text>
          <text x="924" y="358" font-size="9" fill="#52615a" text-anchor="middle">à ses codes</text>
          <line x1="216" y1="214" x2="216" y2="271" stroke="#52615a" stroke-width="1.4" stroke-dasharray="4 3" marker-end="url(#zone-g)"/>
          <line x1="452" y1="242" x2="452" y2="271" stroke="#52615a" stroke-width="1.4" stroke-dasharray="4 3" marker-end="url(#zone-g)"/>
          <rect x="16" y="410" width="44" height="16" rx="4" fill="#ffffff" stroke="#52615a" stroke-width="1.2" stroke-dasharray="4 3"/>
          <text x="68" y="422" font-size="10" fill="#52615a">une ligne dans le lot suivant, sous 100 ms, sans attendre : une panne avant le lot ne perd que ce qui se redemande</text>
          <rect x="16" y="434" width="44" height="16" rx="4" fill="#ffffff" stroke="#1d302a" stroke-width="2.4"/>
          <text x="68" y="446" font-size="10" fill="#52615a">attendue sur le disque avant l'envoi : un engagement ne part jamais sans trace</text>
          <text x="16" y="474" font-size="10.5" fill="#52615a">Le dossier vit en mémoire pendant que la démarche tourne ; ce qu'il obtient descend au journal du moteur, qui fait foi. Si la panne tombe entre</text>
          <text x="16" y="490" font-size="10.5" fill="#52615a">deux lignes, la reprise relit le journal : l'agenda a sa clé et son reçu, il n'est pas refait ; un message parti sans reçu est « non confirmé ».</text>
        </svg>
      </figure>
''')

ecrire("pont", r'''      <figure class="diagram">
        <svg viewBox="0 0 1000 420" role="img" aria-labelledby="pont-t pont-d" font-family="Avenir Next, Avenir, Segoe UI, sans-serif">
          <title id="pont-t">Un module de chaque côté du lien</title>
          <desc id="pont-d">Côté maison, le module du pont garde le journal du moteur, c'est-à-dire les demandes sans issue, les renvoie au retour du lien avec la même clé, et écarte une issue déjà rangée. Côté lointain, un module garde ses issues jusqu'à l'accusé et reconnaît une demande déjà traitée. Pendant une coupure, chacun garde ; au retour, chacun renvoie ce qui n'a pas d'accusé. La machine de Sarah, la secours et le téléphone ont un module de chaque côté ; un service en ligne n'en a pas chez lui : la clé va dans l'objet, et le module relit pour constater.</desc>
          <defs>
            <marker id="pont-g" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#52615a"/></marker>
          </defs>
          <rect x="16" y="30" width="330" height="196" rx="12" fill="#ffffff" stroke="#1d302a" stroke-width="2.2"/>
          <text x="28" y="54" font-size="14" fill="#1d302a" font-weight="700">Côté maison</text>
          <text x="334" y="54" font-size="9.5" fill="#52615a" text-anchor="end">le moteur, la zone de travail</text>
          <rect x="28" y="68" width="306" height="146" rx="8" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="40" y="88" font-size="11" fill="#195744" font-weight="700">Le module du pont, côté maison</text>
          <text x="40" y="108" font-size="10" fill="#1d302a">garde : le journal du moteur, c'est-à-dire</text>
          <text x="40" y="122" font-size="10" fill="#1d302a">les demandes sans issue, dans l'ordre</text>
          <text x="40" y="142" font-size="10" fill="#1d302a">renvoie, au retour du lien, ce qui n'a</text>
          <text x="40" y="156" font-size="10" fill="#1d302a">pas d'accusé, avec la même clé</text>
          <text x="40" y="176" font-size="10" fill="#1d302a">reconnaît : (case, génération, rang) ;</text>
          <text x="40" y="190" font-size="10" fill="#1d302a">une issue déjà rangée est écartée</text>
          <rect x="654" y="30" width="330" height="196" rx="12" fill="#ffffff" stroke="#1d302a" stroke-width="2.2"/>
          <text x="666" y="54" font-size="14" fill="#1d302a" font-weight="700">Côté lointain</text>
          <text x="972" y="54" font-size="9.5" fill="#52615a" text-anchor="end">Sarah, la secours, le téléphone</text>
          <rect x="666" y="68" width="306" height="146" rx="8" fill="#f1f4ee" stroke="#d4ddd4" stroke-width="1"/>
          <text x="678" y="88" font-size="11" fill="#b07d12" font-weight="700">Le module du pont, côté lointain</text>
          <text x="678" y="108" font-size="10" fill="#1d302a">garde : ses issues, jusqu'à l'accusé</text>
          <text x="678" y="122" font-size="10" fill="#1d302a">(la réponse de Sarah, le « Oui » tapé)</text>
          <text x="678" y="142" font-size="10" fill="#1d302a">renvoie, au retour du lien, ce qui n'a</text>
          <text x="678" y="156" font-size="10" fill="#1d302a">pas d'accusé</text>
          <text x="678" y="176" font-size="10" fill="#1d302a">reconnaît : les clés déjà vues ; une</text>
          <text x="678" y="190" font-size="10" fill="#1d302a">demande renvoyée rend la même issue</text>
          <line x1="346" y1="102" x2="650" y2="102" stroke="#52615a" stroke-width="1.6" marker-end="url(#pont-g)"/>
          <text x="498" y="94" font-size="10" fill="#52615a" text-anchor="middle">une demande et sa clé</text>
          <line x1="654" y1="152" x2="350" y2="152" stroke="#52615a" stroke-width="1.6" marker-end="url(#pont-g)"/>
          <text x="498" y="170" font-size="10" fill="#52615a" text-anchor="middle">une issue et sa clé, puis l'accusé</text>
          <polyline points="624,80 608,98 626,116 608,134 624,160" fill="none" stroke="#8b3b22" stroke-width="2.4"/>
          <text x="498" y="196" font-size="10" fill="#8b3b22" font-weight="700" text-anchor="middle">coupure : chacun garde</text>
          <text x="498" y="210" font-size="10" fill="#8b3b22" text-anchor="middle">au retour, chacun renvoie</text>
          <text x="498" y="224" font-size="10" fill="#8b3b22" text-anchor="middle">ce qui n'a pas d'accusé</text>
          <text x="16" y="262" font-size="11.5" fill="#1d302a" font-weight="700">Qui tient le côté lointain</text>
          <text x="16" y="284" font-size="10" fill="#1d302a" font-weight="700">la machine de Sarah (un Mac, un NAS)</text>
          <text x="330" y="284" font-size="10" fill="#1d302a">un module à nous, à côté de Sarah : il garde ses réponses et reconnaît une demande déjà traitée</text>
          <line x1="16" y1="294" x2="984" y2="294" stroke="#d4ddd4" stroke-width="1"/>
          <text x="16" y="310" font-size="10" fill="#1d302a" font-weight="700">la machine de secours</text>
          <text x="330" y="310" font-size="10" fill="#1d302a">le pont vers la secours (V7 § 18), que l'étude de haute disponibilité prolonge d'un pont à reprise</text>
          <line x1="16" y1="320" x2="984" y2="320" stroke="#d4ddd4" stroke-width="1"/>
          <text x="16" y="336" font-size="10" fill="#1d302a" font-weight="700">le téléphone, pour « Demander »</text>
          <text x="330" y="336" font-size="10" fill="#1d302a">l'application : elle garde le « Oui » jusqu'à l'accusé ; la revérification dit s'il s'applique encore</text>
          <line x1="16" y1="346" x2="984" y2="346" stroke="#d4ddd4" stroke-width="1"/>
          <text x="16" y="362" font-size="10" fill="#1d302a" font-weight="700">un service en ligne (agenda, école)</text>
          <text x="330" y="362" font-size="10" fill="#1d302a">personne de notre côté : la clé va dans l'objet (l'identifiant du rendez-vous), et le module relit pour constater</text>
          <text x="16" y="394" font-size="10.5" fill="#52615a">Celui qui envoie garde jusqu'à l'accusé ; celui qui reçoit reconnaît ce qu'il a déjà vu. Côté maison, le tampon est le journal du moteur :</text>
          <text x="16" y="410" font-size="10.5" fill="#52615a">il n'y a pas de seconde file à tenir d'accord avec la première. Quand le lien ne mène pas chez nous, c'est l'objet qui porte la clé.</text>
        </svg>
      </figure>
''')

# ================================================================ Monitoring (docs/proposition/monitoring.md)
BLANC = "#ffffff"


def puce(d, x, y, w, s, coul=VERT, fond=BLANC, taille=9.5):
    d.rect(x, y, w, 24, fond, coul, 12)
    d.t(x + w / 2, y + 16, s, taille, True, coul if fond == BLANC else BLANC, "middle")


# ---------------- les trois étages
d = Dessin("etages")
d.rect(16, 36, 624, 334, BLANC, ENCRE, 12, 2.2)
d.titre(30, 58, "L'équipement : la machine de Sarah Home", ENCRE)
d.rect(32, 72, 380, 214, DOUX, LIGNE, 8)
d.t(44, 92, "Le cœur", 12.5, True)
d.t(400, 92, "moli-os serve : un processus", 9.5, coul=GRIS, ancre="end", mono=True)
for i, nom in enumerate(["hub", "courtier", "moteur", "API, écrans", "pont maison", "Hue", "Tuya", "Sonos"]):
    puce(d, 44 + (i % 4) * 90, 106 + (i // 4) * 34, 82, nom, VERT if i < 5 else VERT2)
d.t(44, 196, "Chaque module dans le cœur : son fil et son runtime (V7 § 20.1).", 9.8)
d.t(44, 214, "CPU : le temps noyau du fil ; réveils et temps occupé du runtime.", 9.8, coul=GRIS)
d.t(44, 230, "Mémoire : tenue par le kit, étiquetée, ou mesurée en l'isolant.", 9.8, coul=GRIS)
d.t(44, 246, "Messages : comptés par le kit (un seul lien vers le courtier).", 9.8, coul=GRIS)
d.t(44, 270, "Le processus : /proc/<pid> et la mémoire anonyme du cgroup.", 9.8, True, VERT)
d.rect(424, 72, 200, 100, DOUX, LIGNE, 8)
d.t(436, 92, "Un module isolé", 12.5, True)
d.t(436, 106, "moli-os module run cloud-x", 8.8, coul=GRIS, mono=True)
puce(d, 436, 116, 82, "cloud-x", AMBRE)
d.t(436, 158, "tout se lit dans son processus", 9.8, coul=GRIS)
d.rect(424, 184, 200, 102, DOUX, LIGNE, 8)
d.t(436, 204, "Les voisins", 12.5, True)
d.t(436, 222, "Zigbee2MQTT, l'archive, Wyoming :", 9.8)
d.t(436, 238, "leur processus quand on le voit ;", 9.8, coul=GRIS)
d.t(436, 254, "ce qu'ils disent d'eux-mêmes", 9.8, coul=GRIS)
d.t(436, 270, "(bridge/health)", 9.8, coul=GRIS, mono=True)
d.rect(32, 298, 592, 56, BLANC, AMBRE, 8, 1.5, "5 4")
d.t(44, 320, "Le gardien", 11.5, True, AMBRE)
d.t(44, 338, "lance les modules isolés et garde ce qui doit survivre au cœur (document Redémarrage à chaud)", 9.8, coul=GRIS)
d.titre(660, 58, "D'où vient chaque chiffre")
blocs = [("Équipement", ["/proc/stat · meminfo · loadavg", "/proc/pressure · /proc/diskstats", "/sys/class/hwmon · get_throttled"]),
         ("Processus", ["/proc/<pid>/stat · status", "smaps_rollup (Pss) · io", "cgroup : memory.stat (anon)"]),
         ("Module dans le cœur", ["/proc/self/task/<tid>/schedstat", "tokio : occupé, réveils", "compteurs du kit · moli-net"]),
         ("Module isolé", ["son processus, comme ci-dessus", "sa connexion au courtier"])]
y = 82
for titre_, lignes_ in blocs:
    d.t(660, y, titre_, 11, True, VERT)
    for j, l in enumerate(lignes_):
        d.t(660, y + 17 + j * 15, l, 9.3, mono=True)
    y += 30 + 15 * len(lignes_)
d.t(16, 394, "Hors de la machine, les équipements disent eux-mêmes leur charge : routeurs ASUSWRT et Netgear, Frigate, Zigbee2MQTT, les PC qui ont l'agent Moli.", 10.5, coul=GRIS)
d.t(16, 412, "Aucune de ces lectures ne demande de privilège : un conteneur ordinaire lit /proc, /sys et son propre cgroup (vérifié).", 10.5, coul=GRIS)
ecrire("etages", figure("etages", "Trois étages, une source de vérité chacun",
                        "La machine contient le cœur, un module isolé et les voisins ; le cœur contient ses modules, chacun sur son fil. "
                        "À droite, les fichiers du noyau d'où vient chaque chiffre, étage par étage.",
                        d.texte(), 420, "L'équipement se lit dans <code>/proc</code> et <code>/sys</code>&nbsp;; le processus dans "
                        "<code>/proc/&lt;pid&gt;</code>&nbsp;; le module selon sa place&nbsp;: isolé, c'est son processus&nbsp;; dans le cœur, "
                        "c'est son fil et ses compteurs.", fleches=False))

# ---------------- où se comptent les messages
d = Dessin("compter")
d.rect(16, 36, 380, 242, DOUX, LIGNE, 10)
d.t(30, 58, "Le cœur", 12.5, True)
for i, nom in enumerate(["Hue", "Tuya", "Sonos", "pont maison"]):
    puce(d, 32, 74 + i * 44, 110, nom, VERT)
    d.ligne(142, 86 + i * 44, 206, 86 + i * 44, GRIS, 1.3)
d.rect(208, 70, 168, 180, BLANC, AMBRE, 8, 1.6)
d.t(220, 90, "Le kit compte", 11.5, True, AMBRE)
for j, l in enumerate(["publiés, octets", "ordres reçus, issues", "doublons écartés", "appels d'API, erreurs", "pire rafale sur 100 ms"]):
    d.t(220, 112 + j * 18, "· " + l, 9.8)
d.t(220, 222, "une addition atomique", 9.3, coul=GRIS)
d.t(220, 236, "par message", 9.3, coul=GRIS)
d.rect(452, 104, 156, 110, VERT, VERT, 12)
d.t(530, 146, "Le courtier", 13.5, True, BLANC, "middle")
d.t(530, 164, "moli-courtier", 9.5, coul=BLANC, ancre="middle", mono=True)
d.t(530, 190, "compte par connexion", 9.3, coul=BLANC, ancre="middle")
d.ligne(376, 160, 448, 160, VERT, 2.2, fleche="v")
d.t(412, 150, "un seul lien", 9.3, True, VERT, "middle", halo=True)
d.t(412, 176, "en mémoire", 9.3, coul=GRIS, ancre="middle", halo=True)
droite = [("cloud-x, isolé", AMBRE, 52, "entrées, sorties, octets, pertes"),
          ("Sarah, isolée", AMBRE, 122, "entrées, sorties, octets, pertes"),
          ("Zigbee2MQTT, extérieur", GRIS, 192, "et son bridge/health : par appareil")]
for nom, coul, yy, note in droite:
    d.rect(680, yy, 300, 42, BLANC, coul, 8, 1.5)
    d.t(692, yy + 17, nom, 10.5, True, coul)
    d.t(692, yy + 33, note, 9.2, coul=GRIS)
    d.ligne(678, yy + 21, 612, 160 + (yy - 122) * 0.35, coul, 1.5, fleche="g")
d.t(16, 304, "Dans le cœur, le courtier ne voit qu'un lien : c'est le kit qui compte, module par module. Isolé ou extérieur, chacun a sa", 10.5, coul=GRIS)
d.t(16, 320, "connexion : c'est le courtier qui compte. rumqttd 0.20 tient déjà un compteur d'entrée par connexion, sans l'exposer ;", 10.5, coul=GRIS)
d.t(16, 336, "notre version l'expose, compte aussi la sortie, retire la table par sujet qui fuit, et signale chaque perte.", 10.5, coul=GRIS)
ecrire("compter", figure("compter", "Où se comptent les messages MQTT",
                         "À gauche, les modules du cœur passent par le kit, qui compte, puis par un seul lien en mémoire. À droite, chaque module "
                         "isolé et chaque client extérieur a sa connexion, que le courtier compte.",
                         d.texte(), 350, "Le kit compte pour les modules du cœur&nbsp;; le courtier compte pour les autres. La pire rafale "
                         "sur 100&nbsp;ms se garde à côté du débit&nbsp;: c'est elle qui fait décrocher (V7 §&nbsp;17.1.2)."))

# ---------------- la bascule d'un module
d = Dessin("bascule")
phases = [("Retenir", 130, 270), ("Finir", 270, 420), ("Passer", 420, 560), ("Lancer", 560, 730), ("Reprendre", 730, 984)]
for nom, x0, x1 in phases:
    d.t((x0 + x1) / 2, 40, nom, 11.5, True, ENCRE, "middle")
    d.ligne(x0, 50, x0, 232, LIGNE, 1, "3 4")
lanes = [("Le hub", 74), ("Dans le cœur", 132), ("Isolé", 190)]
for nom, yy in lanes:
    d.t(16, yy + 18, nom, 11, True, GRIS)
d.rect(132, 74, 596, 28, AMBRE2, AMBRE, 6)
d.t(430, 93, "ordres retenus dans leurs voies, échéance inchangée · appareils « en bascule », jamais « hors ligne »", 9.6, True, ENCRE, "middle")
d.rect(732, 74, 250, 28, VERT, VERT, 6)
d.t(857, 93, "libère les ordres encore valables", 9.6, True, BLANC, "middle")
d.rect(132, 132, 286, 28, VERT2, VERT, 6)
d.t(275, 151, "l'ancienne : finit, vide son tampon", 9.6, True, ENCRE, "middle")
d.rect(424, 132, 132, 28, BLANC, AMBRE, 6, 1.6)
d.t(490, 151, "rend son état ≤ 4 Kio", 9.6, True, AMBRE, "middle")
d.chemin("M 556,146 C 590,146 560,204 600,204", AMBRE, 1.6, None, "a")
d.rect(604, 190, 378, 28, VERT, VERT, 6)
d.t(793, 209, "la nouvelle instance : se connecte, republie $vu", 9.6, True, BLANC, "middle")
d.t(16, 258, "Jamais deux instances à la fois : deux écrivains pour les mêmes appareils, et un identifiant MQTT qui se chasse lui-même.", 10.5, coul=GRIS)
d.t(16, 275, "Retour à l'ancienne place si la nouvelle instance n'est pas prête en 30 s ; pas de bascule pendant une rafale.", 10.5, coul=GRIS)
d.t(16, 292, "Durée : la reconnexion du module à ses appareils, d'une fraction de seconde pour un appareil local à quelques secondes pour un cloud.", 10.5, coul=GRIS)
ecrire("bascule", figure("bascule", "Passer du cœur à l'isolé, à chaud",
                         "Cinq étapes, de gauche à droite. Le hub retient les ordres pendant que l'ancienne instance finit et rend son état ; la "
                         "nouvelle démarre dans l'autre place, puis le hub libère les ordres dont l'échéance court encore.",
                         d.texte(), 304, "Le même module, compilé dans le binaire, tourne aussi dans son propre processus (V7&nbsp;§&nbsp;9.4.2)&nbsp;: "
                         "changer de place ne demande pas de redémarrer le cœur. Le sens inverse, de l'isolé vers le cœur, est le même."))

# ---------------- la page Système (exemple, valeurs fictives)
d = Dessin("systeme")
d.rect(16, 18, 968, 46, DOUX, VERT, 8, 1.4)
d.t(30, 47, "La maison tient", 13.5, True, VERT)
d.t(160, 47, "mémoire anonyme 9,1 Mio sur 32 · CPU au repos 0,21 % · pression 0 % · aucune alerte", 11)
d.t(970, 47, "valeurs d'exemple", 9.5, coul=GRIS, ancre="end", italique=True)
cols = [("Module", 24), ("Place", 150), ("CPU", 232), ("Mémoire", 300), ("Messages/s", 470), ("Rafale", 570),
        ("Erreurs", 650), ("Relances 24 h", 730), ("Budget", 850)]
for nom, x in cols:
    d.t(x, 90, nom, 10, True, GRIS)
d.ligne(16, 98, 984, 98, LIGNE, 1)
NAT = {"tenue": (GRIS, "estimée"), "étiquetée": (AMBRE, "étiquetée"), "mesurée": (VERT, "mesurée")}
rangs = [("hub", "cœur", "0,4 %", "2,1 Mio", "tenue", "3,8", "41", "0", "0", 0.30),
         ("courtier", "cœur", "0,3 %", "2,7 Mio", "tenue", "7,6", "96", "0 perte", "0", 0.40),
         ("hue", "cœur", "0,1 %", "410 Kio", "étiquetée", "0,6", "12", "0", "0", 0.20),
         ("tuya", "isolé", "0,2 %", "1,3 Mio", "mesurée", "0,9", "8", "2 délais", "1", 0.45),
         ("sarah", "isolé", "0,0 %", "4,8 Mio", "mesurée", "0,1", "1", "0", "0", 0.60),
         ("cloud-x", "isolé", "1,9 %", "7,4 Mio", "mesurée", "2,4", "380", "14", "3", 1.10)]
for i, (m, place, cpu, mem, nat, msg, raf, err, rel, part) in enumerate(rangs):
    yy = 122 + i * 32
    alerte = part > 1
    d.t(24, yy, m, 11, True, ROUGE if alerte else ENCRE, mono=True)
    d.rect(150, yy - 15, 56, 21, VERT if place == "cœur" else BLANC, VERT if place == "cœur" else AMBRE, 10)
    d.t(178, yy, place, 9.5, True, BLANC if place == "cœur" else AMBRE, "middle")
    d.t(232, yy, cpu, 10.5)
    d.t(300, yy, mem, 10.5, alerte, ROUGE if alerte else ENCRE)
    coul, lib = NAT[nat]
    d.rect(372, yy - 14, 72, 19, BLANC, coul, 9)
    d.t(408, yy, lib, 9, True, coul, "middle")
    d.t(470, yy, msg, 10.5)
    d.t(570, yy, raf, 10.5, alerte, ROUGE if alerte else ENCRE)
    d.t(650, yy, err, 10.5)
    d.t(730, yy, rel, 10.5, alerte, ROUGE if alerte else ENCRE)
    d.rect(850, yy - 11, 120, 10, DOUX, LIGNE, 5)
    d.rect(850, yy - 11, 120 * min(part, 1.0), 10, ROUGE if alerte else VERT, ROUGE if alerte else VERT, 5)
    d.ligne(16, yy + 11, 984, yy + 11, LIGNE, 0.8)
gestes = [("Relancer", VERT), ("Relancer isolé", AMBRE), ("Relancer dans le cœur · PIN", VERT), ("Mesurer exactement (24 h)", GRIS)]
x = 24
for nom, coul in gestes:
    w = 20 + 6.4 * len(nom)
    d.rect(x, 324, w, 28, BLANC, coul, 14, 1.5)
    d.t(x + w / 2, 342, nom, 10, True, coul, "middle")
    x += w + 14
d.t(x + 8, 342, "← la fiche d'un module", 10, coul=GRIS)
d.t(16, 378, "La nature de chaque mémoire est dite : mesurée (son processus), étiquetée (binaire de mesure), estimée (ce que tient le kit).", 10.5, coul=GRIS)
d.t(16, 395, "Ici, cloud-x dépasse son budget de mémoire et de rafale : une alerte à l'astreinte ; à deux fois le budget, il resterait isolé à sa prochaine relance.", 10.5, coul=GRIS)
ecrire("systeme", figure("systeme", "La page Système, avec ses modules",
                         "Une ligne dit si la maison tient. Dessous, un module par ligne : sa place, son CPU, sa mémoire et sa nature, ses messages, "
                         "sa pire rafale, ses erreurs, ses relances et sa barre de budget. Puis les gestes de la fiche d'un module.",
                         d.texte(), 408, "Exemple aux valeurs fictives. Lire&nbsp;: tout le foyer et les agents&nbsp;; rentrer un module dans le cœur&nbsp;: "
                         "une session PIN&nbsp;; un agent qui le demande passe par la garde.", fleches=False))

# ================================================================ Redémarrage à chaud (docs/proposition/redemarrage-a-chaud.md)
# ---------------- le gardien
d = Dessin("gardien")
d.rect(16, 36, 452, 300, BLANC, AMBRE, 12, 2.2)
d.t(30, 60, "Ce qui reste : le gardien", 14, True)
d.t(456, 60, "moli-os gardien", 9.5, coul=GRIS, ancre="end", mono=True)
d.rect(32, 76, 420, 60, DOUX, LIGNE, 8)
d.t(44, 96, "Les portes", 11.5, True, VERT)
d.t(44, 113, "8790 : l'API et les écrans · 1883 : le courtier", 10)
d.t(44, 128, "un client qui se connecte attend dans la file du noyau", 9.5, coul=GRIS)
d.rect(32, 146, 420, 72, DOUX, LIGNE, 8)
d.t(44, 166, "Le relais et la file", 11.5, True, VERT)
d.t(44, 183, "l'état complet du cœur, dans un memfd scellé : 233 Kio à ×1", 10)
d.t(44, 199, "la file du pont : 10 000 sorties, 0,45 Mio", 10)
d.t(44, 212, "aucun secret : le nouveau cœur relit le coffre", 9.5, coul=GRIS)
d.rect(32, 228, 420, 94, DOUX, LIGNE, 8)
d.t(44, 248, "Les modules isolés : ses enfants", 11.5, True, VERT)
for i, nom in enumerate(["Sarah", "cloud-x", "tuya"]):
    puce(d, 44 + i * 96, 258, 86, nom, AMBRE)
d.t(44, 300, "ils gardent leurs connexions et leurs tampons, et se", 9.8, coul=GRIS)
d.t(44, 314, "reconnectent à une porte qui n'a jamais fermé", 9.8, coul=GRIS)
d.rect(532, 36, 452, 300, BLANC, ENCRE, 12, 2.2)
d.t(546, 60, "Ce qui redémarre : le cœur", 14, True)
d.t(972, 60, "moli-os serve", 9.5, coul=GRIS, ancre="end", mono=True)
for k, (titre_, note, y0) in enumerate([("L'ancien cœur", "se fige au rang N, confie son état, rend la base, sort", 76),
                                       ("Le nouveau cœur", "prêt en reprise avant la coupure ; relit, reprend au rang N", 206)]):
    d.rect(548, y0, 420, 116, DOUX, LIGNE, 8)
    d.t(560, y0 + 20, titre_, 11.5, True, GRIS if k == 0 else VERT)
    for i, nom in enumerate(["hub", "courtier", "moteur", "API", "Hue", "Sonos"]):
        puce(d, 560 + (i % 4) * 100, y0 + 32 + (i // 4) * 32, 90, nom, GRIS if k == 0 else VERT)
    d.t(560, y0 + 108, note, 9.6, coul=GRIS)
d.ligne(546, 150, 456, 176, AMBRE, 1.8, fleche="a")
d.t(500, 150, "confier", 9.6, True, AMBRE, "middle", halo=True)
d.ligne(456, 198, 544, 262, VERT, 1.8, fleche="v")
d.t(510, 216, "reprendre", 9.6, True, VERT, "middle", halo=True)
d.chemin("M 454,100 C 500,100 506,226 544,226", VERT, 1.4, "5 4", "v")
d.t(500, 92, "les portes", 9.6, True, VERT, "middle", halo=True)
d.ligne(454, 290, 544, 296, GRIS, 1.3, "4 4", "g")
d.t(500, 284, "reconnexion", 9.2, coul=GRIS, ancre="middle", halo=True)
d.t(16, 364, "Le gardien ne parle jamais MQTT, ne tient aucun secret, ne décide rien pour la maison, et ne renouvelle jamais le bail à la place du cœur.", 10.5, coul=GRIS)
d.t(16, 381, "Sous systemd, le dépôt de descripteurs garde aussi ses propres portes et son relais ; sous Docker, il est le processus principal du conteneur.", 10.5, coul=GRIS)
ecrire("gardien", figure("gardien", "Ce qui reste, ce qui redémarre",
                         "À gauche, le gardien garde les portes, le relais et la file, et les modules isolés. À droite, l'ancien cœur confie son état "
                         "au relais et sort ; le nouveau, prêt en reprise, reprend l'état, les portes, puis voit les modules se reconnecter.",
                         d.texte(), 394, "Le contenu de la mémoire passe, pas la mémoire&nbsp;: l'état complet de l'étude HA&nbsp;v2, encodé champ par "
                         "champ, se fige en 132&nbsp;µs et se relit en 272&nbsp;µs à ×1."))

# ---------------- un redémarrage prévu, pas à pas
d = Dessin("redemarrage")
ph = [("avant la coupure", 130, 430), ("la coupure", 430, 600), ("après", 600, 984)]
for nom, x0, x1 in ph:
    d.t((x0 + x1) / 2, 36, nom, 11.5, True, ROUGE if nom == "la coupure" else ENCRE, "middle")
    d.ligne(x0, 46, x0, 340, LIGNE, 1, "3 4")
d.rect(430, 46, 170, 294, "#faf1ec", "#faf1ec", 0)
lanes = [("Ancien cœur", 62), ("Nouveau cœur", 122), ("Gardien", 182), ("Module isolé", 242), ("Écran", 302)]
for nom, yy in lanes:
    d.t(16, yy + 18, nom, 11, True, GRIS)
d.rect(132, 62, 296, 28, VERT, VERT, 6)
d.t(280, 81, "sert la maison", 9.8, True, BLANC, "middle")
d.rect(432, 62, 126, 28, AMBRE2, AMBRE, 6)
d.t(495, 81, "fige au rang N, confie", 9.4, True, ENCRE, "middle")
d.t(564, 81, "sort", 9.4, True, GRIS)
d.rect(200, 122, 228, 28, DOUX, GRIS, 6)
d.t(314, 141, "s'initialise en reprise", 9.6, True, GRIS, "middle")
d.rect(432, 122, 136, 28, BLANC, GRIS, 6, 1.2, "4 3")
d.t(500, 141, "attend la main", 9.4, coul=GRIS, ancre="middle")
d.rect(572, 122, 410, 28, VERT, VERT, 6)
d.t(777, 141, "reprend au rang N, ouvre les portes, la maison continue", 9.6, True, BLANC, "middle")
d.ligne(132, 196, 982, 196, AMBRE, 2)
for x, s in [(200, "lance le nouveau"), (560, "passe relais et portes")]:
    d.c.append(f'          <circle cx="{x}" cy="196" r="5" fill="{AMBRE}"/>')
    d.t(x, 186, s, 9.4, True, AMBRE, "middle", halo=True)
d.rect(132, 242, 296, 28, VERT2, VERT, 6)
d.t(280, 261, "garde ses connexions aux appareils", 9.6, True, ENCRE, "middle")
d.rect(432, 242, 166, 28, AMBRE2, AMBRE, 6)
d.t(515, 261, "tamponne", 9.6, True, ENCRE, "middle")
d.rect(602, 242, 380, 28, VERT2, VERT, 6)
d.t(792, 261, "se reconnecte, renvoie ce qui dépasse son accusé", 9.6, True, ENCRE, "middle")
d.rect(132, 302, 296, 28, VERT2, VERT, 6)
d.t(280, 321, "connecté", 9.6, True, ENCRE, "middle")
d.rect(432, 302, 166, 28, AMBRE2, AMBRE, 6)
d.t(515, 321, "attend dans la file", 9.6, True, ENCRE, "middle")
d.rect(602, 302, 380, 28, VERT2, VERT, 6)
d.t(792, 321, "servi, reçoit un instantané", 9.6, True, ENCRE, "middle")
d.t(16, 366, "Mesuré (banc de l'étude HA v2) : figer l'état 132 µs, le relire 272 µs à ×1. Borné : le dernier lot d'historique, 100 ms (D13) ; les ordres d'un module du cœur, 1 s.", 10.3, coul=GRIS)
d.t(16, 383, "Visé : une coupure de 1 s au plus sur la machine de production, 2 s sur un Pi Zero 2 W, sous le délai du bail : la secours ne bascule pas.", 10.3, coul=GRIS)
d.t(16, 400, "Preuve : la même empreinte au gel et à la reprise, et une ligne au journal avec le rang et la durée de la coupure.", 10.3, coul=GRIS)
ecrire("redemarrage", figure("redemarrage", "Un redémarrage prévu, pas à pas",
                             "Cinq lignes, de gauche à droite : l'ancien cœur sert puis se fige ; le nouveau s'initialise avant la coupure puis reprend ; "
                             "le gardien lance et passe la main ; le module isolé tamponne pendant la coupure ; l'écran attend puis est servi.",
                             d.texte(), 412, "La leçon d'Envoy&nbsp;: le nouveau s'initialise avant que l'ancien ne lâche. La coupure ne contient "
                             "plus que le passage de la main&nbsp;: figer, confier, rendre la base, reprendre.", fleches=False))

# ---------------- trois étages de durée
d = Dessin("durees")
pannes = ["un redémarrage prévu", "un plantage du cœur", "une coupure de courant", "la perte de la machine"]
for j, p in enumerate(pannes):
    d.t(330 + j * 166, 40, p, 10.5, True, ENCRE, "middle")
etages_ = [("La mémoire du cœur", ["non", "non", "non", "non"], ["", "", "", ""]),
           ("Le relais et la file (le gardien)", ["oui", "oui", "non", "non"], ["", "au dernier lot de 10 ms", "", ""]),
           ("L'instantané sur le disque", ["oui", "oui", "oui", "non"], ["", "au dernier instantané", "au dernier instantané", ""]),
           ("La mémoire d'un pair (le secours)", ["oui", "oui", "oui", "oui"], ["", "", "s'il vit", "au délai du pont"])]
for i, (nom, oui, note) in enumerate(etages_):
    yy = 66 + i * 48
    d.rect(16, yy, 968, 40, DOUX if i % 2 == 0 else BLANC, LIGNE, 6)
    d.t(28, yy + 25, nom, 11, True)
    for j, v in enumerate(oui):
        cx = 330 + j * 166
        ok = v == "oui"
        d.rect(cx - 75, yy + 8, 150, 24, VERT if ok else "#f6e3da", VERT if ok else ROUGE, 12)
        d.t(cx, yy + 25, ("✓ " if ok else "✗ ") + (note[j] or ("survit" if ok else "perdu")), 9.4, True, BLANC if ok else ROUGE, "middle")
d.t(16, 280, "Chaque donnée choisit son étage. Un engagement (un message, un rendez-vous) s'écrit avant, au moins dans la file du gardien, puis chez le pair.", 10.5, coul=GRIS)
d.t(16, 297, "Coût d'une écriture : rien (mémoire) · un pwrite par lot de 10 ms (file) · 3 ms avec fsync (instantané) · 36 ms au 99e centile (pair, banc HA v2).", 10.5, coul=GRIS)
ecrire("durees", figure("durees", "Trois étages de durée, et ce qui survit à quoi",
                        "Quatre lignes, quatre pannes. La mémoire du cœur ne survit à rien ; le relais et la file survivent au redémarrage et au "
                        "plantage du cœur ; l'instantané survit aussi à une coupure de courant ; la mémoire d'un pair survit même à la perte de la machine.",
                        d.texte(), 310, "Le relais comble le trou entre la mémoire, qui meurt avec le processus, et le disque, que l'étude "
                        "HA&nbsp;v2 réserve à l'instantané&nbsp;: un plantage ne coûte plus qu'un lot de 10&nbsp;ms, sans écrire sur la carte.",
                        fleches=False))

print("figures :", ", ".join(sorted(p.name.split(".")[0] for p in F.glob("*.svg.html"))))
