#!/usr/bin/env python3
"""Vérifie que les tests savent échouer : chaque mutation casse une règle du moteur de référence
(src/moteur.rs) ou du moteur du temps (src/calendrier.rs) ; au moins un test doit alors échouer.
Les fichiers sont toujours restaurés.

Usage : python3 -I tools/memoire-banc/scripts/mutations.py
(cargo et une toolchain Rust récente ; RUSTUP_TOOLCHAIN est respecté s'il est défini)."""
import os
import pathlib
import re
import subprocess
import sys

ICI = pathlib.Path(__file__).resolve().parent.parent
MOTEUR = ICI / "src" / "moteur.rs"
TEMPS = ICI / "src" / "calendrier.rs"

# (ce que la mutation casse, fichier, [(motif exact du code, remplacement), …])
MUTATIONS = [
    ("une attente de déclencheur ne revérifie plus l'instant « changé »", MOTEUR,
     [("etat.vifs[nd.a as usize].change_ms == self.at_depuis[k]\n                    && q != qualite::MAUVAISE",
       "q != qualite::MAUVAISE")]),
    ("le redémarrage reprend toutes les exécutions, même très en retard", MOTEUR,
     [("if e.reveil_s != u32::MAX && e.reveil_s.saturating_add(120) < t.mur_s() {", "if false {")]),
    ("les bascules sont lues à chaque valeur, pas à la fin du message", MOTEUR,
     [("    pub fn message(&mut self, etat: &mut Etat, t: Temps, changements: &[(u32, i64)]) {\n",
       "    pub fn message(&mut self, etat: &mut Etat, t: Temps, changements: &[(u32, i64)]) {\n"
       "        if changements.len() > 1 {\n            for &c in changements {\n                self.message(etat, t, &[c]);\n"
       "            }\n            return;\n        }\n")]),
    ("le registre ne garde plus la valeur lue : le message relit la température en direct", MOTEUR,
     [("self.ex[case as usize].registre = etat.lire(nd.a).0;", "self.ex[case as usize].registre = i64::from(nd.a);"),
      ('.replace("{{lu}}", &e.registre.to_string());', '.replace("{{lu}}", &etat.lire(e.registre as u32).0.to_string());')]),
    ("une recompilation arrête toutes les exécutions, même inchangées", MOTEUR,
     [("let changes: Vec<usize> = (0..nouveau.plans.len()).filter(|&a| change(a)).collect();",
       "let changes: Vec<usize> = (0..nouveau.plans.len()).collect();")]),
    ("la cause n'est plus gardée par l'exécution", MOTEUR,
     [("genre::CAUSE_POINT => e.cause_point == r.point,", "genre::CAUSE_POINT => false,")]),
    ("une issue est acceptée sans vérifier sa génération ni sa demande", MOTEUR,
     [("if e.etat == etat_ex::ISSUE && e.generation == generation && e.demande == demande {",
       "if e.etat != etat_ex::LIBRE {")]),
    ("une attente compte en heure murale au lieu de l'horloge monotone", MOTEUR,
     [("self.at_echeance[k] = t.mono + nd.b;", "self.at_echeance[k] = t.mur_s() + nd.b;")]),
    ("une valeur trop vieille reste bonne", MOTEUR,
     [("etat.vifs[n].qualite = qualite::MAUVAISE;\n                etat.perime[n] = u32::MAX;", "etat.perime[n] = u32::MAX;")]),
    ("une heure répétée (retour à l'heure d'hiver) part à la seconde copie", TEMPS,
     [("AmbiguousOffset::Fold { .. } => amb.earlier().ok()?,", "AmbiguousOffset::Fold { .. } => amb.later().ok()?,")]),
    ("une heure qui n'existe pas (passage à l'heure d'été) est sautée", TEMPS,
     [("                let avant = amb.earlier().ok()?;\n                self.tz.following(avant).next()?.timestamp()",
       "                return None")]),
    ("tout se rattrape, quel que soit le retard", TEMPS,
     [("pub const RATTRAPAGE_MS: u64 = 120_000;", "pub const RATTRAPAGE_MS: u64 = u64::MAX / 4;")]),
    ("une occurrence repart après un recul de l'horloge", TEMPS,
     [("        let apres = if self.derniere[k] > 0 { self.derniere[k] } else { maintenant_ms };",
       "        let apres = maintenant_ms;")]),
    ("le calendrier s'arme sur une horloge pas encore sûre", TEMPS,
     [("    pub fn armer_tout(&mut self, maintenant_ms: u64, valeur: &dyn Fn(u32) -> Option<i64>) -> Vec<Sortie> {\n        if !self.horloge_sure {\n            return Vec::new();\n        }",
       "    pub fn armer_tout(&mut self, maintenant_ms: u64, valeur: &dyn Fn(u32) -> Option<i64>) -> Vec<Sortie> {")]),
    ("un rappel d'agenda en retard ne part plus, même avant l'événement", TEMPS,
     [("((i, e.numero) > apres && (i + RATTRAPAGE_MS >= maintenant_ms || b > maintenant_ms))",
       "((i, e.numero) > apres && (i + RATTRAPAGE_MS >= maintenant_ms))")]),
    ("la partie après minuit d'une plage appartient au jour même, pas à la veille", TEMPS,
     [("(d.yesterday().is_ok_and(|v| jour_ok(f.jours, v)) && s < f.b)", "(jour_ok(f.jours, d) && s < f.b)")]),
    ("deux rendez-vous à la même heure ne font qu'un rappel", TEMPS,
     [("((i, e.numero) > apres && ", "(i > apres.0 && ")]),
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
    originaux = {f: f.read_text(encoding="utf-8") for f in (MOTEUR, TEMPS)}
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
