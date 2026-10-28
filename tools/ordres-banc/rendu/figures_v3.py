#!/usr/bin/env python3
"""Schémas et graphiques de l'analyse « les ordres des appareils », v3
(docs/proposition/ordres-appareils-v3.md), en relief, avec la boîte à outils de la haute
disponibilité v2 (tools/haute-dispo-banc/rendu/relief.py). Ceux de la v2, repris (la demande et « qui
connaît quoi » mis à jour), et ceux des droits. Les graphiques lisent la série v3 du banc
(../sorties/linux/*.txt) et ses fautes injectées (../sorties/mutations.txt) : aucun chiffre recopié, la
table des droits comprise. Écrit les fragments dans rendu/fig3/ (build.py les insère dans la page) et
les SVG autonomes, légende comprise, dans docs/proposition/ordres-appareils-v3/.

Usage : python3 -I tools/ordres-banc/rendu/figures_v3.py [schéma …]"""
import math
import pathlib
import re
import sys

ICI = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ICI.parents[1] / "haute-dispo-banc" / "rendu"))
from relief import ENCRE, GRIS, LIGNE, TONS, Atelier, Dessin, court, fr, largeur, lire_resultats, nb  # noqa: E402

RES = lire_resultats(ICI.parent / "sorties/linux")
A = Atelier(ICI / "fig3", ICI.parents[2] / "docs/proposition/ordres-appareils-v3")
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
        case(d, xx + 1, y, ww - 2, h, ton, None, plein=(k % 2 == 0))
        if ww > largeur(nom, 9) + 6:
            d.t(xx + ww / 2, y + h / 2 - 2, nom, 9, True, "#ffffff" if k % 2 == 0 else TONS[ton][0], "middle")
            d.t(xx + ww / 2, y + h / 2 + 10, f"{n} o", 8.5, coul="#ffffff" if k % 2 == 0 else GRIS, ancre="middle")
        xx += ww
    return xx


# ================================================================ La revue : 25 constats, 25 corrections
if voulu("revue"):
    d = Dessin("revue")
    d.titre(16, 24, "La revue Codex : 25 constats, chacun corrigé, chacun prouvé")
    bloquants = [("B1", "Qui demande : la connexion"), ("B2", "Un appui : l'accusé d'abord"), ("B3", "Reçus : la plus haute séquence"),
                 ("B4", "Numéro : époque et 32 bits"), ("B5", "Texte privé : canal privé")]
    importants = [("I1", "Permis revérifié"), ("I2", "Échéance à chaque frontière"), ("I3", "Forme d'admission"), ("I4", "Lecture fraîche seule"),
                  ("I5", "Oui, non, on ne sait pas"), ("I6", "La dernière admise"), ("I7", "Un seul chemin d'envoi"), ("I8", "n/m par niveau de preuve"),
                  ("I9", "Mère sans échéance"), ("I10", "Cibles rattachées"), ("I11", "Numéros par clé"), ("I12", "Plafonds vérifiés"),
                  ("I13", "Rang de chaque pièce"), ("I14", "Dictionnaire et schéma"), ("I15", "Témoin honnête"), ("I16", "Même corpus"),
                  ("I17", "Contenu égal"), ("I18", "Tailles vérifiées"), ("I19", "Toute la boucle"), ("I20", "Instantané seul")]
    d.t(16, 52, "Bloquants", 11, True, TONS["rouge"][0])
    for k, (c, t) in enumerate(bloquants):
        x = 16 + k * 194
        d.carte(x, 60, 184, 60, c, [t], "rouge", "coche", taille=11.5, sous=9.8)
    d.t(16, 144, "Importants", 11, True, TONS["ambre"][0])
    for k, (c, t) in enumerate(importants):
        x = 16 + (k % 5) * 194
        y = 152 + (k // 5) * 68
        d.carte(x, y, 184, 60, c, [t], "ambre", "coche", taille=11.5, sous=9.8)
    bandeau(d, 428, "Chacun vérifié dans le code, puis corrigé : 20 corrections de conception gardées par 35 fautes injectées, 5 mesures refaites.", "vert")
    A.ecrire(d, "Ce que la revue a trouvé, et ce que la v2 en fait",
             "Vingt-cinq cartes. En haut, les cinq constats bloquants : qui demande vient de la connexion ; un appui attend l'accusé du secours ; les reçus gardent la plus haute séquence ; le numéro d'une demande porte l'époque et une génération de 32 bits ; un texte privé ne passe que par le canal privé. En dessous, les vingt importants : le permis revérifié, l'échéance à chaque frontière, la forme d'admission, la lecture fraîche, les trois réponses de « vient de », la dernière commande admise, un seul chemin d'envoi, n/m par niveau de preuve, la mère sans échéance, les cibles rattachées, les numéros par clé, les plafonds, le rang de chaque pièce, le dictionnaire et le schéma, un témoin honnête, le même corpus, un contenu égal, des tailles vérifiées, toute la boucle chronométrée, l'instantané seul au redémarrage. Chacun est corrigé : ceux de conception sont testés et gardés par des fautes injectées, ceux de mesure sont refaits.",
             476, "La revue a lu la v1, le banc et le code d'aujourd'hui ; aucun constat n'était faux. Le § 3 donne, pour chacun, la correction, le test et le nombre de fautes injectées.")

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
    d.titre(16, 24, "Les valeurs et les ordres : deux registres, un lien par clé")
    d.bloc(390, 64, 220, 120, "ambre", "L'appareil", "16 o", icone="maison")
    d.t(500, 130, "sa liste : ses valeurs,", 10.5, coul=GRIS, ancre="middle")
    d.t(500, 146, "puis ses ordres (des numéros)", 10.5, coul=GRIS, ancre="middle")
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
    d.t(670, 330, "Fiche 40 o · forme partagée", 10.5, coul=GRIS)
    d.fleche("M612 108 C630 100 640 96 652 96", "bleu", 1.8)
    d.fleche("M388 108 C370 100 360 96 348 96", "vert", 1.8)
    d.fleche("M668 140 C600 252 400 252 300 140", "ambre", 1.8, anime=True, tirets="6 4")
    d.etiquette(500, 246, "retour : la valeur de clé « brightness »", "ambre")
    d.fleche("M668 184 C600 312 400 312 300 184", "ambre", 1.8, anime=True, lent=True, tirets="6 4")
    d.etiquette(500, 304, "paramètre : la valeur qu'il règle, par sa clé", "ambre")
    bandeau(d, 384, "Une valeur n'est jamais un ordre ; la forme cite une clé, jamais un rang : une valeur ajoutée au milieu ne déplace rien.", "vert")
    A.ecrire(d, "Les valeurs et les ordres : deux registres",
             "Au centre, l'appareil : une fiche de 16 octets qui pointe sa liste, les numéros de ses valeurs puis de ses ordres, sa pièce et sa composition. À gauche, le registre des valeurs, numéros 1 234 à 1 237 pour la lampe du salon ; le volet RTS n'en a aucune. À droite, le registre des ordres, numéros 87 à 89 pour la même lampe, et les quatre ordres du volet RTS. Deux liens vont des ordres vers les valeurs, par la clé de la valeur : le retour (la valeur qui prouve l'ordre) et le paramètre (la valeur qu'il règle).",
             432, "Deux registres à génération, celui du nettoyage, deux fois. La v1 citait un rang dans l'appareil ; la v2 cite une clé, qu'une redéclaration ne déplace pas (revue I11).")

# ================================================================ L'appareil, l'ordre, la structure
if voulu("trois"):
    d = Dessin("trois")
    d.titre(16, 24, "Ce que chacun doit connaître : l'appareil, l'ordre, la structure de l'ordre")
    cols = [(18, "L'appareil", f"App · {nb(r('catalogue', 1, 'taille_app_o'))} o", "ambre", "maison",
             ["sa liste : ses valeurs, puis ses ordres", "sa composition (8 o par membre)", "sa pièce", "physique ou virtuel"]),
            (344, "L'ordre", f"Fiche · {nb(r('catalogue', 1, 'taille_fiche_ordre_o'))} o", "bleu", "eclair",
             ["sa forme ; sa dernière commande admise", "valeur, instant, qui, origine, issue", "« en cours » : ses demandes vivantes", "sa preuve : la valeur suit-elle ?"]),
            (670, "La structure", f"Forme · {nb(r('catalogue', 1, 'taille_forme_o'))} o", "vert", "pile",
             ["effet : consigne ou appui", "retour : état, pont, accusé, aucun", "la valeur qui prouve : une clé", f"{nb(r('catalogue', 1, 'taille_param_o'))} o par paramètre, 4 o par attendu"])]
    for x, t, s, ton, ic, lignes in cols:
        d.bloc(x, 54, 312, 50, ton, t, s, icone=ic)
        d.carte(x, 118, 312, 122, "Ce qu'il porte", lignes, ton, taille=11.5, sous=10.4)
    d.fleche("M332 80 L342 80", "bleu", 2.2)
    d.fleche("M658 80 L668 80", "vert", 2.2)
    for k, (x, n, ton) in enumerate([(174, f"{nb(r('catalogue', 1, 'appareils'))} appareils, dont {nb(r('catalogue', 1, 'appareils_virtuels'))} virtuels", "ambre"),
                                    (500, f"{nb(r('catalogue', 1, 'ordres'))} ordres", "bleu"),
                                    (826, f"{nb(r('catalogue', 1, 'formes'))} formes en tout", "vert")]):
        pastille(d, x, 254, n, ton)
    bandeau(d, 296, f"Chaque taille est vérifiée à la compilation. {nb(r('catalogue', 1, 'ordres'))} ordres partagent {nb(r('catalogue', 1, 'formes'))} structures ; à ×100, toujours {nb(r('catalogue', 100, 'formes'))}.", "vert", "coche")
    A.ecrire(d, "L'appareil, l'ordre, la structure",
             f"Trois colonnes reliées. L'appareil : une fiche de {nb(r('catalogue', 1, 'taille_app_o'))} octets, sa liste de valeurs et d'ordres, sa composition, sa pièce. L'ordre : une fiche de {nb(r('catalogue', 1, 'taille_fiche_ordre_o'))} octets, sa forme, sa dernière commande admise, ses demandes vivantes, sa preuve. La structure : la forme, {nb(r('catalogue', 1, 'taille_forme_o'))} octets, partagée, plus {nb(r('catalogue', 1, 'taille_param_o'))} octets par paramètre et 4 par valeur attendue ; elle cite les valeurs par leur clé. Dans la maison ×1 du banc, {nb(r('catalogue', 1, 'appareils'))} appareils dont {nb(r('catalogue', 1, 'appareils_virtuels'))} virtuels, {nb(r('catalogue', 1, 'ordres'))} ordres et {nb(r('catalogue', 1, 'formes'))} formes ; à ×100, toujours {nb(r('catalogue', 100, 'formes'))} formes.",
             344, "Ta phrase, en trois fiches. La v1 annonçait 16, 32 et 24 octets, dont deux faux ; la v2 vérifie chaque taille par une assertion à la compilation (revue I18).")

# ================================================================ Qui connaît quoi
if voulu("qui"):
    d = Dessin("qui")
    d.titre(16, 24, "Qui connaît quoi, par où il l'apprend, et en quel nom il demande")
    d.bloc(390, 150, 220, 112, "vert", "Le hub", "seul rédacteur", icone="puce")
    for k, l in enumerate(["les trois registres, le catalogue", "demandes, reçus, retenues", "comptes, personnes, droits"]):
        d.t(500, 206 + k * 15, l, 10.2, coul=GRIS, ancre="middle")
    autour = [(18, 52, "Le moteur", "bleu", "rouage", ["un principal interne", "exécutions (demande), baux"], "meta/, meta/forme/, ord/"),
              (18, 186, "Les modules", "ardoise", "radio", ["la forme de la demande, dans cmd/", "un texte privé : par le canal privé"], "cmd/ (d, f, i), in/…/$issue"),
              (18, 320, "Le secours", "violet", "fleches", ["tout le critique, au rang s", "il accuse chaque rang"], "meta/, ord/, prive/, wf/, val/"),
              (682, 52, "Écrans, appli, sites", "ambre", "ecran", ["un compte, un permis, ses pièces", "le rang de chacune de ses pièces"], "MQTT : etat/, meta/, ord/, val/"),
              (682, 186, "Agents, assistant", "ambre", "personne", ["un compte, un permis", "un outil par ordre, en unités humaines"], "MCP, REST, req/<compte>"),
              (682, 320, "La base", "gris", "base", ["le journal des ordres, par les noms", "jamais un numéro"], "run/, hist/ops")]
    for x, y, t, ton, ic, lignes, canal in autour:
        d.carte(x, y, 300, 98, t, lignes, ton, ic, taille=12, sous=10.2)
        d.pilule(x + 14, y + 72, canal, ton, taille=9)
    for y in (100, 234, 368):
        d.fleche(f"M388 {205 if y == 234 else (180 if y < 234 else 232)} L322 {y}", "vert", 1.6, anime=True, lent=True)
        d.fleche(f"M612 {205 if y == 234 else (180 if y < 234 else 232)} L680 {y}", "vert", 1.6, anime=True, lent=True)
    bandeau(d, 432, "Chacun apprend l'appareil, l'ordre et sa structure par le même canal ; chacun demande au nom de sa connexion, jamais d'un nom qu'il déclare.", "vert")
    A.ecrire(d, "Qui connaît quoi",
             "Au centre, le hub, seul rédacteur : les trois registres, le catalogue, les demandes, les reçus, les retenues, les principaux et leurs permis. Autour, six consommateurs. Le moteur, un principal interne : son programme cite ordres et valeurs par numéro. Les modules : la forme de la demande arrive dans cmd/ ; un texte privé, par le canal privé. Le secours : tout le critique, au rang s, et il accuse chaque rang. Les écrans, l'appli et les sites : un compte, un permis, leurs pièces et le rang de chacune, par MQTT. Les agents et l'assistant : un compte, un permis, un outil par ordre ; par MCP, REST, ou req/ de leur compte. La base : le journal des ordres, par les noms.",
             480, "Le même catalogue pour tous, publié une fois ; chacun garde des numéros et redemande ce qu'il ne connaît pas. Qui demande vient de la connexion (revue B1).")

# ================================================================ La forme, octet par octet
if voulu("forme"):
    d = Dessin("forme")
    d.titre(16, 24, "La structure d'un ordre, octet par octet")
    px = 36
    champs(d, 40, 60, 0, 38, [("params", 4), ("attendus", 4), ("confirme", 2), ("suivi_s", 2), ("tol.", 2), ("schéma", 2), ("nb", 2), ("effet", 1), ("retour", 1), ("drap.", 1), ("rés.", 3)], "vert", "Forme · 24 o, partagée", px)
    champs(d, 40, 140, 0, 38, [("min", 4), ("max", 4), ("pas", 2), ("dico", 2), ("règle", 2), ("nat.", 1), ("éch.", 1), ("unité", 1), ("rés.", 3)], "bleu", "Param · 20 o, un par paramètre", px)
    champs(d, 40, 220, 0, 38, [("attendu", 4), ("attendu", 4)], "ambre", "Valeurs attendues · 4 o chacune", px)
    d.t(500, 248, "« confirme » et « règle » : la clé d'une valeur, sur 16 bits", 10.5, True, TONS["bleu"][0])
    ex = [("Luminosité d'une lampe", "consigne · retour état : la valeur « brightness », ± 2 · 1 paramètre 0–254", "vert"),
          ("Ouvrir un volet RTS", "appui · retour pont : « exécuté » au mieux · aucun paramètre, aucune valeur", "bleu"),
          ("Démarrer un aspirateur", "appui · accusé, puis l'état ∈ {1, 5} · suivi 60 s", "ambre"),
          ("Une annonce", "appui · texte privé : ni sur cmd/, ni sur ord/ ; par le canal privé", "rouge")]
    for k, (t, s, ton) in enumerate(ex):
        d.carte(40 + (k % 2) * 470, 286 + (k // 2) * 70, 452, 58, t, [s], ton, taille=11.5, sous=10)
    A.ecrire(d, "La forme d'un ordre",
             "Trois fiches, octet par octet. La forme, 24 octets partagés : début de ses paramètres et de ses valeurs attendues, la clé de la valeur qui confirme, le suivi, la tolérance, le schéma d'un objet court, leur nombre, l'effet, le retour, les drapeaux. Un paramètre, 20 octets : bornes d'ordre, pas, dictionnaire, la clé de la valeur qu'il règle, nature, échelle, unité. Une valeur attendue, 4 octets. Quatre exemples : la luminosité d'une lampe, ouvrir un volet RTS, démarrer un aspirateur, une annonce au texte privé.",
             440, "Ce que la V7 met dans meta/, rangé en fiches de taille fixe. La v2 cite les valeurs par leur clé, et vérifie le dictionnaire d'une énumération et le schéma d'un objet (revue I11, I14).")

# ================================================================ La demande, 72 octets
if voulu("demande"):
    d = Dessin("demande")
    d.titre(16, 24, "Une demande : 72 octets, dans une case réutilisée, et son numéro de 64 bits")
    px = 13.0
    champs(d, 32, 62, 0, 40, [("valeur", 8), ("séquence", 8), ("échéance", 8), ("reçue", 8), ("mère", 8), ("ordre", 4), ("qui", 4), ("gén.", 4), ("forme·de", 4), ("état…", 4), ("groupe", 4), ("preuve", 4), ("pour", 4)], "bleu", "Demande · 72 o", px)
    d.t(32, 128, "état · issue · origine · raison ; un groupe ; drapeaux, preuve, époque ; v3 : la personne et le mandat, dans la réserve", 10.5, coul=GRIS)
    d.zone(32, 150, 456, 160, "Son numéro : case, époque, génération", "ambre", "cle")
    case(d, 52, 196, 110, 40, "ambre", "case · 16", taille=11)
    case(d, 166, 196, 120, 40, "ambre", "époque · 16", plein=False, taille=11)
    case(d, 290, 196, 176, 40, "ambre", "génération · 32", taille=11)
    d.t(52, 262, "l'époque sépare deux actifs : jamais un numéro redonné ;", 10.5, coul=GRIS)
    d.t(52, 278, "une case épuisée (quatre milliards de fois) est condamnée", 10.5, coul=GRIS)
    d.zone(512, 150, 456, 160, "Une issue tardive", "rouge", "horloge")
    case(d, 532, 196, 128, 40, "gris", "12 · E7 · gén. 6", plein=False, taille=10)
    d.t(670, 220, "→", 14, True, GRIS)
    case(d, 690, 196, 128, 40, "bleu", "12 · E7 · gén. 7", taille=10)
    d.t(532, 262, "l'issue de la 6 arrive quand la 7 occupe la case,", 10.5, coul=GRIS)
    d.t(532, 278, "ou celle de l'ancien actif (E6) : écartée", 10.5, True, TONS["rouge"][0])
    bandeau(d, 326, f"Prendre une case et la rendre : {fr(r('demandes', 1, 'prendre_et_liberer_une_case_ns'), 1)} ns. Retrouver une demande : {fr(r('demandes', 1, 'retrouver_une_demande_ns'), 2)} ns. Sa clé pour l'appelant : principal et séquence.", "vert")
    A.ecrire(d, "La demande : 72 octets",
             f"Une fiche de 72 octets : le premier paramètre, la séquence de l'appelant, l'échéance monotone, l'heure d'arrivée, la demande mère, le numéro de l'ordre, qui demande, la génération, la forme d'admission, le principal, puis l'état, l'issue, l'origine, la raison, les compteurs d'un groupe, les drapeaux, la preuve qu'on a de la personne et l'époque ; enfin la personne pour qui elle est faite et le mandat d'un automatisme, dans les octets que la v2 réservait. Son numéro : 16 bits de case, 16 d'époque, 32 de génération. Une issue tardive de la génération 6, ou de l'ancien actif, arrive quand la génération 7 occupe la case : elle est écartée. Prendre une case et la rendre : {fr(r('demandes', 1, 'prendre_et_liberer_une_case_ns'), 1)} ns ; retrouver une demande : {fr(r('demandes', 1, 'retrouver_une_demande_ns'), 2)} ns.",
             374, "La v1 tenait en 64 octets, avec une génération de 16 bits qui faisait le tour, et sans époque (revue B4). La v3 range la personne, sa preuve et le mandat dans la réserve : la fiche ne grandit pas.")

# ================================================================ Le cycle d'une demande
if voulu("cycle"):
    d = Dessin("cycle")
    d.titre(16, 24, "Le cycle d'une demande, et qui la fait avancer")
    P = {"arrivee": (100, 120), "refusee": (100, 300), "retenue": (330, 300), "file": (330, 120), "suivie": (600, 120), "conclue": (870, 120), "incertaine": (600, 300), "mere": (870, 300)}
    lab = {"arrivee": ("Arrivée", "req/<compte>, ou interne", "gris"), "refusee": ("Refusée", "aucune case, une ligne", "rouge"),
           "retenue": ("Retenue", "un humain, 10 min", "ambre"), "file": ("Remise", "un appui : l'accusé d'abord", "bleu"),
           "suivie": ("Suivie", "accepté, envoyé : la suite", "violet"), "conclue": ("Conclue", "la case se libère", "vert"),
           "incertaine": ("Incertaine", "jamais renvoyée", "rouge"), "mere": ("Mère", "un groupe : ses filles", "ardoise")}
    for k, (x, y) in P.items():
        t, s_, ton = lab[k]
        d.bouton(x, y, 196, 56, t, s_, ton)
    fleches = [("M198 120 L230 120", "vert"), ("M100 150 L100 270", "rouge"), ("M170 150 C200 200 240 230 262 270", "ambre"),
               ("M330 270 L330 150", "ambre"), ("M428 120 L500 120", "violet"), ("M698 120 L770 120", "vert"),
               ("M428 104 C540 56 760 56 860 92", "vert"), ("M420 150 C470 200 500 230 540 270", "rouge"), ("M698 300 C800 300 870 230 870 150", "vert"),
               ("M428 136 C560 220 700 280 772 300", "ardoise"), ("M870 270 L870 150", "ardoise")]
    for path, ton in fleches:
        d.fleche(path, ton, 1.6, anime=True, lent=True)
    for (x, y, t, a) in [(214, 84, "droits, échéance, forme, clé", "middle"), (112, 214, "hors droits · échue · conflit", "start"), (224, 192, "la garde", "start"),
                         (342, 214, "approuvée : tout revérifié", "start"), (464, 84, "accepté · envoyé", "middle"), (734, 106, "la suite", "middle"),
                         (640, 54, "confirmé · exécuté · échec", "middle"), (500, 204, "bascule · arrêt · module mort", "start"), (842, 262, "lecture fraîche", "end")]:
        d.etiquette(x, y, t, "gris", ancre=a, taille=9.5)
    bandeau(d, 352, "Le permis, l'échéance et la forme sont revérifiés à chaque frontière ; une demande garde sa forme d'admission jusqu'au bout.", "vert")
    A.ecrire(d, "Le cycle d'une demande",
             "Huit états. Une demande arrive par req/ de son compte, ou d'un client interne ; hors de ses droits, échue, hors de sa forme, avec une clé en conflit ou périmée, elle est refusée sans prendre de case. La garde peut la retenir : un humain a dix minutes ; à l'approbation, l'échéance, le permis et la forme sont revérifiés. Remise à son module (un appui attend d'abord l'accusé du secours), elle reçoit une issue : confirmée, exécutée ou en échec, elle est conclue ; acceptée ou envoyée, elle est suivie jusqu'à sa suite. Une bascule, un arrêt ou un module mort la rendent incertaine, jamais renvoyée ; le hub la confirme par une lecture faite après la reprise. Un ordre de groupe devient une mère, qui conclut avec sa dernière fille.",
             400, "La V7 donne les neuf issues ; l'analyse leur donne une fiche, une case, un numéro à époque, et des traces au rang s. La v2 ajoute la mère sans échéance (revue I9) et les vérifications à chaque frontière (I1, I2, I3).")

# ================================================================ Appareils physiques et virtuels
if voulu("virtuels"):
    d = Dessin("virtuels")
    d.titre(16, 24, "Physiques et virtuels : posséder, montrer, commander")
    phys = [(40, "hue:sejour_1", "Salon"), (40, "hue:sejour_2", "Salon"), (40, "hue:sejour_3", "Salon"), (40, "hue:sejour_4", "Chambre, protégée")]
    for k, (x, n, p) in enumerate(phys):
        d.carte(x, 52 + k * 70, 250, 58, n, [f"valeur on · ordre on · {p}"], "rouge" if k == 3 else "vert", "eclair", taille=11, sous=9.8)
    d.carte(40, 344, 250, 58, "nuki:porte, z2m:salon_lampe", ["leurs valeurs et leurs ordres"], "vert", "cadenas", taille=11, sous=9.8)
    d.bloc(420, 64, 240, 138, "bleu", "lumieres:sejour", "virtuel", icone="pile")
    for k, l in enumerate(["un ordre de groupe : « on »", "4 cibles rattachées à « on »", "4 membres, 4 valeurs montrées"]):
        d.t(540, 120 + k * 16, l, 10.2, coul=GRIS, ancre="middle")
    d.bloc(420, 276, 240, 116, "ambre", "virtuel:entree", "virtuel", icone="maison")
    for k, l in enumerate(["rien à lui", "montre : la serrure (valeur, ordre),", "le volet RTS, la lampe"]):
        d.t(540, 332 + k * 16, l, 10.2, coul=GRIS, ancre="middle")
    for k in range(4):
        d.fleche(f"M418 {110 + k * 8} C360 {110 + k * 8} 340 {80 + k * 70} 292 {80 + k * 70}", "bleu", 1.4, anime=k == 0, lent=True)
    d.fleche("M418 330 C360 330 340 372 292 372", "ambre", 1.4, anime=True, lent=True)
    d.carte(714, 52, 270, 150, "Un ordre de groupe", ["la mère connaît ses cibles d'abord,", "sans échéance à elle ; une fille par cible,", "autorisée ; la mère prend la preuve", "la plus faible : confirmé, exécuté, n/m"], "bleu", "fleches", taille=11.5, sous=10)
    pastille(d, 849, 212, "3 sur 4 : échec, partiel", "rouge")
    d.carte(714, 250, 270, 150, "Approuvé, ou coupé par une bascule", ["approuvé : le même répartiteur,", "ses filles ne sont pas retenues ;", "coupé : la copie a rejoué son compte,", "le nouvel actif conclut 3 sur 4"], "ambre", "bouclier", taille=11.5, sous=10)
    bandeau(d, 414, f"Une valeur ou un ordre a un seul propriétaire ; une cible est l'ordre d'un appareil physique, rattachée à son ordre de groupe. {nb(r('catalogue', 1, 'appareils_virtuels'))} virtuels à ×1, {nb(r('catalogue', 1, 'membres'))} entrées.", "vert")
    A.ecrire(d, "Appareils physiques et virtuels",
             f"À gauche, des appareils physiques : quatre ampoules du séjour, dont une dans la chambre protégée, la serrure et la lampe du salon. Au centre, deux appareils virtuels. Le groupe lumieres:sejour possède un ordre de groupe, « on », auquel ses quatre cibles sont rattachées ; il compte les ampoules comme membres et montre leurs valeurs. L'entrée ne possède rien ; elle montre la serrure, deux ordres du volet RTS et la lampe. À droite : un ordre de groupe fait une mère, qui connaît ses cibles avant la première fille et n'a pas d'échéance à elle ; chaque fille est autorisée ; la mère prend la preuve la plus faible, et dit 3 sur 4 quand une ampoule ne répond pas. Approuvé, un groupe passe par le même répartiteur ; coupé par une bascule, il garde son compte. Dans la maison ×1 du banc, {nb(r('catalogue', 1, 'appareils_virtuels'))} appareils virtuels et {nb(r('catalogue', 1, 'membres'))} entrées.",
             462, "Aujourd'hui, un luminaire répond « fait » dès qu'une ampoule a obéi. La v2 corrige quatre constats de la revue sur les groupes : un seul chemin d'envoi, n/m par niveau de preuve, la mère sans échéance, les cibles rattachées (I7 à I10).")

# ================================================================ Les workflows interrogent les ordres
if voulu("workflows"):
    d = Dessin("workflows")
    d.titre(16, 24, "Ce qu'un workflow peut demander aux ordres, et ce que cela coûte")
    q = [("La dernière commande", "la dernière demande admise : valeur, issue, origine, instant", f"{fr(r('catalogue', 1, 'lire_derniere_commande_ns'), 2)} ns", "bleu", "document"),
         ("Un ordre est-il en cours ?", "ses demandes vivantes, à part de l'issue", "même fiche", "violet", "horloge"),
         ("Quand un ordre part ou tombe", "un index par ordre, comme l'index par point", "à la trace", "ambre", "eclair"),
         ("La valeur suit-elle la commande ?", "oui, non, ou on ne sait pas", f"{fr(r('catalogue', 1, 'vient_de_ns'), 1)} ns", "vert", "oeil")]
    for k, (t, s, c, ton, ic) in enumerate(q):
        x, y = 16 + (k % 2) * 492, 44 + (k // 2) * 84
        d.carte(x, y, 476, 72, t, [s], ton, ic, taille=12, sous=10.2)
        d.pilule(x + 462, y + 12, c, ton, taille=9.5, ancre="end")
    d.zone(16, 222, 968, 160, "Une soirée : « la valeur suit-elle la dernière commande ? »", "vert", "maison")
    pas = [(140, "20:12", "éteinte, rallumée", "à la main", "non", "rouge"), (380, "20:16", "la maison rallume", "la lampe éteinte", "oui", "vert"),
           (620, "20:41", "une teinte", "déjà là", "on ne sait pas", "ambre"), (860, "20:43", "un geste pendant", "la latence", "on ne sait pas", "ambre")]
    for x, h_, a, b, rep, ton in pas:
        d.bouton(x, 300, 190, 54, h_, a + " " + b, ton)
        pastille(d, x, 340, rep, ton)
    A.ecrire(d, "Les workflows interrogent les ordres",
             f"Quatre questions et leur coût. La dernière commande, la dernière demande admise : {fr(r('catalogue', 1, 'lire_derniere_commande_ns'), 2)} ns. Un ordre en cours : ses demandes vivantes, lues dans la même fiche. Quand un ordre part ou tombe : un index par ordre. La valeur suit-elle la dernière commande : oui, non, ou on ne sait pas, {fr(r('catalogue', 1, 'vient_de_ns'), 1)} ns. En bas, la soirée du banc : à 20:12, éteinte et rallumée à la main, non ; à 20:16, la maison rallume la lampe éteinte, oui ; à 20:41, une teinte déjà là, on ne sait pas ; à 20:43, un geste pendant la latence de l'appareil, on ne sait pas.",
             394, "Une compatibilité, pas une causalité (revue I5) : un geste humain qui fait exactement ce que l'ordre demandait, pendant la latence de l'appareil, reste indécidable sans cause donnée par l'appareil. Un workflow prudent n'agit que sur « oui ».")

# ================================================================ Les clients chargent par MQTT
if voulu("clients"):
    d = Dessin("clients")
    d.titre(16, 24, "Les clients se chargent par MQTT, à contenu égal, et voient leurs trous")
    opts = [(18, "Aujourd'hui", "SSE + REST", "gris", ["GET /api/snapshot : toute la maison en JSON", "puis un flux SSE ; l'ordre par REST", "un trou : tout recharger"]),
            (344, "La V7", "un module WebSocket", "ardoise", ["l'état filtré, par blocs de 64 Kio", "un module traduit MQTT en trames", "ordres et issues sur la connexion"]),
            (670, "La v2, ton choix", "MQTT, directement", "vert", ["l'état du client, filtré par ses droits", "le rang de chacune de ses pièces", "req/<compte>, res/<compte>"])]
    for x, t, s, ton, lignes in opts:
        d.bloc(x, 54, 312, 50, ton, t, s, icone="ecran")
        d.carte(x, 118, 312, 98, "Comment", lignes, ton, taille=11.5, sous=10.2)
    W = 560
    mx = r("clients", 1, "aujourdhui_api_snapshot_json_o")
    barres = [("Aujourd'hui : /api/snapshot, JSON", r("clients", 1, "aujourdhui_api_snapshot_json_o"), "gris"),
              ("Le même contenu, proposé, en JSON : le partage", r("clients", 1, "propose_meme_contenu_json_o"), "ambre"),
              ("Le même contenu, en binaire : l'encodage", r("clients", 1, "mqtt_etat_client_complet_o"), "vert"),
              ("Un écran de deux pièces : le filtrage", r("clients", 1, "mqtt_etat_client_deux_pieces_o"), "bleu")]
    d.t(18, 246, "Ce qu'un écran charge pour démarrer, à contenu égal, maison ×1", 11, True, ENCRE)
    for k, (lab, v, ton) in enumerate(barres):
        y = 260 + k * 30
        d.t(18, y + 14, lab, 10.5, coul=ENCRE)
        d.barre(360, y + 2, W * v / mx, 16, ton)
        d.t(360 + W * v / mx + 6, y + 15, octets(v), 10.5, True, TONS[ton][0])
    bandeau(d, 388, "Chaque sortie porte le rang de sa pièce : un écran filtré voit ce qu'il a perdu ; une époque nouvelle le fait se recharger.", "vert")
    A.ecrire(d, "Les clients se chargent par MQTT",
             f"Trois façons de servir un écran. Aujourd'hui : toute la maison en JSON par /api/snapshot, puis un flux SSE. La V7 : un module WebSocket. La v2, ton choix : l'écran parle MQTT directement ; il demande son état, filtré par ses droits, avec le rang de chacune de ses pièces, puis suit le flux de ses pièces ; il publie ses demandes sur req/ de son compte. Ce qu'un écran charge, à contenu égal, maison ×1 : {octets(r('clients', 1, 'aujourdhui_api_snapshot_json_o'))} de JSON aujourd'hui ; {octets(r('clients', 1, 'propose_meme_contenu_json_o'))} pour le même contenu proposé en JSON, le gain du partage ; {octets(r('clients', 1, 'mqtt_etat_client_complet_o'))} en binaire, celui de l'encodage ; {octets(r('clients', 1, 'mqtt_etat_client_deux_pieces_o'))} pour un écran de deux pièces, celui du filtrage.",
             436, "La v1 comparait des contenus différents (revue I17) ; la v2 décompose le gain. Le rang de chaque pièce répond au constat I13 : avec le seul rang s, un écran filtré ne distinguait pas une perte d'un message qui ne le concernait pas.")

# ================================================================ La bascule
if voulu("bascule"):
    d = Dessin("bascule")
    d.titre(16, 24, "La bascule : ce qui était en cours, et ce qu'un appui attend")
    d.bloc(30, 60, 270, 120, "vert", "L'actif", "jusqu'à 21:00", icone="puce")
    for k, l in enumerate(["chaque pas d'une demande sur ord/", "un appui : l'accusé du secours d'abord", "sinon le témoin : « décroché »"]):
        d.t(165, 118 + k * 16, l, 10.2, coul=GRIS, ancre="middle")
    d.bloc(700, 60, 270, 120, "bleu", "Le secours", "puis l'actif, époque 2", icone="fleches")
    for k, l in enumerate(["les mêmes cases et fiches ; il accuse", "lit le témoin : reprise sûre ou non", "relit ses appareils : lectures fraîches"]):
        d.t(835, 118 + k * 16, l, 10.2, coul=GRIS, ancre="middle")
    d.fleche("M310 104 L690 104", "ambre", 2.4, anime=True)
    d.etiquette(500, 94, "ord/ · prive/ · wf/ · val/ · meta/ — au rang s", "ambre")
    d.fleche("M690 128 L310 128", "violet", 1.8, anime=True, lent=True)
    d.etiquette(500, 142, "l'accusé de chaque rang", "violet")
    cas = [("Remise, partie ou non", "incertaine, jamais renvoyée ; confirmée par une lecture faite après la reprise", "rouge"),
           ("Un appui sans accusé", "jamais parti ; retenté sur une reprise sûre : il part, une fois", "ambre"),
           ("Une reprise non sûre", "une nouvelle tentative d'un appui d'avant : refusée, « ambiguë »", "rouge"),
           ("Un groupe à demi conclu", "la copie a rejoué son compte : 3 sur 4", "bleu"),
           ("Retenue, bail, reçu", "passent ; la retenue revérifiée ; le bail rétabli à son terme", "vert"),
           ("Copies et écrans", "une époque nouvelle : ils se rechargent par l'état complet", "violet")]
    for k, (t, s, ton) in enumerate(cas):
        d.carte(30 + (k % 2) * 480, 202 + (k // 2) * 62, 460, 52, t, [s], ton, taille=11.2, sous=9.8)
    A.ecrire(d, "La bascule",
             "À gauche, l'actif publie chaque pas d'une demande sur ord/, au rang s ; un appui attend l'accusé du secours avant de partir ; sans accusé au bout de 200 ms, il inscrit chez le témoin que le secours a décroché, puis envoie. À droite, le secours tient les mêmes cases et fiches et accuse chaque rang ; promu, il prend une époque nouvelle, lit le témoin pour savoir si la reprise est sûre, et son module relit ses appareils. Six cas : une demande remise devient incertaine, jamais renvoyée, et n'est confirmée que par une lecture faite après la reprise ; un appui sans accusé n'est jamais parti, et part une fois s'il est retenté sur une reprise sûre ; sur une reprise non sûre, une nouvelle tentative d'un appui antérieur est refusée ; un groupe garde son compte ; retenues, baux et reçus passent ; copies et écrans se rechargent.",
             400, "La HA v2 admet la perte des dernières sorties en vol : une trace copiée après ne protège pas un appui fait avant (revue B2). L'accusé avant l'appui, le témoin et la reprise sûre ou non rendent « au plus une fois » vrai.")

# ================================================================ La mémoire, avant et après
if voulu("memoire"):
    d = Dessin("memoire")
    d.titre(16, 24, "Ce que la mémoire des ordres coûte, et ce qu'elle remplace (maison ×1, à information égale)")
    lignes = [("Décrire les appareils, valeurs et ordres", r("catalogue", 1, "aujourdhui_description_o"), r("catalogue", 1, "propose_description_totale_o"), kio),
              ("Le journal en mémoire (500 lignes)", r("demandes", 1, "aujourdhui_journal_500_o"), r("demandes", 1, "memoire_journal_o"), kio),
              ("50 retenues (mesurées)", r("demandes", 1, "aujourdhui_retenues_50_o"), r("demandes", 1, "propose_retenues_50_o"), kio),
              ("Une demande en vol (au mieux)", r("demandes", 1, "aujourdhui_une_demande_en_vol_o"), r("catalogue", 1, "taille_demande_o"), kio),
              ("Copier la maison pour l'assistant, à chaque requête", r("catalogue", 1, "assistant_copie_de_la_maison_o"), 1, kio),
              ("Ce qu'un écran charge, à contenu égal (transfert)", r("clients", 1, "aujourdhui_api_snapshot_json_o"), r("clients", 1, "mqtt_etat_client_complet_o"), octets)]
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
    x = 16
    for lab, ton in (("aujourd'hui, à la convention des pilotes", "gris"), ("proposé", "vert")):
        w = largeur(lab, 10.5) * 1.13 + 26
        pastille(d, x + w / 2, 390, lab, ton)
        x += w + 12
    A.ecrire(d, "La mémoire, avant et après",
             "Six postes, aujourd'hui en gris, proposé en vert, en échelle logarithmique. Décrire les appareils, leurs valeurs et leurs ordres : " + kio(r("catalogue", 1, "aujourdhui_description_o")) + " contre " + kio(r("catalogue", 1, "propose_description_totale_o")) + ". Le journal en mémoire : " + kio(r("demandes", 1, "aujourdhui_journal_500_o")) + " contre " + kio(r("demandes", 1, "memoire_journal_o")) + ". Cinquante retenues, mesurées : " + kio(r("demandes", 1, "aujourdhui_retenues_50_o")) + " contre " + kio(r("demandes", 1, "propose_retenues_50_o")) + ". Une demande en vol : " + kio(r("demandes", 1, "aujourdhui_une_demande_en_vol_o")) + " au mieux contre " + kio(r("catalogue", 1, "taille_demande_o")) + ". La copie de la maison pour l'assistant : " + kio(r("catalogue", 1, "assistant_copie_de_la_maison_o")) + " à chaque requête, contre rien. Ce qu'un écran charge, à contenu égal : " + octets(r("clients", 1, "aujourdhui_api_snapshot_json_o")) + " contre " + octets(r("clients", 1, "mqtt_etat_client_complet_o")) + ".",
             420, "Le témoin d'aujourd'hui suit la convention des pilotes (revue I15) ; chaque poste proposé est mesuré par le compteur d'allocations ou par size_of (I18). Les répliques d'aujourd'hui restent au mieux : ni les canaux de Tokio, ni les tâches, ni la sérialisation ne sont comptés.")

# ======================================================================================== v3 : les droits

MUT = (ICI.parent / "sorties/mutations.txt").read_text(encoding="utf-8") if (ICI.parent / "sorties/mutations.txt").exists() else ""
FAUTES_V3 = len(re.findall(r"^détectée\s+[DMW]\d+ ·", MUT, re.M))
CLASSES = ["confort", "média", "climat", "accès", "alarme", "machine", "vie privée", "système", "message", "désarmer"]
ROLES = ["inconnu", "propriétaire", "adulte", "enfant", "invité", "service"]


def croix_ou_coche(d, x, y, ok):
    ton = "vert" if ok else "rouge"
    d._t(ton)
    d.brut(f'<circle cx="{x + 9}" cy="{y + 9}" r="10" fill="url(#{d.id}-h-{ton})"/>')
    d.icone("coche" if ok else "croix", x + 2, y + 2, 14, "#ffffff", 2.4)


# ================================================================ La seconde revue : 23 constats sur les droits
if voulu("revue2"):
    d = Dessin("revue2")
    d.titre(16, 24, "La seconde revue Codex : 23 constats sur les droits, chacun corrigé, chacun prouvé")
    groupes = [
        ("Les droits", [("D1", "Connexion ≠ personne", 1), ("D2", "Des droits par classe", 1), ("D3", "L'approbateur habilité", 1), ("D4", "La révision de la carte", 1),
                        ("D5", "L'autorité suit l'agent", 1), ("D6", "Revérifiés à la remise", 1), ("D7", "La fin, une échéance", 0), ("D8", "ord/ ne dit pas qui", 0),
                        ("D9", "Désarmer : chemin dédié", 1)]),
        ("La mémoire", [("M1", "Les comptes par leur nom", 1), ("M2", "L'acteur d'une décision", 0), ("M3", "Des droits frais au secours", 0), ("M4", "Le budget refait", 0),
                        ("M5", "« En cours » sur 16 bits", 0), ("M6", "L'empreinte canonique", 0), ("M7", "L'époque d'émission", 1), ("M8", "Un texte privé par forme", 0)]),
        ("Les automatismes", [("W1", "Appelée ou autonome", 1), ("W2", "Le responsable borne", 0), ("W3", "Un bail qui possède", 0), ("W4", "L'empreinte du programme", 0),
                              ("W5", "Les exécutions au secours", 0), ("W6", "20 départs par minute", 0)]),
    ]
    y = 44
    for titre, cartes in groupes:
        d.t(16, y + 8, titre, 11, True, ENCRE)
        y += 16
        for k, (c, t, bloquant) in enumerate(cartes):
            x = 16 + (k % 5) * 194
            yy = y + (k // 5) * 66
            d.carte(x, yy, 184, 58, c, [t], "rouge" if bloquant else "ambre", "coche", taille=11.5, sous=9.8)
        y += ((len(cartes) + 4) // 5) * 66 + 6
    x = 16
    for lab, ton in (("bloquant", "rouge"), ("important", "ambre")):
        w = largeur(lab, 10.5) * 1.13 + 26
        pastille(d, x + w / 2, y + 2, lab, ton)
        x += w + 12
    bandeau(d, y + 34, f"Chacun vérifié dans le code, corrigé, puis gardé par des fautes injectées : {FAUTES_V3} pour cette revue, toutes détectées. M4 est un budget : il est refait (§ 19).", "vert")
    h = y + 82
    A.ecrire(d, "La seconde revue : les droits",
             "Vingt-trois cartes en trois groupes. Les droits : une connexion n'est pas une personne ; des droits par classe d'ordre ; l'approbateur habilité pour l'opération ; la révision de la carte ; l'autorité qui suit l'agent ; les droits revérifiés à la remise au module ; la fin d'un droit comme échéance ; ord/ qui ne dit pas qui ; désarmer par son chemin dédié. La mémoire : les comptes relus par leur nom ; l'acteur d'une décision ; des droits frais au secours ; le budget refait ; « en cours » sur 16 bits ; l'empreinte canonique des arguments ; l'époque d'émission ; un texte privé par forme. Les automatismes : exécution appelée ou autonome ; le responsable qui borne le mandat ; un bail qui possède l'état ; l'empreinte du programme ; les exécutions au secours ; vingt départs par minute. Dix sont bloquants, treize importants.",
             h, "Codex a relu la v2 et le code d'aujourd'hui sous l'angle des droits ; un agent Claude a inventorié ce que le code fait aujourd'hui (§ 5.4). Les vingt-trois constats étaient fondés ; le § 3.5 dit, pour chacun, la correction et sa preuve.")

# ================================================================ Trois identités, une décision
if voulu("identites"):
    d = Dessin("identites")
    d.titre(16, 24, "Trois questions pour chaque ordre : qui le demande, pour qui, sous quel mandat")
    d.carte(16, 46, 430, 108, "Le principal : la connexion", ["le compte de l'écran, du satellite, de l'agent ;", "ses pièces, sa fin, les classes qu'il commande", "ou propose, la meilleure preuve qu'il sait donner"], "bleu", "ecran", mot=f"profil · {nb(r('demandes', 1, 'taille_profil_o'))} o")
    d.carte(16, 168, 430, 108, "La personne, et ce qu'on en sait", ["une session personnelle, son appli, le mandat", "d'un agent, ou un indice (une voix) : le confort", "de sa personne, le sensible comme un inconnu"], "ambre", "personne", mot=f"personne · {nb(r('demandes', 1, 'taille_personne_o'))} o")
    d.carte(16, 290, 430, 108, "Le mandat d'un automatisme", ["ce que son graphe commande : classes et pièces,", "calculé à la validation ; son responsable, ou le", "foyer s'il l'a adopté (par un propriétaire)"], "violet", "rouage", mot=f"mandat · {nb(r('demandes', 1, 'taille_mandat_o'))} o")
    for path in ("M448 100 C478 100 480 186 500 192", "M448 222 L500 222", "M448 344 C478 344 480 256 500 252"):
        d.fleche(path, "vert", 1.8, anime=True, lent=True)
    d.bloc(504, 168, 196, 104, "vert", "La décision", None, icone="bouclier")
    for k, l in enumerate(["le plus petit des trois,", "et du contexte : une chambre,", "un secours incertain"]):
        d.t(602, 222 + k * 15, l, 10.2, coul=GRIS, ancre="middle")
    for k, (t, s_, ton) in enumerate([("Oui", "il part", "vert"), ("Proposer", "retenu : un humain habilité", "ambre"), ("Non", "refusé : « droits »", "rouge")]):
        d.bouton(862, 98 + k * 124, 228, 58, t, s_, ton)
    for k, ton in enumerate(("vert", "ambre", "rouge")):
        d.fleche(f"M718 {214 + k * 12} C732 {214 + k * 12} 730 {98 + k * 124} 746 {98 + k * 124}", ton, 1.8)
    d.pilule(504, 300, "désarmer : jamais par ce chemin", "rouge", taille=9.5, icone="cadenas")
    d.pilule(504, 326, "une voix : le confort, jamais le sensible", "ambre", taille=9.5)
    d.pilule(504, 352, "le client ne déclare rien de tout cela", "bleu", taille=9.5)
    bandeau(d, 414, f"Une décision coûte {fr(r('demandes', 1, 'droits_decider_ns'), 1)} ns ; celle du hub pour un ordre de la maison, ses pièces comprises, {fr(r('demandes', 1, 'droits_niveau_hub_ns'), 0)} ns. Les mêmes tables à l'arrivée, à l'approbation, à la remise.", "vert")
    A.ecrire(d, "Trois identités, une décision",
             f"À gauche, trois cartes. Le principal, c'est-à-dire la connexion : un profil de {nb(r('demandes', 1, 'taille_profil_o'))} octets, ses pièces, sa fin, les classes qu'il commande ou propose, la meilleure preuve qu'il sait donner. La personne, et ce qu'on en sait : une session personnelle, son appli, le mandat d'un agent, ou un simple indice, une voix reconnue, qui vaut sa personne pour le confort et jamais plus qu'un inconnu pour le sensible ; {nb(r('demandes', 1, 'taille_personne_o'))} octets. Le mandat d'un automatisme : ce que son graphe commande, son responsable ; {nb(r('demandes', 1, 'taille_mandat_o'))} octets. Au centre, la décision : le plus petit des trois, et du contexte. À droite, trois issues : oui, il part ; proposer, il est retenu pour un humain habilité ; non, refusé. Désarmer ne passe jamais par ce chemin. Une décision coûte {fr(r('demandes', 1, 'droits_decider_ns'), 1)} nanosecondes.",
             462, "Codex résume : droits effectifs = principal ∩ personne (ou « inconnu ») ∩ mandat ∩ contexte ∩ exigences de l'ordre. La v3 le range en trois fiches de taille fixe et une table ; le serveur établit chacune, jamais le client (§ 11).")

# ================================================================ Chaque canal, et ce qu'il prouve
if voulu("canaux"):
    d = Dessin("canaux")
    d.titre(16, 24, "Chaque canal, et la meilleure preuve qu'il sait donner de la personne")
    lignes = [("ecran", "Tablette partagée", "aucune, sauf session", "gris", "un inconnu ; un PIN personnel ouvre une session de quelques minutes"),
              ("telephone", "Appli, navigateur personnels", "personnelle", "vert", "sa personne : son rôle, ses pièces, sa fenêtre, ses exceptions"),
              ("radio", "Satellite vocal", "un indice, au mieux", "ambre", "une voix reconnue : son confort ; le sensible se confirme sur un écran"),
              ("personne", "L'assistant, depuis un écran", "celle de l'écran", "vert", "l'autorité de l'écran qui lui parle, jamais celle de « Moli »"),
              ("puce", "Agent MCP, script", "un mandat, ou aucune", "ambre", "« pour Alice » : un mandat du serveur ; le texte n'accorde rien"),
              ("eclair", "Bouton, télécommande", "aucune", "gris", "une source connue ; ouvrir par un bouton : une décision explicite"),
              ("cadenas", "Clavier d'alarme", "le code", "violet", "le chemin privé de l'alarme, jamais une demande générique"),
              ("rouage", "Automatisme", "un mandat", "violet", "lancé : les droits de qui le lance ; autonome : ceux du responsable"),
              ("cle", "Invité", "personnelle, datée", "ambre", "ses pièces, sa fenêtre ; il propose ce qu'un adulte commande"),
              ("maison", "Le hub", "interne", "vert", "ses propres gestes (une minuterie) ; aucune personne implicite")]
    d.t(60, 52, "Le canal", 10, True, GRIS)
    d.t(330, 52, "Ce qu'il prouve", 10, True, GRIS)
    d.t(520, 52, "Ce qui en découle", 10, True, GRIS)
    for k, (ic, nom, preuve_, ton, suite) in enumerate(lignes):
        y = 62 + k * 38
        d.brut(f'<rect x="16" y="{y}" width="968" height="32" rx="9" fill="{TONS["gris"][2] if k % 2 else "#ffffff"}" stroke="{LIGNE}"/>')
        d._t(ton)
        d.brut(f'<rect x="24" y="{y + 4}" width="24" height="24" rx="7" fill="url(#{d.id}-h-{ton})"/>')
        d.icone(ic, 28, y + 8, 16)
        d.t(60, y + 21, nom, 11, True, ENCRE)
        pastille(d, 330 + (largeur(preuve_, 9.5) * 1.13 + 26) / 2, y + 6, preuve_, ton, taille=9.5)
        d.t(520, y + 21, suite, 10.2, coul=GRIS)
    bandeau(d, 448, "Une connexion n'est pas une personne : la tablette garde son compte quand Charlie tape son PIN, et redevient anonyme à la fin de la session.", "vert")
    A.ecrire(d, "Chaque canal, et ce qu'il prouve",
             "Dix lignes. La tablette partagée ne prouve personne, sauf une session personnelle de quelques minutes. L'appli ou le navigateur personnels prouvent leur personne. Le satellite vocal donne au mieux un indice : une voix reconnue vaut sa personne pour le confort, et le sensible se confirme sur un écran. L'assistant, depuis un écran, porte l'autorité de cet écran. Un agent MCP ou un script : un mandat du serveur, ou aucune personne ; le texte « pour Alice » n'accorde rien. Un bouton : aucune personne ; ouvrir par un bouton est une décision explicite. Le clavier d'alarme : le code, par le chemin privé de l'alarme. Un automatisme : un mandat. Un invité : sa personne, datée. Le hub : ses propres gestes, sans personne implicite.",
             496, "Ce que le code d'aujourd'hui prouve, canal par canal, est au § 5.4 : la session PIN est commune et liée à l'appelant, la voix n'a pas de locuteur, MCP n'a pas de personne. La v3 range la preuve à côté de la personne, et décide avec.")

# ================================================================ La table rôles × classes
if voulu("decision"):
    d = Dessin("decision")
    d.titre(16, 24, "La table : ce que chaque rôle commande, propose ou approuve, par classe d'ordre")
    x0, y0, cw, rh = 132, 70, 85, 44
    for c, nom in enumerate(CLASSES):
        d.t(x0 + c * cw + cw / 2, y0 - 10, nom, 10, True, GRIS, "middle")
    for ro, nom in enumerate(ROLES):
        y = y0 + ro * rh
        d.t(x0 - 12, y + rh / 2 + 3, nom, 11, True, ENCRE, "end")
        for c in range(len(CLASSES)):
            v = int(r("demandes", 1, f"droits_table_r{ro}_c{c}"))
            niv, approuve = v & 3, v & 4
            ton, lab, plein = {2: ("vert", "oui", True), 1: ("ambre", "propose", True), 0: ("gris", "non", False)}[niv]
            x = x0 + c * cw
            case(d, x + 3, y + 4, cw - 6, rh - 8, ton, lab, plein=plein, taille=10)
            if approuve:
                d.icone("coche", x + cw - 21, y + 7, 12, "#ffffff", 2.4)
    yl = y0 + len(ROLES) * rh + 14
    x = 16
    for lab, ton in (("oui : il part", "vert"), ("propose : un humain décide", "ambre"), ("non", "gris")):
        w = largeur(lab, 10.5) * 1.13 + 26
        pastille(d, x + w / 2, yl, lab, ton)
        x += w + 12
    d.icone("coche", x + 4, yl + 4, 14, TONS["vert"][0], 2.4)
    d.t(x + 24, yl + 15, "approuve aussi ce que d'autres proposent", 10.5, True, TONS["vert"][0])
    bandeau(d, yl + 40, f"Une proposition, que le propriétaire règle (§ 22). Une voix reconnue : sa ligne pour le confort ; le sensible sans preuve : la ligne « inconnu ».", "ambre", "eclair")
    h = yl + 88
    A.ecrire(d, "La table des droits",
             "Une grille de six rôles sur dix classes d'ordres. Le propriétaire commande et approuve tout, sauf désarmer. L'adulte commande et approuve le confort, le média, le climat, l'accès, l'alarme et les machines ; il commande la vie privée et les messages, et propose le système. L'enfant commande le confort, le média et les messages, propose le climat, l'accès et les machines, et n'a ni l'alarme, ni la vie privée, ni le système. L'invité commande le confort, le média, le climat et les messages, et propose l'accès et les machines. Le service commande le confort et le climat et propose l'accès. L'inconnu, celui d'une tablette sans session ou d'une voix, commande le confort, le média, le climat et les messages, et propose l'accès, l'alarme et les machines. Désarmer est fermé à tous.",
             h, "Lue dans le banc (droits::table_par_defaut), comme les mesures. Chaque personne a en plus ses pièces, sa fenêtre et ses exceptions ; chaque canal, ses classes ; une chambre et un secours incertain font proposer.")

# ================================================================ L'approbation habilitée
if voulu("approbation"):
    d = Dessin("approbation")
    d.titre(16, 24, "Une retenue : qui peut la décider, sur quelle carte, et en quel nom")
    d.carte(16, 50, 270, 150, "La retenue", ["un agent veut tirer le pêne", "nuki:porte/tirer · pièce 5", "classe : accès", "sa carte : la révision 7"], "ambre", "cadenas", mot="10 min")
    qui = [("Un agent, un satellite", "ce n'est pas un écran", False), ("La tablette, sans session", "aucune preuve personnelle", False),
           ("Charlie, son PIN", "un enfant n'approuve pas un accès", False), ("Alice, son téléphone", "adulte : approuve l'accès", True),
           ("Bob, son écran", "propriétaire", True)]
    for k, (t, s_, ok) in enumerate(qui):
        y = 50 + k * 62
        d.carte(320, y, 330, 54, t, [s_], "vert" if ok else "gris", attenue=not ok, taille=11.5, sous=10)
        croix_ou_coche(d, 620, y + 17, ok)
        d.fleche(f"M288 {125 if k == 2 else (100 if k < 2 else 150)} L318 {y + 27}", "gris", 1.2, op=".6")
    d.carte(684, 50, 300, 306, "Décidée, une fois", ["la révision vue, puis consommée :", "le même clic rejoué ne vaut rien ;", "une carte renouvelée (ses cibles", "ont changé) a sa révision neuve", "", "tout revérifié : l'échéance, les", "droits du demandeur, la forme,", "les cibles", "", "au journal : la décision au nom", "d'Alice, pas de l'agent"], "vert", "coche", sous=10.2)
    d.fleche("M652 263 L682 230", "vert", 2, anime=True)
    d.fleche("M652 325 L682 260", "vert", 2, anime=True)
    bandeau(d, 372, "Sur un secours aux droits incertains, seul un propriétaire approuve le sensible ; il confirme ensuite les droits, d'un geste.", "ambre", "bouclier")
    A.ecrire(d, "L'approbation habilitée",
             "À gauche, une retenue : un agent veut tirer le pêne de la porte d'entrée, classe accès, pièce 5 ; sa carte porte la révision 7 ; elle vit dix minutes. Au milieu, cinq candidats. Un agent ou un satellite : non, ce n'est pas un écran. La tablette sans session : non, aucune preuve personnelle. Charlie avec son PIN : non, un enfant n'approuve pas un accès. Alice sur son téléphone : oui, une adulte approuve l'accès. Bob sur son écran : oui, le propriétaire. À droite, la décision : la révision vue est consommée, le même clic rejoué ne vaut rien, une carte renouvelée a sa révision neuve ; tout est revérifié ; au journal, la décision est au nom d'Alice. En bas : sur un secours aux droits incertains, seul un propriétaire approuve le sensible.",
             420, "Aujourd'hui, toute session PIN décide de toute retenue (resolve_approval ne vérifie que l'origine ui) et le journal ne nomme pas l'approbateur (§ 5.4). La v3 sépare « approuver » de « commander » (revue D3, D4, M2).")

# ================================================================ Un groupe, une cible interdite (ton choix)
if voulu("sousensemble"):
    d = Dessin("sousensemble")
    d.titre(16, 24, "Un groupe dont une cible est interdite : le sous-ensemble permis, annoncé avant")
    d.carte(16, 50, 300, 178, "La demande", ["Charlie, son PIN, sur la tablette :", "« allume la maison », un groupe", "de trois cibles", "", "ses pièces : le salon et sa", "chambre"], "bleu", "personne", mot="groupe")
    cibles = [("Le salon", "sa pièce : permise", True), ("Le bureau", "pas sa pièce : interdite", False), ("Sa chambre", "sa pièce : permise", True)]
    for k, (t, s_, ok) in enumerate(cibles):
        y = 50 + k * 62
        d.carte(340, y, 290, 54, t, [s_], "vert" if ok else "gris", attenue=not ok, taille=11.5, sous=10)
        croix_ou_coche(d, 600, y + 17, ok)
        d.fleche(f"M318 139 L338 {y + 27}", "gris", 1.2, op=".6")
    d.carte(664, 50, 320, 178, "La carte annoncée", ["« 2 cibles permises sur 3 :", "confirmer le sous-ensemble »", "", "seule Charlie choisit : sa connexion,", "ou un écran avec sa preuve ; un", "adulte ne choisit pas pour elle"], "ambre", "ecran", mot="10 min")
    d.fleche("M632 77 L662 118", "vert", 2, anime=True)
    d.fleche("M632 201 L662 160", "vert", 2, anime=True)
    for k, (t, lignes_, ton, ic) in enumerate([("Elle confirme, ou renonce", ["le salon et sa chambre s'allument ;", "le bureau, jamais ; si elle renonce,", "rien ne part"], "vert", "coche"),
                                               ("Son agent confirme", ["sa chambre se propose (un canal", "gardé) : Bob l'approuve, sur une", "carte neuve ; confirmer n'est pas", "approuver"], "ambre", "cadenas"),
                                               ("Le groupe change pendant le choix", ["une cible ajoutée : ce qui a été", "montré part, et seulement cela ;", "une cible montrée retirée : le", "sous-ensemble du moment, à choisir"], "bleu", "fleches")]):
        d.carte(16 + k * 329, 246, 310, 112, t, lignes_, ton, ic, sous=10.2)
    bandeau(d, 374, "Aucune cible permise : refusé, sans rien annoncer. Un automatisme qui a perdu une cible : refusé, et mis à revoir.", "vert", "bouclier")
    A.ecrire(d, "Un groupe, une cible interdite",
             "En haut à gauche, la demande : Charlie, son PIN tapé sur la tablette, demande « allume la maison », un groupe de trois cibles ; ses pièces sont le salon et sa chambre. Au milieu, les trois cibles : le salon, permis ; le bureau, interdit, ce n'est pas sa pièce ; sa chambre, permise. À droite, la carte annoncée : « 2 cibles permises sur 3 : confirmer le sous-ensemble » ; seule Charlie choisit, depuis sa connexion ou un écran avec sa preuve ; un adulte ne choisit pas pour elle ; la carte vit dix minutes. En bas, trois suites : elle confirme, le salon et sa chambre s'allument, le bureau jamais, ou elle renonce, et rien ne part ; son agent confirme, sa chambre se propose parce que le canal est gardé, et Bob l'approuve sur une carte neuve, car confirmer n'est pas approuver ; le groupe change pendant le choix : une cible ajoutée, ce qui a été montré part, et seulement cela ; une cible montrée retirée, le sous-ensemble du moment est à choisir de nouveau. Un bandeau : aucune cible permise, refusé sans rien annoncer ; un automatisme qui a perdu une cible est refusé et mis à revoir.",
             422, "Ton choix (§ 22) : ni refusé en bloc, ni exécuté en partie d'office. Le banc le prouve (un_groupe_avec_une_cible_interdite_annonce_le_sous_ensemble, un_automatisme_ne_choisit_pas_de_sous_ensemble).")

# ================================================================ La durée des droits
if voulu("duree"):
    d = Dessin("duree")
    d.titre(16, 24, "Les droits se vérifient à chaque frontière, et leur fin réveille le hub")
    lanes = [("Une demande, puis une révocation", "bleu", [(120, "Alice allume", "admise"), (360, "Alice révoquée", "rien n'est parti"), (620, "la remise au module", "droits revérifiés"), (860, "non envoyée", "cause « droits »")]),
             ("Un appui, et l'accusé du secours", "violet", [(120, "« volume + »", "attend l'accusé"), (360, "Alice révoquée", "avant l'accusé"), (620, "retiré de l'attente", "tout de suite"), (860, "ne part jamais", "même accusé")]),
             ("Une invitation qui finit", "ambre", [(120, "18:39, l'invité", "propose d'arrêter"), (360, "la retenue", "dix minutes"), (620, "18:40, sa fin", "une échéance du hub"), (860, "refusée à la seconde", "pas à 18:49")])]
    for k, (titre, ton, pas) in enumerate(lanes):
        y = 52 + k * 118
        d.zone(16, y, 968, 104, titre, ton)
        d.ligne(60, y + 64, 940, y + 64, ton, 2, tete=True)
        for j, (x, t, s_) in enumerate(pas):
            fin = j == len(pas) - 1
            c_ton = "rouge" if fin else ton
            d._t(c_ton)
            d.brut(f'<circle cx="{x}" cy="{y + 64}" r="7" fill="url(#{d.id}-h-{c_ton})"/>')
            d.t(x, y + 46, t, 10.8, True, ENCRE, "middle", halo=True)
            d.t(x, y + 88, s_, 10, coul=GRIS, ancre="middle")
    bandeau(d, 410, "À l'arrivée, à l'approbation, et au dernier point avant l'envoi ; la fin d'une personne ou d'un compte est une échéance : le hub s'y réveille sans battement.", "vert")
    A.ecrire(d, "La durée des droits",
             "Trois frises. Une demande, puis une révocation : Alice allume, la demande est admise ; Alice est révoquée avant que rien ne parte ; à la remise au module, les droits sont revérifiés ; la demande n'est pas envoyée, cause droits. Un appui et l'accusé du secours : le « volume + » attend l'accusé ; Alice est révoquée avant ; l'appui est retiré de l'attente tout de suite, et ne part jamais, même si l'accusé arrive. Une invitation qui finit : à 18 h 39, l'invité propose d'arrêter l'imprimante ; la retenue a dix minutes ; à 18 h 40, sa fin est une échéance du hub ; la retenue est refusée à la seconde, pas à 18 h 49.",
             458, "La v2 vérifiait le permis à l'arrivée et à l'approbation, et retirait des voies ; une commande encore dans le hub pouvait partir après une révocation (revue D6), et la fin d'un permis attendait un battement (D7).")

# ================================================================ Qui répond d'un automatisme
if voulu("mandat"):
    d = Dessin("mandat")
    d.titre(16, 24, "Un automatisme ne fait jamais plus que celui qui en répond")
    d.zone(16, 40, 476, 210, "Lancé par une personne : une routine, une scène", "bleu", "personne")
    d.carte(32, 76, 444, 52, "Charlie lance « Départ » sur la tablette, son PIN", ["les droits de Charlie ∩ ceux de la tablette ∩ le mandat de « Départ »"], "bleu", taille=11.5, sous=10)
    for k, (t, ok) in enumerate([("sa lampe : oui", True), ("la porte : non, ce n'est pas sa pièce", False), ("depuis la tablette anonyme : la chambre se propose", None)]):
        y = 140 + k * 34
        if ok is None:
            d._t("ambre")
            d.brut(f'<circle cx="{41}" cy="{y + 9}" r="10" fill="url(#{d.id}-h-ambre)"/>')
            d.icone("horloge", 34, y + 2, 14, "#ffffff", 2.2)
        else:
            croix_ou_coche(d, 32, y, ok)
        d.t(60, y + 13, t, 10.8, True, ENCRE)
    d.zone(508, 40, 476, 210, "Autonome : un déclencheur", "violet", "rouage")
    d.carte(524, 76, 444, 52, "« Sonnette ouvre » : la sonnette sonne, la porte s'ouvre", ["les droits de son responsable ∩ le mandat ; adopté par le foyer : le mandat"], "violet", taille=11.5, sous=10)
    for k, (t, ok) in enumerate([("n'importe qui peut sonner : à revoir, sans décision", False), ("accepté par Bob, en connaissance de cause : oui", True), ("Alice partie : sa « Nuit » refusée, mise à revoir", False)]):
        y = 140 + k * 34
        croix_ou_coche(d, 524, y, ok)
        d.t(552, y + 13, t, 10.8, True, ENCRE)
    for k, (t, lignes_, ic) in enumerate([("Le manifeste", ["classes et pièces que le graphe", "commande, à la validation ; le", "responsable doit l'approuver"], "document"),
                                          ("Le bail", ["s'ouvre au succès de son ordre ;", "ne rétablit que s'il possède", "encore l'état"], "horloge"),
                                          ("L'exécution", ["son empreinte : jamais dans un", "autre programme ; elle passe", "au secours (wf/ex)"], "fleches"),
                                          ("Les rafales", ["20 départs par minute et par", "automatisme ; une transition", "d'une demande, une fois"], "pouls")]):
        d.carte(16 + k * 244, 266, 234, 114, t, lignes_, "ardoise", ic, sous=10.2)
    bandeau(d, 396, "Désarmer n'est dans aucun mandat. Un agent rédige un brouillon ; seule la validation d'une personne habilitée crée un mandat.", "vert")
    A.ecrire(d, "Qui répond d'un automatisme",
             "Deux zones. Lancé par une personne, une routine ou une scène garde les droits de la personne, ceux de sa connexion et le mandat de l'automatisme : Charlie lance « Départ » sur la tablette ; sa lampe s'allume, la porte non, ce n'est pas sa pièce ; depuis la tablette anonyme, la chambre se propose. Autonome, un automatisme a les droits de son responsable dans la limite de son mandat, ou son seul mandat s'il est adopté par le foyer : « Sonnette ouvre » est à revoir tant que personne n'a accepté qu'un bouton que tout le monde peut presser ouvre la porte ; accepté par Bob, il ouvre ; Alice partie, sa « Nuit » est refusée et mise à revoir. En bas, quatre cartes : le manifeste, le bail, l'exécution, les rafales.",
             444, "Aujourd'hui, un automatisme agit en « automation », une origine que la garde ne retient jamais, et la routine lancée par un appareil anonyme ignore les scènes (§ 5.4, E4 et E5). La v3 distingue l'exécution appelée de l'autonome (revue W1 à W6).")

# ================================================================ Ce que voit chacun
if voulu("prive"):
    d = Dessin("prive")
    d.titre(16, 24, "Ce que chacun voit : ce qui s'est passé, à tous ; qui l'a fait, au pair et à soi")
    cols = [(16, "ord/, val/, meta/", "tous les lecteurs d'une pièce", "bleu", "oeil",
             ["l'ordre, la valeur, l'issue, l'heure", "la dernière commande et sa preuve", "jamais qui, ni pour qui", "ni la preuve qu'on a de lui"]),
            (344, "Le pair, seul", "le canal privé", "violet", "cadenas",
             ["les droits : personnes, comptes,", "table, mandats, par leur nom", "les sessions ; l'identité de chaque", "trace ; les exécutions, les textes"]),
            (672, "Un écran", "son état, filtré", "ambre", "ecran",
             ["les appareils de ses pièces", "ses dernières commandes : « miennes »", "les retenues qu'il peut décider", "rien de l'identité des autres"])]
    for x, t, s_, ton, ic, lignes_ in cols:
        d.bloc(x, 54, 312, 50, ton, t, s_, icone=ic)
        d.carte(x, 118, 312, 116, "Ce qu'il porte", lignes_, ton, taille=11.5, sous=10.4)
    bandeau(d, 250, "Lire l'état d'un appareil et lire qui l'a commandé sont deux droits ; l'audit nominatif est réservé à qui le propriétaire désigne (§ 22).", "vert", "oeil")
    A.ecrire(d, "Ce que voit chacun",
             "Trois colonnes. ord/, val/ et meta/, que lisent tous les lecteurs d'une pièce : l'ordre, la valeur, l'issue, l'heure, la dernière commande et sa preuve, jamais qui, ni pour qui. Le pair seul, par le canal privé : les droits par leur nom, les sessions, l'identité de chaque trace, les exécutions du moteur, les textes privés. Un écran : les appareils de ses pièces, ses propres dernières commandes marquées « miennes », les retenues qu'il peut décider, rien de l'identité des autres.",
             298, "Aujourd'hui, le journal nomme l'acteur en texte libre et le flux SSE le montre à tout le foyer (§ 5.4). La v3 sépare l'état commun de l'audit nominatif sur le même flux MQTT (revue D8).")

print(f"ok : {len(A.noms)} schémas → {A.F} et {A.MD}")
