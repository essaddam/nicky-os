#!/usr/bin/env python3
"""Schémas et graphiques de la proposition haute disponibilité v2 (docs/proposition/haute-disponibilite-v2.md),
en relief (rendu/relief.py). Les graphiques lisent la série du banc (../sorties/linux/*.txt) : aucun
chiffre recopié. Écrit les fragments dans rendu/fig2/ (build.py les insère dans la page) et les SVG
autonomes, légende comprise, dans docs/proposition/haute-disponibilite-v2/ (le Markdown les affiche).

Usage : python3 -I tools/haute-dispo-banc/rendu/figures_v2.py [schéma …]"""
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from relief import GRIS, ENCRE, TONS, Atelier, Dessin, court, fr, lire_resultats, nb  # noqa: E402

ICI = pathlib.Path(__file__).resolve().parent
RES = lire_resultats(ICI.parent / "sorties/linux")
A = Atelier(ICI / "fig2", ICI.parents[2] / "docs/proposition/haute-disponibilite-v2")
CHOIX = set(sys.argv[1:])


def r(sec, e, cle):
    k = (sec, e, cle)
    if k not in RES:
        raise SystemExit(f"résultat absent : {k}")
    return RES[k]


def voulu(nom):
    return not CHOIX or nom in CHOIX


# ================================================================ Les modes
if voulu("modes"):
    d = Dessin("modes")
    d.titre(16, 24, "Une machine locale : ses modes, et ce qui la fait passer de l'un à l'autre")
    W, H = 184, 56
    P = {"reprise": (112, 244), "ratt": (330, 104), "ajour": (606, 104), "actif": (878, 104), "isole": (606, 244), "arret": (878, 384)}
    # Les flèches d'abord, sous les états.
    d.fleche("M112 216 C112 150 170 104 234 104", "bleu", 1.8)
    d.etiquette(128, 196, "un pair porte l'époque :", "bleu", "start", 10)
    d.etiquette(128, 210, "son état complet", "bleu", "start", 10)
    d.fleche("M424 94 L512 94", "bleu", 1.8)
    d.etiquette(468, 86, "rattrapé", "bleu", taille=10)
    d.fleche("M512 116 L424 116", "bleu", 1.8)
    d.etiquette(468, 132, "un trou", "bleu", taille=10)
    d.fleche("M606 134 L606 212", "rouge", 1.8)
    d.etiquette(614, 178, "lien coupé", "rouge", "start", 10)
    d.fleche("M512 250 C440 250 360 210 338 136", "bleu", 1.8)
    d.etiquette(424, 236, "lien revenu", "bleu", taille=10)
    d.fleche("M700 104 L782 104", "vert", 2.2)
    d.etiquette(741, 94, "promotion", "vert", taille=10)
    d.etiquette(741, 124, "époque E+1", "vert", taille=9.5, gras=False)
    d.fleche("M878 74 L878 50 L330 50 L330 74", "bleu", 1.6, tirets="5 4")
    d.etiquette(604, 44, "rétrogradé : une époque plus grande existe, il redevient secours", "bleu", taille=10)
    d.fleche("M206 254 L240 254 L240 314 L846 314 L846 136", "vert", 1.6)
    d.etiquette(470, 308, "aucun pair vivant : l'état le plus avancé, puis l'époque du témoin", "vert", taille=10)
    d.fleche("M972 104 L986 104 L986 384 L972 384", "ardoise", 1.6)
    d.etiquette(982, 250, "SIGTERM, onduleur, mise à jour", "ardoise", "end", 10)
    d.fleche("M784 392 L80 392 L80 276", "ardoise", 1.6)
    d.etiquette(420, 386, "redémarrage", "ardoise", taille=10)
    d.bouton(*P["reprise"], W, H, "Reprise", "cherche le plus avancé", "ardoise", "fleches")
    d.bouton(*P["ratt"], W, H, "Rattrapage", "file, ou état complet", "bleu", "anneau")
    d.bouton(*P["ajour"], W, H, "À jour", "applique, prêt à reprendre", "bleu", "coche")
    d.bouton(*P["actif"], W, H, "Actif", "agit, numérote, publie", "vert", "eclair")
    d.bouton(*P["isole"], W, H, "Isolé", "garde sa mémoire ; alerte", "rouge", "croix")
    d.bouton(*P["arret"], W, H, "Arrêt figé", "l'instantané, sur disque", "ardoise", "disque")
    d.titre(16, 434, "Le secours distant n'a que deux modes, et ne prend jamais l'époque maison")
    d.bouton(250, 476, 216, 50, "Copie", "mémoire, historique, alerte", "violet", "pile")
    d.bouton(750, 476, 216, 50, "Cloud", "porte l'époque cloud", "violet", "nuage")
    d.fleche("M360 466 L640 466", "violet", 1.8)
    d.etiquette(500, 458, "les deux veilles se taisent, puis un geste humain", "violet", taille=10)
    d.fleche("M640 488 L360 488", "violet", 1.6, tirets="5 4")
    d.etiquette(500, 504, "rendre la main : un geste humain", "violet", taille=10)
    A.ecrire(d, "Les modes d'une machine, et les passages",
             "Une machine à six états. Au démarrage, la reprise cherche la mémoire la plus avancée : si un pair porte l'époque, la machine prend son état complet et passe en rattrapage ; sinon, elle prend l'état le plus avancé et demande l'époque au témoin pour devenir active. En rattrapage, puis à jour, le secours applique ; un trou le renvoie en rattrapage, un lien coupé l'isole, le lien revenu le remet en rattrapage. À jour, il peut être promu actif (époque E+1). L'actif rétrogradé redevient secours. L'actif s'arrête en figeant sa mémoire (SIGTERM, onduleur, mise à jour), puis redémarre en reprise. Le secours distant n'a que deux modes : copie, et cloud sur geste humain.",
             520, "L'époque est la seule porte vers l'actif, et le témoin la donne. Les tirets marquent un rôle qu'on rend : l'actif rétrogradé, le cloud rendu à la maison.")

# ================================================================ Les trois classes de données
if voulu("classes"):
    d = Dessin("classes")
    cols = [
        (16, "ambre", "bouclier", "Critique", "exacte, à tout instant",
         [("Ce qu'elle porte", ["valeurs et instants, santé, annuaire", "état des workflows : exécutions,", "attentes, horaires, minuteries, agendas", "coffre (scellé), fichiers métier, époque"]),
          ("Par MQTT", ["val/ vu/ avail/ meta/ evt/", "wf/ conf/ repl/coffre/"]),
          ("Rang", ["s : un seul, sans trou, pour tout"]),
          ("Un trou", ["la file en mémoire (10 000 sorties),", "sinon l'état complet"]),
          ("Sur disque", ["l'instantané seulement"])]),
        (344, "ardoise", "horloge", "Historique", "au mieux en direct, complète plus tard",
         [("Ce qu'elle porte", ["relevés pour les courbes, heures d'énergie", "journal d'exécution des workflows", "notifications envoyées, audit"]),
          ("Par MQTT", ["run/ : en direct, pour les écrans", "hist/ : les opérations rangées"]),
          ("Rang", ["seq (D13), donné par la base de l'actif"]),
          ("Un trou", ["comblé plus tard, depuis la base", "de l'actif, à débit borné"]),
          ("Sur disque", ["la base d'historique (elle existe)"])]),
        (672, "gris", "document", "Hors MQTT", "les octets ailleurs, la référence par MQTT",
         [("Ce qu'elle porte", ["flux vidéo et audio (caméras, Frigate)", "enregistrements, médias, documents", "sauvegardes, archive, micrologiciels"]),
          ("Par MQTT", ["la référence seulement :", "empreinte, taille, adresse"]),
          ("Les octets", ["HTTP avec reprise, rsync,", "protocole de l'archive (D13)"]),
          ("Un trou", ["redemander ; l'empreinte vérifie"]),
          ("Sur disque", ["là où ils sont déjà (NAS, archive)"])]),
    ]
    for x, ton, ic, titre, devise, rubriques in cols:
        f, c, p = d._t(ton)
        w = 312
        d.brut(f'<rect x="{x}" y="16" width="{w}" height="432" rx="16" fill="url(#classes-f-{ton})" stroke="{c}" stroke-opacity=".55" filter="url(#classes-o2)"/>')
        d.brut(f'<path d="M{x} 86 V32 a16 16 0 0 1 16 -16 H{x + w - 16} a16 16 0 0 1 16 16 V86 z" fill="url(#classes-h-{ton})"/>')
        d.brut(f'<rect x="{x + 6}" y="20" width="{w - 12}" height="26" rx="12" fill="#ffffff" opacity=".14"/>')
        d.brut(f'<circle cx="{x + 34}" cy="51" r="19" fill="#ffffff" opacity=".22"/>')
        d.icone(ic, x + 22, 39, 24)
        d.t(x + 64, 49, titre, 17, True, "#ffffff")
        d.t(x + 64, 68, devise, 10.5, coul="#ffffff", op=".92")
        y = 110
        for lab, lignes in rubriques:
            d.t(x + 18, y, lab.upper(), 9.5, True, f, espace=".06em")
            for k, l in enumerate(lignes):
                mono = lab == "Par MQTT" and ("/" in l)
                d.t(x + 18, y + 16 + 15 * k, l, 10.5 if not mono else 10, coul=ENCRE if not mono else f, mono=mono)
            y += 22 + 15 * len(lignes) + 10
    d.brut(f'<rect x="16" y="464" width="968" height="44" rx="12" fill="{TONS["vert"][2]}" stroke="{TONS["vert"][1]}" stroke-opacity=".5"/>')
    d.icone("lettre", 30, 476, 20, TONS["vert"][0])
    d.t(60, 482, "Le maximum passe par MQTT : des messages de 4 Kio au plus (la règle de la V7 sur le réseau) ; un objet", 11, True, ENCRE)
    d.t(60, 498, "d'état plus grand (l'état complet, un fichier métier) se découpe en morceaux. Hors MQTT : ce qui coule en continu, ou n'est pas de l'état.", 11, coul=ENCRE)
    A.ecrire(d, "Trois classes de données",
             "Trois colonnes. Critique : exacte à tout instant ; valeurs, santé, annuaire, état des workflows, coffre, fichiers métier, époque ; par val/, vu/, avail/, meta/, evt/, wf/, conf/, repl/coffre/ ; un seul rang s ; un trou se comble par la file en mémoire, sinon par l'état complet ; sur disque, l'instantané seulement. Historique : au mieux en direct, complète plus tard ; relevés, énergie, journal d'exécution, notifications, audit ; par run/ et hist/ ; rang seq donné par la base de l'actif ; un trou se comble plus tard depuis la base de l'actif. Hors MQTT : vidéo, enregistrements, documents, sauvegardes, archive ; MQTT ne porte que la référence ; les octets passent par HTTP, rsync ou le protocole de l'archive.",
             520, "Une donnée est critique si l'actif décide avec elle et que les appareils ne la redonnent pas. Le reste est de l'historique, ou des octets qui ont déjà leur chemin.")

# ================================================================ Vue d'ensemble
if voulu("ensemble"):
    d = Dessin("ensemble")
    d.zone(16, 34, 650, 476, "La maison", "vert", "maison")
    d.zone(684, 34, 300, 476, "Hors de la maison", "violet", "nuage")
    # Le tube NetBird, au-dessus des machines, de l'actif vers le distant.
    d.brut(f'<path d="M170 92 L170 74 L838 74 L838 96" fill="none" stroke="{TONS["violet"][2]}" stroke-width="16" stroke-linejoin="round" stroke-linecap="round"/>')
    d.brut(f'<path d="M170 92 L170 74 L838 74 L838 96" fill="none" stroke="{TONS["violet"][1]}" stroke-opacity=".5" stroke-width="16" stroke-linejoin="round" stroke-linecap="round" stroke-dasharray="1 0" opacity=".25"/>')
    d.fleche("M170 90 L170 70 L834 70 L834 92", "ambre", 2.2, anime=True)
    d.fleche("M176 92 L176 78 L842 78 L842 96", "ardoise", 1.4, anime=True, lent=True, tete=False)
    d.pilule(704, 64, "NetBird (WireGuard)", "violet", True, 9.5, "middle", "bouclier")
    y0 = d.bloc(40, 104, 256, 186, "vert", "H1 · actif", "porte les époques", icone="eclair")
    for k, (l, ic) in enumerate((("cœur : hub, moteur, mémoire", "puce"), ("un rang s pour tout le critique", "pile"),
                                 ("file en mémoire : 10 000 sorties", "anneau"), ("courtier MQTT embarqué", "lettre"),
                                 ("pilotes, radios, clouds", "radio"), ("disque : l'instantané seulement", "disque"))):
        d.icone(ic, 54, y0 + 9 + 24 * k, 14, TONS["vert"][0], 1.8)
        d.t(76, y0 + 21 + 24 * k, l, 10.5, coul=ENCRE)
    y0 = d.bloc(392, 104, 256, 186, "bleu", "H2 · secours local", "passif, prêt", icone="coche")
    for k, (l, ic) in enumerate((("mémoire copiée, même rang s", "pile"), ("moteur : applique, ne réveille rien", "rouage"),
                                 ("file du secours, pour ses abonnés", "anneau"), ("pilotes et radios arrêtés", "croix"),
                                 ("API et écrans en lecture seule", "ecran"), ("disque : l'instantané seulement", "disque"))):
        d.icone(ic, 406, y0 + 9 + 24 * k, 14, TONS["bleu"][0], 1.8)
        d.t(428, y0 + 21 + 24 * k, l, 10.5, coul=ENCRE)
    # Le pont maison, entre H1 et H2.
    d.fleche("M314 158 L388 158", "ambre", 2.6, anime=True)
    d.fleche("M314 186 L388 186", "ardoise", 1.6, anime=True, lent=True)
    d.fleche("M388 214 L316 214", "gris", 1.3, tirets="3 3")
    d.pilule(350, 126, "pont", "ambre", True, 9.5, "middle")
    d.t(351, 236, "battement,", 9, coul=GRIS, ancre="middle")
    d.t(351, 247, "demandes", 9, coul=GRIS, ancre="middle")
    # Le secours distant.
    y0 = d.bloc(708, 104, 246, 158, "violet", "Secours distant", "copie, alerte, cloud", icone="nuage")
    for k, l in enumerate(("mémoire et historique copiés", "alerte si la maison se tait", "cloud : sur geste humain", "jamais les radios ni le réseau local")):
        d.brut(f'<circle cx="725" cy="{y0 + 16 + 24 * k}" r="3" fill="{TONS["violet"][0]}"/>')
        d.t(736, y0 + 20 + 24 * k, l, 10.5, coul=ENCRE)
    d.carte(708, 300, 266, 74, "Clouds", ["Netatmo, Voltalis, Ariston… : appelés par le", "seul porteur de l'époque cloud, H1 d'habitude"], "violet", "nuage")
    d.fleche("M834 282 L834 298", "violet", 1.4, tirets="4 3")
    d.cylindre(842, 392, 96, 64, "ardoise", "Archive (D13)", "l'export du porteur")
    # Le témoin, l'adresse virtuelle, le NAS.
    d.carte(232, 326, 168, 58, "Témoin", ["vote le bail de l'époque"], "gris", "oeil")
    d.fleche("M300 326 L230 296", "gris", 1.2, tirets="3 3", tete=False)
    d.fleche("M332 326 L420 296", "gris", 1.2, tirets="3 3", tete=False)
    d.pilule(40, 336, "adresse virtuelle de l'actif", "vert", True, 9.5, icone="cible" if False else None)
    d.cylindre(520, 326, 84, 56, "gris", "NAS", "vidéo, documents")
    # Les clients.
    clients = [("Écrans", "navigateur, appli", "ecran"), ("Téléphones", "notifications", "telephone"), ("Appareils", "MQTT du réseau", "radio"),
               ("Radios", "boîtiers A et B", "radio"), ("Caméras", "vidéo : hors MQTT", "camera")]
    for k, (lab, sous, ic) in enumerate(clients):
        x = 32 + k * 124
        d.carte(x, 432, 116, 62, lab, [sous], "gris" if lab == "Caméras" else "vert", ic, taille=10.5, sous=9.5)
        if lab != "Caméras":
            d.fleche(f"M{x + 58} 432 L{x + 58} 418", "vert", 1.2, tete=False)
    d.fleche("M90 418 L462 418", "vert", 1.2, tete=False)
    d.fleche("M110 418 L110 356", "vert", 1.4)
    d.fleche("M110 336 L110 294", "vert", 1.8, anime=True, lent=True)
    d.fleche("M590 432 L566 404", "gris", 1.4, tirets="5 4", anime=True, lent=True)
    # La légende des flux.
    d.fleche("M28 534 L72 534", "ambre", 2.6, anime=True, tete=False)
    d.t(80, 538, "critique : rang s, copie exacte", 10.5, True, TONS["ambre"][0])
    d.fleche("M300 534 L344 534", "ardoise", 1.6, anime=True, lent=True, tete=False)
    d.t(352, 538, "historique : rang seq, au mieux", 10.5, True, TONS["ardoise"][0])
    d.fleche("M580 534 L624 534", "gris", 1.4, tirets="5 4", tete=False)
    d.t(632, 538, "hors MQTT : vidéo, gros fichiers", 10.5, True, TONS["gris"][0])
    A.ecrire(d, "La maison en double, vue d'ensemble",
             "À gauche, la maison : H1, l'actif, porte les époques ; il a le cœur, un rang s pour tout le critique, une file en mémoire de 10 000 sorties, son courtier, ses pilotes ; sur disque, l'instantané seulement. H2, le secours local, reçoit par le pont maison le flux critique et l'historique, et renvoie battements et demandes ; il applique sans rien réveiller, pilotes arrêtés. Un témoin vote le bail. Les clients (écrans, téléphones, appareils MQTT, boîtiers radio) parlent à l'adresse virtuelle de l'actif ; les caméras écrivent sur le NAS, hors MQTT. À droite, hors de la maison : le secours distant reçoit le même flux par un tunnel NetBird ; il copie, alerte et peut reprendre les clouds sur geste humain ; l'archive reçoit l'export du porteur de l'époque.",
             552, "Une seule machine agit à la fois. Le pont maison porte tout ce qui est critique et l'historique ; la vidéo et les gros fichiers ont leur propre chemin. Aucun journal sur disque.")

# ================================================================ Les deux systèmes, en détail
if voulu("systemes"):
    d = Dessin("systemes")

    def machine(x, ton, titre, sous, actif):
        y0 = d.bloc(x, 40, 410, 424, ton, titre, sous, icone="eclair" if actif else "coche")
        cx = x + 14
        if actif:
            d.carte(cx, y0 + 12, 382, 60, "Séquenceur · rang s", ["chaque sortie critique reçoit s + 1, sans trou"], "ambre", "pile", sous=10)
        else:
            d.carte(cx, y0 + 12, 382, 60, "Pont maison · bout secours", ["remet l'ordre de s, comble les trous, réassemble l'état"], "ambre", "fleches", sous=10)
        lab = ([("Hub", ["vérité, garde,", "publie val/ vu/"], "puce"), ("Moteur", ["workflows ;", "publie wf/ run/"], "rouage"), ("Mémoire", ["valeurs, santé,", "exécutions"], "pile")]
               if actif else
               [("Hub", ["applique les sorties", "dans l'ordre de s"], "puce"), ("Moteur", ["applique wf/,", "ne réveille rien"], "rouage"), ("Mémoire", ["la même, au", "même rang s"], "pile")])
        for k, (t, l, ic) in enumerate(lab):
            d.carte(cx + k * 130, y0 + 82, 122, 76, t, l, ton, ic, taille=11.5, sous=10)
        if actif:
            d.carte(cx, y0 + 170, 188, 74, "File en mémoire", ["10 000 sorties ;", "tronçons, état complet"], "ambre", "anneau", taille=11.5, sous=10)
            d.carte(cx + 194, y0 + 170, 188, 74, "Base d'historique", ["abonnée : range, donne seq,", "publie hist/"], "ardoise", "base", taille=11.5, sous=10)
            d.carte(cx, y0 + 256, 188, 60, "API · WebSocket", ["écrans, appli, Sarah"], "vert", "ecran", taille=11.5, sous=10)
            d.carte(cx + 194, y0 + 256, 188, 60, "Pilotes, radios", ["Zigbee2MQTT, Matter, clouds"], "vert", "radio", taille=11.5, sous=10)
        else:
            d.carte(cx, y0 + 170, 188, 74, "File du secours", ["ses abonnés locaux", "y comblent leurs trous"], "ambre", "anneau", taille=11.5, sous=10)
            d.carte(cx + 194, y0 + 170, 188, 74, "Base d'historique", ["copie de reprise :", "applique hist/ par seq"], "ardoise", "base", taille=11.5, sous=10)
            d.carte(cx, y0 + 256, 188, 60, "API · WebSocket", ["lecture seule, « secours »"], "bleu", "ecran", taille=11.5, sous=10)
            d.carte(cx + 194, y0 + 256, 188, 60, "Pilotes, radios", ["démarrent à la promotion"], "gris", "radio", attenue=True, taille=11.5, sous=10, mot="arrêtés")
        f, c, p = d._t(ton)
        d.brut(f'<rect x="{cx}" y="{y0 + 330}" width="382" height="44" rx="10" fill="url(#systemes-h-{ton})"/>')
        d.icone("lettre", cx + 12, y0 + 342, 18)
        d.t(cx + 38, y0 + 350, "Courtier MQTT embarqué (rumqttd)" if actif else "Courtier du secours : seul le pont y publie", 11.5, True, "#ffffff")
        d.t(cx + 38, y0 + 365, "liens en mémoire pour le cœur ; 1883, 8883 pour les clients" if actif else "les abonnés locaux y lisent, comme sur l'actif", 9.5, coul="#ffffff", op=".9")

    machine(16, "vert", "H1 · actif", "porte l'époque", True)
    machine(560, "bleu", "H2 · secours local", "passif, prêt", False)
    # Le lien : du courtier de l'actif au pont du secours.
    flux = [(100, "critique · rang s", "ambre", 2.6, True, False, None), (142, "historique · seq", "ardoise", 1.6, True, True, None),
            (184, "tronçons, état complet", "ambre", 1.8, False, False, "6 4")]
    for y, lab, ton, ep, an, lent, tir in flux:
        d.fleche(f"M446 {y + 26} L556 {y + 26}", ton, ep, anime=an, lent=lent, tirets=tir)
        d.etiquette(501, y + 18, lab, ton, taille=9.5)
    d.fleche("M556 266 L448 266", "gris", 1.4, tirets="3 3")
    d.etiquette(501, 258, "battement, demandes", "gris", taille=9.5)
    d.etiquette(501, 290, "réseau local", "gris", taille=9, gras=False)
    d.etiquette(501, 303, "ou NetBird", "gris", taille=9, gras=False)
    # Les clients de l'actif, sous lui ; le témoin entre les deux.
    y = 500
    cl = [("Écrans", "navigateur, appli", "ecran", "vert"), ("Téléphones", "notifications", "telephone", "vert"),
          ("Appareils", "MQTT du réseau", "radio", "vert"), ("Zigbee", "Zigbee2MQTT", "radio", "vert")]
    for k, (t, l, ic, ton) in enumerate(cl):
        d.carte(16 + k * 104, y + 26, 98, 58, t, [l], ton, None, taille=10.5, sous=9)
        d.fleche(f"M{65 + k * 104} {y + 26} L{65 + k * 104} {y + 12}", "vert", 1.2, tete=False)
    d.fleche(f"M65 {y + 12} L377 {y + 12}", "vert", 1.2, tete=False)
    d.fleche(f"M220 {y + 12} L220 {y - 22}", "vert", 1.6, anime=True, lent=True)
    d.pilule(232, y - 6, "adresse virtuelle de l'actif", "vert", True, 9)
    d.carte(436, y + 26, 148, 58, "Témoin", ["vote le bail des deux"], "gris", "oeil", taille=11, sous=9.5)
    d.fleche(f"M470 {y + 26} L320 {y - 36}", "gris", 1.1, tirets="3 3", tete=False)
    d.fleche(f"M550 {y + 26} L690 {y - 36}", "gris", 1.1, tirets="3 3", tete=False)
    d.carte(610, y + 26, 178, 58, "Écrans en lecture", ["sur le secours, s'il le faut"], "bleu", "ecran", attenue=True, taille=11, sous=9.5)
    d.carte(800, y + 26, 184, 58, "Caméras → NAS", ["vidéo : hors MQTT"], "gris", "camera", taille=11, sous=9.5)
    A.ecrire(d, "Les deux systèmes, en détail : le cœur, les courtiers, les clients",
             "Deux machines côte à côte. H1, l'actif : le séquenceur donne le rang s à chaque sortie critique ; le hub, le moteur des workflows et la mémoire ; la file en mémoire des 10 000 dernières sorties ; la base d'historique, abonnée, qui range et numérote ; l'API et les écrans ; les pilotes et les radios ; le courtier MQTT embarqué ; sur disque, l'instantané seulement. H2, le secours : le bout secours du pont maison, qui remet l'ordre et comble les trous ; le même hub, moteur et mémoire, qui appliquent sans agir ; sa propre file pour ses abonnés ; une base d'historique en copie de reprise ; l'API en lecture seule ; pilotes et radios arrêtés. Entre les deux, le flux critique, l'historique, les tronçons et l'état complet ; en retour, le battement et les demandes. Les clients parlent à l'adresse virtuelle de l'actif ; le témoin vote le bail des deux ; les caméras écrivent sur le NAS.",
             592, "Une même application sur les deux machines ; seul le mode change. Le secours n'a qu'un éditeur, le pont maison, et rien ne part de lui vers un appareil. Sur disque, de chaque côté : l'instantané seulement.")

# ================================================================ Le pont maison
if voulu("pont"):
    d = Dessin("pont")
    d.titre(16, 24, "Bout actif")
    d.titre(340, 24, "Le lien : réseau local, ou NetBird")
    d.titre(678, 24, "Bout secours")
    d.carte(16, 40, 300, 60, "Séquenceur", ["chaque sortie critique : s + 1"], "ambre", "pile")
    d.carte(16, 114, 300, 60, "Courtier de l'actif", ["transport seulement : QoS 0, session propre"], "vert", "lettre")
    d.fleche("M166 100 L166 112", "ambre", 1.8)
    d.anneau(78, 236, 44, "ambre", 30, 24, 10)
    d.t(78, 232, "10 000", 12, True, TONS["ambre"][0], "middle")
    d.t(78, 247, "sorties", 9.5, coul=GRIS, ancre="middle")
    d.t(140, 210, "File en mémoire", 12, True, TONS["ambre"][0])
    d.t(140, 228, "les dernières sorties, en binaire :", 10, coul=GRIS)
    d.t(140, 243, "46 o chacune, 0,45 Mio en tout ;", 10, coul=GRIS)
    d.t(140, 258, "elle sert les tronçons", 10, coul=GRIS)
    d.carte(16, 296, 300, 74, "État complet", ["construit en mémoire au rang s0, à la demande ;", "envoyé en morceaux de 4 Kio, jamais écrit"], "ambre", "pile")
    d.carte(16, 384, 300, 60, "Battement", ["toutes les 500 ms : « le bout est s »"], "gris", "pouls")
    d.brut(f'<rect x="334" y="36" width="332" height="408" rx="18" fill="#ffffff" fill-opacity=".6" stroke="{TONS["gris"][1]}" stroke-dasharray="6 5"/>')
    msgs = [(66, "→", "sorties critiques, en direct", "ambre", 2.6, True, None),
            (112, "→", "battement : bout = s", "gris", 1.4, True, None),
            (158, "←", "demande {de : s}", "ambre", 1.4, False, None),
            (204, "→", "tronçons : 64 sorties à la fois", "ambre", 1.8, False, "6 4"),
            (250, "→", "« hors file » : ce rang n'y est plus", "rouge", 1.6, False, None),
            (296, "←", "demande d'état complet", "ambre", 1.4, False, None),
            (342, "→", "état complet, en morceaux", "ambre", 2.2, True, "7 4"),
            (388, "←", "accusé du rang, 500 ms", "gris", 1.2, False, "3 3")]
    for y, sens, lab, ton, ep, an, tir in msgs:
        if sens == "→":
            d.fleche(f"M346 {y + 8} L654 {y + 8}", ton, ep, anime=an and tir is None, tirets=tir)
        else:
            d.fleche(f"M654 {y + 8} L346 {y + 8}", ton, ep, tirets=tir)
        d.etiquette(500, y, lab, ton, taille=10)
    d.brut(f'<circle class="hd-pouls" cx="352" cy="120" r="4.5" fill="{TONS["gris"][0]}"/>')
    d.carte(678, 40, 306, 118, "Pont maison", ["remet l'ordre de s (grâce de 200 ms)", "garde ce qui arrive en avance", "voit un trou : au rang s, au battement", "demande : un tronçon, sinon l'état complet", "réassemble et applique l'état complet"], "ambre", "fleches", pas=14.5)
    d.carte(678, 170, 306, 60, "Mémoire du secours", ["appliquée dans l'ordre ; même empreinte"], "bleu", "pile")
    d.carte(678, 242, 306, 60, "Courtier du secours", ["seul le pont y publie"], "bleu", "lettre")
    d.fleche("M830 158 L830 168", "bleu", 1.8)
    d.fleche("M830 230 L830 240", "bleu", 1.8)
    d.anneau(722, 352, 30, "bleu", 24, 18, 8)
    d.t(764, 342, "File du secours", 11.5, True, TONS["bleu"][0])
    d.t(764, 358, "ses abonnés locaux y comblent", 10, coul=GRIS)
    d.t(764, 372, "leurs propres trous", 10, coul=GRIS)
    d.carte(678, 400, 306, 44, "Abonnés locaux : moteur, base, écrans", [], "bleu", "ecran", taille=11)
    d.brut(f'<rect x="16" y="462" width="968" height="34" rx="11" fill="{TONS["vert"][2]}" stroke="{TONS["vert"][1]}" stroke-opacity=".5"/>')
    d.icone("disque", 28, 469, 18, TONS["vert"][0])
    d.t(54, 484, "Aucun disque sur ce chemin. Les courtiers ne gardent rien ; c'est le pont qui sait ce qui manque, et le redemande.", 11, True, ENCRE)
    A.ecrire(d, "Le pont maison, ses deux bouts",
             "Trois colonnes. Le bout actif : le séquenceur numérote ; le courtier transporte, sans rien garder ; la file en mémoire des 10 000 dernières sorties (46 octets chacune, 0,45 Mio) sert les tronçons ; l'état complet se construit en mémoire à la demande ; un battement annonce le bout toutes les 500 ms. Le lien : les sorties en direct, le battement, la demande d'un tronçon, les tronçons de 64 sorties, la réponse « hors file » quand le rang n'y est plus, la demande d'état complet, l'état complet en morceaux de 4 Kio, l'accusé de rang. Le bout secours : le pont remet l'ordre de s, garde ce qui arrive en avance, voit un trou au rang ou au battement, demande un tronçon ou l'état complet, l'applique ; la mémoire du secours ; le courtier du secours, où seul le pont publie ; la file du secours pour ses abonnés locaux.",
             508, "Un module de Sarah Home, à deux bouts, présent sur chaque machine : le mode décide quel bout travaille. Le courtier n'est qu'un transport ; les tampons, les trous et la reprise sont au pont, des deux côtés.")

# ================================================================ Les workflows par MQTT
if voulu("workflows"):
    d = Dessin("workflows")
    d.titre(16, 24, "Une exécution sur l'actif, et ce qu'elle publie")
    X0, X1 = 150, 970
    ev = [(150, "21:42:03,120", "porte ouverte", [("val/ · s 1041", "ambre")]),
          (370, "21:42:03,121", "exécution 87 : départ, attente « 10 min »", [("wf/ · s 1042", "ambre"), ("run/ · départ", "ardoise")]),
          (640, "21:52:03,120", "revérifiée : pas 2, la lampe", [("wf/ · s 1873", "ambre"), ("run/ · pas 2", "ardoise")]),
          (880, "21:52:03,410", "fin : réussie", [("wf/ · s 1874", "ambre"), ("run/ · fin", "ardoise")])]
    d.t(16, 66, "Actif", 12, True, TONS["vert"][0])
    d.brut(f'<rect x="{X0 - 10}" y="56" width="{X1 - X0 + 20}" height="14" rx="7" fill="{TONS["vert"][2]}" stroke="{TONS["vert"][1]}" stroke-opacity=".5"/>')
    d.t(16, 118, "Critique, rang s", 11, True, TONS["ambre"][0])
    d.t(16, 132, "vers les secours", 9.5, coul=GRIS)
    d.t(16, 168, "Historique, run/", 11, True, TONS["ardoise"][0])
    d.t(16, 182, "vers les bases", 9.5, coul=GRIS)
    d.fleche("M150 128 L880 128", "ambre", 1.4, tete=False)
    d.fleche("M370 178 L880 178", "ardoise", 1.4, tete=False)
    for x, h, lab, pills in ev:
        d.brut(f'<circle cx="{x}" cy="63" r="7" fill="url(#workflows-h-vert)" stroke="#ffffff" stroke-width="2"/>')
        d.t(x, 46, h, 9.5, True, GRIS, "middle", mono=True)
        d.t(x, 90, lab, 10, coul=ENCRE, ancre="middle")
        bas = 104 if len(pills) == 1 else 154
        d.fleche(f"M{x} 96 L{x} {bas}", "gris", 1, tete=False, tirets="2 3")
    for x, h, lab, pills in ev:
        for k, (pl, ton) in enumerate(pills):
            d.pilule(x, 104 if ton == "ambre" else 154, pl, ton, True, 9.5, "middle")
    d._t("vert")
    d.fleche("M500 63 L620 63", "vert", 1.2, tete=False, tirets="1 5")
    d.t(500, 80, "10 min d'attente", 9, coul=GRIS, ancre="middle", italique=True)
    # En bas : où vont les messages.
    d.carte(16, 236, 300, 100, "Moteur du secours", ["reçoit wf/ dans le rang s : il tient l'exécution 87", "au même nœud, avec la même échéance (heure", "murale) ; il ne réveille rien. À la bascule, il", "reprend là où l'actif en était."], "bleu", "rouage", pas=14)
    d.cylindre(440, 222, 96, 58, "ardoise", "Base de l'actif", "range run/, donne seq", ["publie hist/ (après validation)"])
    d.cylindre(800, 222, 96, 58, "ardoise", "Base du secours", "applique hist/ par seq", ["mêmes rangs : elle peut", "reprendre l'export (D13)"])
    d.fleche("M150 128 L150 232", "ambre", 1.8, anime=True)
    d.etiquette(158, 214, "wf/ (et val/)", "ambre", "start", 10)
    d.fleche("M440 178 L440 218", "ardoise", 1.6, anime=True, lent=True)
    d.fleche("M492 250 L748 250", "ardoise", 2, anime=True)
    d.etiquette(620, 242, "hist/ : opérations, rang seq", "ardoise", taille=10)
    d.fleche("M748 272 L494 272", "ardoise", 1.2, tirets="3 3")
    d.etiquette(620, 288, "un trou de seq : hist/demande, servi par la base", "ardoise", taille=9.5, gras=False)
    d.brut(f'<rect x="16" y="372" width="968" height="58" rx="12" fill="{TONS["ambre"][2]}" stroke="{TONS["ambre"][1]}" stroke-opacity=".55"/>')
    d.icone("rouage", 28, 384, 20, TONS["ambre"][0])
    d.t(58, 394, "Un seul rang pour les valeurs et les workflows : la v1 avait un journal à part (rang j) et une dépendance à gérer ;", 11, True, ENCRE)
    d.t(58, 412, "ici, « attente armée » (s 1042) ne peut pas arriver avant « porte ouverte » (s 1041) : le secours applique dans l'ordre de s.", 11, coul=ENCRE)
    A.ecrire(d, "Les workflows passent par MQTT",
             "Une frise sur l'actif : à 21:42:03,120 la porte s'ouvre (val/, s 1041) ; une milliseconde plus tard l'exécution 87 démarre et arme une attente de 10 minutes (wf/, s 1042, et run/ départ) ; à 21:52:03,120 l'attente est revérifiée et le pas 2 allume la lampe (wf/, s 1873, et run/ pas 2) ; à 21:52:03,410 l'exécution finit (wf/, s 1874, et run/ fin). En bas : le moteur du secours reçoit wf/ dans le rang s et tient l'exécution au même nœud, avec la même échéance, sans rien réveiller ; la base de l'actif range run/, donne le rang seq et publie hist/ ; la base du secours applique hist/ par seq et peut reprendre l'export ; un trou de seq se redemande à la base de l'actif.",
             442, "L'état d'un workflow est critique (wf/, rang s) ; son journal d'exécution est de l'historique (run/, puis hist/ avec le rang seq de la base de l'actif).")

# ================================================================ Redémarrer sans journal
if voulu("reprise"):
    d = Dessin("reprise")
    d.bouton(330, 40, 200, 40, "Démarrage", None, "ardoise", "eclair")
    d.fleche("M330 62 L330 76", "ardoise", 1.6)
    d.carte(170, 78, 320, 60, "Lire son instantané", ["sur le disque : sa mémoire au rang (E, s)"], "ardoise", "disque")
    d.fleche("M330 138 L330 150", "ardoise", 1.6)
    d.carte(170, 152, 320, 60, "Demander (E, s) aux pairs", ["sys/etat, 10 s au plus : local et distant"], "bleu", "fleches")
    d.fleche("M330 212 L330 222", "ardoise", 1.6)
    d.brut(f'<path d="M190 248 L216 224 H444 L470 248 L444 272 H216 z" fill="url(#reprise-f-ambre)" stroke="{TONS["ambre"][1]}" stroke-width="1.4" filter="url(#reprise-o)"/>')
    d._t("ambre")
    d.t(330, 253, "Un pair porte-t-il une époque vivante ?", 11.5, True, ENCRE, "middle")
    d.fleche("M470 248 L560 248", "bleu", 1.8)
    d.etiquette(515, 240, "oui", "bleu", taille=10)
    d.carte(564, 212, 420, 74, "Devenir secours", ["prendre l'état complet du pair, par MQTT (≈ 100 Ko) ;", "puis à jour, rang par rang. Rien n'est lu du disque."], "bleu", "coche")
    d.fleche("M330 272 L330 294", "vert", 1.8)
    d.etiquette(340, 288, "non", "vert", "start", 10)
    d.carte(140, 296, 380, 74, "Prendre l'état du plus avancé", ["un pair (par MQTT) ou son propre instantané :", "le plus grand (E, s) l'emporte"], "vert", "pile")
    d.fleche("M330 370 L330 382", "vert", 1.6)
    d.carte(170, 384, 320, 60, "Demander l'époque E + 1 au témoin", ["le bail, comme la V7"], "vert", "oeil")
    d.fleche("M330 444 L330 456", "vert", 1.6)
    d.bouton(330, 482, 300, 44, "Actif", "grâce du démarrage pour chaque appareil", "vert", "eclair")
    d.carte(564, 310, 420, 130, "Jamais actif avec moins qu'un pair", ["Le piège de Redis : un maître sans persistance qui", "redémarre vide, et ses répliques qui se recopient", "sur lui, vides à leur tour. Ici, chaque état porte", "(E, s) ; un état plus ancien que le sien est refusé ;", "une machine qui redémarre ne prend l'époque qu'avec", "une mémoire au moins aussi avancée que ses pairs."], "rouge", "croix", pas=14)
    d.fleche("M520 333 L560 350", "rouge", 1.2, tirets="3 3", tete=False)
    A.ecrire(d, "Redémarrer sans journal",
             "Un organigramme. Au démarrage, la machine lit son instantané, sa mémoire au rang (E, s), puis demande (E, s) à ses pairs pendant 10 secondes au plus. Si un pair porte une époque vivante, elle devient secours et prend son état complet par MQTT, sans rien lire du disque. Sinon, elle prend l'état du plus avancé, un pair ou son propre instantané, demande l'époque E + 1 au témoin et devient active, avec la grâce du démarrage pour chaque appareil. À côté, la règle : jamais actif avec moins qu'un pair ; un état plus ancien est refusé.",
             516, "La mémoire la plus fraîche est chez les pairs, pas sur le disque : l'instantané ne sert que si personne d'autre ne vit.")

# ================================================================ Plusieurs stockages
if voulu("stockages"):
    d = Dessin("stockages")
    d.carte(16, 40, 230, 92, "Le cœur de l'actif", ["publie val/ (les relevés)", "et run/ (le journal", "d'exécution), en direct"], "vert", "puce")
    d.cylindre(370, 44, 100, 70, "ardoise", "Base de l'actif", "range, donne seq (D13)", ["la seule qui numérote"])
    d.fleche("M246 86 L316 86", "vert", 1.8, anime=True)
    d.cylindre(640, 44, 100, 70, "ardoise", "Base du secours local", "copie de reprise", ["mêmes opérations, mêmes seq"])
    d.cylindre(880, 44, 100, 70, "violet", "Base du distant", "copie de reprise", ["ou de lecture, au choix"])
    d.fleche("M424 78 L586 78", "ardoise", 2, anime=True)
    d.etiquette(505, 70, "hist/ (seq)", "ardoise", taille=10)
    d.fleche("M694 96 L826 96", "ardoise", 1.6, anime=True, lent=True)
    d.etiquette(760, 88, "hist/ par NetBird", "ardoise", taille=9.5)
    d.fleche("M586 104 L424 104", "ardoise", 1.1, tirets="3 3")
    d.etiquette(505, 122, "trou : hist/demande", "ardoise", taille=9, gras=False)
    d.cylindre(370, 230, 100, 60, "gris", "Archive (Parquet)", "le long terme", ["seul le porteur de l'époque exporte"])
    d.fleche("M370 176 L370 226", "gris", 1.6)
    d.etiquette(380, 206, "export", "gris", "start", 10)
    d.fleche("M640 176 C640 230 520 258 424 258", "gris", 1.2, tirets="5 4")
    d.etiquette(560, 236, "après une bascule : la suite, mêmes rangs", "gris", taille=9.5)
    d.carte(600, 300, 384, 96, "Copie de lecture (la V7 : dérivée de val/)", ["ses lots tombent ailleurs, ses heures closent à son", "horloge, un import fait sur l'actif lui manque : ses", "rangs divergent. Elle répond aux questions, elle", "n'exporte jamais."], "rouge", "croix", pas=14)
    d.carte(16, 300, 230, 96, "Qui répond aux questions", ["l'état courant : la mémoire", "le récent : la base de l'actif", "le long terme : l'archive", "maison éteinte : le distant"], "gris", "ecran", pas=14)
    A.ecrire(d, "Plusieurs stockages : qui numérote, qui recopie",
             "Le cœur de l'actif publie val/ et run/ ; la base de l'actif range et donne le rang seq ; elle publie hist/ vers la base du secours local, copie de reprise avec les mêmes rangs, et par NetBird vers la base du distant ; un trou de seq se redemande. Seul le porteur de l'époque exporte vers l'archive ; après une bascule, le secours exporte la suite avec les mêmes rangs. Une copie de lecture dérivée de val/ diverge et n'exporte jamais. Qui répond : la mémoire, la base de l'actif, l'archive, le distant quand la maison est éteinte.",
             410, "Une seule base numérote l'historique ; les autres le reçoivent rangé, par MQTT. Elles peuvent alors reprendre l'export de D13 après une bascule.")

def xlog(v, vmin, vmax, x0, w):
    import math
    v = max(v, vmin)
    return x0 + w * (math.log10(v) - math.log10(vmin)) / (math.log10(vmax) - math.log10(vmin))


def axe_log(d, x0, w, y0, y1, vmin, vmax, unite, ticks):
    for t_, lab in ticks:
        x = xlog(t_, vmin, vmax, x0, w)
        d.brut(f'<line x1="{x:.1f}" y1="{y0}" x2="{x:.1f}" y2="{y1}" stroke="#d4ddd4" stroke-dasharray="2 3"/>')
        d.t(x, y1 + 14, lab, 9.5, coul=GRIS, ancre="middle")
    d.t(x0 + w, y1 + 28, unite, 9.5, coul=GRIS, ancre="end", italique=True)


# ================================================================ Un trou : la file, ou l'état complet
if voulu("trou"):
    d = Dessin("trou")
    d.titre(16, 24, "Trois façons de manquer des sorties, trois réponses du pont")
    X0, C, N = 214, 17, 28

    def bande(y, recus, perdus_dans, perdus_hors, fenetre, titre, sous):
        d.t(16, y + 13, titre, 12, True, ENCRE)
        d.t(16, y + 29, sous, 10, coul=GRIS)
        f0, f1 = fenetre
        xa, xb = X0 + (f0 - 1) * C - 3, X0 + f1 * C + 1
        d.brut(f'<rect x="{xa}" y="{y - 14}" width="{xb - xa}" height="{C + 24}" rx="9" fill="{TONS["ambre"][2]}" stroke="{TONS["ambre"][1]}" stroke-opacity=".7"/>')
        d.t(xa + 6, y - 4, "la file de l'actif", 9, True, TONS["ambre"][0])
        d._t("vert"); d._t("rouge"); d._t("ambre")
        for k in range(1, N + 1):
            x = X0 + (k - 1) * C
            if k in perdus_hors:
                d.brut(f'<rect x="{x + 1}" y="{y + 1}" width="{C - 3}" height="{C - 3}" rx="4" fill="{TONS["rouge"][2]}" stroke="{TONS["rouge"][0]}" stroke-width="1.4"/>')
            elif k in perdus_dans:
                d.brut(f'<rect x="{x + 1}" y="{y + 1}" width="{C - 3}" height="{C - 3}" rx="4" fill="#ffffff" stroke="{TONS["rouge"][1]}" stroke-width="1.4" stroke-dasharray="3 2"/>')
            elif k <= recus or k > max(list(perdus_dans) + list(perdus_hors) + [0]):
                d.brut(f'<rect x="{x + 1}" y="{y + 1}" width="{C - 3}" height="{C - 3}" rx="4" fill="url(#trou-h-vert)"/>')
        d.t(X0 + N * C + 4, y + 13, "s →", 10, True, GRIS)

    ratt1, ratt2 = r("pont", 1, "coupures_rattrapage_1"), r("pont", 1, "coupures_rattrapage_2")
    bande(70, 12, range(13, 19), [], (8, 28), "Une coupure courte", "5 s, puis 10 s, à 200 /s")
    d.carte(726, 52, 258, 72, "Des tronçons de la file", [f"rattrapé en {court(ratt1, 'ms')} puis {court(ratt2, 'ms')}", "aucune perte, mémoire identique"], "vert", "anneau", sous=10)
    d.fleche("M700 84 L722 84", "vert", 1.8)
    etat, rl = r("pont", 1, "longue_octets_etat"), r("pont", 1, "longue_rattrapage_1")
    bande(186, 3, range(16, 21), range(4, 16), (16, 28), "Une coupure longue", "60 s : 12 000 sorties, plus que la file")
    d.carte(726, 168, 258, 72, "L'état complet, puis la suite", [f"{court(etat / 1024, 'Kio')} en morceaux de 4 Kio ;", f"rattrapé en {court(rl, 'ms')}, mémoire identique"], "ambre", "pile", sous=10)
    d.fleche("M700 200 L722 200", "ambre", 1.8)
    bande(302, 27, [28], [], (8, 28), "La dernière sortie perdue", "plus rien ne suit pour la révéler")
    d.brut(f'<circle class="hd-pouls" cx="{X0 + 27 * C + 7}" cy="{302 - 22}" r="5" fill="{TONS["gris"][0]}"/>')
    d.etiquette(X0 + 27 * C - 2, 302 - 18, "battement : « bout = 28 »", "gris", "end", 9.5)
    d.carte(726, 284, 258, 72, "Le battement la révèle", ["« bout = 28 » toutes les 500 ms ;", "demandée après la grâce de 200 ms"], "gris", "pouls", sous=10)
    d.fleche("M700 316 L722 316", "gris", 1.8)
    for k, (lab, ton, plein) in enumerate((("reçue", "vert", True), ("perdue, encore dans la file", "rouge", False), ("perdue, sortie de la file", "rouge", True))):
        x = 214 + k * 210
        f, c, p_ = d._t(ton)
        if plein and ton == "vert":
            d.brut(f'<rect x="{x}" y="392" width="14" height="14" rx="4" fill="url(#trou-h-vert)"/>')
        elif plein:
            d.brut(f'<rect x="{x}" y="392" width="14" height="14" rx="4" fill="{p_}" stroke="{f}" stroke-width="1.4"/>')
        else:
            d.brut(f'<rect x="{x}" y="392" width="14" height="14" rx="4" fill="#ffffff" stroke="{c}" stroke-width="1.4" stroke-dasharray="3 2"/>')
        d.t(x + 22, 404, lab, 10, coul=GRIS)
    A.ecrire(d, "Un trou : la file, ou l'état complet",
             f"Trois bandes de rangs. Une coupure courte : les sorties perdues sont encore dans la file de l'actif ; le pont les redemande en tronçons et rattrape en {court(ratt1, 'ms')} puis {court(ratt2, 'ms')}. Une coupure longue de 60 secondes : une partie des sorties perdues est sortie de la file ; le pont reçoit « hors file », demande l'état complet ({court(etat / 1024, 'Kio')}), puis la suite, et rattrape en {court(rl, 'ms')}. La dernière sortie perdue : rien ne suit pour la révéler ; le battement annonce le bout toutes les 500 ms, et elle est redemandée après la grâce de 200 ms.",
             420, "Mesuré sur le banc (réseau local, 200 sorties par seconde, médiane de trois essais) ; dans les trois cas, la mémoire du secours finit identique à celle de l'actif.")

# ================================================================ Le pont de Mosquitto
if voulu("mosquitto"):
    d = Dessin("mosquitto")
    d.titre(16, 24, "Un pont MQTT ordinaire : la file est au courtier")
    d.bloc(30, 56, 230, 190, "gris", "Courtier de l'actif", "Mosquitto", icone="lettre")
    for k in range(9):
        y = 104 + k * 14
        d.brut(f'<rect x="54" y="{y}" width="120" height="10" rx="3" fill="url(#mosquitto-h-ardoise)" opacity="{.95 - k * .05:.2f}"/>')
    d._t("ardoise")
    d.t(186, 118, "file de la", 9.5, coul=GRIS)
    d.t(186, 131, "session du", 9.5, coul=GRIS)
    d.t(186, 144, "pont, en", 9.5, coul=GRIS)
    d.t(186, 157, "mémoire", 9.5, coul=GRIS)
    d.t(54, 238, "max_queued_messages = 1 000", 9.5, True, TONS["gris"][0], mono=True)
    d.fleche("M114 104 C114 80 300 76 330 112", "rouge", 1.6, tirets="4 3")
    d.carte(300, 116, 170, 70, "Au-delà : jetés", ["une ligne au journal du", "courtier ; l'abonné, rien"], "rouge", "croix", sous=9.5)
    d.carte(300, 202, 170, 70, "Redémarrage", ["persistence false : la", "file disparaît"], "rouge", "eclair", sous=9.5)
    d.t(30, 300, "Le secours n'a aucun rang pour voir ce qui manque : le trou passe en silence.", 10.5, True, TONS["rouge"][0])
    d.titre(520, 24, "Messages perdus, coupure de 10 s à 200 /s")
    cas = [("Mosquitto, file de 1 000 (défaut)", r("mosquitto", 1, "file_defaut_perdus"), r("mosquitto", 1, "file_defaut_publies")),
           ("Mosquitto, file de 100 000", r("mosquitto", 1, "grande_file_perdus"), r("mosquitto", 1, "grande_file_publies")),
           ("Mosquitto, 100 000, courtier redémarré", r("mosquitto", 1, "redemarrage_perdus"), r("mosquitto", 1, "redemarrage_publies"))]
    nous = [("Pont maison, coupures de 5 et 10 s", r("pont", 1, "coupures_perdus"), r("pont", 1, "coupures_messages")),
            ("Pont maison, coupure de 60 s", r("pont", 1, "longue_perdus"), r("pont", 1, "longue_messages"))]
    mx = max(v for _, v, _ in cas) * 1.1
    for k, (lab, v, tot) in enumerate(cas + nous):
        y = 46 + k * 52 + (12 if k >= 3 else 0)
        ton = "rouge" if v > 0 else "vert"
        d.t(520, y + 12, lab, 10.5, True, ENCRE)
        d.brut(f'<rect x="520" y="{y + 20}" width="330" height="16" rx="6" fill="#eef0ec"/>')
        if v > 0:
            d.barre(520, y + 20, 330 * v / mx, 16, ton)
            d.t(520 + 330 * v / mx + 8, y + 33, f"{nb(v)} sur {nb(tot)}", 10.5, True, TONS[ton][0])
        else:
            d.icone("coche", 526, y + 21, 14, TONS["vert"][0], 2.4)
            d.t(546, y + 33, f"0 sur {nb(tot)}", 10.5, True, TONS["vert"][0])
    d.brut(f'<line x1="520" y1="{46 + 3 * 52 + 2}" x2="984" y2="{46 + 3 * 52 + 2}" stroke="#d4ddd4"/>')
    A.ecrire(d, "Le pont de Mosquitto, et le pont maison",
             f"À gauche, un pont MQTT ordinaire : pendant une coupure, le courtier de l'actif garde les messages dans la file de session du pont, en mémoire, 1 000 au plus par défaut ; au-delà il les jette en le notant dans son journal, sans que l'abonné le sache ; un redémarrage du courtier efface la file. À droite, les messages perdus pour une coupure de 10 secondes à 200 messages par seconde : {nb(cas[0][1])} avec la file par défaut, 0 avec une file de 100 000, {nb(cas[2][1])} quand le courtier redémarre pendant la coupure. Le pont maison : 0 sous des coupures de 5 et 10 secondes, 0 sous une coupure de 60 secondes.",
             322, "Mosquitto 2 en conteneurs (QoS 1, session persistante, persistance coupée), essai unique ; le pont maison : banc rumqttd, médiane de trois essais. Une grande file ne suffit pas : un redémarrage la vide, et rien ne dit au secours ce qu'il a manqué.")

# ================================================================ Le retour après une journée
if voulu("resync"):
    d = Dessin("resync")
    d.titre(16, 24, "Le secours revient après une journée d'absence (165 000 sorties manquées)")
    liens = [("réseau local", "jour_local"), ("10 Mbit/s, 40 ms", "jour_10mbit"), ("1 Mbit/s, 40 ms", "jour_1mbit")]
    X0, W, VMIN, VMAX = 170, 520, 0.1, 200
    axe_log(d, X0, W, 44, 262, VMIN, VMAX, "secondes, échelle logarithmique", [(0.1, "0,1"), (1, "1"), (10, "10"), (100, "100")])
    for k, (lab, cle) in enumerate(liens):
        y = 56 + k * 70
        v1, v2 = r("rattrapage", 1, cle + "_rattrapage_1") / 1000, r("resync", 1, cle + "_rattrapage_1") / 1000
        d.t(16, y + 30, lab, 11.5, True, ENCRE)
        for j, (v, ton, nom) in enumerate(((v1, "ardoise", "v1 : rejouer le journal"), (v2, "vert", "v2 : l'état complet"))):
            yy = y + j * 26
            w = xlog(v, VMIN, VMAX, X0, W) - X0
            d.barre(X0, yy + 4, w, 18, ton)
            d.t(X0 + w + 8, yy + 18, f"{court(v, 's')}", 11, True, TONS[ton][0])
            if k == 0:
                d.t(X0 + w + 60, yy + 18, nom, 10, coul=TONS[ton][0])
    o1, o2 = r("rattrapage", 1, "jour_1mbit_octets_retour"), r("resync", 1, "jour_1mbit_octets_retour")
    d.carte(730, 52, 254, 112, "Octets sur le lien", [f"v1 : {fr(o1 / 1e6, 1)} Mo de tronçons", f"v2 : {court(o2 / 1e3, 'Ko')}, l'état complet", f"soit {nb(o1 / o2)} fois moins"], "vert", "pile", sous=10.5, pas=17)
    d.carte(730, 178, 254, 92, "Et sur disque", ["v1 : la journée au journal", "v2 : rien ; l'état complet est", "construit en mémoire"], "ardoise", "disque", sous=10.5, pas=15)
    A.ecrire(d, "Revenir après une journée : rejouer, ou recopier l'état",
             f"Un graphique en échelle logarithmique. Le secours revient après une journée d'absence. Sur le réseau local, la v1 rejoue le journal en {court(r('rattrapage', 1, 'jour_local_rattrapage_1') / 1000, 's')}, la v2 recopie l'état complet en {court(r('resync', 1, 'jour_local_rattrapage_1') / 1000, 's')} ; à 10 Mbit/s, {court(r('rattrapage', 1, 'jour_10mbit_rattrapage_1') / 1000, 's')} contre {court(r('resync', 1, 'jour_10mbit_rattrapage_1') / 1000, 's')} ; à 1 Mbit/s, {court(r('rattrapage', 1, 'jour_1mbit_rattrapage_1') / 1000, 's')} contre {court(r('resync', 1, 'jour_1mbit_rattrapage_1') / 1000, 's')}. Sur le lien, {fr(o1 / 1e6, 1)} Mo contre {court(o2 / 1e3, 'Ko')}.",
             290, "Mesuré sur le banc, médiane de trois essais. En v2, l'essentiel du temps local est la détection : un battement (500 ms) puis la grâce (200 ms). La mémoire du secours finit identique dans les six cas.")

# ================================================================ Le pont, mesuré
if voulu("mesures"):
    d = Dessin("mesures")
    d.titre(16, 24, "Délai de la copie")
    lat = [("local, 200 /s", r("pont", 1, "regime_lat_p50"), r("pont", 1, "regime_lat_p99"), "vert"),
           ("distant, lien qui bat", r("distant", 1, "flappant_lat_p50"), r("distant", 1, "flappant_lat_p99"), "violet")]
    mx = max(b for _, _, b, _ in lat) * 1.25
    for k, (lab, a, b, ton) in enumerate(lat):
        y = 44 + k * 66
        d.t(16, y + 12, lab, 11, True, ENCRE)
        d.barre(16, y + 20, 290 * b / mx, 14, ton, op=".35")
        d.barre(16, y + 20, max(4, 290 * a / mx), 14, ton)
        d.t(16, y + 50, f"médiane {court(a, 'ms')} · 99e centile {court(b, 'ms')}", 10, coul=TONS[ton][0])
    d.titre(350, 24, "Rattrapage après une coupure")
    rat = [("5 s, par la file", r("pont", 1, "coupures_rattrapage_1"), "vert"), ("10 s, par la file", r("pont", 1, "coupures_rattrapage_2"), "vert"),
           ("60 s, par l'état complet", r("pont", 1, "longue_rattrapage_1"), "ambre"), ("3 s, lien distant qui bat", r("distant", 1, "flappant_rattrapage_max"), "violet")]
    mx = max(v for _, v, _ in rat) * 1.3
    for k, (lab, v, ton) in enumerate(rat):
        y = 44 + k * 40
        d.t(350, y + 12, lab, 10.5, True, ENCRE)
        d.barre(350, y + 18, 230 * v / mx, 14, ton)
        d.t(350 + 230 * v / mx + 8, y + 30, court(v, "ms"), 10.5, True, TONS[ton][0])
    d.titre(680, 24, "Messages perdus")
    pertes = [("pont de rumqttd, 2 coupures", r("pont", 1, "rumqttd_perdus"), r("pont", 1, "rumqttd_messages"), "rouge"),
              ("pont maison, 2 coupures", r("pont", 1, "coupures_perdus"), r("pont", 1, "coupures_messages"), "vert"),
              ("pont maison, 60 s coupé", r("pont", 1, "longue_perdus"), r("pont", 1, "longue_messages"), "vert"),
              ("pont maison, lien qui bat", r("distant", 1, "flappant_perdus"), r("distant", 1, "flappant_messages"), "vert")]
    for k, (lab, v, tot, ton) in enumerate(pertes):
        y = 44 + k * 40
        d.t(680, y + 12, lab, 10.5, True, ENCRE)
        d.brut(f'<rect x="680" y="{y + 18}" width="200" height="14" rx="6" fill="#eef0ec"/>')
        if v > 0:
            d.barre(680, y + 18, 200 * v / tot, 14, ton)
        d.t(888, y + 30, f"{nb(v)} sur {nb(tot)}", 10.5, True, TONS[ton][0])
    ex = r("pont", 1, "coupures_executions")
    d.brut(f'<rect x="16" y="214" width="968" height="40" rx="12" fill="{TONS["vert"][2]}" stroke="{TONS["vert"][1]}" stroke-opacity=".5"/>')
    d._t("vert")
    d.icone("coche", 28, 224, 20, TONS["vert"][0], 2.4)
    d.t(56, 238, f"Dans tous les cas du pont maison, la mémoire du secours finit identique à celle de l'actif, exécutions comprises ({nb(ex)} sur 8 000 sorties).", 11, True, ENCRE)
    A.ecrire(d, "Le pont maison, mesuré",
             f"Trois graphiques. Le délai de la copie : sur le réseau local à 200 messages par seconde, médiane {court(lat[0][1], 'ms')} et 99e centile {court(lat[0][2], 'ms')} ; sur le lien distant qui bat, {court(lat[1][1], 'ms')} et {court(lat[1][2], 'ms')}. Le rattrapage après une coupure : {court(rat[0][1], 'ms')} après 5 secondes, {court(rat[1][1], 'ms')} après 10 secondes, {court(rat[2][1], 'ms')} après 60 secondes par l'état complet, {court(rat[3][1], 'ms')} au plus sur le lien distant. Les messages perdus : {nb(pertes[0][1])} sur {nb(pertes[0][2])} pour le pont de rumqttd, aucun pour le pont maison.",
             266, "Deux courtiers rumqttd 0.20 et un relais qui imite le lien, en Linux musl (2 CPU, 128 Mio), médiane de trois essais ; le 99e centile mesure surtout la cohabitation des deux courtiers sur 2 CPU.")

# ================================================================ Les écritures sur disque
if voulu("disque"):
    d = Dessin("disque")
    n_jour = r("rattrapage", 1, "jour_local_messages")
    inst = r("instantane", 1, "octets")
    json_ = r("instantane", 1, "octets_json_valeurs_aujourdhui")
    lignes = [("v1 : journal de réplication", n_jour, n_jour * 4096, "rouge", "validé toutes les 100 ms", "v1 : journal"),
              ("aujourd'hui : state.json", 1440 * 2, 1440 * json_, "ambre", "réécrit chaque minute", "aujourd'hui"),
              ("v2 : instantané, toutes les 15 min", 96 * 2, 96 * inst, "ardoise", "sans secours vivant", "v2 : 15 min"),
              ("v2 : instantané, toutes les heures", 24 * 2, 24 * inst, "vert", "avec un secours", "v2 : 1 h")]
    d.titre(16, 24, "fsync par jour")
    d.titre(516, 24, "Octets écrits par jour")
    X0, W = 236, 250
    axe_log(d, X0, W, 40, 236, 10, 1e6, "échelle logarithmique", [(10, "10"), (1e3, "1 000"), (1e5, "100 000")])
    axe_log(d, 516 + 110, 290, 40, 236, 1e6, 1e10, "échelle logarithmique", [(1e6, "1 Mo"), (1e7, "10 Mo"), (1e8, "100 Mo"), (1e9, "1 Go"), (1e10, "10 Go")])
    for k, (lab, fs, oc, ton, note, court_) in enumerate(lignes):
        y = 52 + k * 46
        d.t(16, y + 13, lab, 10.5, True, ENCRE)
        d.t(16, y + 28, note, 9.5, coul=GRIS)
        w = xlog(fs, 10, 1e6, X0, W) - X0
        d.barre(X0, y + 6, w, 18, ton)
        d.t(X0 + w + 6, y + 20, nb(fs), 10.5, True, TONS[ton][0])
        d.t(516, y + 20, court_, 10.5, True, ENCRE)
        w = xlog(oc, 1e6, 1e10, 626, 290) - 626
        d.barre(626, y + 6, w, 18, ton)
        lib = f"{fr(oc / 1e9, 2)} Go" if oc >= 1e9 else f"{court(oc / 1e6, 'Mo')}"
        d.t(626 + w + 6, y + 20, ("≥ " if k == 0 else "") + lib, 10.5, True, TONS[ton][0])
    d.t(16, 290, f"v1 : une validation par lot de 100 ms où passe au moins une sortie, soit jusqu'à {nb(n_jour)} par jour à la cadence de la maison (864 000 au plafond) ;", 10, coul=GRIS)
    d.t(16, 305, f"chaque fsync écrit au moins un bloc de 4 Kio. Aujourd'hui : state.json ({court(json_ / 1024, 'Kio')} à ×1) réécrit chaque minute avec fsync du fichier et du dossier.", 10, coul=GRIS)
    d.t(16, 320, f"v2 : l'instantané ({court(inst / 1024, 'Kio')}), écrit de la même façon ; la copie elle-même n'écrit rien.", 10, coul=GRIS)
    A.ecrire(d, "Les écritures sur disque, par jour",
             f"Deux graphiques en échelle logarithmique. Les fsync par jour : jusqu'à {nb(n_jour)} pour le journal de la v1, {nb(2880)} pour state.json aujourd'hui, 192 pour un instantané toutes les 15 minutes, 48 pour un instantané par heure. Les octets écrits par jour : au moins {fr(n_jour * 4096 / 1e9, 2)} Go pour la v1, {court(1440 * json_ / 1e6, 'Mo')} pour state.json, {court(96 * inst / 1e6, 'Mo')} et {court(24 * inst / 1e6, 'Mo')} pour les instantanés.",
             334, "Calculé à partir des tailles mesurées (instantané et state.json à ×1) et du nombre de sorties d'une journée du banc. La copie de la v2 n'écrit rien ; l'historique, qui existe déjà, n'est pas compté.")

# ================================================================ Que perd-on sans journal ?
if voulu("pannes"):
    d = Dessin("pannes")

    def machines(x, y, etats):
        for k, (nom, e) in enumerate(etats):
            ton = {"ok": "vert", "ko": "rouge", "zz": "gris", "": "gris"}[e]
            f, c, p_ = d._t(ton)
            xx = x + k * 50
            if e == "":
                d.brut(f'<rect x="{xx}" y="{y}" width="40" height="30" rx="7" fill="none" stroke="{c}" stroke-dasharray="3 3"/>')
                d.t(xx + 20, y + 19, nom, 10, True, c, "middle")
                continue
            d.brut(f'<rect x="{xx}" y="{y + 3}" width="40" height="30" rx="7" fill="{mele_(f)}"/><rect x="{xx}" y="{y}" width="40" height="30" rx="7" fill="url(#pannes-h-{ton})"/>')
            d.t(xx + 20, y + 19, nom, 10.5, True, "#ffffff", "middle")
            if e == "ko":
                d.icone("croix", xx + 26, y - 8, 14, TONS["rouge"][0], 2.6)
            if e == "zz":
                d.icone("eclair", xx + 26, y - 8, 14, TONS["ambre"][0], 2.2)

    def mele_(c):
        from relief import mele
        return mele(c, "#000000", .3)

    lag = r("distant", 1, "cadence_maison_lat_p50")
    p99 = r("pont", 1, "regime_lat_p99")
    jr = r("resync", 1, "jour_1mbit_rattrapage_1") / 1000
    cas = [("L'actif tombe", [("H1", "ko"), ("H2", "ok"), ("D", "ok")], "la mémoire : chez H2, à jour", f"perdu : ce qui était en vol ({court(p99, 'ms')} au 99e centile)", "vert", "≈ 0"),
           ("Les deux machines locales tombent", [("H1", "ko"), ("H2", "ko"), ("D", "ok")], "la mémoire : chez le distant", f"perdu : son retard, {court(lag, 'ms')} en médiane", "vert", "≈ 0"),
           ("Coupure de courant, avec onduleur", [("H1", "zz"), ("H2", "zz"), ("D", "")], "l'arrêt figé, au signal de l'onduleur", "perdu : rien", "vert", "rien"),
           ("Coupure franche, sans onduleur ni distant", [("H1", "ko"), ("H2", "ko"), ("D", "")], "la mémoire : le dernier instantané", "perdu : au plus sa période d'exécutions ; les valeurs, les appareils les redonnent", "ambre", "borné"),
           ("Le lien du distant tombe une journée", [("H1", "ok"), ("H2", "ok"), ("D", "zz")], "le distant garde sa mémoire, figée", f"perdu : rien ; au retour, l'état complet ({court(jr, 's')} à 1\u00a0Mbit/s)", "vert", "rien"),
           ("Un actif redémarre vide", [("H1", "zz"), ("H2", "ok"), ("D", "ok")], "le piège de Redis : ses copies se videraient", "interdit : il revient en secours et prend l'état du plus avancé", "rouge", "règle")]
    for k, (titre, ms, l1, l2, ton, badge) in enumerate(cas):
        x, y = 16 + (k % 3) * 328, 16 + (k // 3) * 186
        f, c, p_ = d._t(ton)
        d.brut(f'<rect x="{x}" y="{y}" width="312" height="172" rx="14" fill="url(#pannes-f-{ton})" stroke="{c}" stroke-opacity=".55" filter="url(#pannes-o)"/>')
        d.t(x + 16, y + 26, titre, 12, True, ENCRE)
        machines(x + 16, y + 46, ms)
        d.pilule(x + 296, y + 50, badge, ton, True, 10.5, "end")
        from relief import couper
        yy = y + 106
        for l in couper(l1, 48):
            d.t(x + 16, yy, l, 10.5, True, f)
            yy += 15
        for l in couper(l2, 52):
            d.t(x + 16, yy, l, 10, coul=GRIS)
            yy += 14
    A.ecrire(d, "Que perd-on, sans journal sur disque ?",
             f"Six cas. L'actif tombe : la mémoire est chez H2, à jour ; perdu, ce qui était en vol, {court(p99, 'ms')} au 99e centile. Les deux machines locales tombent : la mémoire est chez le distant ; perdu, son retard, {court(lag, 'ms')} en médiane à la cadence de la maison. Coupure de courant avec onduleur : l'arrêt figé au signal de l'onduleur ; rien de perdu. Coupure franche sans onduleur ni distant : le dernier instantané ; au plus sa période d'exécutions et d'attentes, les valeurs revenant des appareils. Le lien du distant tombe une journée : rien de perdu, l'état complet au retour en {court(jr, 's')} à 1 Mbit/s. Un actif qui redémarre vide : interdit, il revient en secours.",
             374, "Sans journal, la durabilité vient des autres machines. Le seul cas qui coûte est la panne franche de tout, sans onduleur ni distant : l'instantané en borne la perte.")

# ================================================================ Copier par les sorties, pas par les entrées
if voulu("copie"):
    d = Dessin("copie")
    att = r("copie", 1, "attentes")
    ta, tb = r("copie", 1, "a_attentes_tenues"), r("copie", 1, "b_attentes_tenues")
    d.titre(16, 24, "Une heure de la maison, pont coupé 30 s, horloges à 23 ms")
    for x, ton, titre, sous, ic in ((300, "vert", "A · par les sorties", "le secours recopie val/ (rang s)", "coche"), (646, "rouge", "B · par les entrées", "le secours retraite in/ lui-même", "croix")):
        f, c, p_ = d._t(ton)
        d.brut(f'<rect x="{x}" y="38" width="338" height="262" rx="14" fill="url(#copie-f-{ton})" stroke="{c}" stroke-opacity=".55" filter="url(#copie-o)"/>')
        d.brut(f'<path d="M{x} 80 V52 a14 14 0 0 1 14 -14 H{x + 324} a14 14 0 0 1 14 14 V80 z" fill="url(#copie-h-{ton})"/>')
        d.icone(ic, x + 14, 49, 18)
        d.t(x + 40, 59, titre, 13, True, "#ffffff")
        d.t(x + 40, 73, sous, 9.5, coul="#ffffff", op=".9")
    rows = [("Messages perdus pendant la coupure", f"{nb(r('copie', 1, 'a_perdues_au_pont'))}, vus au rang s, comblés", f"{nb(r('copie', 1, 'b_perdues'))}, que rien ne signale"),
            ("Fiches dont la valeur diffère", "0", nb(r("copie", 1, "b_valeur_differente"))),
            ("Fiches dont « changé » diffère", "0", f"{nb(r('copie', 1, 'b_change_different'))} (écart médian {nb(r('copie', 1, 'b_ecart_change_ms_median'))} ms)"),
            ("Mémoire identique (empreinte)", "oui", "non")]
    for k, (lib, a, b) in enumerate(rows):
        y = 104 + k * 30
        d.brut(f'<line x1="16" y1="{y + 9}" x2="984" y2="{y + 9}" stroke="#e3e8e1"/>')
        d.t(16, y, lib, 11, True, ENCRE)
        d.t(316, y, a, 11, True, TONS["vert"][0])
        d.t(662, y, b, 11, True, TONS["rouge"][0])
    y = 236
    d.t(16, y + 4, "Attentes « depuis 10 min » armées", 11, True, ENCRE)
    d.t(16, y + 19, "sur l'actif, qui se revérifient", 11, True, ENCRE)
    d.t(16, y + 34, "sur le secours après une bascule", 11, True, ENCRE)
    for x, v, ton in ((316, ta, "vert"), (662, tb, "rouge")):
        d.brut(f'<rect x="{x}" y="{y - 8}" width="300" height="20" rx="7" fill="#eef0ec"/>')
        if v > 0:
            d.barre(x, y - 8, 300 * v / att, 20, ton)
        d.t(x, y + 32, f"{nb(v)} sur {nb(att)}", 13, True, TONS[ton][0])
    A.ecrire(d, "Copier la mémoire par les sorties, pas par les entrées",
             "Comparaison sur une heure de la maison. Par les sorties de l'actif, la mémoire du secours est identique et toutes les attentes se revérifient. Par les entrées, les valeurs perdues pendant la coupure restent fausses, les instants « changé » sont ceux du secours, et aucune attente ne se revérifie.",
             312, "Le hub est la seule horloge de la maison. Un secours qui date les valeurs lui-même a d'autres instants « changé » : les attentes armées sur l'actif ne se revérifient plus après la bascule (mémoire v2 § 8.5).")

# ================================================================ Les familles d'intégrations
if voulu("familles"):
    d = Dessin("familles")
    d.titre(16, 24, "Ce que fait le secours, intégration par intégration")
    qs = [(44, "Le secret change-t-il à chaque renouvellement ?", "A · un seul renouveleur ; copie scellée au secours", "Netatmo", "vert", "cle"),
          (102, "Une session par identifiant et mot de passe ?", "B · connexion après la bascule, jamais avant", "Ariston, Voltalis, Landroid, Overkiz cloud, Reolink, Twinkly, routeurs", "vert", "personne"),
          (160, "Une clé fixe, un appairage, une empreinte ?", "C · copier le coffre ; sinon, réappairage humain", "Hue, Samsung, Android TV, Cast, Philips, Nuki, ESPHome, Tuya…", "vert", "cadenas"),
          (246, "Un identifiant MQTT fixe ?", "D · rien ne démarre en mode passif", "Bambu, Frigate, Zigbee2MQTT", "ambre", "lettre"),
          (304, "Des effets visibles (envois, appuis) ?", "E · un seul actif, jamais deux", "Telegram, notifications, récap de 21 h, Toggle", "ambre", "telephone"),
          (362, "Liée à l'adresse de la machine ?", "F · adresse virtuelle, ou réglage par machine", "Sonos, ESPHome, Cast, Netatmo, présence", "ambre", "maison")]
    for k, (y, q, rep_, ex, ton, ic) in enumerate(qs):
        d._t("ambre")
        d.brut(f'<rect x="16" y="{y}" width="370" height="44" rx="22" fill="#ffffff" stroke="{TONS["ambre"][1]}" stroke-width="1.5" filter="url(#familles-o)"/>')
        d.t(201, y + 27, q, 11, True, ENCRE, "middle")
        d.fleche(f"M388 {y + 22} L440 {y + 22}", ton, 1.8)
        d.etiquette(414, y + 15, "oui", ton, taille=10)
        d.carte(444, y - 2, 540, 50, rep_, [], ton, ic, taille=11.5)
        d.t(488, y + 41, ex, 9.5, coul=GRIS)
        if k in (0, 1):
            d.fleche(f"M201 {y + 44} L201 {y + 56}", "gris", 1.4)
            d.etiquette(210, y + 54, "non", "gris", "start", 9.5)
    d.t(16, 232, "Puis, pour toutes, trois questions de plus :", 11, True, GRIS)
    d.brut(f'<rect x="16" y="420" width="968" height="32" rx="10" fill="#eef0ec"/>')
    d.t(30, 441, "G · sinon, rien à faire : météo, Open-Meteo, Tempo, Wyoming, IGD, IPP, profils sans secret.", 10.5, coul=ENCRE)
    A.ecrire(d, "Les intégrations : que fait le secours ?",
             "Un arbre de questions. Le secret change-t-il à chaque renouvellement ? Oui : un seul renouveleur (Netatmo). Sinon, une session par identifiant ? Oui : connexion après la bascule. Sinon, une clé fixe ou un appairage ? Oui : copier le coffre. Puis trois questions pour toutes : identifiant MQTT fixe, effets visibles, lien à l'adresse de la machine. Sinon, rien à faire.",
             464, "Une seule instance active par intégration, toujours ; le secours est prêt (secrets, matériel de session), sans aucun appel avant la bascule.")

# ================================================================ Les jetons qui tournent
if voulu("jetons"):
    d = Dessin("jetons")
    for x0, titre, ton in ((16, "Deux renouveleurs : révoqués", "rouge"), (516, "Un renouveleur, copie en direct", "vert")):
        d.titre(x0, 24, titre, TONS[ton][0])
        for k, (lab, t2) in enumerate((("Actif", "vert"), ("Fournisseur", "gris"), ("Secours", "bleu"))):
            x = x0 + 60 + k * 170
            d.pilule(x, 38, lab, t2, True, 10.5, "middle")
            d.brut(f'<line x1="{x}" y1="58" x2="{x}" y2="292" stroke="#d4ddd4" stroke-dasharray="3 3"/>')

    def msg(xa, xb, y, lab, ton, gras=False):
        d.fleche(f"M{xa} {y} L{xb + (-5 if xb > xa else 5)} {y}", ton, 1.6)
        d.etiquette((xa + xb) / 2, y - 6, lab, ton, taille=10, gras=gras)
    A_, P_, S_ = 76, 246, 416
    msg(A_, P_, 90, "renouveler (jeton 1)", "gris")
    msg(P_, A_, 114, "jeton 2 ; le 1 est invalidé", "vert")
    msg(S_, P_, 154, "renouveler (jeton 1)", "gris")
    msg(P_, S_, 178, "déjà utilisé : révocation", "rouge", True)
    msg(A_, P_, 218, "renouveler (jeton 2)", "gris")
    msg(P_, A_, 242, "révoqué aussi", "rouge", True)
    d.t(16, 312, f"{nb(r('jetons', 1, 'a_deux_renouveleurs_revoques'))} fois sur 1 000, en 6 h au plus.", 11.5, True, TONS["rouge"][0])
    A_, P_, S_ = 576, 746, 916
    msg(A_, P_, 90, "renouveler (jeton 1)", "gris")
    msg(P_, A_, 114, "jeton 2", "vert")
    d.pilule(A_, 124, "coffre, fsync", "ambre", True, 9.5, "middle", "cadenas")
    d.fleche(f"M{A_} 152 C{A_ + 120} 152 {S_ - 120} 168 {S_ - 5} 168", "ambre", 1.8, anime=True)
    d.etiquette((A_ + S_) / 2, 152, "repl/coffre/secours, rang s, scellé", "ambre", taille=10)
    d.pilule(S_, 196, "bascule", "bleu", True, 9.5, "middle")
    msg(S_, P_, 232, "renouveler (jeton 2)", "gris")
    msg(P_, S_, 256, "jeton 3 : la maison continue", "vert", True)
    d.t(516, 312, f"{nb(r('jetons', 1, 'b_local_echecs'))} échec sur 10 000 bascules (secours local).", 11.5, True, TONS["vert"][0])
    A.ecrire(d, "Un jeton qui tourne, deux machines",
             "À gauche, deux machines renouvellent chacune le même jeton : le fournisseur voit revenir un jeton déjà utilisé et révoque tout. À droite, l'actif seul renouvelle, range le nouveau jeton au coffre, le publie scellé au secours dans le rang s, et le secours le présente après la bascule.",
             326, "RFC 9700 § 4.14.2 : un jeton déjà utilisé qui revient fait révoquer le jeton actif. Netatmo fait tourner ses jetons. Simulation sur 30 jours, un renouvellement toutes les 3 h.")

# ================================================================ Le coffre répliqué
if voulu("coffre"):
    d = Dessin("coffre")
    d.titre(16, 24, "Un secret écrit sur l'actif, et où il va")
    d.carte(16, 40, 300, 196, "Coffre de l'actif", ["clé maître M ; écrit d'abord ici (fsync)"], "vert", "cadenas")
    for k, (sec, dom, ton) in enumerate((("netatmo/oauth_tokens · v12", "cloud", "violet"), ("hue/app_key", "maison", "vert"))):
        y = 108 + k * 60
        f, c, p_ = d._t(ton)
        d.brut(f'<rect x="30" y="{y}" width="272" height="48" rx="9" fill="#ffffff" stroke="{c}" stroke-width="1.3"/>')
        d.t(42, y + 20, sec, 10.5, True, ENCRE, mono=True)
        d.pilule(42, y + 27, f"domaine {dom}", ton, False, 9)
    d.brut(f'<rect x="372" y="40" width="252" height="196" rx="16" fill="#ffffff" fill-opacity=".7" stroke="{TONS["ambre"][1]}" stroke-dasharray="6 4"/>')
    d._t("ambre")
    d.icone("lettre", 386, 52, 18, TONS["ambre"][0])
    d.t(410, 66, "MQTT, rang s", 12, True, TONS["ambre"][0])
    for k, l in enumerate(("repl/coffre/<secours> :", "chaque secret, scellé pour", "chaque secours qui y a droit", "", "ChaCha20-Poly1305", f"≈ {fr(r('jetons', 1, 'sceller_2kio'), 1)} µs pour 2 Kio")):
        d.t(386, 94 + k * 17, l, 10.5 if k < 4 else 10, k == 0, ENCRE if k < 4 else GRIS, mono=k in (0, 4))
    d.carte(684, 40, 300, 92, "Secours local", ["reçoit tous les secrets", "les applique en mémoire, puis sur son disque", "clé maître M, installée à l'appairage"], "bleu", "cadenas", pas=14)
    d.carte(684, 144, 300, 92, "Secours distant", ["ne reçoit que les secrets cloud,", "scellés avec sa clé de liaison ;", "son coffre, sa propre clé maître"], "violet", "cadenas", pas=14)
    d.fleche("M316 132 L368 132", "violet", 1.6)
    d.fleche("M316 192 L368 192", "vert", 1.6)
    d.fleche("M624 96 L680 86", "bleu", 1.8, anime=True)
    d.etiquette(652, 80, "les deux", "bleu", taille=9.5)
    d.fleche("M624 170 L680 190", "violet", 1.8, anime=True)
    d.etiquette(650, 196, "cloud", "violet", taille=9.5)
    d.t(16, 262, "Un secret « maison » ne quitte jamais la maison ; la clé maître de la maison non plus. Un secret « mémoire seule » passe en direct, jamais sur un disque.", 10.5, coul=GRIS)
    d.t(16, 280, "Ordre : coffre de l'actif (fsync) → message scellé, rang s → usage par le pilote. Un trou se comble comme le reste : la file, sinon l'état complet.", 10.5, coul=GRIS)
    A.ecrire(d, "Le coffre répliqué, scellé par destinataire",
             "À gauche, le coffre de l'actif avec deux secrets : les jetons Netatmo, du domaine cloud, et la clé Hue, du domaine maison. Au milieu, MQTT dans le rang s : chaque secret est scellé pour chaque secours qui y a droit. À droite, le secours local reçoit les deux et partage la clé maître ; le secours distant ne reçoit que le secret cloud, scellé avec sa propre clé.",
             294, "Décidé le 23 octobre : les jetons et les sessions vont au coffre, et en direct au secours ; le distant reçoit les secrets cloud, chiffrés.")

# ================================================================ Le secours distant
if voulu("distant"):
    d = Dessin("distant")
    d.zone(16, 30, 380, 266, "La maison", "vert", "maison")
    d.carte(32, 66, 170, 60, "H1 · actif", ["époques maison, cloud"], "vert", "eclair", taille=11.5, sous=9.5)
    d.carte(212, 66, 170, 60, "H2 · secours local", ["peut tout reprendre"], "bleu", "coche", taille=11.5, sous=9.5)
    d.carte(32, 140, 170, 60, "Témoin", ["NAS, routeur ou Pi"], "gris", "oeil", taille=11.5, sous=9.5)
    d.carte(212, 140, 170, 60, "Boîtiers radio A, B", ["Zigbee, Thread"], "gris", "radio", taille=11.5, sous=9.5)
    d.fleche("M202 96 L210 96", "ambre", 2)
    for k, l in enumerate(("Le témoin départage H1 et H2 ; il n'arbitre pas", "le distant : sans Internet, la maison doit vivre.")):
        d.t(32, 228 + 15 * k, l, 10, coul=GRIS)
    d.fleche("M398 84 L690 84", "ambre", 2.4, anime=True)
    d.etiquette(544, 76, "le pont maison, par NetBird", "ambre", taille=10.5)
    d.fleche("M398 112 L690 112", "violet", 1.4, tirets="5 4")
    d.etiquette(544, 106, "battement direct en HTTPS : la seconde veille", "violet", taille=10)
    d.brut(f'<rect x="414" y="128" width="262" height="140" rx="16" fill="#ffffff" stroke="{TONS["violet"][1]}" stroke-dasharray="5 4"/>')
    d._t("violet")
    d.icone("bouclier", 428, 140, 18, TONS["violet"][0])
    d.t(452, 154, "NetBird (WireGuard maillé)", 11.5, True, TONS["violet"][0])
    for k, l in enumerate(("pair à pair : ICE, STUN", "sinon un relais : QUIC, ou WebSocket", "sur le port 443 si l'UDP est bloqué", "chiffré de bout en bout", "la file et l'état complet : en mémoire")):
        d.t(430, 178 + 17 * k, l, 10.5, coul=GRIS if k < 4 else TONS["ambre"][0], gras=k == 4)
    d.carte(700, 40, 284, 130, "Secours distant", ["copie : mémoire, historique", "alerte : par son propre chemin", "cloud : sur geste humain (époque cloud)", "peut héberger l'archive (D13)", "jamais les radios ni le réseau local"], "violet", "nuage", pas=14.5)
    d.carte(700, 196, 284, 72, "Clouds : Netatmo, Voltalis, Ariston…", ["un seul porteur de l'époque cloud", "les appelle"], "violet", "nuage", taille=11)
    d.fleche("M842 170 L842 192", "violet", 1.6)
    d.t(16, 318, "Secrets : le distant ne reçoit que ceux du domaine cloud, scellés avec sa clé ; la clé maître de la maison ne sort jamais.", 10.5, coul=GRIS)
    A.ecrire(d, "Le secours distant, par NetBird",
             "À gauche, la maison : l'actif, le secours local, le témoin et les boîtiers radio. Le pont maison part vers le secours distant par NetBird, pair à pair ou par un relais, en QUIC ou en WebSocket sur le port 443 ; un battement HTTPS direct sert de seconde veille ; la file et l'état complet restent en mémoire. À droite, le secours distant : copie, alerte, cloud sur geste humain, archive.",
             330, "Le même pont qu'en local, sur l'adresse NetBird de l'actif. Le WebSocket, NetBird le fait déjà, en dessous, quand l'UDP est bloqué.")

# ================================================================ La bascule
if voulu("bascule"):
    d = Dessin("bascule")
    d.titre(16, 24, "Une bascule de H1 vers H2, et ce qui est incertain")
    X0, W = 70, 900

    def TB(s_):
        return X0 + W * s_ / 8.0
    for s_ in range(0, 9):
        d.brut(f'<line x1="{TB(s_):.1f}" y1="44" x2="{TB(s_):.1f}" y2="196" stroke="#e1e6df"/>')
        d.t(TB(s_), 212, f"{s_} s", 10, coul=GRIS, ancre="middle")
    d.t(16, 66, "H1", 12, True, TONS["vert"][0])
    d._t("vert"); d._t("rouge"); d._t("ambre"); d._t("bleu")
    for s_ in (0, 1, 2):
        d.brut(f'<circle cx="{TB(s_):.1f}" cy="62" r="5" fill="url(#bascule-h-vert)"/>')
    d.t(TB(0) + 10, 84, "bail renouvelé chaque seconde", 10, coul=GRIS)
    d.barre(TB(2.3), 54, TB(4.3) - TB(2.3), 16, "rouge")
    d.t(TB(2.3), 49, "gel de H1", 10, True, TONS["rouge"][0])
    d.t(16, 118, "Témoin", 12, True, GRIS)
    d.barre(TB(2), 106, TB(5) - TB(2), 16, "ambre")
    d.t(TB(2) + 8, 118, "le bail N court encore (3 s)", 10, True, "#ffffff")
    d.t(16, 168, "H2", 12, True, TONS["bleu"][0])
    d.brut(f'<rect x="{TB(2.3):.1f}" y="152" width="{TB(5) - TB(2.3):.1f}" height="24" rx="7" fill="{TONS["rouge"][2]}" stroke="{TONS["rouge"][1]}" stroke-dasharray="4 3"/>')
    d.t(TB(2.3) + 8, 168, "incertain : « peut-être déjà parti », jamais rejoué", 10, True, TONS["rouge"][0])
    d.barre(TB(5), 152, TB(8) - TB(5), 24, "vert")
    d.t(TB(5) + 8, 168, "époque N + 1 : H2 agit ; rattrapage normal (2 min)", 10, True, "#ffffff")
    d.fleche(f"M{TB(3.3):.1f} 126 L{TB(3.3):.1f} 148", "gris", 1.3)
    d.etiquette(TB(3.3) + 6, 142, "silence ≥ 1 s : H2 demande N + 1", "gris", "start", 9.5, False)
    d.t(16, 238, "Domaine maison : le secours local seulement. Domaine cloud : le secours local avec la maison, ou le distant sur geste humain.", 10.5, coul=GRIS)
    d.t(16, 256, "Chaque ordre porte l'époque de qui l'émet : un module refuse une époque plus ancienne.", 10.5, coul=GRIS)
    A.ecrire(d, "La bascule et la fenêtre d'incertitude",
             "Une frise de 8 secondes. H1 renouvelle son bail chaque seconde puis gèle ; le témoin laisse courir le bail N trois secondes ; H2 demande l'époque suivante après une seconde de silence et la reçoit à la fin du bail. Entre la dernière sortie reçue et la fin du bail, ce qui était dû est incertain ; après, H2 agit et rattrape normalement.",
             268, "La fenêtre d'incertitude dure le bail : quelques secondes. Ce qui y tombe n'est ni rejoué ni perdu en silence.")

# ================================================================ L'époque sur chaque ordre
if voulu("cloture"):
    d = Dessin("cloture")
    d.titre(16, 22, "Un ordre d'une époque ancienne est refusé par le module lui-même")
    X = {"h1": 120, "t": 380, "h2": 640, "m": 890}
    for k, (lab, ton) in (("h1", ("H1 (époque N)", "vert")), ("t", ("Témoin", "gris")), ("h2", ("H2", "bleu")), ("m", ("Module", "ardoise"))):
        d.pilule(X[k], 36, lab, ton, True, 10.5, "middle")
        d.brut(f'<line x1="{X[k]}" y1="56" x2="{X[k]}" y2="300" stroke="#d4ddd4" stroke-dasharray="3 3"/>')

    def msg(y, a, b, lab, ton="gris", gras=False):
        d.fleche(f"M{X[a]} {y} L{X[b] + (-5 if X[b] > X[a] else 5)} {y}", ton, 1.6)
        d.etiquette((X[a] + X[b]) / 2, y - 6, lab, ton, taille=10, gras=gras)
    msg(86, "h1", "t", "renouveler le bail (N)")
    d._t("rouge")
    d.brut(f'<rect x="{X["h1"] - 8}" y="100" width="16" height="112" rx="6" fill="{TONS["rouge"][2]}" stroke="{TONS["rouge"][1]}"/>')
    d.t(X["h1"] + 14, 160, "gelé", 10, True, TONS["rouge"][0])
    msg(126, "h2", "t", "demander N + 1", "ambre")
    msg(148, "t", "h2", "accordée : le bail N a expiré", "vert", True)
    msg(178, "h2", "m", "ordre, époque N + 1", "vert", True)
    d.t(X["m"] - 8, 200, "plus grande époque vue : N + 1", 9.5, coul=GRIS, ancre="end")
    msg(232, "h1", "m", "ordre, époque N (réveillé)")
    msg(254, "m", "h1", "refusé : époque ancienne", "rouge", True)
    msg(282, "h1", "t", "renouveler (N) → refusé : H1 redevient secours")
    A.ecrire(d, "L'époque sur chaque ordre",
             "Diagramme de séquence. H1, époque N, renouvelle son bail puis gèle. H2 demande l'époque N + 1, que le témoin accorde à l'expiration du bail. H2 envoie un ordre d'époque N + 1 au module. H1 se réveille et envoie un ordre d'époque N : le module le refuse. H1 apprend qu'il a perdu le bail et redevient secours.",
             312, "Le jeton de clôture des systèmes distribués : un module garde la plus grande époque vue et refuse tout ordre plus ancien.")

# ================================================================ L'arrêt figé
if voulu("arret"):
    d = Dessin("arret")
    d.titre(16, 24, "L'arrêt figé : budget visé 2 s, sous les 10 s de Docker")
    etapes = [("1", "annonce", "sys/arret ; plus d'ordre accepté", 0.0, 0.05, "vert"), ("2", "ordres en vol", "1 s au plus, sinon « incertain »", 0.05, 1.05, "ambre"),
              ("3", "pilotes", "arrêt, déconnexion", 1.05, 1.35, "vert"), ("4", "figer", "la mémoire au rang s = N", 1.35, 1.4, "vert"),
              ("5", "instantané", "écrit, fsync : le seul disque", 1.4, 1.55, "ardoise"), ("6", "relève", "si un secours est à jour : même empreinte, E + 1", 1.55, 1.8, "bleu"),
              ("7", "fin", "le processus s'arrête", 1.8, 1.85, "vert")]
    X0, W, Y0, PAS = 480, 490, 46, 30

    def TX(s_):
        return X0 + W * s_ / 2.0
    for s_ in (0, 0.5, 1, 1.5, 2):
        d.brut(f'<line x1="{TX(s_):.1f}" y1="{Y0 - 4}" x2="{TX(s_):.1f}" y2="{Y0 + PAS * len(etapes)}" stroke="#e1e6df"/>')
        d.t(TX(s_), Y0 + PAS * len(etapes) + 16, f"{fr(s_, 1)} s", 10, coul=GRIS, ancre="middle")
    for k, (num, lab, note, a_, b_, ton) in enumerate(etapes):
        y = Y0 + k * PAS
        f, c, p_ = d._t(ton)
        d.brut(f'<circle cx="28" cy="{y + 10}" r="11" fill="url(#arret-h-{ton})"/>')
        d.t(28, y + 14, num, 11, True, "#ffffff", "middle")
        d.t(48, y + 14, lab, 11.5, True, ENCRE)
        d.t(150, y + 14, note, 10.5, coul=GRIS)
        d.barre(TX(a_), y + 3, max(6, TX(b_) - TX(a_)), 15, ton)
    A.ecrire(d, "L'arrêt figé, pas à pas",
             "Sept étapes sur une frise de 0 à 2 secondes : annonce, attente des ordres en vol (au plus une seconde), arrêt des pilotes, gel de la mémoire au rang N, écriture de l'instantané, relève éventuelle vers un secours à jour, fin.",
             Y0 + PAS * len(etapes) + 30, "Seule l'attente des ordres en vol prend du temps ; figer et écrire l'instantané se compte en millisecondes. Il n'y a plus de journal à marquer : l'instantané porte son rang.")

print(f"ok : {len(A.noms)} schémas ({', '.join(A.noms)})")
