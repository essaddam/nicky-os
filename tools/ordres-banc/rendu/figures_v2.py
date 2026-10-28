#!/usr/bin/env python3
"""Schémas et graphiques de l'analyse « les ordres des appareils », v2
(docs/proposition/ordres-appareils-v2.md), en relief, avec la boîte à outils de la haute
disponibilité v2 (tools/haute-dispo-banc/rendu/relief.py). Les graphiques lisent la série v2 du banc
(../sorties/linux-v2/*.txt) : aucun chiffre recopié. Écrit les fragments dans rendu/fig2/ (build.py les
insère dans la page) et les SVG autonomes, légende comprise, dans docs/proposition/ordres-appareils-v2/.

Usage : python3 -I tools/ordres-banc/rendu/figures_v2.py [schéma …]"""
import math
import pathlib
import sys

ICI = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ICI.parents[1] / "haute-dispo-banc" / "rendu"))
from relief import ENCRE, GRIS, LIGNE, TONS, Atelier, Dessin, court, fr, largeur, lire_resultats, nb  # noqa: E402

RES = lire_resultats(ICI.parent / "sorties/linux-v2")
A = Atelier(ICI / "fig2", ICI.parents[2] / "docs/proposition/ordres-appareils-v2")
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
    for k, l in enumerate(["les trois registres, le catalogue", "demandes, reçus, retenues", "les principaux, leurs permis"]):
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
    champs(d, 32, 62, 0, 40, [("valeur", 8), ("séquence", 8), ("échéance", 8), ("reçue", 8), ("mère", 8), ("ordre", 4), ("qui", 4), ("gén.", 4), ("forme·de", 4), ("état…", 4), ("groupe", 4), ("drapeaux·époque", 8)], "bleu", "Demande · 72 o", px)
    d.t(32, 128, "état · issue · origine · raison ; cibles · conclues · confirmées · exécutées ; drapeaux, époque", 10.5, coul=GRIS)
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
             f"Une fiche de 72 octets : le premier paramètre, la séquence de l'appelant, l'échéance monotone, l'heure d'arrivée, la demande mère, le numéro de l'ordre, qui demande, la génération, la forme d'admission, le principal, puis l'état, l'issue, l'origine, la raison, les compteurs d'un groupe, les drapeaux et l'époque. Son numéro : 16 bits de case, 16 d'époque, 32 de génération. Une issue tardive de la génération 6, ou de l'ancien actif, arrive quand la génération 7 occupe la case : elle est écartée. Prendre une case et la rendre : {fr(r('demandes', 1, 'prendre_et_liberer_une_case_ns'), 1)} ns ; retrouver une demande : {fr(r('demandes', 1, 'retrouver_une_demande_ns'), 2)} ns.",
             374, "La v1 tenait en 64 octets, avec une génération de 16 bits qui faisait le tour, et sans époque : un actif qui reprend pouvait redonner un numéro (revue B4).")

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

print(f"ok : {len(A.noms)} schémas → {A.F} et {A.MD}")
