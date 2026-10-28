#!/usr/bin/env python3
"""Vérifie que les tests savent échouer : chaque mutation casse une règle du catalogue, des
demandes, des droits, du hub, du kit, du moteur ou d'un écran ; au moins un test doit alors échouer.
Les fichiers sont toujours restaurés. v2 : au moins une mutation par constat de conception de la revue
Codex (B1 à B5, I1 à I14, I20), en plus de 18 de la v1 (les deux autres sont remplacées). v3 : au moins
une par constat de la revue des droits (D1 à D9, M1 à M8 sauf M4, qui est un budget, W1 à W6) ; six
motifs de la v2 suivent le code v3 (B1, B2, I1, I7) ; et une par choix du propriétaire qui
change le code (la voix, le groupe, l'urgence).

Usage : python3 -I tools/ordres-banc/scripts/mutations.py [--motifs]
(--motifs : vérifie seulement que chaque motif existe une fois ; cargo et une toolchain Rust récente ;
RUSTUP_TOOLCHAIN est respecté s'il est défini)."""
import os
import pathlib
import re
import subprocess
import sys

ICI = pathlib.Path(__file__).resolve().parent.parent
SRC = ICI / "src"
CATALOGUE = SRC / "catalogue.rs"
DEMANDES = SRC / "demandes.rs"
HUB = SRC / "hub.rs"
DROITS = SRC / "droits.rs"
MODULE = SRC / "module.rs"
MOTEUR = SRC / "moteur.rs"
SOIREE = SRC / "soiree.rs"

# (ce que la mutation casse, fichier, [(motif exact du code, remplacement), …])
MUTATIONS = [
    # --- la v1, adaptée ---
    ("une issue tardive touche la demande qui occupe la case (génération ignorée)", DEMANDES,
     [("self.fiches.get(r.case()).filter(|d| d.etat != etat::LIBRE && d.generation == r.generation() && d.epoque == r.epoque())",
       "self.fiches.get(r.case()).filter(|d| d.etat != etat::LIBRE && d.epoque == r.epoque())"),
      ("self.fiches.get_mut(r.case()).filter(|d| d.etat != etat::LIBRE && d.generation == r.generation() && d.epoque == r.epoque())",
       "self.fiches.get_mut(r.case()).filter(|d| d.etat != etat::LIBRE && d.epoque == r.epoque())")]),
    ("le hub ne valide plus les arguments contre la forme", HUB,
     [("        if let Err(x) = self.catalogue.valider(f, &r.args) {", "        if let Err(x) = Ok::<(), u8>(()) {")]),
    ("la voie fusionne aussi les appuis", MODULE,
     [("if !appui && let Some(last) = v.file.back() && last.ordre == c.ordre {",
       "if let Some(last) = v.file.back() && last.ordre == c.ordre && appui == appui {")]),
    ("la voie fusionne une consigne par-dessus un autre ordre", MODULE,
     [("if !appui && let Some(last) = v.file.back() && last.ordre == c.ordre {",
       "if !appui && let Some(_last) = v.file.back() {")]),
    ("une décision tombe sur une autre carte que celle montrée", HUB,
     [("        if d.ordre != ordre_vu.0 {", "        if d.ordre != ordre_vu.0 && false {")]),
    ("la bascule renvoie ce qui était remis à un module", HUB,
     [("                    self.issue_puis_suivi(t, x, issue::INCERTAIN, raison::BASCULE);",
       "                    let c = Cmd { d: x, ordre: Numero(d.ordre), appareil: self.app_de_ordre(Numero(d.ordre)) as u32, forme: d.forme, args: self.args_publics(&d, x), echeance_ms: t.mono_ms + ECHEANCE_MS, epoque: self.epoque };\n"
       "                    self.cmd.push(c);")]),
    ("un module mort rend « non envoyé » au lieu d'« incertain »", HUB,
     [("            self.issue_puis_suivi(t, x, issue::INCERTAIN, raison::MODULE);",
       "            self.issue_puis_suivi(t, x, issue::NON_ENVOYE, raison::MODULE);")]),
    ("la copie ne tient pas la dernière commande des ordres", HUB,
     [("                    self.catalogue.fiches[ordre.place()] = *fiche;", "                    let _ = fiche;")]),
    ("la copie ne tient pas les reçus", HUB,
     [("                    self.recus.noter(demande.de, *r);", "                    let _ = r;")]),
    ("la copie garde l'échéance monotone de l'actif", HUB,
     [("let demande = &Demande { echeance_ms: t.mono_de(*echeance_mur_ms), ..*demande };",
       "let demande = &Demande { echeance_ms: echeance_mur_ms.saturating_sub(t.mur_ms - t.mono_ms + 1_800_000), ..*demande };")]),
    ("le lot ne remet pas à zéro la fiche d'un ordre libéré", HUB,
     [("            self.catalogue.effacer(p as usize);", "            let _ = p;")]),
    ("un client à vieux cache commande le nouvel occupant de la place", HUB,
     [("let Some(o) = o.filter(|&o| self.ordres.nom(o).is_some()) else {",
       "let Some(o) = o.map(|o| self.ordres.numero_de(o.place())).filter(|&o| self.ordres.nom(o).is_some()) else {")]),
    ("la garde ne lit que la pièce de l'appareil virtuel, pas celles de ses cibles et membres", HUB,
     [("        for c in self.cibles_de(o) {\n            v.push", "        for c in Vec::<u32>::new() {\n            v.push"),
      ("if m.sorte == sorte::APPAREIL && m.role == role::MEMBRE && self.appareils.vivant(Numero(m.numero)) {",
       "if m.sorte == sorte::APPAREIL && m.role == role::MEMBRE && false {")]),
    ("un ordre de groupe se dit confirmé dès qu'une cible a obéi", HUB,
     [("    if m.confirmees == m.cibles {\n        (issue::CONFIRME, 0)", "    if m.confirmees > 0 {\n        (issue::CONFIRME, 0)")]),
    ("le hub ne suit plus les demandes héritées d'une bascule ou d'un arrêt", HUB,
     [("        for (h, depuis) in self.heritees.clone() {", "        for (h, depuis) in Vec::<(Ref, u64)>::new() {")]),
    ("une valeur en mouvement confirme en passant par la cible", CATALOGUE,
     [("        if x.drapeaux & drapeau::A_L_ARRET != 0 && en_mouvement {", "        if x.drapeaux & drapeau::A_L_ARRET != 0 && en_mouvement && false {")]),
    ("un bail échu pendant un arrêt n'est plus rétabli", MOTEUR,
     [("            if b.etat == 1 && b.terme_mur_ms <= t.mur_ms {", "            if b.etat == 1 && b.terme_mur_ms <= t.mur_ms && t.mur_ms - b.terme_mur_ms < 1_000 {")]),
    ("un écran garde l'ancienne structure d'un ordre redéclaré", SOIREE,
     [("                    self.ordres.insert(n.0, (format!(\"{nom}/{cle}\"), *f));",
       "                    self.ordres.entry(n.0).or_insert((format!(\"{nom}/{cle}\"), *f));")]),
    # --- B1 : le principal, ses pièces, sa personne ---
    ("B1 · le canal commande hors de ses pièces", DROITS,
     [("            if t_mur_ms >= p.fin_mur_ms || ps & !p.pieces != 0 {", "            if t_mur_ms >= p.fin_mur_ms {")]),
    ("B1 · la demande prend l'acteur que le client déclare", HUB,
     [("qui: if p.est_interne() || r.parent != Ref(0) { r.qui } else { p.qui },", "qui: r.qui,")]),
    # --- B2 : l'accusé du secours, la reprise non sûre ---
    ("B2 · un appui part sans attendre l'accusé du secours", HUB,
     [("        if forme.effet == effet::APPUI && self.secours_en_phase {", "        if false {")]),
    ("B2 · une tentative ambiguë est refaite après une reprise non sûre", HUB,
     [("            if self.epoque_non_sure != 0\n                && r.emise_epoque < self.epoque_non_sure", "            if false\n                && r.emise_epoque < self.epoque_non_sure")]),
    ("B2 · l'actif envoie sans inscrire le décrochage chez le témoin", HUB,
     [("            self.decroche = true;\n            self.secours_en_phase = false;", "            self.secours_en_phase = false;")]),
    # --- B3 : les reçus ---
    ("B3 · une clé plus ancienne que la fenêtre repart", DEMANDES,
     [("        if seq.saturating_add(RECU_FENETRE as u64) <= f.plus_haute {", "        if false {")]),
    ("B3 · la même clé pour un autre ordre passe pour la même demande", DEMANDES,
     [("return if r.ordre == ordre && r.args == args { Verdict::Double(*r) } else { Verdict::Conflit };", "return Verdict::Double(*r);")]),
    # --- B4 : le numéro et son époque ---
    ("B4 · un numéro d'une autre époque désigne la demande de la case", DEMANDES,
     [("self.fiches.get(r.case()).filter(|d| d.etat != etat::LIBRE && d.generation == r.generation() && d.epoque == r.epoque())",
       "self.fiches.get(r.case()).filter(|d| d.etat != etat::LIBRE && d.generation == r.generation())")]),
    # --- B5 : les textes privés ---
    ("B5 · un texte privé voyage en clair sur cmd/ et ord/", HUB,
     [("                    prive = Some(x.clone());\n                    *a = Arg::P(x.len() as u32);", "                    prive = Some(x.clone());")]),
    ("B5 · un texte privé reste en mémoire après la conclusion", HUB,
     [("        self.cartes.remove(&x);\n        self.prives.remove(&x);", "        self.cartes.remove(&x);")]),
    # --- I1 : le permis ---
    ("I1 · l'approbation ne revérifie pas les droits du demandeur", HUB,
     [("        if niv == niveau::NON {", "        if false {")]),
    ("I1 · la fin d'un permis ne fait rien tomber", HUB,
     [("        for de in finis {\n            self.fin_de_permis(t, de);", "        for de in finis {\n            let _ = de;")]),
    ("I1 · une révocation laisse partir ce qui attend dans la voie", HUB,
     [("self.demandes.fiches[x.case()].de == de && self.demandes.fiches[x.case()].etat == etat::EN_FILE).collect();\n        self.annulations.extend(en_file);",
       "self.demandes.fiches[x.case()].de == de && self.demandes.fiches[x.case()].etat == etat::EN_FILE).collect();\n        let _ = en_file;")]),
    # --- I2 : les échéances ---
    ("I2 · l'échéance de l'appelant est ignorée", HUB,
     [("        if r.echeance_mur_ms != 0 && r.echeance_mur_ms <= t.mur_ms {", "        if false {")]),
    ("I2 · une retenue échue s'approuve tant que le balayage n'est pas passé", HUB,
     [("        if t.mono_ms >= d.echeance_ms {", "        if false {")]),
    # --- I3 : la forme d'admission ---
    ("I3 · une approbation vaut pour une forme qu'elle n'a pas montrée", HUB,
     [("        if f != d.forme || *carte.cibles != *cibles {", "        if false {")]),
    # --- I4 : les lectures fraîches ---
    ("I4 · une lecture d'avant la reprise confirme une demande héritée", HUB,
     [("            if d.etat != etat::SUIVIE || lu_ms < depuis {", "            if d.etat != etat::SUIVIE {")]),
    ("I4 · une lecture d'avant l'envoi prouve l'ordre en vol", MODULE,
     [("is_some_and(|e| lu_ms >= e.depuis_mur_ms && Self::prouve(", "is_some_and(|e| Self::prouve(")]),
    # --- I5 : « vient de » ---
    ("I5 · une variation avant la preuve passe pour l'œuvre de l'ordre", MOTEUR,
     [("    if f.issue == issue::CONFIRME && f.preuve & preuve::CHANGEMENT != 0 && f.preuve & preuve::PREMIER != 0 {",
       "    if f.issue == issue::CONFIRME && f.preuve & preuve::CHANGEMENT != 0 {")]),
    ("I5 · une confirmation sans variation passe pour l'œuvre de l'ordre", HUB,
     [("fi.preuve = if i == issue::CONFIRME && fi.changes >= 1 { preuve::CHANGEMENT | if fi.changes == 1 { preuve::PREMIER } else { 0 } } else { 0 };",
       "fi.preuve = if i == issue::CONFIRME { preuve::CHANGEMENT | preuve::PREMIER } else { 0 };")]),
    ("I5 · un geste à la main après la preuve passe inaperçu", MOTEUR,
     [("|| f.preuve & preuve::MODIFIEE != 0 {", "|| false {")]),
    # --- I6 : la dernière commande ---
    ("I6 · une issue tardive remplace la dernière commande", HUB,
     [("            if fi.demande == x {\n                fi.issue = i;", "            if true {\n                fi.issue = i;")]),
    # --- I7, I8, I9, I10 : les groupes ---
    ("I7 · un groupe approuvé ne passe pas par le répartiteur", HUB,
     [("        self.tracer(t, x, o, 3, None, false, acteur);\n        self.lancer(t, x);\n        Ok(x)",
       "        self.tracer(t, x, o, 3, None, false, acteur);\n        let c = Cmd { d: x, ordre: o, appareil: self.app_de_ordre(o) as u32, forme: d.forme, args: self.args_publics(&d, x), echeance_ms: t.mono_ms + ECHEANCE_MS, epoque: self.epoque };\n        self.cmd.push(c);\n        Ok(x)")]),
    ("I8 · la copie ne rejoue pas la progression d'un groupe", HUB,
     [("                        compter_fille(m, demande.issue);", "                        let _ = m;")]),
    ("I8 · des exécutions font une confirmation", HUB,
     [("    if m.confirmees == m.cibles {\n        (issue::CONFIRME, 0)", "    if m.confirmees + m.executees == m.cibles {\n        (issue::CONFIRME, 0)")]),
    ("I9 · la mère conclut dès sa première fille", HUB,
     [("        if self.actif && m.conclues >= m.cibles {", "        if self.actif && m.conclues >= 1 {")]),
    ("I9 · la mère garde une échéance à elle", HUB,
     [("            m.echeance_ms = u64::MAX;\n            m.cibles = cibles.len() as u8;", "            m.echeance_ms = t.mono_ms + ECHEANCE_MS;\n            m.cibles = cibles.len() as u8;")]),
    ("I10 · un ordre de groupe commande les cibles des autres", HUB,
     [("&& usize::from(m.source) == k && self.ordres.vivant(Numero(m.numero)))", "&& self.ordres.vivant(Numero(m.numero)))")]),
    ("I10 · un groupe cible l'ordre d'un autre groupe", HUB,
     [("                if self.apps.get(ac).is_none_or(|x| x.genre != genre::PHYSIQUE) {", "                if false {")]),
    # --- I11 : la redéclaration ---
    ("I11 · une clé qui n'est plus déclarée reste en service", HUB,
     [("                self.points.retirer(Numero(x), t.mur_ms);", "                let _ = x;")]),
    # --- I12 : les plafonds ---
    ("I12 · la 51e retenue est acceptée", HUB,
     [(".count() >= RETENUES_MAX {", ".count() >= usize::MAX {")]),
    # --- I13 : le rang des pièces ---
    ("I13 · une sortie ne fait pas avancer le rang de sa pièce", HUB,
     [("        let c = self.rangs_pieces.entry(piece).or_insert(0);\n        *c += 1;", "        let c = self.rangs_pieces.entry(piece).or_insert(0);\n        *c += u64::from(piece == u16::MAX);")]),
    # --- I14 : la validation ---
    ("I14 · un code hors du dictionnaire passe entre les bornes", CATALOGUE,
     [("                if p.nature == nature::ENUM && !contient(p.dico, v as i32) {", "                if false {")]),
    ("I14 · un objet hors de son schéma passe", CATALOGUE,
     [("                if !objet_conforme(t, &sch) {", "                if false {")]),
    # --- I20 : l'instantané ---
    ("I20 · l'instantané du moteur oublie les baux", MOTEUR,
     [("        put32(&mut b, self.baux.len() as u32);\n        for x in &self.baux {", "        put32(&mut b, 0);\n        for x in &self.baux[..0] {")]),
    # ===== v3 : les droits (revue Codex des droits) =====
    # --- D1 : qui demande, et la preuve qu'on en a ---
    ("D1 · un indice vaut une preuve : la voix reconnue a les droits de sa personne, même pour le sensible", DROITS,
     [("if a.preuve >= preuve::PERSONNELLE || (a.preuve == preuve::INDICE && b & classe::SENSIBLES == 0) { x } else { x.min(inconnu) }", "x")]),
    ("D1 · une voix reconnue ne vaut rien pour le confort", DROITS,
     [("if a.preuve >= preuve::PERSONNELLE || (a.preuve == preuve::INDICE && b & classe::SENSIBLES == 0) { x } else { x.min(inconnu) }", "if a.preuve >= preuve::PERSONNELLE { x } else { x.min(inconnu) }")]),
    ("D1 · la voix reconnue vaut une preuve personnelle", HUB,
     [("            return (indice, droits::preuve::INDICE);", "            return (indice, droits::preuve::PERSONNELLE);")]),
    ("D1 · une session personnelle échue garde sa personne", HUB,
     [("        if let Some(s) = p.session\n            && t.mur_ms < s.fin_mur_ms\n", "        if let Some(s) = p.session\n")]),
    # --- D2 : des droits par classe, par personne ---
    ("D2 · la personne ne compte plus : le canal décide seul", DROITS,
     [("    n = n.min(np);", "    let _ = np;")]),
    ("D2 · une personne commande hors de ses pièces", DROITS,
     [("            if t_mur_ms < p.debut_mur_ms || t_mur_ms >= p.fin_mur_ms || ps & !p.pieces != 0 {", "            if t_mur_ms < p.debut_mur_ms || t_mur_ms >= p.fin_mur_ms {")]),
    ("D2 · une chambre ne retient plus un canal gardé", DROITS,
     [("    if ps & ctx.protegees != 0 && (garde || a.preuve < preuve::PERSONNELLE) {", "    if ps & ctx.protegees != 0 && a.preuve < preuve::PERSONNELLE {")]),
    # --- D3 : l'approbateur habilité ---
    ("D3 · quiconque peut commander approuve", DROITS,
     [("    x & APPROUVE != 0 || (id == pour && id != 0 && x & 3 == niveau::OUI)", "    x & 3 != niveau::NON")]),
    ("D3 · un agent ou un satellite approuve", DROITS,
     [("    if profil.drapeaux & drapeau_p::DECIDE == 0 || t_mur_ms", "    if t_mur_ms")]),
    ("D3 · une retenue se décide sans habilitation", HUB,
     [("        } else if !self.peut_approuver(t, decideur, &d) {", "        } else if false {")]),
    # --- D4 : la révision de la carte ---
    ("D4 · une ancienne révision décide de la carte renouvelée", HUB,
     [("        if self.cartes.get(&x).is_none_or(|c| c.revision != revision_vue) {", "        if self.cartes.get(&x).is_none() {")]),
    # --- D5 et W1 : l'autorité ne se déclare pas ---
    ("W1 · un client déclare l'autorité d'une exécution", HUB,
     [("        if p.est_interne() {\n            let Some(a) = r.autorite else {", "        if p.est_interne() || r.autorite.is_some() {\n            let Some(a) = r.autorite else {")]),
    ("D5 · un client se dit fille d'une demande vivante et hérite de son autorité", HUB,
     [("        if r.parent != Ref(0) {\n            return self.refuser(t, &Req { parent: Ref(0), ..r }", "        if false {\n            return self.refuser(t, &Req { parent: Ref(0), ..r }")]),
    # --- D6 : le dernier point avant l'envoi ---
    ("D6 · la remise au module ne revérifie pas les droits", HUB,
     [("            if self.niveau(t, &self.aut_de(&d), c.ordre) == niveau::NON {", "            if false {")]),
    ("D6 · une personne révoquée laisse partir ce qui attend l'accusé du secours", HUB,
     [("self.demandes.get(c.d).is_some_and(|d| d.pour == id)", "self.demandes.get(c.d).is_some_and(|d| d.pour == id && false)")]),
    # --- D7 : la fin d'une personne est une échéance ---
    ("D7 · la fin d'une personne ne réveille pas le hub", HUB,
     [("        demandes.chain(secours).chain(fins).min()", "        let _ = fins;\n        demandes.chain(secours).min()")]),
    ("D7 · la fin d'une personne ne fait rien tomber", HUB,
     [("        for p in personnes {\n            self.fin_de_personne(t, p);", "        for p in personnes {\n            let _ = p;")]),
    # --- D8 : ord/ ne dit pas qui ---
    ("D8 · ord/ porte la personne", HUB,
     [("        let publique = Demande { echeance_ms: 0, pour: 0, preuve: 0, ..d };", "        let publique = Demande { echeance_ms: 0, ..d };")]),
    ("D8 · le pair ne reçoit pas l'identité", HUB,
     [("        if d.pour != 0 || fiche.pour != 0 || ligne.pour != 0 {", "        if false {")]),
    # --- D9 : désarmer ---
    ("D9 · désarmer se retient et s'approuve par le chemin générique", HUB,
     [("        if forme.classe() == classe::DESARMER {\n            return self.refuser(t, &r, o, issue::REFUSE, raison::DEDIE);", "        if false {\n            return self.refuser(t, &r, o, issue::REFUSE, raison::DEDIE);")]),
    ("D9 · une table ou un profil ouvre le désarmement", DROITS,
     [("    if c == classe::DESARMER {\n        return NON;\n    }\n    n", "    n")]),
    # --- M1 : les comptes par leur nom ---
    ("M1 · l'instantané relit les comptes par position", HUB,
     [("        h.aligner_principaux(&noms);\n", "        let _ = &noms;\n")]),
    ("M1 · la copie garde ses positions", HUB,
     [("        self.aligner_principaux(&noms);\n        for (p, (_, x)) in self.principaux.iter_mut().zip(profils) {", "        let _ = &noms;\n        for (p, (_, x)) in self.principaux.iter_mut().zip(profils) {")]),
    # --- M2 : l'acteur de la décision ---
    ("M2 · la décision est journalisée au nom du demandeur", HUB,
     [("        let (de, pour, qui) = acteur.unwrap_or((d.de, d.pour, d.qui));", "        let (de, pour, qui) = (d.de, d.pour, d.qui);")]),
    # --- M3 : la fraîcheur des droits du secours ---
    ("M3 · un secours promu sans reprise sûre croit ses droits frais", HUB,
     [("        // sans un humain, jusqu'à ce qu'un propriétaire les confirme (revue v3, M3).\n        self.droits_frais = sure;", "        // sans un humain, jusqu'à ce qu'un propriétaire les confirme (revue v3, M3).\n        self.droits_frais = true;")]),
    ("M3 · un secours aux droits incertains laisse passer le sensible", DROITS,
     [("    if !ctx.droits_frais && b & classe::SENSIBLES != 0 {", "    if false {")]),
    ("M3 · une révocation n'attend pas l'accusé du secours", HUB,
     [("        if self.secours_en_phase {\n            self.droits_a_accuser = Some((self.rang, t.mono_ms));", "        if false {\n            self.droits_a_accuser = Some((self.rang, t.mono_ms));")]),
    ("M3 · un adulte approuve le sensible sur un secours aux droits incertains", HUB,
     [("        if !self.droits_frais && classe::bit(c) & classe::SENSIBLES != 0 && personne.is_none_or", "        if false && personne.is_none_or")]),
    # --- M5 : « en cours » ---
    ("M5 · le compteur d'un ordre sature à 255", HUB,
     [("            fi.actives += 1;", "            fi.actives = (fi.actives + 1).min(255);"),
      ("            fi.actives -= 1;", "            fi.actives = fi.actives.saturating_sub(1);")]),
    # --- M6 : l'empreinte canonique ---
    ("M6 · l'empreinte concatène les textes", HUB,
     [("                h.u64(1);\n                h.u64(t.len() as u64);\n                h.octets(t.as_bytes());", "                h.octets(t.as_bytes());"),
      ("    h.u64(args.len() as u64);\n    for a in args {", "    for a in args {")]),
    # --- M7 : l'époque d'émission ---
    ("M7 · une première tentative d'avant la reprise repart", HUB,
     [("            if self.epoque_non_sure != 0\n                && r.emise_epoque < self.epoque_non_sure", "            if self.epoque_non_sure != 0\n                && r.tentative > 1\n                && r.emise_epoque < self.epoque_non_sure")]),
    ("M7 · une demande qui ne dit pas sa session passe pour neuve", HUB,
     [("                && r.emise_epoque < self.epoque_non_sure\n", "                && r.emise_epoque != 0\n                && r.emise_epoque < self.epoque_non_sure\n")]),
    ("M7 · une reprise sûre plus tard efface l'ambiguïté d'avant", HUB,
     [("        h.epoque_non_sure = get32(b)?;", "        let _ = get32(b)?;")]),
    # --- M8 : un texte privé par forme ---
    ("M8 · deux textes privés dans une forme", CATALOGUE,
     [("count() > 1 {\n            return Err(\"un seul texte privé par forme\");", "count() > 2 {\n            return Err(\"un seul texte privé par forme\");")]),
    # --- W1 : délégation et mandat ---
    ("W1 · une routine lancée par une personne prend le mandat de son responsable", HUB,
     [("            if a.appele {\n                if self.principaux", "            if false {\n                if self.principaux")]),
    ("W1 · un automatisme commande hors de son manifeste", DROITS,
     [("        && (m.drapeaux & drapeau_m::VIVANT == 0 || m.classes & b == 0 || ps & !m.pieces != 0)", "        && (m.drapeaux & drapeau_m::VIVANT == 0)")]),
    ("W1 · un déclencheur ouvert lance une action sensible sans décision", MOTEUR,
     [("Some(_) if ouvert && mandat.classes & classe::SENSIBLES != 0 && !resp.declencheur_ouvert =>", "Some(_) if false =>")]),
    # --- W2 : le responsable ---
    ("W2 · un responsable accorde un mandat qu'il n'approuverait pas", DROITS,
     [("&& table[usize::from(p.role)][usize::from(c)] & APPROUVE != 0)", "&& table[usize::from(p.role)][usize::from(c)] & 3 != 0)")]),
    ("W2 · un non-propriétaire fait adopter un automatisme par le foyer", MOTEUR,
     [("Some(p) if resp.foyer && p.role != droits::role::PROPRIETAIRE =>", "Some(p) if false && p.role != droits::role::PROPRIETAIRE =>")]),
    ("W2 · un mandat refusé à l'exécution ne met pas l'automatisme à revoir", MOTEUR,
     [("if raison == crate::catalogue::raison::DROITS && !x.appele {", "if false {")]),
    # --- W3 : les baux ---
    ("W3 · un bail s'ouvre sur un ordre refusé", MOTEUR,
     [("if matches!(i, issue::CONFIRME | issue::EXECUTE | issue::ACCEPTE | issue::ENVOYE) {", "if true {")]),
    ("W3 · un bail rétablit par-dessus un humain", MOTEUR,
     [("if hub.catalogue.fiches[o.place()].demande.0 != b.demande || vient_de(hub, o) == resultat::NON {", "if false {")]),
    # --- W4 : l'empreinte du programme ---
    ("W4 · une exécution reprend dans un autre programme", MOTEUR,
     [("if self.autos.get(x.auto as usize).is_some_and(|a| a.empreinte == x.empreinte) {", "if true {")]),
    # --- W5 : les exécutions au secours ---
    ("W5 · les exécutions ne passent pas au secours", MOTEUR,
     [("hub.sortir(Sortie::Execution { c: c as u32, ex: (x.etat != etat_ex::LIBRE).then_some(x) });", "let _ = x;")]),
    # --- W6 : les rafales ---
    ("W6 · pas de quota de départs", MOTEUR,
     [("        if fenetre.len() >= DEPARTS_PAR_MINUTE {", "        if false {")]),
    ("W6 · une même transition relance l'automatisme", MOTEUR,
     [("                && self.dernier[k as usize] != (d.0, i)\n", "")]),
    # --- v3 § 22 : les choix du propriétaire ---
    ("choix · un groupe exécute d'office son sous-ensemble permis", HUB,
     [("        let retenue = !approuvee && (n == niveau::PROPOSER || sous_ensemble.is_some());", "        let retenue = !approuvee && n == niveau::PROPOSER;")]),
    ("choix · un automatisme se voit annoncer un sous-ensemble au lieu d'être mis à revoir", HUB,
     [("forme.drapeaux & drapeau::GROUPE != 0 && !interne && !approuvee", "forme.drapeaux & drapeau::GROUPE != 0 && r.parent == Ref(0) && !approuvee")]),
    ("choix · un autre que le demandeur confirme le sous-ensemble", HUB,
     [("            if !self.peut_confirmer(t, decideur, &d) {", "            if false {")]),
    ("choix · confirmer le sous-ensemble vaut approbation", HUB,
     [("        if a_confirmer && niv == niveau::PROPOSER {", "        if false {")]),
    ("choix · un automatisme adopté par le foyer meurt avec son auteur", HUB,
     [("            let foyer = m.drapeaux & drapeau_m::FOYER != 0;\n            return Ok(Aut {", "            let foyer = false;\n            return Ok(Aut {")]),
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
    fichiers = (CATALOGUE, DEMANDES, DROITS, HUB, MODULE, MOTEUR, SOIREE)
    originaux = {f: f.read_text(encoding="utf-8") for f in fichiers}
    for nom, fichier, paires in MUTATIONS:
        for motif, _ in paires:
            n = originaux[fichier].count(motif)
            if n != 1:
                sys.exit(f"motif trouvé {n} fois pour « {nom} » : {motif[:80]!r}")
    if "--motifs" in sys.argv:
        print(f"{len(MUTATIONS)} mutations, chaque motif trouvé une fois")
        return
    echecs, sortie = tests()
    if echecs is None or echecs:
        sys.exit(f"le code d'origine ne passe pas ses tests :\n{sortie[-2000:]}")
    print("code d'origine : tous les tests passent\n")
    rate = 0
    try:
        for nom, fichier, paires in MUTATIONS:
            mute = originaux[fichier]
            for motif, remplacement in paires:
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
