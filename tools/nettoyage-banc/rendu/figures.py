#!/usr/bin/env python3
"""Schémas et graphiques de la proposition « nettoyage des équipements »
(docs/proposition/nettoyage-equipements.md), en relief, avec la boîte à outils de la haute
disponibilité v2 (tools/haute-dispo-banc/rendu/relief.py). Les graphiques lisent la série du banc
(../sorties/linux/*.txt) : aucun chiffre recopié. Écrit les fragments dans rendu/fig/ (build.py les
insère dans la page) et les SVG autonomes, légende comprise, dans docs/proposition/nettoyage-equipements/.

Usage : python3 -I tools/nettoyage-banc/rendu/figures.py [schéma …]"""
import math
import pathlib
import sys

ICI = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ICI.parents[1] / "haute-dispo-banc" / "rendu"))
from relief import ENCRE, GRIS, LIGNE, TONS, Atelier, Dessin, court, fr, lire_resultats, nb  # noqa: E402

RES = lire_resultats(ICI.parent / "sorties/linux")
A = Atelier(ICI / "fig", ICI.parents[2] / "docs/proposition/nettoyage-equipements")
CHOIX = set(sys.argv[1:])


def r(sec, e, cle):
    k = (sec, e, cle)
    if k not in RES:
        raise SystemExit(f"résultat absent : {k}")
    return RES[k]


def voulu(nom):
    return not CHOIX or nom in CHOIX


def xlog(v, vmin, vmax, x0, w):
    v = max(v, vmin)
    return x0 + w * (math.log10(v) - math.log10(vmin)) / (math.log10(vmax) - math.log10(vmin))


def axe_log(d, x0, w, y0, y1, vmin, vmax, unite, ticks):
    for t_, lab in ticks:
        x = xlog(t_, vmin, vmax, x0, w)
        d.brut(f'<line x1="{x:.1f}" y1="{y0}" x2="{x:.1f}" y2="{y1}" stroke="{LIGNE}" stroke-dasharray="2 3"/>')
        d.t(x, y1 + 14, lab, 9.5, coul=GRIS, ancre="middle")
    d.t(x0 + w, y1 + 28, unite, 9.5, coul=GRIS, ancre="end", italique=True)


def bandeau(d, y, texte, ton="vert", icone="coche", h=36):
    d._t(ton)
    d.brut(f'<rect x="16" y="{y}" width="968" height="{h}" rx="12" fill="{TONS[ton][2]}" stroke="{TONS[ton][1]}" stroke-opacity=".5"/>')
    d.icone(icone, 28, y + (h - 20) / 2, 20, TONS[ton][0], 2.2)
    d.t(56, y + h / 2 + 4, texte, 11, True, ENCRE)


def case(d, x, y, w, h, ton, texte=None, plein=True, tirets=False, taille=9):
    f, c, p = d._t(ton)
    if plein:
        d.brut(f'<rect x="{x:.1f}" y="{y:.1f}" width="{w}" height="{h}" rx="5" fill="url(#{d.id}-h-{ton})"/>')
    else:
        t = ' stroke-dasharray="4 3"' if tirets else ""
        d.brut(f'<rect x="{x:.1f}" y="{y:.1f}" width="{w}" height="{h}" rx="5" fill="{p}" stroke="{c}" stroke-opacity=".8"{t}/>')
    if texte:
        d.t(x + w / 2, y + h / 2 + taille * .36, texte, taille, True, "#ffffff" if plein else f, "middle")


def pastille(d, cx, y, texte, ton, taille=10.5, icone=None):
    """Une pastille pleine, centrée, assez large pour son texte en gras."""
    from relief import largeur
    w = largeur(texte, taille) * 1.13 + 26 + (18 if icone else 0)
    h = taille + 11
    x0 = cx - w / 2
    d._t(ton)
    d.brut(f'<rect x="{x0:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="{h / 2:.1f}" fill="url(#{d.id}-h-{ton})"/>')
    tx = x0 + 13
    if icone:
        d.icone(icone, x0 + 10, y + (h - 13) / 2, 13, "#ffffff", 2)
        tx += 18
    d.t(tx, y + h / 2 + taille * .36, texte, taille, True, "#ffffff")


LOT1, LOT100 = r("lot", 1, "lot_duree"), r("lot", 100, "lot_duree")
DM1, DM100 = r("lot", 1, "demenagement_duree"), r("lot", 100, "demenagement_duree")


def duree(us):
    return court(us, "µs") if us < 1000 else court(us / 1000, "ms")


# ================================================================ Trois rythmes
if voulu("rythmes"):
    d = Dessin("rythmes")
    d.titre(16, 24, "Trois rythmes, comme une maison fait son ménage")
    etapes = [
        (170, "Retirer", "au fil de l'eau, un geste", "ambre", "croix",
         ["plus lu, plus commandé", "attentes et minuteries terminées", "ses automatismes « à revoir »", "rien n'est libéré : réversible"],
         "une sortie critique, au rang s"),
        (500, "Libérer : le lot", "par lots, la nuit", "vert", "fleches",
         ["fiches remises à zéro, nom effacé", "génération suivante", "avec l'instantané de la nuit", "les appareils neufs reprennent la place"],
         f"{duree(LOT1)} à ×1 · {court(r('lot', 1, 'lot_message') / 1000, 'Ko')}"),
        (830, "Déménager", "rarement, en fin de refonte", "bleu", "pile",
         ["les fiches descendent vers le bas", "tout ce qui cite un numéro suit", "l'empreinte par les noms ne bouge pas", "la mémoire est rendue"],
         f"{duree(DM1)} à ×1 · {court(r('lot', 1, 'demenagement_message') / 1000, 'Ko')}"),
    ]
    for k, (cx, titre, sous, ton, ic, lignes, cout) in enumerate(etapes):
        d.bouton(cx, 74, 290, 58, titre, sous, ton, ic)
        d.carte(cx - 145, 122, 290, 124, "Ce qu'il fait", lignes, ton, None, taille=11.5, sous=10.4)
        d.pilule(cx, 258, cout, ton, ancre="middle", taille=10)
    d.fleche("M318 74 L350 74", "vert", 2.2, anime=True)
    d.etiquette(334, 112, "30 jours", "vert", taille=9.5)
    d.fleche("M648 74 L680 74", "bleu", 2.2, anime=True)
    d.etiquette(664, 112, "un seuil", "bleu", taille=9.5)
    bandeau(d, 292, "Le numéro entier n'est jamais redonné : seule la place se réutilise, à la génération suivante.")
    A.ecrire(d, "Trois rythmes : retirer, libérer, déménager",
             f"Trois étapes de gauche à droite. Retirer, au fil de l'eau, par un geste : l'appareil n'est plus lu ni commandé, ses attentes et minuteries sont terminées, ses automatismes passent à revoir, rien n'est libéré et le retrait reste réversible ; c'est une sortie critique au rang s. Trente jours plus tard, le lot de nuit libère : fiches remises à zéro, nom effacé, génération suivante, avec l'instantané de la nuit ; les appareils neufs reprennent la place ; {duree(LOT1)} à ×1. Au-delà d'un seuil, en fin de refonte, le déménagement resserre les fiches ; tout ce qui cite un numéro suit, l'empreinte par les noms ne bouge pas, la mémoire est rendue ; {duree(DM1)} à ×1.",
             340, "Retirer ne libère rien et reste réversible ; le lot libère et réutilise ; le déménagement, rare, rend la mémoire. Durées mesurées au banc (Linux musl, 2 CPU, 128 Mio, médiane de trois essais).")

# ================================================================ Le cycle d'un équipement
if voulu("cycle"):
    d = Dessin("cycle")
    d.titre(16, 24, "Le cycle d'une place : qui la fait passer d'un état à l'autre")
    W, H = 196, 56
    P = {"service": (150, 120), "absent": (150, 318), "retire": (500, 218), "libre": (850, 120), "condamnee": (850, 318)}
    # Les flèches d'abord, sous les états.
    d.fleche("M118 150 L118 288", "ambre", 1.8)
    d.etiquette(110, 214, "l'intégration", "ambre", "end", 10)
    d.etiquette(110, 228, "ne le donne plus", "ambre", "end", 10)
    d.fleche("M182 288 L182 150", "vert", 1.8)
    d.etiquette(190, 214, "il revient :", "vert", "start", 10)
    d.etiquette(190, 228, "tout est là", "vert", "start", 10)
    d.fleche("M250 132 C320 150 360 190 400 206", "rouge", 2)
    d.etiquette(318, 150, "un geste : retirer, remplacer", "rouge", taille=10)
    d.fleche("M250 306 C320 290 360 248 400 232", "rouge", 1.6, tirets="5 4")
    d.etiquette(330, 298, "30 jours absent : un rappel,", "rouge", taille=10)
    d.etiquette(330, 312, "puis un geste", "rouge", taille=10)
    d.fleche("M470 190 C430 128 330 112 252 112", "ambre", 1.5, tirets="4 4")
    d.etiquette(390, 104, "annuler, pendant la grâce", "ambre", taille=10)
    d.fleche("M600 206 C650 170 700 140 748 128", "vert", 2.2, anime=True)
    d.etiquette(672, 186, "le lot, après", "vert", taille=10)
    d.etiquette(672, 200, "le délai de grâce", "vert", taille=10)
    d.fleche("M850 92 C850 52 160 52 150 90", "bleu", 1.8)
    d.etiquette(500, 46, "un appareil neuf : la même place, un nouveau numéro (génération + 1)", "bleu", taille=10)
    d.fleche("M850 150 L850 288", "gris", 1.5, tirets="3 3")
    d.etiquette(858, 214, "127e génération", "gris", "start", 10)
    d.bouton(*P["service"], W, H, "En service", "lu, commandé", "vert", "coche")
    d.bouton(*P["absent"], W, H, "Absent", "tout est gardé", "ambre", "pouls")
    d.bouton(*P["retire"], W, H, "Retiré", "plus lu ni commandé", "rouge", "croix")
    d.bouton(*P["libre"], W, H, "Libre", "nom effacé, à zéro", "gris", "fleches")
    d.bouton(*P["condamnee"], W, H, "Condamnée", "ne sert plus", "gris", "cadenas")
    d.carte(398, 262, 204, 74, "Pendant la grâce", ["la place garde son nom ;", "l'écran dit « remplacé par … »"], "rouge", None, taille=11, sous=10)
    bandeau(d, 364, "Absent n'est pas retiré : un pont qui redémarre ne casse rien. Retirer est un geste, réversible jusqu'au lot.", "ambre", "pouls")
    A.ecrire(d, "Le cycle d'un équipement",
             "Cinq états. En service : lu et commandé. Absent, quand l'intégration ne le donne plus : tout est gardé, et il revient tout seul. Retiré, par un geste (retirer, remplacer), ou après trente jours d'absence, sur un rappel puis un geste : plus lu ni commandé ; pendant le délai de grâce la place garde son nom, et le retrait peut être annulé. Libre, après le lot : nom effacé, fiches à zéro ; un appareil neuf y prend un nouveau numéro, à la génération suivante. Condamnée, après la 127e génération.",
             412, "Chaque passage est une sortie critique au rang s, décidée par l'actif et rejouée par les copies. Seul le lot libère, et seulement après le délai de grâce.")

# ================================================================ Le numéro
if voulu("numero"):
    d = Dessin("numero")
    d.titre(16, 24, "Les 32 bits du numéro")
    X, Y, U = 40, 46, 920 / 32
    for x0, bits, ton, titre, sous in [(0, 1, "gris", "", "réservé"), (1, 7, "violet", "génération", "0 à 127"), (8, 24, "vert", "place", "0 à 16 777 215 : l'adresse de la fiche")]:
        x = X + x0 * U
        w = bits * U - 4
        d.brut(f'<rect x="{x:.1f}" y="{Y + 4}" width="{w:.1f}" height="46" rx="10" fill="#000000" opacity=".12"/>')
        case(d, x, Y, w, 46, ton)
        d.brut(f'<rect x="{x + 4:.1f}" y="{Y + 3}" width="{w - 8:.1f}" height="16" rx="6" fill="#ffffff" opacity=".17"/>')
        if titre:
            d.t(x + 12, Y + 21, titre, 12.5, True, "#ffffff")
            d.t(x + 12, Y + 37, sous, 10, coul="#ffffff", op=".92")
    d.t(984, 24, "le bit 31 reste aux marques de cause (horloge, agenda)", 10, coul=GRIS, ancre="end")
    d.t(X + 2, Y + 64, "bit 31", 9.5, coul=GRIS)
    d.t(X + U + 2, Y + 64, "bits 24 à 30", 9.5, coul=GRIS)
    d.t(X + 32 * U - 4, Y + 64, "bits 0 à 23", 9.5, coul=GRIS, ancre="end")

    d.titre(16, 150, "Une place, deux occupants")
    y = 168
    d.carte(16, y, 270, 76, "Génération 0 : numéro 1 234", ["z2m:0x00158d00…a1/temperature", "l'ancien capteur du salon"], "violet", "radio", taille=11.5, sous=10)
    d.fleche(f"M290 {y + 38} L338 {y + 38}", "rouge", 1.8)
    d.carte(342, y, 150, 76, "Retiré", ["le nom reste,", "pour annuler"], "rouge", "croix", taille=11.5, sous=10)
    d.fleche(f"M496 {y + 38} L544 {y + 38}", "vert", 1.8, anime=True)
    d.carte(548, y, 150, 76, "Le lot", ["nom effacé, à zéro ;", "génération 1"], "gris", "fleches", taille=11.5, sous=10)
    d.fleche(f"M702 {y + 38} L726 {y + 38}", "vert", 1.8)
    d.carte(730, y, 254, 76, "Génération 1 : n° 16 778 450", ["z2m:0x00158d00…b2/temperature", "le nouveau, sur la même place"], "vert", "radio", taille=11.5, sous=10)
    d.titre(16, 276, "Ce qui en découle")
    for k, (ic, ton, a, b) in enumerate([
            ("puce", "vert", "L'adresse : place × taille", "une instruction de plus, rien d'autre"),
            ("bouclier", "violet", "L'ancien numéro est inerte", "1 234 ne désigne plus rien : génération 0"),
            ("lettre", "bleu", "Sur MQTT, « i » = le numéro entier", "inconnu d'un abonné : il redemande meta/")]):
        x = 16 + k * 328
        d.carte(x, 288, 312, 62, a, [b], ton, ic, taille=11.5, sous=10)
    A.ecrire(d, "Le numéro : une place et sa génération",
             "En haut, les 32 bits du numéro : le bit 31 reste réservé aux marques de cause ; les bits 24 à 30 portent la génération, de 0 à 127 ; les bits 0 à 23 la place, de 0 à 16 777 215, qui est l'adresse de la fiche. Au milieu, une place et deux occupants : génération 0, numéro 1 234, l'ancien capteur du salon ; il est retiré, le nom reste pour pouvoir annuler ; le lot efface le nom, remet à zéro et passe à la génération 1 ; le nouveau capteur prend la même place sous le numéro 16 778 450. En bas : l'adresse reste place fois taille ; l'ancien numéro est inerte ; sur MQTT, i porte le numéro entier, et un abonné qui ne le connaît pas redemande meta/.",
             362, "Le numéro entier n'est jamais redonné ; seule la place se réutilise. C'est la forme des identifiants d'entité des moteurs de jeu (EnTT : 20 bits d'entité, 12 de version).")

# ================================================================ Aujourd'hui
if voulu("aujourdhui"):
    d = Dessin("aujourdhui")
    d.titre(16, 24, "Aujourd'hui : un appareil sort, et ce qui le cite reste")
    d.carte(16, 48, 220, 92, "Un capteur sort", ["l'intégration ne le donne", "plus : remove_device", "(catalogue relu)"], "rouge", "radio", taille=12, sous=10.2)
    d.fleche("M240 94 L284 94", "rouge", 2, anime=True)
    d.carte(288, 48, 220, 92, "Le hub", ["valeurs, disponibilité et", "définition retirées ;", "DeviceRemoved publié"], "vert", "maison", taille=12, sous=10.2)
    d.fleche("M512 94 L552 94", "gris", 1.8)
    graves = [("Automatismes", "approuvés, ne partent plus, sans trace", "rouge", "rouage"),
              ("Énergie", "la dernière puissance, comptée sans fin", "rouge", "eclair"),
              ("Le remplaçant", "refusé : l'ancien reste « connu »", "rouge", "bouclier")]
    for k, (t_, s_, ton, ic) in enumerate(graves):
        d.carte(556, 40 + k * 66, 428, 58, t_, [s_], ton, ic, taille=11.5, sous=10.2, mot="défaut")
    restent = [("Étiquette", "gardée ; plus effaçable", "cle"), ("État restauré", "gardé sans limite", "disque"),
               ("Plan, accueil", "gardés ; masqués à l'écran", "maison"), ("Aides", "dormantes, une place sur 128", "rouage"),
               ("Historique", "sous son nom ; purgé à son rythme", "base"), ("Secrets Tuya", "la clé reste au coffre", "cadenas")]
    for k, (t_, s_, ic) in enumerate(restent):
        x, y = 16 + (k % 3) * 328, 252 + (k // 3) * 66
        d.carte(x, y, 312, 58, t_, [s_], "gris", ic, taille=11.5, sous=10.2, mot="orphelin")
    d.titre(16, 240, "Ce qui reste, sans danger mais pour toujours")
    bandeau(d, 392, "Une intégration retirée de la configuration : ses appareils ne sont ni rechargés, ni retirés. Remplacer un appareil : impossible.", "rouge", "croix")
    A.ecrire(d, "Aujourd'hui, quand un appareil sort",
             "Un capteur sort : l'intégration ne le donne plus, et son pilote appelle remove_device. Le hub retire ses valeurs, sa disponibilité et sa définition, et publie DeviceRemoved. Trois défauts : les automatismes qui le citent restent approuvés et ne partent plus, sans trace ; un compteur d'énergie continue de compter sa dernière puissance ; un remplaçant est refusé parce que l'ancien reste connu. Six orphelins restent pour toujours : l'étiquette, l'état restauré, le plan et l'accueil, les aides, l'historique sous son nom, la clé Tuya au coffre. Une intégration retirée de la configuration n'est ni rechargée ni retirée ; remplacer un appareil est impossible.",
             440, "Relevé dans le code au commit a835893 : les lignes sont citées au § 4.3. Les trois défauts se corrigent sans attendre la mémoire v2.")

# ================================================================ Qui garde un numéro
if voulu("qui"):
    d = Dessin("qui")
    d.titre(16, 24, "Tout ce qui garde un numéro, et pourquoi aucun ne se trompe")
    zones = [
        (16, 44, "Recompilé : jamais périmé", "vert", "rouage",
         [("Le programme", "règles, nœuds, index par point"), ("Les conditions composées", "feuilles, vérité reconstruite"), ("Les fiches horaires", "« à l'heure portée par une valeur »")]),
        (508, 44, "Vérifie la génération", "ambre", "bouclier",
         [("Les exécutions", "leur cause, leur cible"), ("Les attentes à durée", "revérifiées à l'échéance"), ("Minuteries, baux, l'écran", "une référence périmée est inerte")]),
        (16, 262, "Suit le rang s", "bleu", "pile",
         [("La file des 10 000 sorties", "chaque sortie, la numérotation de son rang"), ("État complet, instantané", "annuaire, générations, places libres"), ("Abonnés internes", "numéro inconnu → meta/")]),
        (508, 262, "Ne garde que des noms", "ardoise", "document",
         [("La base, l'archive", "le catalogue par le nom"), ("Le journal d'exécution", "run/, hist/ops, l'audit"), ("Les clients extérieurs", "les sujets, jamais « i »")]),
    ]
    for x, y, titre, ton, ic, cartes in zones:
        d.zone(x, y, 476, 206, titre, ton, ic)
        for k, (t_, s_) in enumerate(cartes):
            d.carte(x + 16, y + 40 + k * 54, 444, 46, t_, [], ton, None, taille=11.5)
            d.t(x + 452, y + 40 + k * 54 + 28, s_, 10, coul=GRIS, ancre="end")
    pastille(d, 500, 244, "le numéro : place + génération", "violet", 11, "puce")
    A.ecrire(d, "Qui garde un numéro, et ce que chacun en fait",
             "Quatre zones. Recompilé, jamais périmé : le programme, les conditions composées, les fiches horaires portées par une valeur. Vérifie la génération : les exécutions (cause et cible), les attentes à durée revérifiées à l'échéance, les minuteries, baux et l'écran des exécutions finies, pour qui une référence périmée est inerte. Suit le rang s : la file des 10 000 sorties, l'état complet et l'instantané (annuaire, générations, places libres), les abonnés internes, qui redemandent meta/ pour un numéro inconnu. Ne garde que des noms : la base et l'archive, le journal d'exécution et l'audit, les clients extérieurs.",
             482, "Chaque endroit a sa façon de ne jamais confondre deux occupants d'une même place : se recompiler, comparer la génération, suivre l'ordre des sorties, ou ne parler que par les noms.")

# ================================================================ La succession
if voulu("succession"):
    d = Dessin("succession")
    d.titre(16, 24, "Remplacer un appareil : un geste, une validation")
    cles = ["temperature", "humidity", "battery"]
    d.carte(16, 44, 300, 150, "L'ancien capteur", ["z2m:0x00158d00…a1"], "rouge", "radio", taille=12, sous=10.2, mot="retiré")
    d.carte(684, 44, 300, 150, "Le nouveau capteur", ["z2m:0x00158d00…b2"], "vert", "radio", taille=12, sous=10.2, mot="en service")
    for k, c in enumerate(cles):
        y = 106 + k * 28
        d.pilule(32, y, c, "rouge", taille=10)
        d.pilule(700, y, c, "vert", taille=10)
        d.fleche(f"M150 {y + 9} L692 {y + 9}", "vert", 1.5, anime=True, lent=True)
    d.etiquette(500, 100, "les points s'apparient par leur clé ; nature et unité vérifiées", "vert", taille=10)
    choses = [("Automatismes", "récrits par les noms", "rouage"), ("Étiquette, pièce", "passées au nouveau", "cle"),
              ("Plan, accueil", "la même place", "maison"), ("Compteur d'énergie", "il suit le nouveau", "eclair")]
    for k, (t_, s_, ic) in enumerate(choses):
        x = 16 + k * 244
        d.carte(x, 226, 228, 62, t_, [s_], "bleu", ic, taille=11.5, sous=10.2)
    pastille(d, 500, 302, "une seule validation humaine : elle vaut pour chaque automatisme récrit", "violet", 10.5, "bouclier")
    d.t(16, 350, "L'ancien est retiré : pendant la grâce, l'écran dit « remplacé par … » ; le lot libère sa place.", 10.5, coul=GRIS)
    d.t(16, 366, "L'historique garde deux courbes ; un lien « suite de » dans le catalogue de la base reste à décider (D13).", 10.5, coul=GRIS)
    A.ecrire(d, "Remplacer un appareil : la succession",
             "À gauche, l'ancien capteur, retiré ; à droite, le nouveau, en service. Leurs points s'apparient par la clé (temperature, humidity, battery), nature et unité vérifiées. En dessous, ce qui passe au nouveau : les automatismes, récrits par les noms ; l'étiquette et la pièce ; le plan et l'accueil ; le compteur d'énergie. Une seule validation humaine vaut pour chaque automatisme récrit. L'ancien reste retiré pendant la grâce, avec « remplacé par … », puis le lot libère sa place ; l'historique garde deux courbes.",
             380, "Au banc : 75 appareils remplacés ; les 25 automatismes qui les citaient commandent tous de nouveau, sur le nouvel appareil.")

# ================================================================ Le lot
if voulu("lot"):
    d = Dessin("lot")
    d.titre(16, 24, "Sur l'actif")
    d.titre(680, 24, "Sur chaque copie")
    pas = [("1", "Choisir", "les places retirées depuis le délai de grâce", "gris"),
           ("2", "Publier meta/lot", "au rang s : les places, en plages de 8 o", "ambre"),
           ("3", "Appliquer", "fiches à zéro, nom effacé, génération + 1, plages recollées", "vert"),
           ("4", "Recompiler", "le programme ; la vérité des composées (3,6 µs)", "vert"),
           ("5", "L'instantané de la nuit", "une seule écriture pour le lot", "ardoise")]
    for k, (num, t_, s_, ton) in enumerate(pas):
        y = 40 + k * 64
        d.carte(16, y, 560, 54, f"{num}. {t_}", [], ton, None, taille=12)
        d.t(566, y + 34, s_, 10.2, coul=GRIS, ancre="end")
    d.fleche("M580 132 C620 132 640 92 676 92", "ambre", 2.2, anime=True)
    d.etiquette(626, 90, "rang s", "ambre", taille=9.5)
    d.carte(680, 40, 304, 118, "Secours local, distant", ["reçoivent meta/lot au même rang", "appliquent 3 et 4, avec le même code", "ne décident rien", "même empreinte que l'actif"], "bleu", "fleches", taille=12, sous=10.2)
    d.carte(680, 172, 304, 118, "Celui qui a décroché", ["la file : le lot à sa place, dans l'ordre", "au-delà : l'état complet, qui porte", "l'annuaire, les générations", "et les places libres"], "violet", "nuage", taille=12, sous=10.2)
    d.titre(16, 372, "Une rangée de places, avant et après")
    noms = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p"]
    retires = {2, 3, 4, 9, 10, 14}
    for k, nm in enumerate(noms):
        x = 16 + k * 30
        case(d, x, 384, 26, 26, "rouge" if k in retires else "vert", nm, taille=10)
        x2 = 516 + k * 30
        if k in retires:
            case(d, x2, 384, 26, 26, "gris", "g+1", plein=False, tirets=True, taille=8)
        else:
            case(d, x2, 384, 26, 26, "vert", nm, taille=10)
    d.fleche("M496 397 L510 397", "vert", 1.8)
    d.t(16, 428, "en rouge : retirées depuis plus que le délai de grâce", 10, coul=GRIS)
    d.t(516, 428, "libres, à la génération suivante : les neufs les prennent d'abord", 10, coul=GRIS)
    l1, l10, l100 = r("lot", 1, "lot_duree"), r("lot", 10, "lot_duree"), r("lot", 100, "lot_duree")
    d.pilule(680, 312, f"mesuré : {duree(l1)} à ×1, {duree(l10)} à ×10, {duree(l100)} à ×100", "vert", taille=10)
    A.ecrire(d, "Le lot : libérer, sur l'actif et chez les copies",
             f"À gauche, les cinq pas du lot sur l'actif : choisir les places retirées depuis le délai de grâce ; publier meta/lot au rang s, les places en plages de 8 octets ; appliquer, fiches à zéro, nom effacé, génération suivante, plages recollées ; recompiler le programme et reconstruire la vérité des conditions composées ; l'instantané de la nuit enregistre le tout. À droite, les copies reçoivent meta/lot au même rang et appliquent les pas 3 et 4 avec le même code ; une copie qui a décroché le trouve dans la file, ou prend l'état complet. En bas, une rangée de seize places : six retirées deviennent libres à la génération suivante. Mesuré : {duree(l1)} à ×1, {duree(l10)} à ×10, {duree(l100)} à ×100.",
             442, "Le lot est une seule sortie critique : une copie l'applique en entier ou pas du tout, au même rang que l'actif.")

# ================================================================ Le déménagement
if voulu("demenagement"):
    d = Dessin("demenagement")
    d.titre(16, 24, "Les fiches glissent vers le bas, dans l'ordre ; la queue se rend")
    occ = "vvvLvvLLvLvvvLLvvLvvbbbbbbbb"
    C, X = 31, 110
    d.t(16, 66, "avant", 11, True, ENCRE)
    d.t(16, 82, "après le lot", 9.5, coul=GRIS)
    d.t(16, 246, "après", 11, True, ENCRE)
    d.t(16, 262, "le déménagement", 9.5, coul=GRIS)
    vivants = [k for k, ch in enumerate(occ) if ch != "L"]
    for j, k in enumerate(vivants):
        x0, x1 = X + k * C + (C - 5) / 2, X + j * C + (C - 5) / 2
        ton = "bleu" if occ[k] == "b" else "vert"
        if j == k:
            d.brut(f'<line x1="{x0:.1f}" y1="88" x2="{x1:.1f}" y2="226" stroke="{TONS[ton][1]}" stroke-opacity=".35" stroke-width="1.2"/>')
        else:
            d.fleche(f"M{x0:.1f} 90 C{x0:.1f} 160 {x1:.1f} 156 {x1:.1f} 224", ton, 1.4, op=".85")
    for k, ch in enumerate(occ):
        x = X + k * C
        if ch == "L":
            case(d, x, 56, C - 5, 30, "gris", None, plein=False, tirets=True)
        else:
            case(d, x, 56, C - 5, 30, "bleu" if ch == "b" else "vert")
    for j in range(len(occ)):
        x = X + j * C
        if j < len(vivants):
            case(d, x, 228, C - 5, 30, "bleu" if occ[vivants[j]] == "b" else "vert")
        else:
            case(d, x, 228, C - 5, 30, "gris", None, plein=False, tirets=True)
    xr = X + len(vivants) * C
    d.etiquette(xr + (len(occ) - len(vivants)) * C / 2 - 3, 276, "rendu à la mémoire", "gris", taille=10)
    d.carte(16, 296, 476, 92, "Ce qui suit chaque point déménagé", ["exécutions : cause et cible ; attentes ; minuteries, baux ;", "la file des exécutions finies ; le programme se recompile ;", "la vérité des conditions composées se reconstruit"], "bleu", "fleches", taille=12, sous=10.2)
    d.carte(508, 296, 476, 92, "La preuve", ["l'empreinte par les noms, identique avant et après ;", "une exécution en cours commande le bon appareil ;", "une attente en cours part à l'heure"], "vert", "coche", taille=12, sous=10.2)
    m1, m100 = r("lot", 1, "demenagement_mouvements"), r("lot", 100, "demenagement_mouvements")
    d.pilule(16, 400, f"{nb(m1)} mouvements à ×1 en {duree(DM1)} ; {nb(m100)} à ×100 en {duree(DM100)}", "bleu", taille=10)
    A.ecrire(d, "Le déménagement : resserrer les colonnes",
             f"En haut, une rangée de places après un lot : des trous dispersés, et au bout les derniers appareils venus. Chaque fiche glisse vers le bas, dans l'ordre : les lignes la suivent jusqu'à sa nouvelle place, sans jamais se croiser ; celles qui ne bougent pas restent droites. En bas, la rangée serrée, et la queue rendue à la mémoire. Ce qui suit chaque point déménagé : exécutions, attentes, minuteries, baux, la file des exécutions finies ; le programme se recompile, la vérité des conditions composées se reconstruit. La preuve : l'empreinte par les noms est identique, une exécution en cours commande le bon appareil, une attente en cours part à l'heure. Mesuré : {nb(m1)} mouvements à ×1 en {duree(DM1)}, {nb(m100)} à ×100 en {duree(DM100)}.",
             428, "Chaque point déménagé prend la génération de sa nouvelle place, et la place quittée passe à la suivante : aucun numéro entier n'est redonné. Les points d'un appareil restent ensemble.")

# ================================================================ Les copies
if voulu("copies"):
    d = Dessin("copies")
    d.titre(16, 24, "Les sorties critiques de l'actif, dans l'ordre du rang s")
    flux = [("val", "vert"), ("val", "vert"), ("retiré", "rouge"), ("val", "vert"), ("val", "vert"), ("meta/lot", "ambre"),
            ("meta/ neuf", "bleu"), ("val", "vert"), ("val", "vert"), ("val", "vert")]
    X0, X1 = 290, 790
    larg = [d.pilule(-1000, -1000, lab, ton, plein=True, taille=9.5) for lab, ton in flux]
    d.c = [l for l in d.c if 'x="-' not in l]
    pas = (X1 - X0 - sum(larg)) / (len(flux) - 1)
    x = X0
    pos = []
    for (lab, ton), w in zip(flux, larg):
        d.pilule(x, 44, lab, ton, plein=True, taille=9.5)
        pos.append((x, w))
        x += w + pas
    d.t(16, 56, "l'actif publie", 11, True, ENCRE)
    d.fleche(f"M{X0 - 14} 76 L{X1 + 10} 76", "gris", 1.4, anime=True, lent=True)
    lignes = [(104, "Secours qui suit", "bleu", "fleches", "il applique tout, dans l'ordre", "identique à l'actif"),
              (190, "Secours qui a décroché", "violet", "nuage", "le lot passe pendant son absence", "identique, par l'état complet")]
    for y, t_, ton, ic, s1, s2 in lignes:
        d.carte(16, y, 250, 70, t_, [s1], ton, ic, taille=12, sous=10.2)
        d.pilule(984, y + 26, s2, "vert", ancre="end", taille=10, icone="coche")
    d.fleche(f"M{X0} 139 L{X1} 139", "bleu", 2, anime=True)
    xa, xb = pos[4][0] + pos[4][1] + 4, pos[6][0] + pos[6][1] + 6
    d.fleche(f"M{X0} 225 L{xa:.1f} 225", "violet", 2)
    d.brut(f'<rect x="{xa:.1f}" y="210" width="{xb - xa:.1f}" height="30" rx="9" fill="{TONS["gris"][2]}" stroke="{TONS["gris"][1]}" stroke-dasharray="5 4"/>')
    d.t((xa + xb) / 2, 229, "absent", 10, True, GRIS, "middle")
    d.fleche(f"M{xb:.1f} 225 L{X1} 225", "violet", 2, anime=True)
    d.etiquette((xb + X1) / 2, 204, "état complet, puis la suite", "violet", taille=10)
    for k in (5, 6):
        cx = pos[k][0] + pos[k][1] / 2
        d.brut(f'<line x1="{cx:.1f}" y1="64" x2="{cx:.1f}" y2="208" stroke="{TONS["ambre"][1]}" stroke-opacity=".45" stroke-dasharray="3 3"/>')
    d.carte(16, 290, 250, 96, "Client avec un cache", ["il revient le lendemain", "avec ses noms de la veille"], "ardoise", "ecran", taille=12, sous=10.2)
    d.carte(290, 290, 330, 96, "Avant le lot : i = 1 234", ["dans son cache : « cave/porte »", "c'est juste"], "gris", "lettre", taille=12, sous=10.2)
    d.carte(644, 290, 340, 96, "Après le lot : i = 16 778 450", ["inconnu de son cache : il redemande meta/", "« salon/porte » : jamais confondu"], "vert", "lettre", taille=12, sous=10.2)
    d.fleche("M624 338 L640 338", "vert", 1.6)
    A.ecrire(d, "Les copies, et le client qui revient",
             "En haut, les sorties critiques de l'actif dans l'ordre du rang s : des valeurs, un retrait, le lot (meta/lot), la déclaration d'un appareil neuf, d'autres valeurs. Un secours qui suit applique tout dans l'ordre et finit identique. Un secours qui a décroché pendant le lot reprend par l'état complet, puis la suite, et finit identique. En bas, un client revient le lendemain avec son cache de noms : avant le lot, le numéro 1 234 désignait cave/porte ; après, la place porte le numéro 16 778 450, inconnu de son cache : il redemande meta/ et reçoit salon/porte, sans jamais confondre.",
             398, f"Au banc : {nb(r('refonte', 1, 'sorties'))} sorties en trois jours ; les deux copies finissent identiques à l'actif ; le client ne confond aucun nom ({nb(r('refonte', 1, 'client_erreurs'))} erreur).")

# ================================================================ Vingt ans
if voulu("vingt-ans"):
    d = Dessin("vingt-ans")
    d.titre(16, 24, "Les places de la maison ×1, année après année")
    X0, X1, Y0, Y1 = 66, 610, 50, 318
    VMAX = 12_000

    def px(an):
        return X0 + (X1 - X0) * an / 20

    def py(v):
        return Y1 - (Y1 - Y0) * v / VMAX

    for v in (0, 3000, 6000, 9000, 12000):
        d.brut(f'<line x1="{X0}" y1="{py(v):.1f}" x2="{X1}" y2="{py(v):.1f}" stroke="{LIGNE}"/>')
        d.t(X0 - 8, py(v) + 4, nb(v), 10, coul=GRIS, ancre="end")
    for an in (0, 5, 10, 15, 20):
        d.t(px(an), Y1 + 18, f"an {an}" if an else "départ", 10, coul=GRIS, ancre="middle")
    for an in (2.5, 7.5, 12.5, 17.5):
        d.brut(f'<line x1="{px(an):.1f}" y1="{Y0}" x2="{px(an):.1f}" y2="{Y1}" stroke="{TONS["rouge"][1]}" stroke-opacity=".4" stroke-dasharray="4 4"/>')
    d.etiquette(px(2.5) + 4, Y0 + 10, "grande refonte", "rouge", "start", 9.5)
    pts0 = r("lot", 1, "points")
    series = [("vie_jamais", "places", "jamais libérer", "rouge", None),
              ("vie_lots", "places", "les lots", "ambre", None),
              ("vie_demenagement", "colonnes", "lots et déménagement", "vert", None),
              ("vie_lots", "en_service", "points en service", "gris", "5 4")]
    fins = []
    for sec, cle, lab, ton, tir in series:
        pts = [(0, pts0)] + [(an, r(sec, 1, f"an{an:02}_{cle}")) for an in range(1, 21)]
        dd = "M" + " L".join(f"{px(a):.1f} {py(v):.1f}" for a, v in pts)
        d.fleche(dd, ton, 2.4 if not tir else 1.6, tirets=tir, tete=False)
        fins.append((pts[-1][1], lab, ton))
    fins.sort()
    ys = []
    for v, lab, ton in fins:
        y = py(v) + 4
        if ys and y > ys[-1] - 14:
            y = ys[-1] - 14
        ys.append(y)
        d.t(X1 + 8, y, f"{nb(v)} : {lab}", 10, True, TONS[ton][0])
    d.titre(780, 24, "Au bout de vingt ans")
    for j, (e, unite) in enumerate([(1, "×1"), (10, "×10")]):
        y0 = 48 + j * 140
        d.t(780, y0 + 12, f"mémoire, maison {unite}", 10.5, True, ENCRE)
        vals = [(r("vie_jamais", e, "memoire"), "rouge"), (r("vie_lots", e, "memoire"), "ambre"), (r("vie_demenagement", e, "memoire"), "vert")]
        mx = max(v for v, _ in vals)
        for k, (v, ton) in enumerate(vals):
            y = y0 + 22 + k * 30
            w = 120 * v / mx
            d.barre(780, y, w, 16, ton)
            d.t(780 + w + 6, y + 12, court(v / 1e6, "Mo"), 10.5, True, TONS[ton][0])
    A.ecrire(d, "Vingt ans de la maison : les places, selon la règle",
             f"Un graphique sur vingt ans, maison ×1, avec quatre grandes refontes marquées. Sans jamais libérer, les places montent par marches jusqu'à {nb(r('vie_jamais', 1, 'places'))}. Avec les lots, elles suivent le pic et plafonnent à {nb(r('vie_lots', 1, 'places'))}. Avec un déménagement après chaque refonte, les fiches restent à {nb(r('vie_demenagement', 1, 'colonnes'))}, au niveau des points en service ({nb(r('vie_lots', 1, 'an20_en_service'))}). À droite, la mémoire au bout de vingt ans : {court(r('vie_jamais', 1, 'memoire') / 1e6, 'Mo')}, {court(r('vie_lots', 1, 'memoire') / 1e6, 'Mo')} et {court(r('vie_demenagement', 1, 'memoire') / 1e6, 'Mo')} à ×1 ; {court(r('vie_jamais', 10, 'memoire') / 1e6, 'Mo')}, {court(r('vie_lots', 10, 'memoire') / 1e6, 'Mo')} et {court(r('vie_demenagement', 10, 'memoire') / 1e6, 'Mo')} à ×10.",
             346, "Chaque mois : 0,5 % des appareils remplacés, 0,3 % retirés, 0,3 % ajoutés ; une grande refonte tous les cinq ans ; un lot par mois, 30 jours de grâce. Mesuré au banc, mémoire comptée par l'allocateur.")

# ================================================================ Le passage, mesuré
if voulu("mesures"):
    d = Dessin("mesures")
    d.titre(16, 24, "Durée")
    d.titre(516, 24, "Message, une fois par opération")
    lignes = [("lot ×1", "lot", 1, "lot", "vert"), ("lot ×10", "lot", 10, "lot", "vert"), ("lot ×100", "lot", 100, "lot", "vert"),
              ("déménagement ×1", "lot", 1, "demenagement", "bleu"), ("déménagement ×10", "lot", 10, "demenagement", "bleu"), ("déménagement ×100", "lot", 100, "demenagement", "bleu")]
    axe_log(d, 160, 300, 40, 286, 10, 1e5, "échelle logarithmique", [(10, "10 µs"), (100, "100 µs"), (1e3, "1 ms"), (1e4, "10 ms"), (1e5, "100 ms")])
    axe_log(d, 640, 300, 40, 286, 1e3, 1e7, "échelle logarithmique", [(1e3, "1 Ko"), (1e4, "10 Ko"), (1e5, "100 Ko"), (1e6, "1 Mo"), (1e7, "10 Mo")])
    for k, (lab, sec, e, quoi, ton) in enumerate(lignes):
        y = 48 + k * 40
        us = r(sec, e, f"{quoi}_duree")
        o = r(sec, e, f"{quoi}_message")
        d.t(16, y + 19, lab, 10.5, True, ENCRE)
        w = xlog(us, 10, 1e5, 160, 300) - 160
        d.barre(160, y + 6, w, 18, ton)
        d.t(160 + w + 6, y + 20, duree(us), 10.5, True, TONS[ton][0])
        d.t(516, y + 19, lab, 10.5, True, ENCRE)
        w = xlog(o, 1e3, 1e7, 640, 300) - 640
        d.barre(640, y + 6, w, 18, ton)
        d.t(640 + w + 6, y + 20, court(o / 1e3, "Ko") if o < 1e6 else court(o / 1e6, "Mo"), 10.5, True, TONS[ton][0])
    A.ecrire(d, "Le passage, mesuré",
             f"Deux graphiques en échelle logarithmique, pour 30 % des appareils retirés. La durée : le lot prend {duree(r('lot', 1, 'lot_duree'))} à ×1, {duree(r('lot', 10, 'lot_duree'))} à ×10, {duree(r('lot', 100, 'lot_duree'))} à ×100 ; le déménagement {duree(r('lot', 1, 'demenagement_duree'))}, {duree(r('lot', 10, 'demenagement_duree'))} et {duree(r('lot', 100, 'demenagement_duree'))}. Le message : {court(r('lot', 1, 'lot_message') / 1e3, 'Ko')}, {court(r('lot', 10, 'lot_message') / 1e3, 'Ko')} et {court(r('lot', 100, 'lot_message') / 1e3, 'Ko')} pour le lot ; {court(r('lot', 1, 'demenagement_message') / 1e3, 'Ko')}, {court(r('lot', 10, 'demenagement_message') / 1e3, 'Ko')} et {court(r('lot', 100, 'demenagement_message') / 1e6, 'Mo')} pour le déménagement.",
             318, "Linux musl, 2 CPU, 128 Mio, médiane de trois essais ; la machine de mesure était chargée : les durées sont des ordres de grandeur. Un message de plus de 4 Kio part en morceaux, comme un fichier métier.")

# ================================================================ Lire une valeur
if voulu("lecture"):
    d = Dessin("lecture")
    d.titre(16, 24, "Lire une valeur au hasard, par son numéro (ns)")
    formes = [("le numéro est l'adresse (v2)", "numero_adresse_v2", "vert"),
              ("la place, tirée du numéro", "place_et_generation", "vert"),
              ("la place, génération vérifiée", "place_et_generation_verifiee", "ambre"),
              ("O4 : un numéro fixe, par une table", "numero_fixe_par_une_table", "gris")]
    mx = max(r("lecture", e, c) for e in (1, 10, 100) for _, c, _ in formes) * 1.15
    W = 520
    for k, (lab, cle, ton) in enumerate(formes):
        y = 44 + k * 64
        d.t(16, y + 14, lab, 11, True, ENCRE)
        for j, e in enumerate((1, 10, 100)):
            v = r("lecture", e, cle)
            yy = y + 20 + j * 13
            d.t(330, yy + 9, f"×{e}", 9, coul=GRIS, ancre="end")
            d.barre(338, yy, W * v / mx, 10, ton, op=None if j == 0 else (".8" if j == 1 else ".6"))
            d.t(338 + W * v / mx + 6, yy + 9, fr(v, 2), 9.5, True, TONS[ton][0])
    A.ecrire(d, "Lire une valeur : ce que coûte la génération",
             "Quatre façons de lire une valeur au hasard, à ×1, ×10 et ×100, en nanosecondes. Le numéro est l'adresse (la v2) : " + ", ".join(fr(r("lecture", e, "numero_adresse_v2"), 2) for e in (1, 10, 100)) + ". La place tirée du numéro : " + ", ".join(fr(r("lecture", e, "place_et_generation"), 2) for e in (1, 10, 100)) + ". La place, génération vérifiée : " + ", ".join(fr(r("lecture", e, "place_et_generation_verifiee"), 2) for e in (1, 10, 100)) + ". Un numéro fixe par une table : " + ", ".join(fr(r("lecture", e, "numero_fixe_par_une_table"), 2) for e in (1, 10, 100)) + ".",
             306, "La génération ne se vérifie qu'aux endroits qui gardent un numéro longtemps ; le programme, recompilé à chaque lot, lit par la place seule. Médiane de trois essais, Linux musl.")

print(f"ok : {len(A.noms)} schémas → {A.F} et {A.MD}")
