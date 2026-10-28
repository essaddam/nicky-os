"""La boîte à outils des schémas en relief de la proposition haute disponibilité v2 : cartes arrondies et
ombrées, blocs en perspective, cylindres, pastilles, flèches dont le flux s'anime (sauf quand le lecteur
demande moins de mouvement), petites icônes. Chaque schéma a ses propres dégradés, préfixés par son
identifiant : plusieurs schémas vivent dans la même page HTML."""
import glob
import pathlib

ENCRE, GRIS, LIGNE, FOND = "#1d302a", "#52615a", "#d4ddd4", "#fffdf8"
FONT = 'font-family="Avenir Next, Avenir, Segoe UI, sans-serif"'
MONO = 'font-family="SFMono-Regular, Consolas, monospace"'
# (foncé, clair, pâle) par rôle.
TONS = {
    "vert": ("#195744", "#3d9a75", "#e3f1e9"),     # l'actif
    "bleu": ("#1b5873", "#3b8db4", "#e1eef6"),     # le secours local
    "violet": ("#4b3d82", "#7e6cc2", "#ebe7f7"),   # le secours distant
    "ambre": ("#94660a", "#e0a12a", "#fbefd3"),    # les données critiques
    "ardoise": ("#3b5676", "#7494bb", "#e6ecf5"),  # l'historique
    "gris": ("#4f5d57", "#97a39d", "#eef0ec"),     # hors MQTT, arrêté
    "rouge": ("#8b3b22", "#d06a46", "#fae7dd"),    # perte, danger
}
STYLE = ("<style>.hd-flux{stroke-dasharray:1.5 11;stroke-linecap:round;animation:hd-flux 1.2s linear infinite}"
         ".hd-lent{stroke-dasharray:1.5 13;stroke-linecap:round;animation:hd-lent 2.6s linear infinite}"
         ".hd-pouls{animation:hd-pouls 1.6s ease-in-out infinite}"
         "@keyframes hd-flux{to{stroke-dashoffset:-12.5}}@keyframes hd-lent{to{stroke-dashoffset:-14.5}}"
         "@keyframes hd-pouls{0%,100%{opacity:.3}50%{opacity:1}}"
         "@media (prefers-reduced-motion:reduce){.hd-flux,.hd-lent,.hd-pouls{animation:none}}</style>")

# Icônes dans une boîte de 24 × 24, au trait.
ICONES = {
    "ecran": "M3 5h18v11H3z M8 20h8 M12 16v4",
    "telephone": "M8 2.5h8a1.5 1.5 0 0 1 1.5 1.5v16a1.5 1.5 0 0 1-1.5 1.5H8A1.5 1.5 0 0 1 6.5 20V4A1.5 1.5 0 0 1 8 2.5z M11 18.5h2",
    "radio": "M12 13v8 M8 21h8 M8.5 8.5a5 5 0 0 0 0 7 M15.5 8.5a5 5 0 0 1 0 7 M5.6 5.6a9 9 0 0 0 0 12.8 M18.4 5.6a9 9 0 0 1 0 12.8 M12 10.4a1.6 1.6 0 1 1 0 3.2a1.6 1.6 0 1 1 0-3.2z",
    "nuage": "M7 18h10a4 4 0 0 0 .5-7.97A5.5 5.5 0 0 0 7.1 9.1A4.5 4.5 0 0 0 7 18z",
    "camera": "M3 7h12v10H3z M15 10l6-3v10l-6-3z",
    "document": "M6 3h8l4 4v14H6z M14 3v4h4 M9 12h6 M9 16h6",
    "oeil": "M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12z M12 9a3 3 0 1 1 0 6a3 3 0 1 1 0-6z",
    "disque": "M7 3h8l3 3v15H7z M10 3v4 M12.5 3v4 M15 3v4",
    "cadenas": "M6 11h12v10H6z M8.5 11V8a3.5 3.5 0 0 1 7 0v3",
    "puce": "M7 7h10v10H7z M10 10h4v4h-4z M9 4v3 M12 4v3 M15 4v3 M9 17v3 M12 17v3 M15 17v3 M4 9h3 M4 12h3 M4 15h3 M17 9h3 M17 12h3 M17 15h3",
    "eclair": "M13 2L5 13h6l-1 9 8-11h-6z",
    "horloge": "M12 3a9 9 0 1 1 0 18a9 9 0 1 1 0-18z M12 7v5l3 2",
    "base": "M5 6c0-1.7 3.1-3 7-3s7 1.3 7 3v12c0 1.7-3.1 3-7 3s-7-1.3-7-3z M5 6c0 1.7 3.1 3 7 3s7-1.3 7-3 M5 12c0 1.7 3.1 3 7 3s7-1.3 7-3",
    "maison": "M3 11l9-7 9 7 M5 10v10h14V10 M10 20v-5h4v5",
    "personne": "M12 4.5a3.5 3.5 0 1 1 0 7a3.5 3.5 0 1 1 0-7z M5 20c0-3.9 3.1-6.5 7-6.5s7 2.6 7 6.5",
    "batterie": "M3 8h16v8H3z M21 11v2 M6 10.5v3 M9 10.5v3 M12 10.5v3",
    "cle": "M8 11a4 4 0 1 1 0 8a4 4 0 1 1 0-8z M11 12l8-8 M16 7l2 2 M14 9l2 2",
    "bouclier": "M12 3l7 3v5c0 4.4-3 8.3-7 10-4-1.7-7-5.6-7-10V6z",
    "fleches": "M4 10a8 8 0 0 1 14.5-3.5 M19 3v4h-4 M20 14a8 8 0 0 1-14.5 3.5 M5 21v-4h4",
    "rouage": "M12 8.5a3.5 3.5 0 1 1 0 7a3.5 3.5 0 1 1 0-7z M12 2v3 M12 19v3 M2 12h3 M19 12h3 M4.9 4.9L7 7 M17 17l2.1 2.1 M4.9 19.1L7 17 M17 7l2.1-2.1",
    "pouls": "M2 12h5l2-5 3 10 2-5h8",
    "croix": "M6 6l12 12 M18 6L6 18",
    "coche": "M5 12.5l4.5 4.5L19 7.5",
    "lettre": "M3 6h18v12H3z M3 7l9 6 9-6",
    "pile": "M4 7l8-4 8 4-8 4z M4 12l8 4 8-4 M4 17l8 4 8-4",
    "anneau": "M12 3a9 9 0 1 1-9 9 M3 12l-1.5-2.5 M3 12l2.5-1.5",
}


def lire_resultats(dossier):
    res = {}
    for f in sorted(glob.glob(str(pathlib.Path(dossier) / "*.txt"))):
        for l in open(f, encoding="utf-8"):
            if l.startswith("RESULTAT|"):
                _, sec, e, cle, v, _u = l.strip().split("|")
                res[(sec, int(e), cle)] = float(v)
    return res


def esc(s):
    return str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def fr(x, d=1):
    return f"{x:.{d}f}".replace(".", ",")


def nb(v):
    return f"{v:,.0f}".replace(",", "\u00a0")


def court(v, unite=""):
    if v >= 100:
        s = nb(v)
    elif v >= 10:
        s = fr(v, 0)
    elif v >= 1:
        s = fr(v, 1)
    else:
        s = fr(v, 2)
    return s + ((" " + unite) if unite else "")


def largeur(s, taille):
    """Largeur approchée d'un texte en Avenir Next (sans mesure réelle)."""
    w = 0.0
    for ch in str(s):
        if ch in " il.,:;'’|!()[]":
            w += 0.28
        elif ch in "mwMW":
            w += 0.82
        elif ch.isupper() or ch.isdigit():
            w += 0.64
        else:
            w += 0.53
    return w * taille


def couper(texte, largeur_max):
    lignes, cur = [], ""
    for m in texte.split(" "):
        if cur and len(cur) + 1 + len(m) > largeur_max:
            lignes.append(cur)
            cur = m
        else:
            cur = (cur + " " + m).strip()
    return lignes + ([cur] if cur else [])


def mele(a, b, k):
    """Le mélange de deux couleurs : k = 0 donne a, k = 1 donne b."""
    a, b = a.lstrip("#"), b.lstrip("#")
    c = [round(int(a[i:i + 2], 16) * (1 - k) + int(b[i:i + 2], 16) * k) for i in (0, 2, 4)]
    return "#" + "".join(f"{v:02x}" for v in c)


class Dessin:
    def __init__(self, id_):
        self.id, self.c, self.tons, self.sol = id_, [], set(), False

    def _t(self, ton):
        self.tons.add(ton)
        return TONS[ton]

    def brut(self, s):
        self.c.append("          " + s)

    def t(self, x, y, s, taille=11, gras=False, coul=ENCRE, ancre="start", mono=False, halo=False, italique=False, op=None, espace=None):
        a = [f'x="{x:.1f}"', f'y="{y:.1f}"', f'font-size="{taille}"', f'fill="{coul}"']
        if gras:
            a.append('font-weight="700"')
        if ancre != "start":
            a.append(f'text-anchor="{ancre}"')
        if mono:
            a.append(MONO)
        if italique:
            a.append('font-style="italic"')
        if op is not None:
            a.append(f'opacity="{op}"')
        if espace:
            a.append(f'letter-spacing="{espace}"')
        if halo:
            a.append(f'paint-order="stroke" stroke="{FOND}" stroke-width="4" stroke-linejoin="round"')
        self.brut(f'<text {" ".join(a)}>{esc(s)}</text>')

    def titre(self, x, y, s, coul=GRIS):
        self.t(x, y, s.upper(), 11, True, coul, espace=".06em")

    def etiquette(self, x, y, s, ton="gris", ancre="middle", taille=10.5, gras=True):
        self.t(x, y, s, taille, gras, self._t(ton)[0], ancre, halo=True)

    def icone(self, nom, x, y, s=18, coul="#ffffff", epais=1.9):
        k = s / 24
        self.brut(f'<g transform="translate({x:.1f} {y:.1f}) scale({k:.4f})" fill="none" stroke="{coul}" '
                  f'stroke-width="{epais / k:.2f}" stroke-linecap="round" stroke-linejoin="round"><path d="{ICONES[nom]}"/></g>')

    def zone(self, x, y, w, h, titre, ton, icone=None):
        f, c, p = self._t(ton)
        self.brut(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="20" fill="{p}" fill-opacity=".45" stroke="{c}" stroke-opacity=".45" stroke-width="1.2" stroke-dasharray="7 5"/>')
        dx = 18
        if icone:
            self.icone(icone, x + 16, y + 11, 16, f, 1.8)
            dx = 38
        self.t(x + dx, y + 24, titre.upper(), 11, True, f, espace=".07em")

    def carte(self, x, y, w, h, titre, lignes=(), ton="vert", icone=None, attenue=False, taille=12, sous=10.2, mot=None, pas=15):
        f, c, p = self._t(ton)
        if attenue:
            self.brut('<g opacity=".62">')
            self.brut(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="12" fill="{TONS["gris"][2]}" stroke="{TONS["gris"][1]}" stroke-width="1.2" stroke-dasharray="5 4"/>')
        else:
            self.brut(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="12" fill="url(#{self.id}-f-{ton})" stroke="{c}" stroke-opacity=".55" stroke-width="1.1" filter="url(#{self.id}-o)"/>')
        tc = GRIS if attenue else f
        if icone:
            fond = TONS["gris"][1] if attenue else f"url(#{self.id}-h-{ton})"
            self.brut(f'<rect x="{x + 10}" y="{y + 9}" width="26" height="26" rx="8" fill="{fond}"/>')
            self.icone(icone, x + 15, y + 14, 16)
            self.t(x + 44, y + 27, titre, taille, True, tc)
            y0 = y + 49
        else:
            self.brut(f'<circle cx="{x + 15}" cy="{y + 19}" r="4.5" fill="{TONS["gris"][1] if attenue else f"url(#{self.id}-h-{ton})"}"/>')
            self.t(x + 27, y + 23, titre, taille, True, tc)
            y0 = y + 41
        for k, l in enumerate(lignes):
            self.t(x + 14, y0 + k * pas, l, sous, coul=GRIS)
        if mot:
            self.pilule(x + w - 10, y + 12, mot, "gris" if attenue else ton, taille=9, ancre="end")
        if attenue:
            self.brut("</g>")

    def bouton(self, cx, cy, w, h, titre, sous, ton, icone=None):
        """Un état, en relief : un socle foncé, un dessus en dégradé, un reflet."""
        f, c, p = self._t(ton)
        x, y, rx = cx - w / 2, cy - h / 2, min(18, h / 2)
        self.brut(f'<g filter="url(#{self.id}-o2)"><rect x="{x:.1f}" y="{y + 5:.1f}" width="{w}" height="{h}" rx="{rx}" fill="{mele(f, "#000000", .25)}"/>'
                  f'<rect x="{x:.1f}" y="{y:.1f}" width="{w}" height="{h}" rx="{rx}" fill="url(#{self.id}-h-{ton})"/></g>')
        self.brut(f'<rect x="{x + 5:.1f}" y="{y + 3:.1f}" width="{w - 10}" height="{h * .42:.1f}" rx="{rx - 4}" fill="#ffffff" opacity=".17"/>')
        tx = x + 16
        if icone:
            self.brut(f'<circle cx="{x + 27:.1f}" cy="{cy:.1f}" r="15" fill="#ffffff" opacity=".2"/>')
            self.icone(icone, x + 18, cy - 9, 18)
            tx = x + 50
        self.t(tx, cy - (2 if sous else -5), titre, 13, True, "#ffffff")
        if sous:
            self.t(tx, cy + 14, sous, 10, coul="#ffffff", op=".9")

    def bloc(self, x, y, w, h, ton, titre, sous=None, prof=16, icone=None):
        """Une machine en perspective : dessus, flanc, face, bandeau de titre. Rend le haut utile de la face."""
        f, c, p = self._t(ton)
        self.sol = True
        dx, dy = prof, -prof * .55
        self.brut(f'<ellipse cx="{x + w / 2 + dx / 2:.1f}" cy="{y + h + 7:.1f}" rx="{w / 2 + 14:.1f}" ry="11" fill="url(#{self.id}-sol)"/>')
        self.brut(f'<polygon points="{x + 6},{y} {x + 6 + dx},{y + dy} {x + w + dx},{y + dy} {x + w},{y}" fill="url(#{self.id}-t-{ton})"/>')
        self.brut(f'<polygon points="{x + w},{y} {x + w + dx},{y + dy} {x + w + dx},{y + h + dy - 6} {x + w},{y + h}" fill="url(#{self.id}-s-{ton})"/>')
        self.brut(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="8" fill="url(#{self.id}-f-{ton})" stroke="{c}" stroke-opacity=".5"/>')
        self.brut(f'<path d="M{x} {y + 34} V{y + 8} a8 8 0 0 1 8 -8 H{x + w - 8} a8 8 0 0 1 8 8 V{y + 34} z" fill="url(#{self.id}-h-{ton})"/>')
        tx = x + 14
        if icone:
            self.icone(icone, x + 12, y + 8, 18)
            tx = x + 38
        self.t(tx, y + 22.5, titre, 13, True, "#ffffff")
        if sous:
            self.t(x + w - 12, y + 22.5, sous, 10, coul="#ffffff", ancre="end", op=".9")
        return y + 34

    def cylindre(self, cx, y, w, h, ton, titre=None, sous=None, lignes=()):
        f, c, p = self._t(ton)
        rx, ry = w / 2, max(6, w * .13)
        self.brut(f'<g filter="url(#{self.id}-o)"><path d="M{cx - rx:.1f} {y + ry:.1f} V{y + h - ry:.1f} A{rx:.1f} {ry:.1f} 0 0 0 {cx + rx:.1f} {y + h - ry:.1f} V{y + ry:.1f} z" fill="url(#{self.id}-c-{ton})"/></g>')
        for k in (.42, .7):
            yy = y + ry + (h - 2 * ry) * k
            self.brut(f'<path d="M{cx - rx:.1f} {yy:.1f} A{rx:.1f} {ry:.1f} 0 0 0 {cx + rx:.1f} {yy:.1f}" fill="none" stroke="#ffffff" stroke-opacity=".28" stroke-width="1.2"/>')
        self.brut(f'<ellipse cx="{cx:.1f}" cy="{y + ry:.1f}" rx="{rx:.1f}" ry="{ry:.1f}" fill="url(#{self.id}-t-{ton})" stroke="{f}" stroke-opacity=".35"/>')
        if titre:
            self.t(cx, y + h + 16, titre, 11.5, True, f, "middle")
        if sous:
            self.t(cx, y + h + 31, sous, 10, coul=GRIS, ancre="middle")
        for k, l in enumerate(lignes):
            self.t(cx, y + h + 46 + 14 * k, l, 10, coul=GRIS, ancre="middle")

    def pilule(self, x, y, s, ton="gris", plein=False, taille=10, ancre="start", icone=None):
        f, c, p = self._t(ton)
        w = largeur(s, taille) + 16 + (14 if icone else 0)
        h = taille + 9
        x0 = x if ancre == "start" else (x - w / 2 if ancre == "middle" else x - w)
        fond = f"url(#{self.id}-h-{ton})" if plein else p
        self.brut(f'<rect x="{x0:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="{h / 2:.1f}" fill="{fond}" stroke="{c}" stroke-opacity="{0 if plein else .7}"/>')
        tx = x0 + 8
        if icone:
            self.icone(icone, x0 + 6, y + (h - 12) / 2, 12, "#ffffff" if plein else f, 2)
            tx += 14
        self.t(tx, y + h / 2 + taille * .36, s, taille, True, "#ffffff" if plein else f)
        return w

    def fleche(self, d, ton="vert", epais=2.0, anime=False, lent=False, tirets=None, tete=True, op=None):
        f, c, p = self._t(ton)
        t = f' stroke-dasharray="{tirets}"' if tirets else ""
        m = f' marker-end="url(#{self.id}-m-{ton})"' if tete else ""
        o = f' opacity="{op}"' if op else ""
        self.brut(f'<path d="{d}" fill="none" stroke="{f}" stroke-width="{epais}" stroke-linecap="round" stroke-linejoin="round"{t}{m}{o}/>')
        if anime:
            cl = "hd-lent" if lent else "hd-flux"
            self.brut(f'<path class="{cl}" d="{d}" fill="none" stroke="#ffffff" stroke-opacity=".95" stroke-width="{max(1.3, epais * .55):.1f}"/>')

    def ligne(self, x1, y1, x2, y2, ton="gris", epais=1.6, **k):
        self.fleche(f"M{x1:.1f} {y1:.1f} L{x2:.1f} {y2:.1f}", ton, epais, **k)

    def anneau(self, cx, cy, r, ton, n=28, pleins=22, ep=9):
        """Une file circulaire : les cases pleines, puis les libres, et la tête d'écriture."""
        f, c, p = self._t(ton)
        import math
        for k in range(n):
            a0 = -math.pi / 2 + 2 * math.pi * k / n + .03
            a1 = -math.pi / 2 + 2 * math.pi * (k + 1) / n - .03
            x0, y0 = cx + r * math.cos(a0), cy + r * math.sin(a0)
            x1, y1 = cx + r * math.cos(a1), cy + r * math.sin(a1)
            coul = f"url(#{self.id}-h-{ton})" if k < pleins else p
            self.brut(f'<path d="M{x0:.1f} {y0:.1f} A{r} {r} 0 0 1 {x1:.1f} {y1:.1f}" fill="none" stroke="{coul}" stroke-width="{ep}"/>')
        a = -math.pi / 2 + 2 * math.pi * pleins / n
        self.brut(f'<circle class="hd-pouls" cx="{cx + r * math.cos(a):.1f}" cy="{cy + r * math.sin(a):.1f}" r="5.5" fill="{f}"/>')

    def barre(self, x, y, w, h, ton, op=None):
        f, c, p = self._t(ton)
        o = f' opacity="{op}"' if op else ""
        self.brut(f'<rect x="{x:.1f}" y="{y:.1f}" width="{max(w, 2):.1f}" height="{h}" rx="{min(h / 2, 6)}" fill="url(#{self.id}-b-{ton})"{o}/>')
        if w > 8:
            self.brut(f'<rect x="{x + 2:.1f}" y="{y + 2:.1f}" width="{max(w - 4, 1):.1f}" height="{h * .38:.1f}" rx="{min(h / 2, 6) - 1}" fill="#ffffff" opacity=".2"/>')

    def texte(self):
        return "\n".join(self.c)

    def defs(self):
        i = self.id
        out = [f'<filter id="{i}-o" x="-8%" y="-12%" width="116%" height="140%"><feDropShadow dx="0" dy="2.2" stdDeviation="2.6" flood-color="#1d302a" flood-opacity=".13"/></filter>',
               f'<filter id="{i}-o2" x="-10%" y="-15%" width="120%" height="150%"><feDropShadow dx="0" dy="4" stdDeviation="4.5" flood-color="#1d302a" flood-opacity=".2"/></filter>']
        if self.sol:
            out.append(f'<radialGradient id="{i}-sol"><stop offset="0" stop-color="#1d302a" stop-opacity=".22"/><stop offset="1" stop-color="#1d302a" stop-opacity="0"/></radialGradient>')
        for t in sorted(self.tons):
            f, c, p = TONS[t]
            out += [
                f'<linearGradient id="{i}-f-{t}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="{mele(p, "#ffffff", .25)}"/></linearGradient>',
                f'<linearGradient id="{i}-h-{t}" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{c}"/><stop offset="1" stop-color="{f}"/></linearGradient>',
                f'<linearGradient id="{i}-b-{t}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{c}"/><stop offset="1" stop-color="{mele(f, c, .25)}"/></linearGradient>',
                f'<linearGradient id="{i}-t-{t}" x1="0" y1="1" x2="0" y2="0"><stop offset="0" stop-color="{mele(c, "#ffffff", .35)}"/><stop offset="1" stop-color="{mele(p, "#ffffff", .2)}"/></linearGradient>',
                f'<linearGradient id="{i}-s-{t}" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="{f}"/><stop offset="1" stop-color="{mele(f, "#000000", .3)}"/></linearGradient>',
                f'<linearGradient id="{i}-c-{t}" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="{mele(f, "#000000", .1)}"/><stop offset=".45" stop-color="{c}"/><stop offset="1" stop-color="{mele(f, "#000000", .15)}"/></linearGradient>',
                f'<marker id="{i}-m-{t}" viewBox="0 0 10 10" refX="8.6" refY="5" markerWidth="6.5" markerHeight="6.5" orient="auto-start-reverse"><path d="M1 1.2 L9 5 L1 8.8 Q2.6 5 1 1.2z" fill="{f}"/></marker>',
            ]
        return "          <defs>\n" + "\n".join("            " + o for o in out) + "\n          </defs>\n"


class Atelier:
    """Écrit chaque schéma deux fois : le fragment de la page HTML, et le SVG autonome du Markdown."""

    def __init__(self, fragments, autonomes):
        self.F, self.MD = pathlib.Path(fragments), pathlib.Path(autonomes)
        self.F.mkdir(exist_ok=True, parents=True)
        self.MD.mkdir(exist_ok=True, parents=True)
        self.noms = []

    def ecrire(self, d, titre, desc, h, legende):
        nom = d.id
        defs, corps = d.defs(), d.texte()
        html = (f'      <figure class="diagram">\n        <svg viewBox="0 0 1000 {h}" role="img" aria-labelledby="{nom}-t {nom}-d" {FONT}>\n'
                f'          <title id="{nom}-t">{esc(titre)}</title>\n          <desc id="{nom}-d">{esc(desc)}</desc>\n'
                f'          {STYLE}\n{defs}{corps}\n        </svg>\n        <figcaption>{esc(legende)}</figcaption>\n      </figure>\n')
        (self.F / f"{nom}.svg.html").write_text(html, encoding="utf-8")
        lignes = couper(legende, 150)
        hh = h + 18 + 17 * len(lignes)
        leg = "\n".join(f'  <text x="18" y="{h + 12 + 17 * (k + 1)}" font-size="12" fill="{GRIS}" font-style="italic">{esc(l)}</text>' for k, l in enumerate(lignes))
        (self.MD / f"{nom}.svg").write_text(
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 {hh}" width="1000" height="{hh}" role="img" aria-labelledby="{nom}-t {nom}-d" {FONT}>\n'
            f'  <title id="{nom}-t">{esc(titre)}</title>\n  <desc id="{nom}-d">{esc(desc)}</desc>\n  {STYLE}\n'
            f'  <rect width="1000" height="{hh}" rx="16" fill="{FOND}"/>\n{defs}{corps}\n{leg}\n</svg>\n', encoding="utf-8")
        self.noms.append(nom)
