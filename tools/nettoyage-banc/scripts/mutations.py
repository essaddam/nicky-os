#!/usr/bin/env python3
"""Vérifie que les tests savent échouer : chaque mutation casse une règle du registre, de la mémoire
ou du moteur réduit ; au moins un test doit alors échouer. Les fichiers sont toujours restaurés.

Usage : python3 -I tools/nettoyage-banc/scripts/mutations.py
(cargo et une toolchain Rust récente ; RUSTUP_TOOLCHAIN est respecté s'il est défini)."""
import os
import pathlib
import re
import subprocess
import sys

ICI = pathlib.Path(__file__).resolve().parent.parent
NUMEROS = ICI / "src" / "numeros.rs"
MEMOIRE = ICI / "src" / "memoire.rs"
MOTEUR = ICI / "src" / "moteur.rs"

# (ce que la mutation casse, fichier, [(motif exact du code, remplacement), …])
MUTATIONS = [
    ("le lot ne fait pas passer la place à la génération suivante", NUMEROS,
     [("                self.generation[i] += 1;\n                self.statut[i] = LIBRE;\n                neuves.push(p);",
       "                self.statut[i] = LIBRE;\n                neuves.push(p);")]),
    ("le lot ignore le délai de grâce", NUMEROS,
     [(".filter(|r| r.depuis_ms.saturating_add(grace_ms) <= maintenant_ms)", ".filter(|_| true)")]),
    ("l'état complet ne porte pas les générations", NUMEROS,
     [("        b.extend_from_slice(&self.generation);\n", "        b.extend_from_slice(&vec![0; self.len()]);\n")]),
    ("le compilateur accepte un point retiré", MOTEUR,
     [("m.points.numero(nom).filter(|&n| m.points.vivant(n))", "m.points.numero(nom)")]),
    ("une attente part sans vérifier que la valeur se lit encore (absent, retiré)", MOTEUR,
     [("let lisible = m.lire(a.point).is_some_and(|(_, q)| q != qualite::MAUVAISE);", "let lisible = true;")]),
    ("une exécution commande une cible retirée qui a encore son nom", MOTEUR,
     [("            if m.points.vivant(e.cible) {", "            if m.points.nom(e.cible).is_some() {")]),
    ("le client range son cache par place, sans la génération", MOTEUR,
     [("        let cle = n.0;", "        let cle = n.place() as u32;")]),
    ("au déménagement, les attentes ne suivent pas leur point", MOTEUR,
     [("        for a in &mut self.attentes {\n            suivre(&mut a.point);\n        }\n", "")]),
    ("au déménagement, les exécutions ne suivent pas leur cible", MOTEUR,
     [("            suivre(&mut e.cible);\n", "")]),
    ("au déménagement, un point ne suit pas son appareil", MEMOIRE,
     [("            if let Some(&b) = app.get(&def.appareil) {\n                def.appareil = b;\n            }\n", "")]),
    ("une place libérée garde la valeur de son ancien occupant", MEMOIRE,
     [("            self.vifs[p] = Vif { qualite: qualite::INCERTAINE, ..Vif::default() };\n", ""),
      ("                self.vifs[i] = Vif::default();\n                self.perime[i] = JAMAIS;\n                self.fraicheur[i] = 0;\n",
       "                self.perime[i] = JAMAIS;\n                self.fraicheur[i] = 0;\n")]),
]


def tests():
    env = dict(os.environ, CARGO_TARGET_DIR=str(ICI / "target"))
    r = subprocess.run(["cargo", "test", "--release", "--offline"], cwd=ICI, env=env,
                       capture_output=True, text=True)
    sortie = r.stdout + r.stderr
    if "test result" not in sortie:
        return None, sortie
    return re.findall(r"^test (\S+) \.\.\. FAILED", sortie, re.M), sortie


def main():
    originaux = {f: f.read_text(encoding="utf-8") for f in (NUMEROS, MEMOIRE, MOTEUR)}
    echecs, sortie = tests()
    if echecs is None or echecs:
        sys.exit(f"le code d'origine ne passe pas ses tests :\n{sortie[-2000:]}")
    print("code d'origine : tous les tests passent\n")
    rate = 0
    try:
        for nom, fichier, paires in MUTATIONS:
            mute = originaux[fichier]
            for motif, remplacement in paires:
                if mute.count(motif) != 1:
                    sys.exit(f"motif introuvable ou ambigu pour « {nom} »")
                mute = mute.replace(motif, remplacement, 1)
            fichier.write_text(mute, encoding="utf-8")
            echecs, sortie = tests()
            fichier.write_text(originaux[fichier], encoding="utf-8")
            if echecs is None:
                print(f"NE COMPILE PAS  {nom}\n{sortie[-1500:]}")
                rate += 1
            elif not echecs:
                print(f"PAS DÉTECTÉE    {nom}")
                rate += 1
            else:
                court = ", ".join(e.rsplit("::", 1)[-1] for e in echecs)
                print(f"détectée        {nom}\n                → {len(echecs)} test(s) en échec : {court}")
    finally:
        for f, texte in originaux.items():
            f.write_text(texte, encoding="utf-8")
    print(f"\n{len(MUTATIONS) - rate} mutations détectées sur {len(MUTATIONS)} ; code restauré")
    sys.exit(1 if rate else 0)


if __name__ == "__main__":
    main()
