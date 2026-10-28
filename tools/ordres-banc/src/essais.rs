//! Les essais ciblés de la v2 : ce que la soirée ne rejoue pas (revue Codex B2, I1, I8, I9, I10,
//! I12). Chacun monte un petit hub, ses appareils simulés, et vérifie une règle.

use crate::catalogue::{Arg, FormeDecl, binaire, drapeau, effet, mesure, raison, retour};
use crate::demandes::{Ref, etat, issue, origine};
use crate::hub::{AppDecl, Cible, Hub, Principal, Req, Sortie, T, role, sorte};
use crate::module::{Kit, Monde, Rep};

pub(crate) const T0: u64 = 1_800_000_000_000;

pub(crate) fn t(ms: u64) -> T {
    T { mono_ms: 1_000 + ms, mur_ms: T0 + ms }
}

pub(crate) struct Mini {
    pub(crate) hub: Hub,
    pub(crate) kit: Kit,
    pub(crate) monde: Monde,
    pub(crate) script: u16,
    pub(crate) agent: u16,
    pub(crate) ecran: u16,
}

impl Mini {
    pub(crate) fn new(decls: &[AppDecl]) -> Mini {
        let mut hub = Hub::default();
        let tout = crate::droits::Personne { pieces: crate::droits::TOUTES_PIECES, fin_mur_ms: u64::MAX, role: crate::droits::role::PROPRIETAIRE, ..Default::default() };
        hub.ajouter_personne("Bob", tout).expect("personne");
        let ecran = hub.ajouter_principal(Principal::humain("écran", 1)).expect("principal");
        let agent = hub.ajouter_principal(Principal::agent("agent", origine::MCP, 2)).expect("principal");
        let script = hub.ajouter_principal(Principal::agent("script", origine::API, 3)).expect("principal");
        for d in decls {
            hub.declarer(t(0), d).expect("déclaration");
        }
        hub.sorties.clear();
        Mini { hub, kit: Kit::default(), monde: Monde::default(), script, agent, ecran }
    }

    pub(crate) fn rep(&mut self, appareil: &str, r: Rep) {
        let a = self.hub.appareils.numero(appareil).expect("appareil").place() as u32;
        self.monde.reps.insert(a, r);
    }

    pub(crate) fn req(&self, de: u16, seq: u64, ordre: &str, args: Vec<Arg>, ms: u64) -> Req {
        // Le client dit sur quelle session il l'émet : celle de l'époque qu'il connaît (revue v3, M7).
        Req { emise_epoque: self.hub.epoque, ..Req::nouvelle(de, seq, Cible::Nom(ordre.into()), args, t(ms)) }
    }

    /// Fait tourner de `de_ms` à `a_ms` : les ordres au kit, les réponses des appareils, les
    /// échéances ; réveil au plus proche.
    pub(crate) fn tourner(&mut self, de_ms: u64, a_ms: u64) {
        let mut ms = de_ms;
        loop {
            let tt = t(ms);
            for c in self.hub.remettre(tt) {
                self.kit.recevoir(&mut self.hub, &mut self.monde, tt, c);
            }
            for q in self.monde.echus(tt.mur_ms) {
                self.kit.evenement(&mut self.hub, &mut self.monde, tt, q);
            }
            for c in self.hub.remettre(tt) {
                self.kit.recevoir(&mut self.hub, &mut self.monde, tt, c);
            }
            self.kit.tic(&mut self.hub, &mut self.monde, tt);
            self.hub.tic(tt);
            let annulees = std::mem::take(&mut self.hub.annulations);
            if !annulees.is_empty() {
                self.kit.annuler(&mut self.hub, tt, &annulees);
            }
            if ms >= a_ms {
                break;
            }
            let suivant = [self.monde.prochain(), self.hub.prochaine(tt), self.kit.prochaine(tt)].into_iter().flatten().map(|m| m - T0).filter(|&m| m > ms).min().unwrap_or(a_ms);
            ms = suivant.min(a_ms);
        }
    }

    pub(crate) fn issues(&self, seq: u64) -> Vec<(u8, u8)> {
        self.hub.res.iter().filter(|r| r.seq == seq).map(|r| (r.issue, r.raison)).collect()
    }
}

pub(crate) fn sonnette(nom: &str, piece: u16) -> AppDecl {
    AppDecl::physique(nom, piece, &[], vec![("appuyer", FormeDecl { drapeaux: drapeau::PROPRIO_HUB, ..FormeDecl::sans_param(effet::APPUI, retour::PONT) })])
}

pub(crate) fn haut_parleur() -> AppDecl {
    AppDecl::physique("sonos:salon", 0, &["volume"], vec![("volume_plus", FormeDecl::sans_param(effet::APPUI, retour::PONT)), ("volume", FormeDecl::regler(mesure(0, 100, 1, 0, 2), "volume", 0))])
}

pub(crate) fn lampe(nom: &str, piece: u16, lum_max: i32) -> AppDecl {
    AppDecl::physique(nom, piece, &["on", "brightness"], vec![("on", FormeDecl::regler(binaire(), "on", 0)), ("brightness", FormeDecl::regler(mesure(0, lum_max, 1, 0, 0), "brightness", 2))])
}

pub(crate) fn groupe(nom: &str, ordres: Vec<(&str, FormeDecl)>, membres: Vec<(u8, &str, u8, &str)>) -> AppDecl {
    AppDecl {
        nom: nom.into(),
        piece: 0,
        points: Vec::new(),
        defs: Vec::new(),
        ordres: ordres.into_iter().map(|(k, f)| (k.to_string(), f)).collect(),
        membres: membres.into_iter().map(|(s, n, r, src)| (s, n.to_string(), r, src.to_string())).collect(),
    }
}

pub(crate) fn ordre_de_groupe(f: FormeDecl) -> FormeDecl {
    FormeDecl { drapeaux: f.drapeaux | drapeau::GROUPE, confirme: String::new(), params: f.params.into_iter().map(|p| crate::catalogue::ParamDecl { regle: String::new(), ..p }).collect(), ..f }
}

/// B2 : après un redémarrage sur un instantané plus ancien qu'un appui exécuté, une nouvelle
/// tentative de cet appui est refusée (« incertain, ambiguë ») ; une demande neuve passe.
#[test]
fn une_reprise_non_sure_refuse_une_tentative_ambigue() {
    let mut m = Mini::new(&[haut_parleur()]);
    m.rep("sonos:salon", Rep::Pont { ms: 100 });
    let vieux = m.hub.ranger(t(1_000));
    let r = m.req(m.script, 7, "sonos:salon/volume_plus", vec![], 2_000);
    m.hub.demander(t(2_000), r).expect("admise");
    m.tourner(2_000, 3_000);
    assert_eq!(m.kit.envois.len(), 1, "l'appui est parti");
    // La machine tombe ; elle redémarre sur l'instantané d'avant l'appui.
    let principaux = m.hub.principaux.clone();
    let mut h = Hub::relire(&vieux, t(10_000), principaux).expect("instantané");
    h.redemarre(t(10_000), false);
    // Émise sur la session d'avant la reprise (l'époque 1) : c'est le serveur qui la date (v3, M7).
    let retente = Req { tentative: 2, emise_epoque: 1, ..m.req(m.script, 7, "sonos:salon/volume_plus", vec![], 10_500) };
    assert_eq!(h.demander(t(10_500), retente), Err((issue::INCERTAIN, raison::AMBIGUE)), "elle a pu partir avant : on ne la refait pas");
    assert!(h.cmd.is_empty());
    let neuve = Req { emise_epoque: h.epoque, ..m.req(m.script, 8, "sonos:salon/volume_plus", vec![], 10_600) };
    assert!(h.demander(t(10_600), neuve).is_ok());
    assert_eq!(h.cmd.len(), 1, "une demande neuve part");
    // Une consigne retentée, elle, se rejoue sans risque : la même valeur une seconde fois.
    let consigne = Req { tentative: 2, emise_epoque: 1, ..m.req(m.script, 9, "sonos:salon/volume", vec![Arg::N(20)], 10_700) };
    assert!(h.demander(t(10_700), consigne).is_ok());
}

/// B2 : un appui attend l'accusé du secours ; sans accusé au bout de 200 ms, l'actif inscrit chez le
/// témoin que le secours a décroché, puis l'envoie ; une copie promue après cela n'est pas sûre.
#[test]
fn un_secours_qui_decroche_est_inscrit_avant_l_envoi() {
    let mut m = Mini::new(&[haut_parleur()]);
    m.hub.secours_en_phase = true;
    let image = m.hub.ranger(t(500));
    let r = m.req(m.script, 1, "sonos:salon/volume_plus", vec![], 1_000);
    m.hub.demander(t(1_000), r).expect("admise");
    assert!(m.hub.cmd.is_empty(), "il attend l'accusé");
    let rang = m.hub.rang;
    m.hub.acquitter(rang);
    assert_eq!(m.hub.cmd.len(), 1, "accusé : il part");
    m.hub.cmd.clear();
    let r = m.req(m.script, 2, "sonos:salon/volume_plus", vec![], 2_000);
    m.hub.demander(t(2_000), r).expect("admise");
    m.hub.tic(t(2_100));
    assert!(m.hub.cmd.is_empty() && !m.hub.decroche, "100 ms : il attend encore");
    m.hub.tic(t(2_200));
    assert!(m.hub.decroche, "inscrit chez le témoin");
    assert_eq!(m.hub.cmd.len(), 1, "puis envoyé sans accusé");
    assert!(!m.hub.secours_en_phase);
    // La copie n'avait que l'image d'avant : promue, sa reprise n'est pas sûre.
    let mut copie = Hub::relire(&image, t(5_000), m.hub.principaux.clone()).expect("état complet");
    copie.promouvoir(t(5_000), !m.hub.decroche);
    let retente = Req { tentative: 2, emise_epoque: 1, ..m.req(m.script, 2, "sonos:salon/volume_plus", vec![], 5_100) };
    assert_eq!(copie.demander(t(5_100), retente), Err((issue::INCERTAIN, raison::AMBIGUE)));
}

/// I9 et I8 : les filles d'un groupe qui concluent aussitôt (des ordres du hub) ne font pas conclure
/// la mère avant la dernière ; des exécutions ne font pas une confirmation.
#[test]
fn un_groupe_attend_toutes_ses_cibles_meme_conclues_aussitot() {
    let g = ordre_de_groupe(FormeDecl::sans_param(effet::APPUI, retour::PONT));
    let mut m = Mini::new(&[
        sonnette("helpers:a", 0),
        sonnette("helpers:b", 0),
        groupe("groupe:sonnettes", vec![("appuyer", g)], vec![(sorte::ORDRE, "helpers:a/appuyer", role::CIBLE, "appuyer"), (sorte::ORDRE, "helpers:b/appuyer", role::CIBLE, "appuyer")]),
    ]);
    let r = m.req(m.script, 1, "groupe:sonnettes/appuyer", vec![], 1_000);
    m.hub.demander(t(1_000), r).expect("admise");
    let fin: Vec<_> = m.hub.res.iter().filter(|r| r.seq == 1).map(|r| (r.issue, r.raison, r.n, r.m)).collect();
    assert_eq!(fin, vec![(issue::EXECUTE, 0, 2, 2)], "deux sur deux, exécutées, pas confirmées");
}

/// I9 : une mère n'a pas d'échéance à elle ; elle ne réveille personne pendant le suivi de ses filles.
#[test]
fn une_mere_n_a_pas_d_echeance() {
    let mut accuse = FormeDecl::regler(binaire(), "on", 0);
    accuse.retour = retour::ACCUSE;
    accuse.suivi_s = 60;
    let cible = |n: &str| AppDecl::physique(n, 0, &["on"], vec![("on", accuse.clone())]);
    let mut m = Mini::new(&[
        cible("cloud:a"),
        cible("cloud:b"),
        groupe("groupe:cloud", vec![("on", ordre_de_groupe(accuse.clone()))], vec![(sorte::ORDRE, "cloud:a/on", role::CIBLE, "on"), (sorte::ORDRE, "cloud:b/on", role::CIBLE, "on")]),
    ]);
    m.rep("cloud:a", Rep::Accuse { accuse_ms: 100, etat_ms: 50_000, valeur: None });
    m.rep("cloud:b", Rep::Accuse { accuse_ms: 100, etat_ms: 50_000, valeur: None });
    let r = m.req(m.script, 1, "groupe:cloud/on", vec![Arg::N(1)], 1_000);
    let mere = m.hub.demander(t(1_000), r).expect("admise");
    m.tourner(1_000, 10_000);
    let d = m.hub.demandes.get(mere).expect("mère vivante");
    assert_eq!((d.etat, d.echeance_ms), (etat::MERE, u64::MAX));
    assert!(m.hub.prochaine(t(10_000)).is_some_and(|p| p > T0 + 10_000), "aucun réveil dû à la mère");
    m.tourner(10_000, 60_000);
    assert_eq!(m.issues(1).last(), Some(&(issue::CONFIRME, 0)), "les deux confirmées par leur état, dans la fenêtre de suivi");
}

/// I10 et I12 : une composition invalide, ou un plafond dépassé, est refusé à la déclaration.
#[test]
fn une_composition_invalide_est_refusee() {
    let on = ordre_de_groupe(FormeDecl::regler(binaire(), "", 0));
    let mut m = Mini::new(&[lampe("hue:a", 0, 254), lampe("hue:b", 0, 100)]);
    let tt = t(1_000);
    m.hub.declarer(tt, &groupe("groupe:g1", vec![("on", on.clone())], vec![(sorte::ORDRE, "hue:a/on", role::CIBLE, "on")])).expect("un groupe simple");
    let r = m.hub.declarer(tt, &groupe("groupe:g2", vec![("on", on.clone())], vec![(sorte::ORDRE, "groupe:g1/on", role::CIBLE, "on")]));
    assert_eq!(r, Err("une cible est l'ordre d'un appareil physique"), "pas de groupe de groupes");
    let lum = |max| ordre_de_groupe(FormeDecl::regler(mesure(0, max, 1, 0, 0), "", 2));
    let r = m.hub.declarer(tt, &groupe("groupe:g3", vec![("brightness", lum(254))], vec![(sorte::ORDRE, "hue:b/brightness", role::CIBLE, "brightness")]));
    assert_eq!(r, Err("une cible de forme incompatible avec son ordre de groupe"), "0–254 ne tient pas dans 0–100");
    assert!(m.hub.declarer(tt, &groupe("groupe:g4", vec![("brightness", lum(100))], vec![(sorte::ORDRE, "hue:a/brightness", role::CIBLE, "brightness")])).is_ok(), "0–100 tient dans 0–254");
    let r = m.hub.declarer(tt, &groupe("groupe:g5", vec![("on", FormeDecl::regler(binaire(), "", 0))], vec![(sorte::ORDRE, "hue:a/on", role::CIBLE, "on")]));
    assert_eq!(r, Err("la source d'une cible n'est pas un ordre de groupe"));
    let lampes: Vec<AppDecl> = (0..65).map(|k| lampe(&format!("hue:l{k}"), 0, 254)).collect();
    for l in &lampes {
        m.hub.declarer(tt, l).expect("lampe");
    }
    let noms: Vec<String> = (0..65).map(|k| format!("hue:l{k}/on")).collect();
    let r = m.hub.declarer(tt, &groupe("groupe:trop", vec![("on", on)], noms.iter().map(|n| (sorte::ORDRE, n.as_str(), role::CIBLE, "on")).collect()));
    assert_eq!(r, Err("plus de 64 cibles pour un ordre de groupe"));
    let gros = AppDecl::physique("z2m:gros", 0, &[], vec![]);
    let gros = AppDecl { points: (0..256).map(|k| format!("p{k}")).collect(), ..gros };
    assert_eq!(m.hub.declarer(tt, &gros), Err("plus de 255 valeurs, ordres ou membres"));
    assert!(m.hub.appareils.numero("z2m:gros").is_none() && m.hub.appareils.numero("groupe:trop").is_none(), "rien n'a été pris avant le refus");
}

/// I10 : deux ordres de groupe du même appareil virtuel commandent chacun leurs cibles.
#[test]
fn deux_ordres_de_groupe_ont_chacun_leurs_cibles() {
    let on = ordre_de_groupe(FormeDecl::regler(binaire(), "", 0));
    let lum = ordre_de_groupe(FormeDecl::regler(mesure(0, 100, 1, 0, 0), "", 2));
    let mut m = Mini::new(&[
        lampe("hue:a", 0, 254),
        lampe("hue:b", 0, 254),
        groupe(
            "groupe:salon",
            vec![("on", on), ("brightness", lum)],
            vec![
                (sorte::ORDRE, "hue:a/on", role::CIBLE, "on"),
                (sorte::ORDRE, "hue:b/on", role::CIBLE, "on"),
                (sorte::ORDRE, "hue:a/brightness", role::CIBLE, "brightness"),
            ],
        ),
    ]);
    let r = m.req(m.ecran, 1, "groupe:salon/on", vec![Arg::N(1)], 1_000);
    m.hub.demander(t(1_000), r).expect("admise");
    let ordres: Vec<String> = m.hub.cmd.iter().map(|c| m.hub.nom_ordre(c.ordre)).collect();
    assert_eq!(ordres, vec!["hue:a/on".to_string(), "hue:b/on".to_string()], "les cibles de « on », pas celles de la luminosité");
    m.hub.cmd.clear();
    let r = m.req(m.ecran, 2, "groupe:salon/brightness", vec![Arg::N(50)], 2_000);
    m.hub.demander(t(2_000), r).expect("admise");
    assert_eq!(m.hub.cmd.len(), 1);
}

/// I12 : la 51e retenue est refusée, et rien n'a bougé.
#[test]
fn les_plafonds_refusent_avant_de_muter() {
    let lampes: Vec<AppDecl> = (0..60).map(|k| lampe(&format!("hue:l{k}"), 3, 254)).collect();
    let mut m = Mini::new(&lampes);
    m.hub.pieces_protegees = vec![3];
    for k in 0..50u64 {
        let r = m.req(m.agent, k + 1, &format!("hue:l{k}/on"), vec![Arg::N(1)], 1_000);
        assert!(m.hub.demander(t(1_000), r).is_ok());
    }
    let avant = (m.hub.demandes.en_cours, m.hub.rang);
    let r = m.req(m.agent, 51, "hue:l50/on", vec![Arg::N(1)], 1_000);
    assert_eq!(m.hub.demander(t(1_000), r), Err((issue::REFUSE, raison::SATURE)));
    assert_eq!((m.hub.demandes.en_cours, m.hub.rang), avant, "aucune case prise, aucune sortie");
}

/// I1 : un permis révoqué retire des voies ce qui n'est pas parti ; ce qui est en vol va à son issue.
#[test]
fn une_revocation_retire_ce_qui_n_est_pas_parti() {
    let mut m = Mini::new(&[lampe("hue:a", 0, 254)]);
    m.rep("hue:a", Rep::Etat { ms: 2_000 });
    let r1 = m.req(m.script, 1, "hue:a/on", vec![Arg::N(1)], 1_000);
    m.hub.demander(t(1_000), r1).expect("admise");
    let r2 = m.req(m.script, 2, "hue:a/brightness", vec![Arg::N(80)], 1_000);
    m.hub.demander(t(1_000), r2).expect("admise");
    m.tourner(1_000, 1_100);
    assert_eq!(m.kit.envois.len(), 1, "l'un en vol, l'autre dans la voie");
    m.hub.revoquer(t(1_200), m.script);
    assert!(m.hub.sorties.iter().any(|p| matches!(p.sortie, Sortie::Revocation { .. })), "la révocation est une sortie critique");
    m.tourner(1_200, 4_000);
    assert_eq!(m.kit.envois.len(), 1, "le second n'est jamais parti");
    assert_eq!(m.issues(2), vec![(issue::NON_ENVOYE, raison::DROITS)]);
    assert_eq!(m.issues(1).last(), Some(&(issue::CONFIRME, 0)), "celui qui était en vol va à son issue");
    let r3 = m.req(m.script, 3, "hue:a/on", vec![Arg::N(0)], 4_000);
    assert_eq!(m.hub.demander(t(4_000), r3), Err((issue::REFUSE, raison::DROITS)));
    let _ = Ref(0);
}

/// I1 : l'approbation revérifie le permis de celui qui demandait (ses pièces ont changé depuis).
#[test]
fn une_approbation_reverifie_le_permis() {
    let porte = AppDecl::physique("nuki:porte", 5, &["lock"], vec![("tirer", FormeDecl { confirme: "lock".into(), attendus: vec![3], drapeaux: drapeau::OUVRE, ..FormeDecl::sans_param(effet::APPUI, retour::ETAT) })]);
    let mut m = Mini::new(&[porte]);
    m.hub.principaux[usize::from(m.agent)].pieces = crate::droits::pieces(&[5]);
    let r = m.req(m.agent, 1, "nuki:porte/tirer", vec![], 1_000);
    let x = m.hub.demander(t(1_000), r).expect("retenue");
    assert_eq!(m.issues(1), vec![(issue::RETENUE, raison::GARDE)]);
    // L'agent n'a plus l'entrée (une configuration changée, sans fin de permis datée).
    m.hub.principaux[usize::from(m.agent)].pieces = crate::droits::pieces(&[6]);
    let o = m.hub.ordre("nuki:porte/tirer");
    let f = m.hub.cartes[&x].revision;
    assert!(m.hub.decider(t(2_000), x, o, f, true, m.ecran).is_ok());
    assert_eq!(m.issues(1).last(), Some(&(issue::REFUSE, raison::DROITS)));
    assert!(m.hub.cmd.is_empty() && m.hub.attente_secours.is_empty(), "rien n'est parti");
}

/// I4 : une lecture commencée avant l'envoi, livrée après, ne prouve pas l'ordre en vol.
#[test]
fn une_lecture_commencee_avant_l_envoi_ne_prouve_rien() {
    let mut m = Mini::new(&[lampe("hue:a", 0, 254)]);
    m.rep("hue:a", Rep::Muet);
    let p = m.hub.point("hue:a/on");
    // Une lecture de 900 ms, qui trouve la lampe allumée, arrive à 1 100 ms.
    m.monde.prevoir(T0 + 1_100, crate::module::Quoi::Valeur { point: p, v: 1, mouvement: false, lu_ms: T0 + 900 });
    let r = m.req(m.script, 1, "hue:a/on", vec![Arg::N(1)], 1_000);
    m.hub.demander(t(1_000), r).expect("admise");
    m.tourner(1_000, 8_000);
    assert_eq!(m.issues(1), vec![(issue::ENVOYE, 0)], "aucune lecture fraîche : envoyé, pas confirmé");
}
