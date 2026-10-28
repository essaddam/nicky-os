#!/usr/bin/env python3
"""Schémas et graphiques de l'analyse « les ordres des appareils » (docs/proposition/ordres-appareils.md),
en relief, avec la boîte à outils de la haute disponibilité v2 (tools/haute-dispo-banc/rendu/relief.py).
Les graphiques lisent la série de la v1 (../sorties/linux-v1/*.txt) : aucun chiffre recopié. Écrit les
fragments dans rendu/fig/ (build.py les insère dans la page) et les SVG autonomes, légende comprise,
dans docs/proposition/ordres-appareils/.

Usage : python3 -I tools/ordres-banc/rendu/figures.py [schéma …]"""
import math
import pathlib
import sys

ICI = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ICI.parents[1] / "haute-dispo-banc" / "rendu"))
from relief import ENCRE, GRIS, LIGNE, TONS, Atelier, Dessin, court, fr, largeur, lire_resultats, nb  # noqa: E402

RES = lire_resultats(ICI.parent / "sorties/linux-v1")
A = Atelier(ICI / "fig", ICI.parents[2] / "docs/proposition/ordres-appareils")
CHOIX = set(a for a in sys.argv[1:] if not a.startswith("--"))


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
        d.brut(f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h}" rx="5" fill="url(#{d.id}-h-{ton})"/>')
    else:
        t = ' stroke-dasharray="4 3"' if tirets else ""
        d.brut(f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h}" rx="5" fill="{p}" stroke="{c}" stroke-opacity=".8"{t}/>')
    if texte:
        d.t(x + w / 2, y + h / 2 + taille * .36, texte, taille, True, "#ffffff" if plein else f, "middle")


def pastille(d, cx, y, texte, ton, taille=10.5, icone=None):
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


def octets(o):
    if o >= 1e6:
        return court(o / 1e6, "Mo")
    if o >= 1e3:
        return court(o / 1e3, "Ko")
    return court(o, "o")


def kio(o):
    if o >= 1048576:
        return court(o / 1048576, "Mio")
    if o >= 1024:
        return court(o / 1024, "Kio")
    return court(o, "o")


def champs(d, x, y, w, h, liste, ton, titre=None, unite_px=None):
    """Une fiche, octet par octet : des cases proportionnelles à leur taille."""
    total = sum(n for _, n in liste)
    px = unite_px or w / total
    xx = x
    if titre:
        d.t(x, y - 8, titre, 11, True, TONS[ton][0])
    for k, (nom, n) in enumerate(liste):
        ww = n * px
        alt = "ambre" if ton == "ambre" else ton
        case(d, xx + 1, y, ww - 2, h, alt, None, plein=(k % 2 == 0))
        if ww > largeur(nom, 9) + 6:
            d.t(xx + ww / 2, y + h / 2 - 2, nom, 9, True, "#ffffff" if k % 2 == 0 else TONS[alt][0], "middle")
            d.t(xx + ww / 2, y + h / 2 + 10, f"{n} o", 8.5, coul="#ffffff" if k % 2 == 0 else GRIS, ancre="middle")
        xx += ww
    return xx


# ================================================================ Le trou : ce que la mémoire range, ce qui manque
if voulu("trou"):
    d = Dessin("trou")
    d.titre(16, 24, "La mémoire v2 range les valeurs ; rien ne range les ordres")
    d.zone(16, 40, 460, 300, "Ce que la mémoire v2 range, par point et par appareil", "vert", "base")
    for k, (t, s) in enumerate([("Def · 16 o", "appareil, libellé, clé, dictionnaire, nature, unité, échelle, et un octet « sens »"),
                                ("Vif · 40 o", "valeur, précédente, observé, changé, reçu, genres, qualité"),
                                ("Échéances · 4 + 4 o", "péremption, fraîcheur ; santé et échéance par appareil"),
                                ("Annuaire · ≈ 52 o", "le nom, une fois ; le numéro est l'adresse")]):
        d.carte(32, 76 + k * 64, 428, 54, t, [s], "vert", taille=11.5, sous=10)
    d.zone(500, 40, 484, 300, "Ce qu'un ordre demande, et où c'est aujourd'hui", "rouge", "croix")
    manque = [("La structure de l'ordre", "effet, retour, suivi, bornes d'ordre : sur meta/ (V7), dans aucune fiche"),
              ("Les demandes en cours", "des tâches, des canaux et des verrous, une par appelant"),
              ("La dernière commande", "nulle part : l'écran garde la sienne 8 s"),
              ("Retenues, reçus, baux, compositions", "à part, en textes ; les retenues meurent au redémarrage")]
    for k, (t, s) in enumerate(manque):
        d.carte(516, 76 + k * 64, 452, 54, t, [s], "rouge", taille=11.5, sous=10)
    bandeau(d, 352, "Le seul octet de la v2 pour un ordre : « sens », c'est-à-dire lecture et écriture. Le reste est à ranger, à part des valeurs.", "ambre", "eclair")
    A.ecrire(d, "Ce que la mémoire range, et ce qui manque",
             "Deux colonnes. À gauche, ce que la mémoire v2 range pour une valeur : la définition de 16 octets, la valeur vive de 40 octets, les échéances, l'annuaire. À droite, ce qu'un ordre demande et où c'est aujourd'hui : sa structure n'est que sur meta/, les demandes vivent dans des tâches et des canaux, la dernière commande n'existe nulle part, les retenues, reçus, baux et compositions sont à part, en textes. Un bandeau : la v2 ne garde pour un ordre qu'un octet, « sens ».",
             400, "La mémoire v2 a tout rangé du côté des valeurs. Du côté des ordres, la V7 décrit la structure sur meta/, mais aucune fiche ne la garde, et l'état d'un ordre en cours vit dans des tâches.")

# ================================================================ Deux registres
if voulu("separes"):
    d = Dessin("separes")
    d.titre(16, 24, "Les valeurs et les ordres : deux registres, un lien par référence")
    d.bloc(390, 64, 220, 120, "ambre", "L'appareil", "16 o", icone="maison")
    d.t(500, 130, "ses valeurs : une plage", 10.5, coul=GRIS, ancre="middle")
    d.t(500, 146, "ses ordres : une plage", 10.5, coul=GRIS, ancre="middle")
    d.t(500, 162, "sa pièce, sa composition", 10.5, coul=GRIS, ancre="middle")
    d.zone(16, 40, 330, 330, "Les valeurs : ce que l'appareil est", "vert", "oeil")
    for k, (n, t) in enumerate([("1 234", "salon_lampe/on"), ("1 235", "salon_lampe/brightness"), ("1 236", "salon_lampe/color_temp"), ("1 237", "salon_lampe/linkquality")]):
        case(d, 32, 80 + k * 44, 90, 32, "vert", n, taille=10)
        d.t(132, 100 + k * 44, t, 10.5, True, ENCRE)
    d.t(32, 280, "Def 16 o · Vif 40 o · annuaire", 10.5, coul=GRIS)
    d.t(32, 296, "le volet RTS : aucune valeur", 10.5, True, TONS["rouge"][0])
    d.zone(654, 40, 330, 330, "Les ordres : ce qu'on demande", "bleu", "eclair")
    for k, (n, t) in enumerate([("87", "salon_lampe/on"), ("88", "salon_lampe/brightness"), ("89", "salon_lampe/color_temp")]):
        case(d, 670, 80 + k * 44, 70, 32, "bleu", n, taille=10)
        d.t(750, 100 + k * 44, t, 10.5, True, ENCRE)
    for k, t in enumerate(["volet_salon/ouvrir", "volet_salon/fermer", "…/stop, …/favori"]):
        case(d, 670, 222 + k * 30, 70, 24, "bleu", None, plein=False)
        d.t(750, 238 + k * 30, t, 10.5, coul=GRIS)
    d.t(670, 330, "Fiche 32 o · forme partagée", 10.5, coul=GRIS)
    d.fleche("M612 108 C630 100 640 96 652 96", "bleu", 1.8)
    d.fleche("M388 108 C370 100 360 96 348 96", "vert", 1.8)
    # Les deux liens passent sous l'appareil, d'un ordre vers la fin du nom de sa valeur.
    d.fleche("M668 140 C600 252 400 252 300 140", "ambre", 1.8, anime=True, tirets="6 4")
    d.etiquette(500, 246, "retour : la valeur qui le prouve", "ambre")
    d.fleche("M668 184 C600 312 400 312 300 184", "ambre", 1.8, anime=True, lent=True, tirets="6 4")
    d.etiquette(500, 304, "paramètre : la valeur qu'il règle", "ambre")
    bandeau(d, 384, "Une valeur n'est jamais un ordre, un ordre n'a jamais de valeur : chacun son registre, ses numéros (place + génération), ses fiches.", "vert")
    A.ecrire(d, "Les valeurs et les ordres : deux registres",
             "Au centre, l'appareil : une fiche de 16 octets qui dit où commencent ses valeurs et ses ordres, sa pièce et sa composition. À gauche, le registre des valeurs, numéros 1 234 à 1 237 pour la lampe du salon ; le volet RTS n'en a aucune. À droite, le registre des ordres, numéros 87 à 89 pour la même lampe, et les quatre ordres du volet RTS. Deux liens par référence vont des ordres vers les valeurs : le retour (la valeur qui prouve l'ordre) et le paramètre (la valeur qu'il règle).",
             432, "Deux registres à génération, celui du nettoyage, deux fois. Le seul lien entre un ordre et une valeur est un rang dans l'appareil, porté par la forme de l'ordre.")

# ================================================================ L'appareil, l'ordre, la structure
if voulu("trois"):
    d = Dessin("trois")
    d.titre(16, 24, "Ce que chacun doit connaître : l'appareil, l'ordre, la structure de l'ordre")
    cols = [(18, "L'appareil", "App · 16 o", "ambre", "maison",
             ["où sont ses valeurs (plage)", "où sont ses ordres (plage)", "sa composition (8 o par membre)", "sa pièce, physique ou virtuel"]),
            (344, "L'ordre", "Fiche · 32 o", "bleu", "eclair",
             ["son numéro : place + génération", "sa forme (un numéro)", "sa dernière commande : valeur,", "instant, issue, origine, qui, demande"]),
            (670, "La structure", "Forme · 24 o", "vert", "pile",
             ["effet : consigne ou appui", "retour : état, pont, accusé, aucun", "suivi, tolérance, drapeaux", "16 o par paramètre, 4 o par attendu"])]
    for x, t, s, ton, ic, lignes in cols:
        d.bloc(x, 54, 312, 50, ton, t, s, icone=ic)
        d.carte(x, 118, 312, 122, "Ce qu'il porte", lignes, ton, taille=11.5, sous=10.4)
    d.fleche("M332 80 L342 80", "bleu", 2.2)
    d.fleche("M658 80 L668 80", "vert", 2.2)
    for k, (x, n, ton) in enumerate([(174, f"{nb(r('catalogue', 1, 'appareils'))} appareils, dont {nb(r('catalogue', 1, 'appareils_virtuels'))} virtuels", "ambre"),
                                    (500, f"{nb(r('catalogue', 1, 'ordres'))} ordres", "bleu"),
                                    (826, f"{nb(r('catalogue', 1, 'formes'))} formes en tout", "vert")]):
        pastille(d, x, 254, n, ton)
    bandeau(d, 296, f"La maison ×1 du banc : {nb(r('catalogue', 1, 'ordres'))} ordres partagent {nb(r('catalogue', 1, 'formes'))} structures. À ×100, toujours {nb(r('catalogue', 100, 'formes'))} : la structure ne grandit pas avec la maison.", "vert", "coche")
    A.ecrire(d, "L'appareil, l'ordre, la structure",
             f"Trois colonnes reliées. L'appareil : une fiche de 16 octets, la plage de ses valeurs, celle de ses ordres, sa composition, sa pièce. L'ordre : une fiche de 32 octets, son numéro à génération, sa forme, sa dernière commande. La structure : la forme, 24 octets, partagée, plus 16 octets par paramètre et 4 par valeur attendue. Dans la maison ×1 du banc, {nb(r('catalogue', 1, 'appareils'))} appareils dont {nb(r('catalogue', 1, 'appareils_virtuels'))} virtuels, {nb(r('catalogue', 1, 'ordres'))} ordres et {nb(r('catalogue', 1, 'formes'))} formes ; à ×100, toujours {nb(r('catalogue', 100, 'formes'))} formes.",
             344, "Ta phrase, en trois fiches : il faut connaître l'appareil, l'ordre et la structure de l'ordre. Chacune a son numéro ; la troisième est partagée, comme un dictionnaire d'énumération.")

# ================================================================ Qui connaît quoi
if voulu("qui"):
    d = Dessin("qui")
    d.titre(16, 24, "Qui connaît quoi, et par où il l'apprend")
    d.bloc(390, 150, 220, 112, "vert", "Le hub", "seul rédacteur", icone="puce")
    for k, l in enumerate(["les trois registres", "catalogue, demandes, reçus", "retenues, journal"]):
        d.t(500, 206 + k * 15, l, 10.2, coul=GRIS, ancre="middle")
    autour = [(18, 52, "Le moteur", "bleu", "rouage", ["programme : ordres et valeurs par numéro", "exécutions (demande), baux"], "meta/, meta/forme/, ord/"),
              (18, 186, "Les modules", "ardoise", "radio", ["la forme qu'ils déclarent", "voies, en vol, suivi"], "cmd/ (d, i), in/…/$issue"),
              (18, 320, "Le secours", "violet", "fleches", ["tout le critique, au rang s", "pas les demandes refusées"], "meta/, ord/, wf/, val/"),
              (682, 52, "Écrans, appli, sites", "ambre", "ecran", ["formes en cache par numéro", "dernières commandes"], "MQTT : etat/, meta/, ord/, val/"),
              (682, 186, "Agents, assistant", "ambre", "personne", ["un outil par ordre, tiré de la forme", "les noms, jamais les numéros"], "MCP, REST"),
              (682, 320, "La base", "gris", "base", ["le journal des ordres, par les noms", "jamais un numéro"], "run/, hist/ops")]
    for x, y, t, ton, ic, lignes, canal in autour:
        d.carte(x, y, 300, 98, t, lignes, ton, ic, taille=12, sous=10.2)
        d.pilule(x + 14, y + 72, canal, ton, taille=9)
    for y in (100, 234, 368):
        d.fleche(f"M388 {205 if y == 234 else (180 if y < 234 else 232)} L322 {y}", "vert", 1.6, anime=True, lent=True)
        d.fleche(f"M612 {205 if y == 234 else (180 if y < 234 else 232)} L680 {y}", "vert", 1.6, anime=True, lent=True)
    bandeau(d, 432, "Chacun apprend l'appareil, l'ordre et la structure de l'ordre par le même canal, et garde des numéros ; un numéro inconnu, il le redemande.", "vert")
    A.ecrire(d, "Qui connaît quoi",
             "Au centre, le hub, seul rédacteur : les trois registres, le catalogue, les demandes, les reçus, les retenues, le journal. Autour, six consommateurs. Le moteur : son programme cite ordres et valeurs par numéro, ses exécutions la demande attendue, ses baux ; il apprend par meta/, meta/forme/ et ord/. Les modules : la forme qu'ils déclarent, leurs voies ; ils reçoivent cmd/ avec le numéro de la demande et celui de l'ordre. Le secours : tout le critique, au rang s. Les écrans, l'appli et les sites : les formes en cache par numéro, les dernières commandes ; par MQTT. Les agents et l'assistant : un outil par ordre, tiré de la forme ; les noms, par MCP et REST. La base : le journal des ordres, par les noms.",
             480, "Le même catalogue pour tous, publié une fois ; chacun garde des numéros et redemande ce qu'il ne connaît pas (la règle de la mémoire v2, § 4.4).")

# ================================================================ La forme, octet par octet
if voulu("forme"):
    d = Dessin("forme")
    d.titre(16, 24, "La structure d'un ordre, octet par octet")
    px = 38
    champs(d, 40, 60, 0, 38, [("params", 4), ("attendus", 4), ("nb", 2), ("effet", 1), ("retour", 1), ("conf.", 1), ("drap.", 1), ("suivi_s", 2), ("tol.", 2), ("schéma", 2), ("réserve", 4)], "vert", "Forme · 24 o, partagée", px)
    champs(d, 40, 140, 0, 38, [("min", 4), ("max", 4), ("pas", 2), ("dico", 2), ("nat.", 1), ("éch.", 1), ("unité", 1), ("règle", 1)], "bleu", "Param · 16 o, un par paramètre", px)
    champs(d, 40, 220, 0, 38, [("attendu", 4), ("attendu", 4)], "ambre", "Valeurs attendues · 4 o chacune", px)
    ex = [("Luminosité d'une lampe", "consigne · retour état (le point « brightness », ± 2) · 1 paramètre 0–254", "vert"),
          ("Ouvrir un volet RTS", "appui · retour pont : « exécuté » au mieux · aucun paramètre, aucune valeur", "bleu"),
          ("Démarrer un aspirateur", "appui · accusé, puis l'état ∈ {1, 5} · suivi 60 s", "ambre"),
          ("Tirer le pêne", "appui · état ∈ {3} · « ouvre » : un agent demande, un humain décide", "rouge")]
    for k, (t, s, ton) in enumerate(ex):
        d.carte(40 + (k % 2) * 470, 286 + (k // 2) * 70, 452, 58, t, [s], ton, taille=11.5, sous=10)
    A.ecrire(d, "La forme d'un ordre",
             "Trois fiches, octet par octet. La forme, 24 octets partagés : début de ses paramètres et de ses valeurs attendues, leur nombre, l'effet (consigne ou appui), le retour (état, pont, accusé, aucun), le rang du point qui confirme, les drapeaux, le suivi, la tolérance, le schéma d'un objet court. Un paramètre, 16 octets : bornes d'ordre, pas, dictionnaire, nature, échelle, unité, et le rang du point qu'il règle. Une valeur attendue, 4 octets. Quatre exemples : la luminosité d'une lampe, ouvrir un volet RTS, démarrer un aspirateur, tirer le pêne d'une serrure.",
             440, "Ce que la V7 met dans meta/ (§ 5.1 : effet, retour, suivi, bornes d'ordre, sensibilité), rangé en fiches de taille fixe. Le retour par code de la V7 devient un ordre par geste : ouvrir, fermer, stop ont chacun leur forme.")

# ================================================================ La demande, 64 octets
if voulu("demande"):
    d = Dessin("demande")
    d.titre(16, 24, "Une demande : 64 octets, une ligne de cache, dans une case réutilisée")
    px = 14.6
    champs(d, 32, 62, 0, 40, [("id (req/, res/, carte)", 16), ("valeur", 8), ("échéance", 8), ("reçue", 8), ("ordre", 4), ("qui", 4), ("mère", 4), ("params", 4), ("gén.", 2), ("état…", 6)], "bleu", "Demande · 64 o", px)
    d.t(32, 128, "état · issue · origine · raison · cibles · faites : six octets", 10.5, coul=GRIS)
    d.zone(32, 150, 456, 160, "Son numéro : la case et sa génération", "ambre", "cle")
    case(d, 52, 196, 200, 40, "ambre", "case · 16 bits", taille=11)
    case(d, 256, 196, 210, 40, "ambre", "génération · 16 bits", plein=False, taille=11)
    d.t(52, 262, "il voyage dans cmd/ et dans l'issue du module ;", 10.5, coul=GRIS)
    d.t(52, 278, "c'est le champ « demande » d'une exécution (mémoire v2 § 8.4)", 10.5, coul=GRIS)
    d.zone(512, 150, 456, 160, "Une issue tardive", "rouge", "horloge")
    case(d, 532, 196, 128, 40, "gris", "12 · gén. 6", plein=False, taille=10)
    d.t(670, 220, "→", 14, True, GRIS)
    case(d, 690, 196, 128, 40, "bleu", "12 · gén. 7", taille=10)
    d.t(532, 262, "l'issue de la 6 arrive quand la 7 occupe la case :", 10.5, coul=GRIS)
    d.t(532, 278, "écartée, journalisée ; la 7 attend la sienne", 10.5, True, TONS["rouge"][0])
    bandeau(d, 326, f"Prendre une case et la rendre : {fr(r('demandes', 1, 'prendre_et_liberer_une_case_ns'), 1)} ns. Retrouver une demande par son numéro : {fr(r('demandes', 1, 'retrouver_une_demande_ns'), 2)} ns. Une retenue est une demande comme une autre.", "vert")
    A.ecrire(d, "La demande : 64 octets",
             f"Une fiche de 64 octets : l'identifiant de la demande (16 octets, celui de req/ et res/, que porte aussi une carte d'approbation), le premier paramètre, l'échéance monotone, l'heure d'arrivée, le numéro de l'ordre, qui demande, la demande mère, les paramètres suivants, la génération, puis six octets : état, issue, origine, raison, cibles, faites. Son numéro : 16 bits de case, 16 de génération. Une issue tardive de la génération 6 arrive quand la génération 7 occupe la case : elle est écartée. Prendre une case et la rendre : {fr(r('demandes', 1, 'prendre_et_liberer_une_case_ns'), 1)} ns ; retrouver une demande : {fr(r('demandes', 1, 'retrouver_une_demande_ns'), 2)} ns.",
             374, "La règle des exécutions de la mémoire v2 (§ 8.4), appliquée aux demandes. La génération de 16 bits ne fait le tour qu'après 65 536 réutilisations de la même case : bien plus que les 180 s d'un suivi.")

# ================================================================ Le cycle d'une demande
if voulu("cycle"):
    d = Dessin("cycle")
    d.titre(16, 24, "Le cycle d'une demande, et qui la fait avancer")
    P = {"arrivee": (100, 120), "refusee": (100, 300), "retenue": (330, 300), "file": (330, 120), "suivie": (600, 120), "conclue": (870, 120), "incertaine": (600, 300)}
    lab = {"arrivee": ("Arrivée", "req/ ou un client interne", "gris"), "refusee": ("Refusée", "aucune case, une ligne", "rouge"),
           "retenue": ("Retenue", "un humain, 10 min", "ambre"), "file": ("Remise", "cmd/ : sa voie", "bleu"),
           "suivie": ("Suivie", "accepté, envoyé : la suite", "violet"), "conclue": ("Conclue", "la case se libère", "vert"),
           "incertaine": ("Incertaine", "jamais renvoyée", "rouge")}
    for k, (x, y) in P.items():
        t, s_, ton = lab[k]
        d.bouton(x, y, 196, 56, t, s_, ton)
    fleches = [("M198 120 L230 120", "vert"), ("M100 150 L100 270", "rouge"), ("M170 150 C200 200 240 230 262 270", "ambre"),
               ("M330 270 L330 150", "ambre"), ("M428 120 L500 120", "violet"), ("M698 120 L770 120", "vert"),
               ("M428 104 C540 56 760 56 860 92", "vert"), ("M420 150 C470 200 500 230 540 270", "rouge"), ("M698 300 C800 300 870 230 870 150", "vert")]
    for path, ton in fleches:
        d.fleche(path, ton, 1.8, anime=True, lent=True)
    for (x, y, t, a) in [(214, 84, "forme, garde", "middle"), (112, 214, "hors forme · retiré", "start"), (224, 192, "la garde", "start"),
                         (342, 214, "approuvée : revalidée", "start"), (464, 84, "accepté · envoyé", "middle"), (734, 106, "la suite", "middle"),
                         (640, 54, "confirmé · exécuté · échec", "middle"), (500, 204, "bascule · arrêt · module mort", "start"), (842, 262, "le hub confirme sur val/", "end")]:
        d.etiquette(x, y, t, "gris", ancre=a, taille=9.5)
    bandeau(d, 352, "Une demande entre au flux critique quand elle quitte le hub (remise, retenue), et en sort à son issue : le secours voit tout ce qui a pu partir.", "vert")
    A.ecrire(d, "Le cycle d'une demande",
             "Sept états. Une demande arrive par req/ ou d'un client interne ; hors de sa forme ou visant un appareil retiré, elle est refusée sans prendre de case. La garde peut la retenir : un humain a dix minutes ; approuvée, elle est revalidée contre la forme du moment, puis remise. Remise à son module par cmd/, elle reçoit une issue : confirmée, exécutée ou en échec, elle est conclue ; acceptée ou envoyée, elle est suivie jusqu'à sa suite, une fois. Une bascule, un arrêt ou un module mort la rendent incertaine, jamais renvoyée ; le hub la confirme lui-même sur val/ s'il peut. Conclue, sa case se libère.",
             400, "La V7 (§ 5.6, § 20.7) donne les neuf issues ; l'analyse leur donne une fiche, une case et une génération, et des traces au rang s.")

# ================================================================ Appareils physiques et virtuels
if voulu("virtuels"):
    d = Dessin("virtuels")
    d.titre(16, 24, "Physiques et virtuels : posséder, montrer, commander")
    phys = [(40, "hue:sejour_1", "Salon"), (40, "hue:sejour_2", "Salon"), (40, "hue:sejour_3", "Salon"), (40, "hue:sejour_4", "Chambre, protégée")]
    for k, (x, n, p) in enumerate(phys):
        d.carte(x, 52 + k * 70, 250, 58, n, [f"valeur on · ordre on · {p}"], "rouge" if k == 3 else "vert", "eclair", taille=11, sous=9.8)
    d.carte(40, 344, 250, 58, "nuki:porte, z2m:salon_lampe", ["leurs valeurs et leurs ordres"], "vert", "cadenas", taille=11, sous=9.8)
    d.bloc(420, 64, 240, 138, "bleu", "lumieres:sejour", "virtuel", icone="pile")
    for k, l in enumerate(["un ordre à lui : « on », de groupe", "4 cibles · 4 membres · 4 valeurs", "montrées : 12 entrées de 8 o"]):
        d.t(540, 120 + k * 16, l, 10.2, coul=GRIS, ancre="middle")
    d.bloc(420, 276, 240, 116, "ambre", "virtuel:entree", "virtuel", icone="maison")
    for k, l in enumerate(["rien à lui", "montre : la serrure (valeur, ordre),", "le volet RTS, la lampe"]):
        d.t(540, 332 + k * 16, l, 10.2, coul=GRIS, ancre="middle")
    for k in range(4):
        d.fleche(f"M418 {110 + k * 8} C360 {110 + k * 8} 340 {80 + k * 70} 292 {80 + k * 70}", "bleu", 1.4, anime=k == 0, lent=True)
    d.fleche("M418 330 C360 330 340 372 292 372", "ambre", 1.4, anime=True, lent=True)
    d.carte(714, 52, 270, 150, "Un ordre de groupe", ["une demande mère, une fille par cible", "chaque fille : sa voie, son issue", "la mère : confirmée si toutes,", "sinon échec, « partiel », n/m"], "bleu", "fleches", taille=11.5, sous=10)
    pastille(d, 849, 212, "3 sur 4 : échec, partiel", "rouge")
    d.carte(714, 250, 270, 150, "Ce que lit la garde", ["la pièce du virtuel, et celles", "de ses cibles et de ses membres :", "un agent sur le groupe : retenu,", "une ampoule est en chambre"], "ambre", "bouclier", taille=11.5, sous=10)
    bandeau(d, 414, f"Une valeur ou un ordre a un seul propriétaire ; un appareil virtuel le montre ou le commande par référence. {nb(r('catalogue', 1, 'appareils_virtuels'))} virtuels à ×1, {nb(r('catalogue', 1, 'membres'))} entrées : {octets(8 * r('catalogue', 1, 'membres'))}.", "vert")
    A.ecrire(d, "Appareils physiques et virtuels",
             f"À gauche, des appareils physiques : quatre ampoules du séjour, dont une dans la chambre protégée, la serrure et la lampe du salon ; chacun possède ses valeurs et ses ordres. Au centre, deux appareils virtuels. Le groupe lumieres:sejour possède un seul ordre, « on », de groupe ; il cible les quatre « on » des ampoules, les compte comme membres, et montre leurs valeurs : douze entrées de huit octets. L'entrée ne possède rien ; elle montre la valeur et l'ordre de la serrure, deux ordres du volet RTS et la valeur de la lampe. À droite : un ordre de groupe fait une demande mère et une fille par cible ; trois ampoules sur quatre ont obéi, la mère dit échec, partiel, 3 sur 4. La garde lit les pièces des cibles et des membres : un agent qui commande le groupe est retenu. Dans la maison ×1 du banc, {nb(r('catalogue', 1, 'appareils_virtuels'))} appareils virtuels et {nb(r('catalogue', 1, 'membres'))} entrées.",
             462, "Aujourd'hui, un groupe garde la liste de ses appareils (`members`), et les luminaires répondent « fait » dès qu'une ampoule a obéi. La composition garde des numéros à génération : une place réutilisée n'entre jamais dans un virtuel.")

# ================================================================ Les workflows interrogent les ordres
if voulu("workflows"):
    d = Dessin("workflows")
    d.titre(16, 24, "Ce qu'un workflow peut demander aux ordres, et ce que cela coûte")
    q = [("La dernière commande", "fiche[ordre] : valeur, issue, origine, instant", f"{fr(r('catalogue', 1, 'lire_derniere_commande_ns'), 2)} ns", "bleu", "document"),
         ("Un ordre est-il en cours ?", "son issue vaut « en cours », « retenue » ou « accepté »", f"{fr(r('catalogue', 1, 'lire_derniere_commande_ns'), 2)} ns", "violet", "horloge"),
         ("Quand un ordre part ou tombe", "un index par ordre, comme l'index par point", "à la trace", "ambre", "eclair"),
         ("La valeur vient-elle d'un ordre ?", "demandée, aboutie, et pas changée depuis", f"{fr(r('catalogue', 1, 'valeur_expliquee_ns'), 1)} ns", "vert", "oeil")]
    for k, (t, s, c, ton, ic) in enumerate(q):
        x, y = 16 + (k % 2) * 492, 44 + (k // 2) * 84
        d.carte(x, y, 476, 72, t, [s], ton, ic, taille=12, sous=10.2)
        d.pilule(x + 462, y + 12, c, ton, taille=9.5, ancre="end")
    d.zone(16, 222, 968, 170, "Une soirée : éteindre seulement ce que la maison a allumé", "vert", "maison")
    pas = [(110, "20:00", "allumée par l'écran", "reste : un humain", "ambre"), (330, "20:10", "interrupteur 0, puis 1", "aucun ordre ne l'explique", "rouge"),
           (560, "20:15", "la maison rallume", "l'ordre l'explique", "bleu"), (800, "20:40", "Extinction auto", "éteinte", "vert")]
    for x, h_, a, b, ton in pas:
        d.bouton(x, 296, 190, 54, h_, a, ton)
        d.t(x, 348, b, 10.5, True, TONS[ton][0], "middle")
    for x in (205, 435, 665):
        d.fleche(f"M{x} 296 L{x + 30} 296", "vert", 1.8, anime=True)
    A.ecrire(d, "Les workflows interrogent les ordres",
             f"Quatre questions et leur coût. La dernière commande : la fiche de l'ordre, {fr(r('catalogue', 1, 'lire_derniere_commande_ns'), 2)} ns. Un ordre en cours : son issue, même coût. Quand un ordre part ou tombe : un index par ordre, comme l'index par point. La valeur vient-elle d'un ordre : demandée, aboutie et pas changée depuis, {fr(r('catalogue', 1, 'valeur_expliquee_ns'), 1)} ns. En bas, la soirée du banc : à 20:00 la lampe allumée par l'écran reste ; à 20:10 un interrupteur l'éteint et la rallume, aucun ordre ne l'explique ; à 20:15 la maison la rallume ; à 20:40 l'extinction automatique l'éteint.",
             404, "Une règle sur un ordre est une règle de 16 octets, comme une règle sur une valeur : son numéro est celui d'un ordre. Le « geste à la main » se voit parce que l'instant « changé » de la valeur est plus récent que la dernière commande.")

# ================================================================ Les clients chargent par MQTT
if voulu("clients"):
    d = Dessin("clients")
    d.titre(16, 24, "Les clients se chargent par MQTT, comme un secours")
    opts = [(18, "Aujourd'hui", "SSE + REST", "gris", ["GET /api/snapshot : toute la maison en JSON", "puis un flux SSE ; l'ordre par REST", "un trou : tout recharger"]),
            (344, "La V7", "un module WebSocket", "ardoise", ["l'état filtré, par blocs de 64 Kio", "un module traduit MQTT en trames", "ordres et issues sur la connexion"]),
            (670, "L'analyse", "MQTT, directement", "vert", ["etat/demande → l'état du client, filtré", "meta/, ord/, val/ : le flux, au rang s", "req/ du compte, res/<client>"])]
    for x, t, s, ton, lignes in opts:
        d.bloc(x, 54, 312, 50, ton, t, s, icone="ecran")
        d.carte(x, 118, 312, 98, "Comment", lignes, ton, taille=11.5, sous=10.2)
    W = 600
    mx = r("clients", 1, "aujourdhui_api_snapshot_json_o")
    barres = [("Aujourd'hui : /api/snapshot, JSON", r("clients", 1, "aujourdhui_api_snapshot_json_o"), "gris"),
              ("MQTT : l'état complet d'un écran", r("clients", 1, "mqtt_etat_client_complet_o"), "vert"),
              ("MQTT : un écran de deux pièces, un invité", r("clients", 1, "mqtt_etat_client_deux_pieces_o"), "bleu")]
    d.t(18, 246, "Ce qu'un écran charge pour démarrer, maison ×1", 11, True, ENCRE)
    for k, (lab, v, ton) in enumerate(barres):
        y = 260 + k * 30
        d.t(18, y + 14, lab, 10.5, coul=ENCRE)
        d.barre(330, y + 2, W * v / mx, 16, ton)
        d.t(330 + W * v / mx + 6, y + 15, octets(v), 10.5, True, TONS[ton][0])
    bandeau(d, 360, "Un écran devient une copie partielle : le même état complet, le même rang s, la même règle du numéro inconnu que le secours ; l'historique et les médias restent en HTTP.", "vert")
    A.ecrire(d, "Les clients se chargent par MQTT",
             f"Trois façons de servir un écran. Aujourd'hui : toute la maison en JSON par /api/snapshot, puis un flux SSE, l'ordre par REST ; un trou fait tout recharger. La V7 : un module WebSocket qui envoie l'état filtré par blocs et traduit MQTT en trames. L'analyse : l'écran parle MQTT directement ; il demande son état sur etat/, filtré par ses droits, puis suit meta/, ord/ et val/ au rang s ; il publie ses demandes sur req/ avec son compte et reçoit ses issues sur res/. Ce qu'un écran charge pour démarrer, maison ×1 : {octets(r('clients', 1, 'aujourdhui_api_snapshot_json_o'))} de JSON aujourd'hui, {octets(r('clients', 1, 'mqtt_etat_client_complet_o'))} par MQTT, {octets(r('clients', 1, 'mqtt_etat_client_deux_pieces_o'))} pour un écran de deux pièces.",
             408, "Ta remarque : puisque tout est en mémoire, l'appareil, ses ordres et ses valeurs, un client peut se recharger par MQTT, sans module dédié. Ce qu'il en coûte par connexion est au § 14.4.")

# ================================================================ La bascule
if voulu("bascule"):
    d = Dessin("bascule")
    d.titre(16, 24, "La bascule : ce qui était en cours sur l'ancien actif")
    d.bloc(30, 60, 270, 120, "vert", "L'actif", "jusqu'à 21:00", icone="puce")
    for k, l in enumerate(["publie chaque pas d'une demande", "sur ord/, au rang s", "les retenues et les baux sur wf/"]):
        d.t(165, 118 + k * 16, l, 10.2, coul=GRIS, ancre="middle")
    d.bloc(700, 60, 270, 120, "bleu", "Le secours", "puis l'actif", icone="fleches")
    for k, l in enumerate(["les mêmes cases, générations, fiches", "convertit chaque échéance", "dans sa propre horloge"]):
        d.t(835, 118 + k * 16, l, 10.2, coul=GRIS, ancre="middle")
    d.fleche("M310 110 L690 110", "ambre", 2.4, anime=True)
    d.etiquette(500, 100, "ord/ · wf/ · val/ · meta/ — au rang s", "ambre")
    cas = [("Remise, partie ou non", "incertaine (bascule), jamais renvoyée ; le hub la confirme sur val/", "rouge"),
           ("Suivie (accepté, envoyé)", "la suite arrive au nouvel actif ; il conclut lui-même", "violet"),
           ("Retenue", "la carte reste ; l'humain décide sur le nouvel actif ; revalidée", "ambre"),
           ("Un bail « pendant »", "rétabli à son terme par le nouvel actif, même en retard", "vert"),
           ("Un appui retenté", "le reçu est venu par la trace : il ne part pas deux fois", "bleu")]
    for k, (t, s, ton) in enumerate(cas):
        d.carte(30 + (k % 2) * 480, 202 + (k // 2) * 62, 460, 52, t, [s], ton, taille=11.2, sous=9.8)
    A.ecrire(d, "La bascule",
             "À gauche, l'actif publie chaque pas d'une demande sur ord/, au rang s, et ses retenues et ses baux sur wf/. À droite, le secours tient les mêmes cases, générations et fiches ; il convertit chaque échéance dans sa propre horloge. Cinq cas à la bascule : une demande remise devient incertaine, jamais renvoyée, et le hub du nouvel actif la confirme sur val/ s'il peut ; une demande suivie reçoit sa suite au nouvel actif ; une retenue reste à décider et sera revalidée ; un bail est rétabli à son terme, même en retard ; un appui retenté ne part pas deux fois, le reçu étant venu par la trace.",
             392, "La règle de la HA v2 (§ 15) : un ordre envoyé sans issue est « incertain », jamais renvoyé. L'analyse la rend précise : le secours sait quelles demandes ont pu partir, et le hub, qui connaît la forme, conclut.")

# ================================================================ La mémoire, avant et après
if voulu("memoire"):
    d = Dessin("memoire")
    d.titre(16, 24, "Ce que la mémoire des ordres coûte, et ce qu'elle remplace (maison ×1)")
    lignes = [("Décrire les appareils, valeurs et ordres", r("catalogue", 1, "aujourdhui_description_o"), r("catalogue", 1, "propose_description_totale_o"), kio),
              ("Le journal en mémoire (500 lignes)", r("demandes", 1, "aujourdhui_journal_500_o"), r("demandes", 1, "memoire_journal_o"), kio),
              ("50 retenues", r("demandes", 1, "aujourdhui_retenues_50_o"), r("demandes", 1, "propose_retenues_50_o"), kio),
              ("Une demande en vol (au mieux)", r("demandes", 1, "aujourdhui_une_demande_en_vol_o"), 64, kio),
              ("Copier la maison pour l'assistant, à chaque requête", r("catalogue", 1, "assistant_copie_de_la_maison_o"), 1, kio),
              ("Ce qu'un écran charge pour démarrer (transfert)", r("clients", 1, "aujourdhui_api_snapshot_json_o"), r("clients", 1, "mqtt_etat_client_complet_o"), octets)]
    x0, W, vmin, vmax = 420, 470, 30, 2e6
    for k, (lab, avant, apres, u) in enumerate(lignes):
        y = 50 + k * 50
        d.t(16, y + 16, lab, 10.8, True, ENCRE)
        d.barre(x0, y + 2, xlog(avant, vmin, vmax, x0, W) - x0, 13, "gris")
        d.t(xlog(avant, vmin, vmax, x0, W) + 6, y + 13, u(avant), 10, True, TONS["gris"][0])
        if apres > 1:
            d.barre(x0, y + 19, xlog(apres, vmin, vmax, x0, W) - x0, 13, "vert")
            d.t(xlog(apres, vmin, vmax, x0, W) + 6, y + 30, u(apres), 10, True, TONS["vert"][0])
        else:
            d.t(x0 + 4, y + 30, "rien : lu dans les colonnes", 10, True, TONS["vert"][0])
    axe_log(d, x0, W, 46, 352, vmin, vmax, "octets, échelle logarithmique", [(100, "100 o"), (1e3, "10³ o"), (1e4, "10⁴ o"), (1e5, "10⁵ o"), (1e6, "10⁶ o")])
    d.pilule(16, 392, "aujourd'hui", "gris", plein=True)
    d.pilule(116, 392, "proposé", "vert", plein=True)
    A.ecrire(d, "La mémoire, avant et après",
             "Six postes, aujourd'hui en gris, proposé en vert, en échelle logarithmique. Décrire les appareils, leurs valeurs et leurs ordres : " + kio(r("catalogue", 1, "aujourdhui_description_o")) + " contre " + kio(r("catalogue", 1, "propose_description_totale_o")) + ". Le journal en mémoire : " + kio(r("demandes", 1, "aujourdhui_journal_500_o")) + " contre " + kio(r("demandes", 1, "memoire_journal_o")) + ". Cinquante retenues : " + kio(r("demandes", 1, "aujourdhui_retenues_50_o")) + " contre " + kio(r("demandes", 1, "propose_retenues_50_o")) + ". Une demande en vol : " + kio(r("demandes", 1, "aujourdhui_une_demande_en_vol_o")) + " au mieux contre 64 octets. La copie de la maison pour l'assistant : " + kio(r("catalogue", 1, "assistant_copie_de_la_maison_o")) + " à chaque requête, contre rien. Ce qu'un écran charge : " + octets(r("clients", 1, "aujourdhui_api_snapshot_json_o")) + " contre " + octets(r("clients", 1, "mqtt_etat_client_complet_o")) + ".",
             420, "Les répliques d'aujourd'hui sont au mieux : ni les canaux de Tokio, ni les tâches, ni la sérialisation ne sont comptés. Le proposé compte tout ce qu'il range, dont des choses qui n'existent pas aujourd'hui : la structure complète des ordres, leur dernière commande, les compositions.")

print(f"ok : {len(A.noms)} schémas → {A.F} et {A.MD}")
