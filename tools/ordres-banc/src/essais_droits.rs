//! Les 29 essais ciblés de la v3 : les droits sur les ordres (revue Codex des droits, D1 à D9, M1 à M8,
//! W1 à W6). Chacun monte un petit hub (`essais::Mini`), parfois son moteur et une copie, et vérifie
//! une règle ; la soirée rejoue le reste.

use crate::catalogue::{Arg, FormeDecl, binaire, drapeau, effet, mesure, raison, retour, texte};
use crate::demandes::{etat, issue, origine};
use crate::droits::{Personne, TOUTES_PIECES, classe, pieces, preuve, role as rp};
use crate::essais::{Mini, T0, groupe, haut_parleur, lampe, ordre_de_groupe, sonnette, t};
use crate::hub::{AppDecl, AutoriteMoteur, Cible, Hub, PERSONNES_MAX, PIECE_PRIVEE, PRINCIPAUX_MAX, Principal, Req, Sortie, empreinte_args, role, sorte};
use crate::module::{Quoi, Rep};
use crate::moteur::{DeclDef, Def, Moteur, PasDef, Regle, Responsabilite, etat_ex, genre};

// ---- la maison des essais ----------------------------------------------------------------------

/// Bob (propriétaire, 1), Alice (adulte, 2), Charlie (enfant, 3 : le salon et sa chambre) ; l'écran
/// de Bob, un agent, un script, la tablette partagée du salon, un satellite vocal, le téléphone de
/// Alice. Les chambres (3, 4) sont protégées.
fn maison(decls: &[AppDecl]) -> Mini {
    let mut m = Mini::new(decls);
    let p = |r: u8, ps: u64| Personne { pieces: ps, fin_mur_ms: u64::MAX, role: r, ..Personne::default() };
    m.hub.ajouter_personne("Alice", p(rp::ADULTE, TOUTES_PIECES)).expect("personne");
    m.hub.ajouter_personne("Charlie", p(rp::ENFANT, pieces(&[0, 3]))).expect("personne");
    m.hub.ajouter_principal(Principal::tablette("tablette du salon")).expect("principal");
    m.hub.ajouter_principal(Principal::satellite("satellite du salon")).expect("principal");
    let alice = m.hub.personne("Alice");
    m.hub.ajouter_principal(Principal::humain("téléphone d'Alice", u32::from(alice))).expect("principal");
    m.hub.pieces_protegees = vec![3, 4];
    m
}

fn compte(m: &Mini, nom: &str) -> u16 {
    m.hub.principaux.iter().position(|p| p.nom == nom).expect("compte") as u16
}

/// Le sort d'une demande à son arrivée : partie (en file), retenue, ou refusée.
fn essai_req(m: &mut Mini, r: Req, ms: u64) -> Result<u8, (u8, u8)> {
    let x = m.hub.demander(t(ms), r)?;
    Ok(m.hub.demandes.get(x).map_or(etat::LIBRE, |d| d.etat))
}

fn essai(m: &mut Mini, de: u16, seq: u64, ordre: &str, args: Vec<Arg>, ms: u64) -> Result<u8, (u8, u8)> {
    let r = m.req(de, seq, ordre, args, ms);
    essai_req(m, r, ms)
}

const PART: Result<u8, (u8, u8)> = Ok(etat::EN_FILE);
const RETENUE: Result<u8, (u8, u8)> = Ok(etat::RETENUE);
const REFUS: Result<u8, (u8, u8)> = Err((issue::REFUSE, raison::DROITS));

fn porte() -> AppDecl {
    AppDecl::physique("nuki:porte", 5, &["lock"], vec![("tirer", FormeDecl { confirme: "lock".into(), attendus: vec![3], drapeaux: drapeau::OUVRE, ..FormeDecl::sans_param(effet::APPUI, retour::ETAT) })])
}

fn camera() -> AppDecl {
    AppDecl::physique("cam:salon", 0, &["on"], vec![("on", FormeDecl { classe: classe::VIE_PRIVEE, ..FormeDecl::regler(binaire(), "on", 0) })])
}

fn thermostat() -> AppDecl {
    AppDecl::physique("tado:salon", 0, &["consigne"], vec![("consigne", FormeDecl { classe: classe::CLIMAT, ..FormeDecl::regler(mesure(50, 250, 5, -1, 1), "consigne", 0) })])
}

/// Un appareil d'un seul ordre, un appui de la classe donnée.
fn appui(nom: &str, piece: u16, cle: &str, c: u8) -> AppDecl {
    AppDecl::physique(nom, piece, &[], vec![(cle, FormeDecl { classe: c, ..FormeDecl::sans_param(effet::APPUI, retour::PONT) })])
}

fn alarme() -> AppDecl {
    let f = |c| FormeDecl { classe: c, ..FormeDecl::sans_param(effet::APPUI, retour::PONT) };
    AppDecl::physique("alarme:maison", 5, &[], vec![("armer", f(classe::ALARME)), ("desarmer", f(classe::DESARMER))])
}

/// Une machine au travail qu'on arrête (une imprimante 3D) : la classe « machine », par son drapeau.
fn machine(nom: &str, piece: u16) -> AppDecl {
    AppDecl::physique(nom, piece, &[], vec![("arreter", FormeDecl { drapeaux: drapeau::CONTROLE, ..FormeDecl::sans_param(effet::APPUI, retour::PONT) })])
}

/// Les dernières commandes qu'un écran montre comme « miennes » à `lecteur`, par le JSON client.
fn miennes(h: &Hub, lecteur: u16) -> Vec<String> {
    let json = h.ranger_client_json(&|_| true, &|s: &str| s.to_string(), lecteur);
    let mut v = Vec::new();
    for app in json.split("{\"n\":").skip(1) {
        let nom = app.split("\"nom\":\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or("");
        let ordres = app.split("\"ordres\":[").nth(1).and_then(|s| s.split("],\"compo\"").next()).unwrap_or("");
        for o in ordres.split("],[") {
            let champs: Vec<&str> = o.trim_matches(|c| c == '[' || c == ']').split(',').collect();
            if champs.len() == 10 && champs[9] == "1" {
                v.push(format!("{nom}/{}", champs[1].trim_matches('"')));
            }
        }
    }
    v
}

// ---- un hub, son moteur, une copie ---------------------------------------------------------------

/// Assez de sorties pour toute la soirée d'un essai ; au-delà, un moteur qui tourne sans fin échoue
/// au lieu de bloquer le banc.
const BORNE: usize = 20_000;

struct Banc {
    m: Mini,
    moteur: Moteur,
    /// Le pair : il applique chaque sortie de l'actif, dans l'ordre de `s`, son moteur aussi.
    copie: Option<(Hub, Moteur)>,
    vues: usize,
}

impl Banc {
    fn new(decls: &[AppDecl], defs: Vec<Def>, resp: Vec<(&str, Responsabilite)>) -> Banc {
        let mut m = maison(decls);
        let de = m.hub.ajouter_principal(Principal::interne("moteur", origine::AUTOMATISME)).expect("principal");
        let mut moteur = Moteur { actif: true, de, responsable_defaut: "Bob".into(), defs, ..Moteur::default() };
        moteur.responsabilites = resp.into_iter().map(|(n, r)| (n.to_string(), r)).collect();
        moteur.compiler(&mut m.hub);
        Banc { m, moteur, copie: None, vues: 0 }
    }

    /// Le pair, depuis l'état complet de l'actif et l'image de son moteur ; il suit le flux ensuite.
    fn attacher_copie(&mut self, ms: u64) {
        self.propager(ms);
        let mut h = Hub::relire(&self.m.hub.ranger(t(ms)), t(ms), self.m.hub.principaux.clone()).expect("état complet");
        h.pieces_protegees = self.m.hub.pieces_protegees.clone();
        let mut mo = Moteur::relire(&self.moteur.ranger(), self.moteur.de).expect("moteur");
        mo.actif = false;
        mo.responsable_defaut = self.moteur.responsable_defaut.clone();
        mo.responsabilites = self.moteur.responsabilites.clone();
        mo.defs = self.moteur.defs.clone();
        mo.compiler(&mut h);
        self.copie = Some((h, mo));
    }

    /// Comme la soirée : les annulations et les ordres vont au kit, les sorties de l'actif à son moteur
    /// (les valeurs, les traces) et au pair, jusqu'à ce qu'il n'y ait plus rien.
    fn propager(&mut self, ms: u64) {
        let tt = t(ms);
        loop {
            let annulees = std::mem::take(&mut self.m.hub.annulations);
            if !annulees.is_empty() {
                self.m.kit.annuler(&mut self.m.hub, tt, &annulees);
            }
            for c in self.m.hub.remettre(tt) {
                self.m.kit.recevoir(&mut self.m.hub, &mut self.m.monde, tt, c);
            }
            let sorties = std::mem::take(&mut self.m.hub.sorties);
            if sorties.is_empty() && self.m.hub.cmd.is_empty() {
                return;
            }
            for p in sorties {
                self.vues += 1;
                assert!(self.vues < BORNE, "le moteur tourne sans fin");
                donner(&mut self.moteur, &mut self.m.hub, t(ms), &p.sortie);
                if let Some((h, mo)) = &mut self.copie {
                    h.appliquer(&p, t(ms)).expect("la copie applique la sortie");
                    mo.appliquer(&p.sortie);
                }
            }
        }
    }

    /// Le temps passe : les ordres au kit, les réponses des appareils, les échéances du hub et du
    /// moteur ; réveil au plus proche.
    fn tourner(&mut self, de_ms: u64, a_ms: u64) {
        let mut ms = de_ms;
        loop {
            let tt = t(ms);
            self.propager(ms);
            for q in self.m.monde.echus(tt.mur_ms) {
                self.m.kit.evenement(&mut self.m.hub, &mut self.m.monde, tt, q);
                self.propager(ms);
            }
            self.m.kit.tic(&mut self.m.hub, &mut self.m.monde, tt);
            self.m.hub.tic(tt);
            self.moteur.echeances(&mut self.m.hub, tt);
            self.propager(ms);
            if ms >= a_ms {
                break;
            }
            let suivant = [self.m.monde.prochain(), self.m.hub.prochaine(tt), self.m.kit.prochaine(tt), self.moteur.prochaine()].into_iter().flatten().map(|m| m - T0).filter(|&m| m > ms).min().unwrap_or(a_ms);
            ms = suivant.min(a_ms);
        }
    }

    fn parts(&self, nom: &str) -> usize {
        self.moteur.journal.iter().filter(|(_, a, x)| a == nom && x.starts_with("part")).count()
    }
}

/// Ce que le moteur lit des sorties : les valeurs, les traces des ordres.
fn donner(moteur: &mut Moteur, hub: &mut Hub, tt: crate::hub::T, s: &Sortie) {
    match s {
        Sortie::Valeur { point, valeur, .. } => moteur.changement(hub, tt, *point, *valeur),
        Sortie::Trace { d, demande, ordre, .. } => {
            let i = if matches!(demande.etat, etat::LIBRE | etat::SUIVIE | etat::RETENUE) { demande.issue } else { issue::EN_COURS };
            moteur.trace(hub, tt, *d, *ordre, i, demande.etat == etat::LIBRE);
        }
        _ => {}
    }
}

fn routine(nom: &str, pas: Vec<PasDef>) -> Def {
    Def { nom: nom.into(), declencheur: DeclDef::Main, pas }
}

fn commander(ordre: &str, args: Vec<Arg>) -> PasDef {
    PasDef::Commander(ordre.into(), args)
}

// ---- l'identité : qui demande, pour qui, avec quelle preuve ---------------------------------------

/// D1 : une tablette partagée n'est personne tant qu'aucune session personnelle n'y est ouverte (le
/// profil « inconnu ») ; le PIN personnel de Charlie lui donne ses droits à elle, pas plus, pour quelques
/// minutes ; à la fin de la session, la tablette redevient anonyme d'elle-même.
#[test]
fn une_tablette_partagee_n_est_personne_sans_session() {
    let mut m = maison(&[lampe("hue:salon", 0, 254), lampe("hue:charlie", 3, 254), lampe("hue:cuisine", 1, 254), porte(), camera()]);
    let tab = compte(&m, "tablette du salon");
    let charlie = m.hub.personne("Charlie");
    assert_eq!(essai(&mut m, tab, 1, "hue:salon/on", vec![Arg::N(1)], 1_000), PART);
    assert_eq!(essai(&mut m, tab, 2, "nuki:porte/tirer", vec![], 1_000), RETENUE, "anonyme : la porte se propose");
    assert_eq!(essai(&mut m, tab, 3, "hue:charlie/on", vec![Arg::N(1)], 1_000), RETENUE, "anonyme : une chambre se propose");
    assert_eq!(essai(&mut m, tab, 4, "cam:salon/on", vec![Arg::N(1)], 1_000), REFUS, "anonyme : jamais une caméra");
    let qui = |m: &Mini, seq| {
        let d = m.hub.demandes.get(m.hub.demandes.par_cle(tab, seq).expect("demande")).expect("vivante");
        (d.pour, d.preuve)
    };
    assert_eq!(qui(&m, 1), (0, preuve::AUCUNE));
    // Charlie tape son PIN personnel : cinq minutes à elle.
    let fin = T0 + 2_000 + 300_000;
    m.hub.ouvrir_session(tab, charlie, preuve::PERSONNELLE, fin);
    assert!(m.hub.sorties.iter().any(|p| matches!(p.sortie, Sortie::Session { .. }) && p.piece == PIECE_PRIVEE), "la session ne va qu'au pair");
    assert_eq!(essai(&mut m, tab, 5, "hue:charlie/on", vec![Arg::N(1)], 2_000), PART, "sa chambre, avec sa preuve");
    assert_eq!(qui(&m, 5), (charlie, preuve::PERSONNELLE));
    assert_eq!(essai(&mut m, tab, 6, "hue:cuisine/on", vec![Arg::N(1)], 2_000), REFUS, "la cuisine n'est pas à elle : ses gestes sont les siens");
    assert_eq!(essai(&mut m, tab, 7, "nuki:porte/tirer", vec![], 2_000), REFUS, "l'entrée non plus : pas même proposée");
    assert_eq!(essai(&mut m, tab, 8, "cam:salon/on", vec![Arg::N(1)], 2_000), REFUS);
    // Sa session finit : la tablette redevient anonyme, sans qu'on ait rien à faire.
    assert_eq!(essai(&mut m, tab, 9, "hue:charlie/on", vec![Arg::N(0)], fin - T0), RETENUE);
    assert_eq!(qui(&m, 9), (0, preuve::AUCUNE));
    // Une session fermée l'est tout de suite.
    m.hub.ouvrir_session(tab, charlie, preuve::PERSONNELLE, u64::MAX);
    m.hub.fermer_session(tab);
    assert_eq!(essai(&mut m, tab, 10, "hue:charlie/on", vec![Arg::N(1)], fin - T0), RETENUE);
}

/// D1 et la voix : un satellite ne sait donner qu'un indice de qui parle. Pour le confort, l'indice vaut
/// sa personne (le choix du propriétaire) : la voix de Charlie n'a que les droits de Charlie ; pour le
/// sensible, jamais plus qu'une voix inconnue : la voix reconnue du propriétaire n'ouvre pas.
#[test]
fn une_voix_reconnue_ne_sert_que_le_confort() {
    let mut m = maison(&[lampe("hue:cuisine", 1, 254), thermostat(), porte(), camera()]);
    let sat = compte(&m, "satellite du salon");
    let (bob, charlie) = (m.hub.personne("Bob"), m.hub.personne("Charlie"));
    let voix = |m: &mut Mini, seq, ordre: &str, args, indice| {
        let r = Req { indice, ..m.req(sat, seq, ordre, args, 1_000) };
        essai_req(m, r, 1_000)
    };
    assert_eq!(voix(&mut m, 1, "hue:cuisine/on", vec![Arg::N(1)], 0), PART, "une voix inconnue : le confort");
    assert_eq!(voix(&mut m, 2, "tado:salon/consigne", vec![Arg::N(200)], 0), PART);
    assert_eq!(voix(&mut m, 3, "nuki:porte/tirer", vec![], 0), RETENUE);
    assert_eq!(voix(&mut m, 4, "cam:salon/on", vec![Arg::N(1)], 0), REFUS);
    assert_eq!(voix(&mut m, 5, "nuki:porte/tirer", vec![], bob), RETENUE, "la voix du propriétaire n'ouvre pas plus");
    assert_eq!(voix(&mut m, 6, "cam:salon/on", vec![Arg::N(1)], bob), REFUS);
    let d = m.hub.demandes.get(m.hub.demandes.par_cle(sat, 5).expect("demande")).expect("vivante");
    assert_eq!((d.pour, d.preuve), (bob, preuve::INDICE), "l'indice est rangé comme tel");
    assert_eq!(voix(&mut m, 7, "hue:cuisine/on", vec![Arg::N(0)], charlie), REFUS, "la voix de Charlie : ses pièces");
    assert_eq!(voix(&mut m, 8, "tado:salon/consigne", vec![Arg::N(220)], charlie), RETENUE, "et ses classes");
    // Une tablette qui croit reconnaître un visage : un indice aussi ; son canal commande tout, la porte
    // reste proposée.
    let tab = compte(&m, "tablette du salon");
    let r = Req { indice: bob, ..m.req(tab, 1, "nuki:porte/tirer", vec![], 1_000) };
    assert_eq!(essai_req(&mut m, r, 1_000), RETENUE, "un visage reconnu n'ouvre pas plus qu'un inconnu");
}

/// B1 et W1 : l'autorité d'une demande est établie par le serveur : un client ne se dit ni exécution du
/// moteur, ni quelqu'un d'autre, ni fille d'un groupe.
#[test]
fn un_client_ne_declare_pas_son_autorite() {
    let mut m = maison(&[porte()]);
    let bob = m.hub.personne("Bob");
    let menteuse = AutoriteMoteur { appele: true, de: m.ecran, pour: bob, preuve: preuve::PERSONNELLE, mandat: 1 };
    let r = Req { autorite: Some(menteuse), qui: 999, ..m.req(m.script, 1, "nuki:porte/tirer", vec![], 1_000) };
    let x = m.hub.demander(t(1_000), r).expect("retenue");
    let d = *m.hub.demandes.get(x).expect("vivante");
    assert_eq!((d.etat, d.de, d.pour, d.mandat, d.qui), (etat::RETENUE, m.script, 0, 0, 3), "le script, anonyme : ce qu'il déclare ne compte pas");
    // Une fille de groupe naît du répartiteur : se dire fille d'une demande vivante ne donne rien.
    let r = Req { parent: x, ..m.req(m.script, 2, "nuki:porte/tirer", vec![], 1_100) };
    assert_eq!(m.hub.demander(t(1_100), r), Err((issue::REFUSE, raison::DROITS)));
}

/// D2 : un invité commande l'éclairage de sa pièce, mais la serrure de cette même pièce se propose :
/// ses droits sont par classe, pas seulement par pièce ; l'origine « écran » n'y change rien ; et il
/// n'approuve pas sa propre demande.
#[test]
fn un_invite_ne_fait_pas_tout_dans_sa_piece() {
    let mut m = maison(&[lampe("hue:entree", 5, 254), porte()]);
    let inv = m.hub.ajouter_personne("invité", Personne { pieces: pieces(&[5]), fin_mur_ms: u64::MAX, role: rp::INVITE, ..Personne::default() }).expect("personne");
    let tel = m.hub.ajouter_principal(Principal::invite("téléphone de l'invité", u32::from(inv), vec![5], u64::MAX)).expect("principal");
    assert_eq!(essai(&mut m, tel, 1, "hue:entree/on", vec![Arg::N(1)], 1_000), PART);
    assert_eq!(essai(&mut m, tel, 2, "nuki:porte/tirer", vec![], 1_000), RETENUE, "sa pièce, mais une ouverture");
    let x = m.hub.demandes.par_cle(tel, 2).expect("retenue");
    let (o, rev) = (m.hub.ordre("nuki:porte/tirer"), m.hub.cartes[&x].revision);
    assert_eq!(m.hub.decider(t(2_000), x, o, rev, true, tel), Err("ce décideur n'est pas habilité pour cette opération"));
    assert_eq!(m.hub.decider(t(2_000), x, o, rev, true, m.ecran), Ok(x), "le propriétaire, si");
}

/// D5 : un agent qui agit pour une personne garde les plafonds de cette personne et ceux de son canal ;
/// la personne confirme elle-même ce qu'elle a demandé à son agent. Une exception personnelle
/// (Alice n'a pas le média) vaut quel que soit son rôle.
#[test]
fn un_agent_garde_les_plafonds_de_sa_personne_et_de_son_canal() {
    let mut m = maison(&[lampe("hue:salon", 0, 254), lampe("hue:charlie", 3, 254), porte(), camera(), appui("nas:serveur", 1, "redemarrer", classe::SYSTEME), appui("sonos:cuisine", 1, "suivant", classe::MEDIA)]);
    let (alice, charlie) = (m.hub.personne("Alice"), m.hub.personne("Charlie"));
    let tel = compte(&m, "téléphone d'Alice");
    let pour_alice = m.hub.ajouter_principal(Principal::agent("agent d'Alice", origine::MCP, 20).pour(alice)).expect("principal");
    let pour_lea = m.hub.ajouter_principal(Principal::agent("agent de Charlie", origine::MCP, 21).pour(charlie)).expect("principal");
    assert_eq!(essai(&mut m, pour_alice, 1, "hue:salon/on", vec![Arg::N(1)], 1_000), PART);
    assert_eq!(essai(&mut m, pour_alice, 2, "nuki:porte/tirer", vec![], 1_000), RETENUE, "son canal propose ce qu'Alice ferait");
    assert_eq!(essai(&mut m, pour_alice, 3, "nas:serveur/redemarrer", vec![], 1_000), REFUS, "son canal ne touche pas au système");
    assert_eq!(essai(&mut m, pour_alice, 4, "cam:salon/on", vec![Arg::N(1)], 1_000), RETENUE);
    assert_eq!(essai(&mut m, pour_lea, 1, "nuki:porte/tirer", vec![], 1_000), REFUS, "Charlie n'a pas l'entrée");
    assert_eq!(essai(&mut m, pour_lea, 2, "hue:charlie/on", vec![Arg::N(1)], 1_000), RETENUE, "une chambre : un humain, même pour Charlie");
    // Alice confirme sur son téléphone ce qu'elle a demandé à son agent ; Charlie ne confirme pas pour elle.
    let x = m.hub.demandes.par_cle(pour_alice, 4).expect("retenue");
    let (o, rev) = (m.hub.ordre("cam:salon/on"), m.hub.cartes[&x].revision);
    let tab = compte(&m, "tablette du salon");
    m.hub.ouvrir_session(tab, charlie, preuve::PERSONNELLE, u64::MAX);
    assert!(m.hub.decider(t(2_000), x, o, rev, true, tab).is_err());
    assert_eq!(m.hub.decider(t(2_000), x, o, rev, true, tel), Ok(x));
    // L'exception personnelle.
    m.hub.personnes[usize::from(alice)].refuse = classe::bit(classe::MEDIA);
    assert_eq!(essai(&mut m, tel, 5, "sonos:cuisine/suivant", vec![], 3_000), REFUS);
}

// ---- l'approbation ---------------------------------------------------------------------------------

/// D3 et M2 : une retenue ne se décide que par quelqu'un d'habilité pour cette opération et ces pièces,
/// depuis un écran, avec une preuve personnelle ; la ligne de la décision nomme l'approbateur, pas le
/// demandeur ; `ord/` ne la nomme pas, le pair si.
#[test]
fn seul_un_approbateur_habilite_decide() {
    let mut m = maison(&[porte()]);
    let tab = compte(&m, "tablette du salon");
    let tel = compte(&m, "téléphone d'Alice");
    let (alice, charlie) = (m.hub.personne("Alice"), m.hub.personne("Charlie"));
    let r = m.req(m.agent, 1, "nuki:porte/tirer", vec![], 1_000);
    let x = m.hub.demander(t(1_000), r).expect("retenue");
    let (o, rev) = (m.hub.ordre("nuki:porte/tirer"), m.hub.cartes[&x].revision);
    let non = Err("ce décideur n'est pas habilité pour cette opération");
    assert_eq!(m.hub.decider(t(2_000), x, o, rev, true, m.agent), non, "un agent ne décide pas");
    assert_eq!(m.hub.decider(t(2_000), x, o, rev, true, tab), non, "une tablette sans personne non plus");
    m.hub.ouvrir_session(tab, charlie, preuve::PERSONNELLE, u64::MAX);
    assert_eq!(m.hub.decider(t(2_000), x, o, rev, true, tab), non, "ni un enfant qui sait cliquer");
    m.hub.sorties.clear();
    assert_eq!(m.hub.decider(t(3_000), x, o, rev, true, tel), Ok(x), "Alice, adulte, sur son téléphone");
    let l = *m.hub.journal.back().expect("ligne");
    assert_eq!((l.de, l.pour, l.origine, l.action), (tel, alice, origine::UI, 3), "la décision est au nom d'Alice");
    assert!(m.hub.sorties.iter().all(|p| !matches!(&p.sortie, Sortie::Trace { ligne, .. } if ligne.pour != 0)));
    assert!(m.hub.sorties.iter().any(|p| matches!(p.sortie, Sortie::Autorite { ligne_pour, .. } if ligne_pour == alice)));
    assert_eq!(m.hub.demandes.get(x).map(|d| d.etat), Some(etat::EN_FILE));
}

/// D4 : une décision nomme la révision de la carte qu'elle a vue, et ne sert qu'une fois ; la carte
/// renouvelée (ses cibles ont changé) ne s'approuve que sous sa nouvelle révision.
#[test]
fn une_ancienne_approbation_ne_vaut_pas_pour_la_carte_renouvelee() {
    let on = ordre_de_groupe(FormeDecl::regler(binaire(), "", 0));
    let cible = |n: &'static str| (sorte::ORDRE, n, role::CIBLE, "on");
    let mut m = maison(&[lampe("hue:a", 3, 254), lampe("hue:b", 3, 254), groupe("groupe:chambre", vec![("on", on.clone())], vec![cible("hue:a/on")])]);
    let r = m.req(m.agent, 1, "groupe:chambre/on", vec![Arg::N(1)], 1_000);
    let x = m.hub.demander(t(1_000), r).expect("retenue : une chambre, un canal gardé");
    let o = m.hub.ordre("groupe:chambre/on");
    let r1 = m.hub.cartes[&x].revision;
    // B rejoint A, sous la même forme.
    m.hub.declarer(t(1_500), &groupe("groupe:chambre", vec![("on", on)], vec![cible("hue:a/on"), cible("hue:b/on")])).expect("redéclaration");
    assert_eq!(m.hub.decider(t(2_000), x, o, r1, true, m.ecran), Ok(x), "le premier clic présente la nouvelle carte");
    let r2 = m.hub.cartes[&x].revision;
    assert_ne!(r2, r1);
    assert_eq!(m.issues(1).last(), Some(&(issue::RETENUE, raison::FORME_CHANGEE)));
    assert_eq!(m.hub.decider(t(2_100), x, o, r1, true, m.ecran), Err("la carte montrée n'est plus la bonne"), "le même clic, rejoué, ne vaut rien");
    assert!(m.hub.cmd.is_empty());
    assert_eq!(m.hub.decider(t(2_200), x, o, r2, true, m.ecran), Ok(x), "la nouvelle carte, vue, s'approuve");
    assert_eq!(m.hub.cmd.len(), 2, "A et B");
    assert_eq!(m.hub.decider(t(2_300), x, o, r2, true, m.ecran), Err("pas une retenue"), "une seule fois");
}

/// Le choix du propriétaire (v3 § 22) : un groupe dont une cible est interdite n'est ni refusé en bloc,
/// ni exécuté en partie d'office ; le sous-ensemble permis est annoncé, et seul le demandeur le confirme.
/// Une chambre dans ce sous-ensemble, par un canal gardé, attend ensuite un humain habilité ; un refus
/// ne fait rien partir ; un groupe qui change pendant le choix présente le sous-ensemble du moment.
#[test]
fn un_groupe_avec_une_cible_interdite_annonce_le_sous_ensemble() {
    let on = ordre_de_groupe(FormeDecl::regler(binaire(), "", 0));
    let cible = |n: &'static str| (sorte::ORDRE, n, role::CIBLE, "on");
    let groupe_ = |cibles: Vec<&'static str>| groupe("groupe:maison", vec![("on", on.clone())], cibles.into_iter().map(cible).collect());
    let mut m = maison(&[lampe("hue:salon", 0, 254), lampe("hue:bureau", 1, 254), lampe("hue:charlie", 3, 254), lampe("hue:cave", 2, 254), groupe_(vec!["hue:salon/on", "hue:bureau/on", "hue:charlie/on"])]);
    let tab = compte(&m, "tablette du salon");
    let charlie = m.hub.personne("Charlie");
    m.hub.ouvrir_session(tab, charlie, preuve::PERSONNELLE, u64::MAX);
    // Charlie (le salon et sa chambre) : deux cibles sur trois, annoncées ; rien ne part avant son choix.
    let r = m.req(tab, 1, "groupe:maison/on", vec![Arg::N(1)], 1_000);
    let x = m.hub.demander(t(1_000), r).expect("annoncé");
    assert_eq!(m.issues(1), vec![(issue::RETENUE, raison::SOUS_ENSEMBLE)]);
    assert_eq!(m.hub.res.iter().find(|r| r.seq == 1).map(|r| (r.n, r.m)), Some((2, 3)), "deux cibles sur trois");
    assert!(m.hub.cmd.is_empty());
    let (o, rev) = (m.hub.ordre("groupe:maison/on"), m.hub.cartes[&x].revision);
    assert_eq!(m.hub.decider(t(2_000), x, o, rev, true, m.ecran), Err("seul le demandeur choisit le sous-ensemble"), "un autre ne choisit pas pour lui");
    assert_eq!(m.hub.decider(t(2_000), x, o, rev, true, tab), Ok(x), "Charlie confirme");
    let ordres: Vec<String> = m.hub.cmd.iter().map(|c| m.hub.nom_ordre(c.ordre)).collect();
    assert_eq!(ordres, vec!["hue:salon/on".to_string(), "hue:charlie/on".to_string()], "jamais le bureau");
    m.hub.cmd.clear();
    // L'agent de Charlie : le même sous-ensemble ; il le confirme ; sa chambre (un canal gardé) attend un
    // humain habilité, sur une carte neuve.
    let agent = m.hub.ajouter_principal(Principal::agent("agent de Charlie", origine::MCP, 22).pour(charlie)).expect("principal");
    let r = m.req(agent, 1, "groupe:maison/on", vec![Arg::N(0)], 3_000);
    let y = m.hub.demander(t(3_000), r).expect("annoncé");
    let rev = m.hub.cartes[&y].revision;
    assert_eq!(m.hub.decider(t(3_100), y, o, rev, true, agent), Ok(y));
    assert_eq!(m.hub.demandes.get(y).map(|d| (d.etat, d.raison)), Some((etat::RETENUE, raison::GARDE)), "confirmer n'est pas approuver");
    assert!(m.hub.cmd.is_empty());
    assert_eq!(m.hub.decider(t(3_150), y, o, rev, true, m.ecran), Err("la carte montrée n'est plus la bonne"));
    let rev = m.hub.cartes[&y].revision;
    assert_eq!(m.hub.decider(t(3_200), y, o, rev, true, m.ecran), Ok(y), "Bob approuve");
    assert_eq!(m.hub.cmd.len(), 2, "le salon et la chambre, jamais le bureau");
    m.hub.cmd.clear();
    // Le demandeur refuse : rien ne part.
    let r = m.req(tab, 2, "groupe:maison/on", vec![Arg::N(0)], 4_000);
    let z = m.hub.demander(t(4_000), r).expect("annoncé");
    let rev = m.hub.cartes[&z].revision;
    assert_eq!(m.hub.decider(t(4_100), z, o, rev, false, tab), Ok(z));
    assert_eq!(m.issues(2).last(), Some(&(issue::REFUSE, raison::REFUSEE)));
    assert!(m.hub.cmd.is_empty());
    // Le groupe change pendant le choix. La cave le rejoint : ce qui a été montré part, et seulement cela.
    let r = m.req(tab, 3, "groupe:maison/on", vec![Arg::N(1)], 5_000);
    let w = m.hub.demander(t(5_000), r).expect("annoncé");
    let rev = m.hub.cartes[&w].revision;
    m.hub.declarer(t(5_050), &groupe_(vec!["hue:salon/on", "hue:cave/on", "hue:charlie/on"])).expect("redéclaration");
    assert_eq!(m.hub.decider(t(5_100), w, o, rev, true, tab), Ok(w));
    let ordres: Vec<String> = m.hub.cmd.iter().map(|c| m.hub.nom_ordre(c.ordre)).collect();
    assert_eq!(ordres, vec!["hue:salon/on".to_string(), "hue:charlie/on".to_string()], "jamais la cave");
    m.hub.cmd.clear();
    // Sa chambre le quitte : le sous-ensemble du moment, à choisir de nouveau.
    let r = m.req(tab, 4, "groupe:maison/on", vec![Arg::N(0)], 5_500);
    let w = m.hub.demander(t(5_500), r).expect("annoncé");
    let rev = m.hub.cartes[&w].revision;
    m.hub.declarer(t(5_550), &groupe_(vec!["hue:salon/on", "hue:cave/on"])).expect("redéclaration");
    assert_eq!(m.hub.decider(t(5_600), w, o, rev, true, tab), Ok(w));
    assert_eq!(m.issues(4).last(), Some(&(issue::RETENUE, raison::SOUS_ENSEMBLE)), "à choisir de nouveau");
    assert!(m.hub.cmd.is_empty());
    let rev = m.hub.cartes[&w].revision;
    assert_eq!(m.hub.decider(t(5_700), w, o, rev, true, tab), Ok(w));
    let ordres: Vec<String> = m.hub.cmd.iter().map(|c| m.hub.nom_ordre(c.ordre)).collect();
    assert_eq!(ordres, vec!["hue:salon/on".to_string()]);
    m.hub.cmd.clear();
    // Aucune cible permise : refusé, sans rien annoncer.
    let invite = m.hub.ajouter_personne("invité", Personne { pieces: pieces(&[0]), fin_mur_ms: u64::MAX, role: rp::INVITE, ..Personne::default() }).expect("personne");
    m.hub.ouvrir_session(tab, invite, preuve::PERSONNELLE, u64::MAX);
    m.hub.declarer(t(6_000), &groupe("groupe:etage", vec![("on", on)], vec![cible("hue:bureau/on"), cible("hue:charlie/on")])).expect("groupe");
    assert_eq!(essai(&mut m, tab, 5, "groupe:etage/on", vec![Arg::N(1)], 6_000), REFUS);
}

/// Le sous-ensemble ne s'annonce qu'à une personne qui demande : un automatisme dont le responsable a
/// perdu une cible de son groupe est refusé et mis à revoir, comme pour toute perte de droits (W2).
#[test]
fn un_automatisme_ne_choisit_pas_de_sous_ensemble() {
    let on = ordre_de_groupe(FormeDecl::regler(binaire(), "", 0));
    let cible = |n: &'static str| (sorte::ORDRE, n, role::CIBLE, "on");
    let mut b = Banc::new(
        &[lampe("hue:salon", 0, 254), lampe("hue:bureau", 1, 254), groupe("groupe:maison", vec![("on", on)], vec![cible("hue:salon/on"), cible("hue:bureau/on")])],
        vec![routine("Tout éteindre", vec![commander("groupe:maison/on", vec![Arg::N(0)])])],
        vec![("Tout éteindre", Responsabilite { responsable: "Alice".into(), ..Responsabilite::default() })],
    );
    assert!(b.moteur.a_revoir().is_empty());
    let alice = b.m.hub.personne("Alice");
    b.m.hub.personnes[usize::from(alice)].pieces = pieces(&[0]);
    b.moteur.lancer_nom(&mut b.m.hub, t(1_000), "Tout éteindre");
    b.tourner(1_000, 3_000);
    assert!(b.m.kit.envois.is_empty());
    assert!(b.m.hub.demandes.refs().all(|x| b.m.hub.demandes.fiches[x.case()].etat != etat::RETENUE), "rien d'annoncé");
    assert!(b.moteur.a_revoir().iter().any(|(n, r)| n == "Tout éteindre" && r.contains("mandat refusé à l'exécution")));
}

// ---- la durée des droits ---------------------------------------------------------------------------

/// D6 : les droits se revérifient au dernier point avant l'envoi. Une personne révoquée, ou dont la
/// configuration a changé, entre l'admission et la remise au module ne fait rien partir ; ce qui
/// attend l'accusé du secours tombe tout de suite.
#[test]
fn une_revocation_avant_la_remise_ne_laisse_rien_partir() {
    let mut m = maison(&[lampe("hue:salon", 0, 254)]);
    let (tel, tab) = (compte(&m, "téléphone d'Alice"), compte(&m, "tablette du salon"));
    let (alice, charlie) = (m.hub.personne("Alice"), m.hub.personne("Charlie"));
    let r = m.req(tel, 1, "hue:salon/on", vec![Arg::N(1)], 1_000);
    m.hub.demander(t(1_000), r).expect("admise");
    assert_eq!(m.hub.cmd.len(), 1, "admise, pas encore remise au module");
    m.hub.revoquer_personne(t(1_100), alice);
    m.tourner(1_100, 3_000);
    assert!(m.kit.envois.is_empty(), "rien n'est parti");
    assert_eq!(m.issues(1), vec![(issue::NON_ENVOYE, raison::DROITS)]);
    // Sans révocation datée : la configuration de Charlie change (elle perd le salon).
    m.hub.ouvrir_session(tab, charlie, preuve::PERSONNELLE, u64::MAX);
    let r = m.req(tab, 2, "hue:salon/on", vec![Arg::N(1)], 4_000);
    m.hub.demander(t(4_000), r).expect("admise");
    m.hub.personnes[usize::from(charlie)].pieces = pieces(&[3]);
    m.tourner(4_000, 6_000);
    assert!(m.kit.envois.is_empty());
    assert_eq!(m.issues(2), vec![(issue::NON_ENVOYE, raison::DROITS)]);
    // Un appui qui attend l'accusé du secours.
    let mut m = maison(&[haut_parleur()]);
    let tel = compte(&m, "téléphone d'Alice");
    m.hub.secours_en_phase = true;
    let r = m.req(tel, 1, "sonos:salon/volume_plus", vec![], 1_000);
    m.hub.demander(t(1_000), r).expect("admise");
    assert_eq!(m.hub.attente_secours.len(), 1, "il attend l'accusé");
    m.hub.revoquer_personne(t(1_050), alice);
    assert!(m.hub.attente_secours.is_empty(), "il ne partira pas");
    assert_eq!(m.issues(1), vec![(issue::NON_ENVOYE, raison::DROITS)]);
    let rang = m.hub.rang;
    m.hub.acquitter(rang);
    assert!(m.hub.cmd.is_empty());
}

/// D7 : la fin d'une personne (une invitation qui finit) est une échéance du hub : il s'y réveille de
/// lui-même, sans battement, et la retenue de l'invité tombe à cette seconde, pas dix minutes après.
#[test]
fn la_fin_d_une_invitation_reveille_le_hub() {
    let mut m = maison(&[machine("bambu:imprimante", 6)]);
    let fin = T0 + 5_000;
    let inv = m.hub.ajouter_personne("invité", Personne { pieces: pieces(&[6]), fin_mur_ms: fin, role: rp::INVITE, ..Personne::default() }).expect("personne");
    let tel = m.hub.ajouter_principal(Principal::invite("téléphone de l'invité", u32::from(inv), vec![6], u64::MAX)).expect("principal");
    assert_eq!(essai(&mut m, tel, 1, "bambu:imprimante/arreter", vec![], 1_000), RETENUE, "arrêter une machine se propose");
    assert_eq!(m.hub.prochaine(t(1_000)), Some(fin), "le hub se réveillera à la fin de l'invitation");
    m.tourner(1_000, 700_000);
    assert_eq!(m.issues(1), vec![(issue::RETENUE, raison::GARDE), (issue::REFUSE, raison::DROITS)]);
    let l = m.hub.journal.iter().rev().find(|l| l.raison == raison::DROITS).expect("ligne");
    assert_eq!(l.instant_ms, fin, "à la seconde même");
    // La fin d'un compte, la personne gardée (le téléphone prêté jusqu'à 18 h 40, l'invité restant) :
    // même réveil, même refus.
    let mut m = maison(&[machine("bambu:imprimante", 6)]);
    let inv = m.hub.ajouter_personne("invité", Personne { pieces: pieces(&[6]), fin_mur_ms: u64::MAX, role: rp::INVITE, ..Personne::default() }).expect("personne");
    let tel = m.hub.ajouter_principal(Principal::invite("téléphone prêté", u32::from(inv), vec![6], fin)).expect("principal");
    assert_eq!(essai(&mut m, tel, 1, "bambu:imprimante/arreter", vec![], 1_000), RETENUE);
    assert_eq!(m.hub.prochaine(t(1_000)), Some(fin));
    m.tourner(1_000, 700_000);
    assert_eq!(m.issues(1), vec![(issue::RETENUE, raison::GARDE), (issue::REFUSE, raison::DROITS)]);
    let l = m.hub.journal.iter().rev().find(|l| l.raison == raison::DROITS).expect("ligne");
    assert_eq!(l.instant_ms, fin, "la fin du compte, à la seconde même");
}

/// M3 : un secours promu sans reprise sûre a peut-être manqué une révocation : rien de sensible ne
/// part sans un humain, et seul un propriétaire l'approuve, jusqu'à ce qu'un propriétaire confirme les
/// droits ; le confort, lui, continue. Une reprise sûre garde ses droits pour frais.
#[test]
fn un_secours_aux_droits_incertains_retient_le_sensible() {
    let m = maison(&[lampe("hue:salon", 0, 254), porte()]);
    let tel = compte(&m, "téléphone d'Alice");
    let image = m.hub.ranger(t(500));
    let mut s = Hub::relire(&image, t(1_000), m.hub.principaux.clone()).expect("état complet");
    s.promouvoir(t(1_000), false);
    let req = |s: &Hub, seq, ordre: &str, args| Req { emise_epoque: s.epoque, ..Req::nouvelle(tel, seq, Cible::Nom(ordre.into()), args, t(1_100)) };
    let x = s.demander(t(1_100), req(&s, 1, "nuki:porte/tirer", vec![])).expect("admise");
    assert_eq!(s.demandes.get(x).map(|d| d.etat), Some(etat::RETENUE), "Alice ouvrirait d'ordinaire ; ici, un humain décide");
    let y = s.demander(t(1_100), req(&s, 2, "hue:salon/on", vec![Arg::N(1)])).expect("admise");
    assert_eq!(s.demandes.get(y).map(|d| d.etat), Some(etat::EN_FILE), "le confort continue");
    let (o, rev) = (s.ordre("nuki:porte/tirer"), s.cartes[&x].revision);
    assert_eq!(s.decider(t(1_200), x, o, rev, true, tel), Err("ce décideur n'est pas habilité pour cette opération"), "Alice approuverait d'ordinaire ; pas ici");
    assert_eq!(s.confirmer_droits(t(1_300), tel), Err("seul un propriétaire confirme les droits"));
    assert_eq!(s.decider(t(1_400), x, o, rev, true, m.ecran), Ok(x), "le propriétaire, si");
    assert!(s.confirmer_droits(t(1_500), m.ecran).is_ok());
    let z = s.demander(t(1_600), req(&s, 3, "nuki:porte/tirer", vec![])).expect("admise");
    assert_eq!(s.demandes.get(z).map(|d| d.etat), Some(etat::EN_FILE), "droits confirmés : Alice ouvre");
    let mut s2 = Hub::relire(&image, t(1_000), m.hub.principaux.clone()).expect("état complet");
    s2.promouvoir(t(1_000), true);
    assert!(s2.droits_frais);
}

/// M3 : une réduction de droits n'est acquise que lorsque le secours l'a accusée, comme un appui. Sans
/// accusé au bout de 200 ms, l'actif inscrit chez le témoin que le secours a décroché : le secours promu
/// ensuite, qui n'a peut-être pas la révocation, ne croit pas ses droits frais. Avec l'accusé, rien.
#[test]
fn une_revocation_attend_l_accuse_du_secours() {
    let mut m = maison(&[porte()]);
    let tel = compte(&m, "téléphone d'Alice");
    let alice = m.hub.personne("Alice");
    m.hub.secours_en_phase = true;
    let image = m.hub.ranger(t(500));
    m.hub.revoquer_personne(t(1_000), alice);
    assert_eq!(m.hub.prochaine(t(1_000)), Some(T0 + 1_200), "le hub se réveillera pour l'accusé");
    m.hub.tic(t(1_100));
    assert!(!m.hub.decroche, "100 ms : il attend encore");
    m.hub.tic(t(1_200));
    assert!(m.hub.decroche, "inscrit chez le témoin");
    // Le secours n'avait que l'image d'avant la révocation : promu, ses droits ne sont pas frais.
    let mut s = Hub::relire(&image, t(2_000), m.hub.principaux.clone()).expect("état complet");
    s.promouvoir(t(2_000), !m.hub.decroche);
    let r = Req { emise_epoque: s.epoque, ..Req::nouvelle(tel, 1, Cible::Nom("nuki:porte/tirer".into()), vec![], t(2_100)) };
    let x = s.demander(t(2_100), r).expect("Alice n'est pas révoquée, ici");
    assert_eq!(s.demandes.get(x).map(|d| d.etat), Some(etat::RETENUE), "un humain décide");
    // Le même, accusé à temps : le secours suit, rien n'est inscrit.
    let mut m = maison(&[porte()]);
    m.hub.secours_en_phase = true;
    m.hub.revoquer_personne(t(1_000), alice);
    let rang = m.hub.rang;
    m.hub.acquitter(rang);
    m.hub.tic(t(1_300));
    assert!(!m.hub.decroche && m.hub.droits_a_accuser.is_none());
}

// ---- l'alarme ----------------------------------------------------------------------------------------

/// D9 : désarmer ne passe jamais par le chemin générique : ni le propriétaire avec son PIN, ni un agent
/// (pas même retenu), ni un automatisme ; armer, si (proposé par un agent).
#[test]
fn desarmer_ne_passe_jamais_par_les_ordres() {
    let desarme = routine("Désarme le matin", vec![commander("alarme:maison/desarmer", vec![])]);
    let mut b = Banc::new(&[alarme()], vec![desarme], vec![]);
    let (ecran, agent) = (b.m.ecran, b.m.agent);
    let dedie = Err((issue::REFUSE, raison::DEDIE));
    assert_eq!(essai(&mut b.m, ecran, 1, "alarme:maison/desarmer", vec![], 1_000), dedie, "le propriétaire, avec son PIN");
    assert_eq!(essai(&mut b.m, agent, 1, "alarme:maison/desarmer", vec![], 1_000), dedie, "un agent");
    assert_eq!(b.m.hub.demandes.en_cours, 0, "rien n'est retenu");
    assert_eq!(essai(&mut b.m, agent, 2, "alarme:maison/armer", vec![], 1_000), RETENUE, "armer : un agent propose");
    assert!(b.moteur.a_revoir().iter().any(|(n, r)| n == "Désarme le matin" && r.contains("mandat refusé")), "aucun responsable n'accorde ce mandat");
}

/// Le choix du propriétaire (v3 § 22) : en urgence, la maison agit par un automatisme adopté par le foyer,
/// déclenché par une source qualifiée (un détecteur que nul ordre ne règle), dans son seul mandat ; il
/// agit même si son auteur est parti ; jamais un mode qui contourne les droits, jamais désarmer.
#[test]
fn une_urgence_est_un_mandat_du_foyer() {
    let feu = |nom: &str, pas| Def { nom: nom.into(), declencheur: DeclDef::Valeur("fumee:couloir/alarme".into(), Some(1)), pas };
    let foyer = |n: &'static str| (n, Responsabilite { responsable: "Bob".into(), foyer: true, ..Responsabilite::default() });
    let mut b = Banc::new(
        &[AppDecl::physique("fumee:couloir", 2, &["alarme"], vec![]), lampe("hue:couloir", 2, 254), porte(), alarme()],
        vec![feu("Incendie", vec![commander("hue:couloir/on", vec![Arg::N(1)]), commander("nuki:porte/tirer", vec![])]), feu("Incendie, et l'alarme", vec![commander("alarme:maison/desarmer", vec![])])],
        vec![foyer("Incendie"), foyer("Incendie, et l'alarme")],
    );
    let revoir: Vec<String> = b.moteur.a_revoir().into_iter().map(|(n, _)| n).collect();
    assert_eq!(revoir, vec!["Incendie, et l'alarme"], "désarmer n'est dans aucun mandat");
    let bob = b.m.hub.personne("Bob");
    b.m.hub.revoquer_personne(t(500), bob);
    let p = b.m.hub.point("fumee:couloir/alarme");
    b.m.monde.prevoir(T0 + 1_000, Quoi::Valeur { point: p, v: 1, mouvement: false, lu_ms: T0 + 1_000 });
    b.tourner(500, 5_000);
    assert_eq!(b.parts("Incendie"), 1);
    assert_eq!(b.m.kit.envois.len(), 2, "la lumière du couloir et la porte, sans attendre personne");
}

// ---- la mémoire --------------------------------------------------------------------------------------

/// M1 : un compte garde son numéro d'un instantané à l'autre, même si la configuration a été réordonnée ;
/// la révocation, la session et les demandes suivent le nom. Un compte que la configuration n'a plus
/// devient une tombe sans droits, un compte nouveau prend un numéro après les autres ; une copie
/// s'aligne sur les numéros de l'actif.
#[test]
fn les_comptes_se_relisent_par_leur_nom() {
    let mut m = maison(&[lampe("hue:salon", 0, 254), porte()]);
    let tab = compte(&m, "tablette du salon");
    let charlie = m.hub.personne("Charlie");
    m.hub.ouvrir_session(tab, charlie, preuve::PERSONNELLE, T0 + 600_000);
    let r = m.req(m.agent, 1, "nuki:porte/tirer", vec![], 1_000);
    let x = m.hub.demander(t(1_000), r).expect("retenue");
    m.hub.revoquer(t(1_000), m.script);
    let image = m.hub.ranger(t(1_000));
    // La configuration relue au redémarrage : les mêmes comptes dans un autre ordre, moins le téléphone
    // d'Alice, plus un écran neuf ; ni révocation ni session : elles ne vivent que dans l'instantané.
    let mut config: Vec<Principal> = m
        .hub
        .principaux
        .iter()
        .filter(|p| p.nom != "téléphone d'Alice")
        .cloned()
        .map(|mut p| {
            p.p.fin_mur_ms = u64::MAX;
            p.session = None;
            p
        })
        .collect();
    config.reverse();
    config.push(Principal::humain("écran de la cuisine", 1));
    let h = Hub::relire(&image, t(2_000), config).expect("instantané");
    let noms = |h: &Hub| h.principaux.iter().map(|p| p.nom.clone()).collect::<Vec<_>>();
    let mut attendus = noms(&m.hub);
    attendus.push("écran de la cuisine".into());
    assert_eq!(noms(&h), attendus, "chacun garde son numéro ; le nouveau vient après");
    assert_eq!(h.principaux[usize::from(m.script)].fin_mur_ms, T0 + 1_000, "la révocation suit le nom");
    assert_eq!(h.principaux.iter().filter(|p| p.fin_mur_ms != u64::MAX).count(), 2, "le script, et la tombe");
    let tombe = &h.principaux[usize::from(compte(&m, "téléphone d'Alice"))];
    assert_eq!((tombe.p.oui, tombe.p.pieces, tombe.p.fin_mur_ms), (0, 0, 0), "un compte supprimé n'a plus aucun droit");
    assert_eq!(h.principaux[usize::from(tab)].session.map(|s| s.personne), Some(charlie), "la session de Charlie reste sur la tablette");
    assert_eq!(h.principaux[usize::from(h.demandes.get(x).expect("retenue").de)].nom, "agent", "la retenue est toujours celle de l'agent");
    // Une copie dont la configuration est dans un autre ordre s'aligne sur l'actif.
    let mut desordre = m.hub.principaux.clone();
    desordre.reverse();
    let mut copie = Hub { actif: false, principaux: desordre, ..Hub::default() };
    m.hub.publier_droits();
    let Some(Sortie::Droits { rev, personnes, profils, table, mandats }) = m.hub.sorties.last().map(|p| p.sortie.clone()) else { panic!("les droits") };
    copie.appliquer_droits(rev, &personnes, &profils, &table, &mandats);
    assert_eq!(noms(&copie), noms(&m.hub));
    assert_eq!(copie.noms_personnes, m.hub.noms_personnes);
}

/// M5 : « en cours » compte au-delà de 255 demandes d'un même ordre : le compteur ne sature pas avant le
/// plafond des demandes.
#[test]
fn en_cours_compte_au_dela_de_255() {
    let mut m = maison(&[lampe("hue:salon", 0, 254)]);
    for k in 0..300u64 {
        let r = m.req(m.script, k + 1, "hue:salon/brightness", vec![Arg::N(k as i64 % 200)], 1_000);
        m.hub.demander(t(1_000), r).expect("admise");
    }
    let o = m.hub.ordre("hue:salon/brightness");
    let en_cours = Regle { numero: o.0, genre: genre::EN_COURS, ..Regle::default() };
    assert_eq!(m.hub.catalogue.fiches[o.place()].actives, 300);
    m.tourner(1_000, 1_000);
    assert_eq!(m.hub.catalogue.fiches[o.place()].actives, 2, "298 remplacées dans la voie ; une en vol, une qui attend");
    assert!(Moteur::tient(&m.hub, t(1_000), &en_cours));
    m.tourner(1_000, 20_000);
    assert_eq!(m.hub.catalogue.fiches[o.place()].actives, 0);
    assert!(!Moteur::tient(&m.hub, t(20_000), &en_cours));
}

/// M6 : l'empreinte des arguments est canonique (nombre, type, longueur, valeur) : deux textes découpés
/// autrement sont d'autres arguments, et la même clé avec eux est un conflit.
#[test]
fn des_arguments_differents_ne_passent_pas_pour_les_memes() {
    let deux = FormeDecl { params: vec![texte(64), texte(64)], ..FormeDecl::sans_param(effet::APPUI, retour::PONT) };
    let mut m = maison(&[AppDecl::physique("telegram:famille", 0, &[], vec![("envoyer", deux)])]);
    let a = |x: &str, y: &str| vec![Arg::T(x.into()), Arg::T(y.into())];
    assert_ne!(empreinte_args(&a("ab", "c")), empreinte_args(&a("a", "bc")));
    assert_ne!(empreinte_args(&[Arg::N(1), Arg::N(2)]), empreinte_args(&[Arg::N(2), Arg::N(1)]));
    let r = m.req(m.script, 1, "telegram:famille/envoyer", a("ab", "c"), 1_000);
    let x = m.hub.demander(t(1_000), r).expect("admise");
    let r = m.req(m.script, 1, "telegram:famille/envoyer", a("a", "bc"), 1_100);
    assert_eq!(m.hub.demander(t(1_100), r), Err((issue::REFUSE, raison::CONFLIT)));
    let r = m.req(m.script, 1, "telegram:famille/envoyer", a("ab", "c"), 1_200);
    assert_eq!(m.hub.demander(t(1_200), r), Ok(x), "la même demande, la même issue");
    assert_eq!(m.hub.cmd.len(), 1, "partie une fois");
}

/// M7 : après une reprise non sûre, un appui émis sur la session d'avant reste ambigu, même répété tel
/// quel, « première tentative » ; c'est l'époque de la session qui le date, et une demande qui ne la
/// dit pas est d'une session inconnue. Une reprise sûre plus tard n'efface rien.
#[test]
fn une_premiere_tentative_d_avant_la_reprise_reste_ambigue() {
    let mut m = Mini::new(&[haut_parleur()]);
    m.rep("sonos:salon", Rep::Pont { ms: 100 });
    let vieux = m.hub.ranger(t(1_000));
    let r = m.req(m.script, 7, "sonos:salon/volume_plus", vec![], 2_000);
    m.hub.demander(t(2_000), r).expect("admise");
    m.tourner(2_000, 3_000);
    assert_eq!(m.kit.envois.len(), 1, "l'appui est parti");
    let mut h = Hub::relire(&vieux, t(10_000), m.hub.principaux.clone()).expect("instantané");
    h.redemarre(t(10_000), false);
    let ambigue = Err((issue::INCERTAIN, raison::AMBIGUE));
    let repete = m.req(m.script, 7, "sonos:salon/volume_plus", vec![], 2_000);
    assert_eq!(repete.tentative, 1);
    assert_eq!(h.demander(t(10_500), repete.clone()), ambigue, "le message d'origine, répété tel quel");
    assert_eq!(h.demander(t(10_500), Req { emise_epoque: 0, ..repete.clone() }), ambigue, "sans époque : d'une session inconnue");
    let neuve = Req { emise_epoque: h.epoque, ..m.req(m.script, 8, "sonos:salon/volume_plus", vec![], 10_600) };
    assert!(h.demander(t(10_600), neuve).is_ok(), "émise sur la session nouvelle : elle part");
    // Un arrêt propre plus tard (une reprise sûre) ne rend pas le message d'avant moins ambigu.
    let image = h.ranger(t(20_000));
    let mut h2 = Hub::relire(&image, t(21_000), m.hub.principaux.clone()).expect("instantané");
    h2.redemarre(t(21_000), true);
    assert_eq!(h2.demander(t(21_500), repete), ambigue);
}

/// M8 et les bornes des droits : une forme à deux textes privés est refusée (le tiroir privé n'en garde
/// qu'un), comme une classe inconnue, une pièce au-delà de 63, une cible de groupe d'une autre classe
/// que son ordre de groupe ; les plafonds des comptes et des personnes sont des refus, jamais une panne.
#[test]
fn une_declaration_hors_des_bornes_des_droits_est_refusee() {
    let mut m = maison(&[]);
    let tt = t(1_000);
    let prive = |n| FormeDecl { params: vec![texte(64); n], drapeaux: drapeau::TEXTE_PRIVE, ..FormeDecl::sans_param(effet::APPUI, retour::PONT) };
    assert_eq!(m.hub.declarer(tt, &AppDecl::physique("telegram:a", 0, &[], vec![("envoyer", prive(2))])), Err("un seul texte privé par forme"));
    assert!(m.hub.declarer(tt, &AppDecl::physique("telegram:b", 0, &[], vec![("envoyer", prive(1))])).is_ok());
    assert_eq!(m.hub.declarer(tt, &appui("x:c", 0, "a", 42)), Err("classe inconnue"));
    assert_eq!(m.hub.declarer(tt, &lampe("hue:grenier", 64, 254)), Err("64 pièces au plus"));
    m.hub.declarer(tt, &porte()).expect("porte");
    let tirer = |c| ordre_de_groupe(FormeDecl { classe: c, ..FormeDecl::sans_param(effet::APPUI, retour::PONT) });
    let cible = vec![(sorte::ORDRE, "nuki:porte/tirer", role::CIBLE, "tirer")];
    assert_eq!(m.hub.declarer(tt, &groupe("groupe:entree", vec![("tirer", tirer(classe::CONFORT))], cible.clone())), Err("une cible de forme incompatible avec son ordre de groupe"), "un groupe de confort ne cache pas une ouverture");
    assert!(m.hub.declarer(tt, &groupe("groupe:entree", vec![("tirer", tirer(classe::ACCES))], cible)).is_ok());
    while m.hub.personnes.len() < PERSONNES_MAX {
        let n = format!("personne {}", m.hub.personnes.len());
        m.hub.ajouter_personne(&n, Personne::default()).expect("place");
    }
    assert_eq!(m.hub.ajouter_personne("une de trop", Personne::default()), Err("plafond des personnes"));
    assert_eq!(m.hub.ajouter_principal(Principal::humain("écran fantôme", 999)), Err("personne inconnue"));
    while m.hub.principaux.len() < PRINCIPAUX_MAX {
        let n = format!("tablette {}", m.hub.principaux.len());
        m.hub.ajouter_principal(Principal::tablette(&n)).expect("place");
    }
    assert_eq!(m.hub.ajouter_principal(Principal::tablette("une de trop")), Err("plafond des principaux"));
}

/// D8 : `ord/` dit ce qui s'est passé, jamais qui ; le pair reçoit l'identité par une sortie privée et
/// finit identique ; un écran montre à chacun lesquelles des dernières commandes sont les siennes, et
/// rien des autres.
#[test]
fn ord_ne_dit_pas_qui() {
    let mut m = maison(&[lampe("hue:salon", 0, 254), lampe("hue:cuisine", 1, 254)]);
    let tel = compte(&m, "téléphone d'Alice");
    let (bob, alice) = (m.hub.personne("Bob"), m.hub.personne("Alice"));
    let image = m.hub.ranger(t(500));
    let mut copie = Hub::relire(&image, t(500), m.hub.principaux.clone()).expect("état complet");
    let r = m.req(tel, 1, "hue:salon/on", vec![Arg::N(1)], 1_000);
    m.hub.demander(t(1_000), r).expect("admise");
    let r = m.req(m.ecran, 1, "hue:cuisine/on", vec![Arg::N(1)], 1_000);
    m.hub.demander(t(1_000), r).expect("admise");
    m.tourner(1_000, 3_000);
    for p in &m.hub.sorties {
        if let Sortie::Trace { demande, fiche, ligne, .. } = &p.sortie {
            assert_eq!((demande.pour, demande.preuve, fiche.pour, fiche.preuve_pour, ligne.pour), (0, 0, 0, 0, 0), "ord/ ne dit pas qui");
        }
        if matches!(p.sortie, Sortie::Autorite { .. }) {
            assert_eq!(p.piece, PIECE_PRIVEE, "l'identité ne va qu'au pair");
        }
        copie.appliquer(p, t(3_000)).expect("la copie applique");
    }
    assert!(m.hub.sorties.iter().any(|p| matches!(p.sortie, Sortie::Autorite { pour, .. } if pour == alice)));
    assert_eq!(copie.empreinte(), m.hub.empreinte(), "le pair sait qui, comme l'actif");
    assert_eq!(miennes(&m.hub, alice), vec!["hue:salon/on"]);
    assert_eq!(miennes(&m.hub, bob), vec!["hue:cuisine/on"]);
    assert!(miennes(&m.hub, 0).is_empty(), "un écran sans personne ne voit à personne");
}

// ---- les automatismes --------------------------------------------------------------------------------

/// W1 et E5 : une routine lancée par une personne garde ses droits et ceux de sa connexion, dans la limite
/// du mandat : Charlie ne fait pas ouvrir la porte par « Départ » ; une tablette anonyme qui la lance ne
/// commande pas une chambre sans un humain ; le propriétaire, si.
#[test]
fn une_routine_lancee_garde_les_droits_de_qui_la_lance() {
    let depart = routine("Départ", vec![commander("hue:charlie/on", vec![Arg::N(0)]), commander("nuki:porte/tirer", vec![])]);
    let mut b = Banc::new(&[lampe("hue:charlie", 3, 254), porte()], vec![depart], vec![]);
    let tab = compte(&b.m, "tablette du salon");
    let (bob, charlie) = (b.m.hub.personne("Bob"), b.m.hub.personne("Charlie"));
    // Charlie, son PIN sur la tablette : sa lampe, oui ; la porte, non.
    b.m.hub.ouvrir_session(tab, charlie, preuve::PERSONNELLE, u64::MAX);
    b.moteur.lancer_par(&mut b.m.hub, t(1_000), "Départ", tab, 0);
    b.tourner(1_000, 3_000);
    assert_eq!(b.m.kit.envois.len(), 1, "sa lampe seulement");
    assert!(b.moteur.journal.iter().any(|(_, a, x)| a == "Départ" && x == "ordre refusé (droits)"));
    assert!(b.moteur.a_revoir().is_empty(), "le refus est celui de Charlie, pas celui du mandat");
    // La tablette anonyme : la chambre se propose, l'exécution attend un humain.
    b.m.hub.fermer_session(tab);
    b.moteur.lancer_par(&mut b.m.hub, t(4_000), "Départ", tab, 0);
    b.tourner(4_000, 6_000);
    let retenue = b.m.hub.demandes.refs().map(|x| b.m.hub.demandes.fiches[x.case()]).find(|d| d.etat == etat::RETENUE).expect("retenue");
    assert_eq!((retenue.de, retenue.pour, retenue.mandat), (tab, 0, 1), "la tablette, personne, sous le mandat de « Départ »");
    assert_eq!(b.m.kit.envois.len(), 1);
    // Le propriétaire, son PIN sur la tablette : les deux.
    b.m.hub.ouvrir_session(tab, bob, preuve::PERSONNELLE, u64::MAX);
    b.moteur.lancer_par(&mut b.m.hub, t(7_000), "Départ", tab, 0);
    b.tourner(7_000, 10_000);
    assert_eq!(b.m.kit.envois.len(), 3);
    let porte = b.m.hub.ordre("nuki:porte/tirer").0;
    assert!(b.m.hub.journal.iter().any(|l| l.ordre == porte && l.de == tab && l.pour == bob), "au journal : la tablette, pour Bob");
}

/// W1 : un mandat ne couvre que son manifeste : une demande du moteur hors de ce que le graphe commande
/// est refusée, même sous un responsable propriétaire.
#[test]
fn un_mandat_ne_couvre_que_son_manifeste() {
    let mut b = Banc::new(&[lampe("hue:salon", 0, 254), porte()], vec![routine("Lampe", vec![commander("hue:salon/on", vec![Arg::N(1)])])], vec![]);
    let de = b.moteur.de;
    let mandat = AutoriteMoteur { appele: false, de, pour: 0, preuve: 0, mandat: 1 };
    let r = Req { autorite: Some(mandat), ..Req::nouvelle(de, 0, Cible::Nom("nuki:porte/tirer".into()), vec![], t(1_000)) };
    assert_eq!(b.m.hub.demander(t(1_000), r), Err((issue::REFUSE, raison::DROITS)), "la porte n'est pas dans le manifeste de « Lampe »");
    let r = Req { autorite: Some(mandat), ..Req::nouvelle(de, 0, Cible::Nom("hue:salon/on".into()), vec![Arg::N(1)], t(1_000)) };
    assert!(b.m.hub.demander(t(1_000), r).is_ok());
}

/// W1 : un déclencheur que d'autres peuvent provoquer (la trace d'un ordre, une valeur qu'un ordre règle)
/// ne lance pas d'action sensible, sauf décision explicite de son responsable ; un capteur que nul ordre
/// ne règle n'est pas ouvert.
#[test]
fn un_declencheur_ouvert_ne_lance_pas_le_sensible_sans_decision() {
    let ouvre = |nom: &str, d: DeclDef| Def { nom: nom.into(), declencheur: d, pas: vec![commander("nuki:porte/tirer", vec![])] };
    let sonnette_ = || DeclDef::Ordre("helpers:sonnette/appuyer".into(), 1 << issue::EXECUTE);
    let accepte = Responsabilite { responsable: "Bob".into(), declencheur_ouvert: true, ..Responsabilite::default() };
    let mut b = Banc::new(
        &[sonnette("helpers:sonnette", 5), lampe("hue:salon", 0, 254), AppDecl::physique("capteur:entree", 5, &["presence"], vec![]), porte()],
        vec![
            ouvre("Sonnette ouvre", sonnette_()),
            ouvre("Sonnette ouvre, accepté", sonnette_()),
            ouvre("Lampe ouvre", DeclDef::Valeur("hue:salon/on".into(), Some(1))),
            ouvre("Présence ouvre", DeclDef::Valeur("capteur:entree/presence".into(), Some(1))),
        ],
        vec![("Sonnette ouvre, accepté", accepte)],
    );
    let revoir: Vec<String> = b.moteur.a_revoir().into_iter().map(|(n, _)| n).collect();
    assert_eq!(revoir, vec!["Sonnette ouvre", "Lampe ouvre"]);
    // Un agent appuie sur la sonnette : seul l'automatisme accepté ouvre, sous son mandat.
    let agent = b.m.agent;
    let r = b.m.req(agent, 1, "helpers:sonnette/appuyer", vec![], 1_000);
    b.m.hub.demander(t(1_000), r).expect("admise");
    b.tourner(1_000, 3_000);
    let parts: Vec<&str> = b.moteur.journal.iter().filter(|(_, _, x)| x.starts_with("part")).map(|(_, a, _)| a.as_str()).collect();
    assert_eq!(parts, vec!["Sonnette ouvre, accepté"]);
    assert_eq!(b.m.kit.envois.len(), 1, "la porte");
}

/// W2 : un automatisme autonome ne dépasse pas son responsable : Alice partie, sa « Nuit » est refusée à
/// sa prochaine exécution et mise à revoir ; un automatisme adopté par le foyer (par un propriétaire)
/// survit au départ ; un enfant ne valide pas de mandat, un adulte ne fait pas adopter.
#[test]
fn un_automatisme_ne_survit_pas_a_son_responsable_sauf_adopte() {
    let nuit = |nom: &str| routine(nom, vec![commander("hue:salon/on", vec![Arg::N(0)])]);
    let resp = |qui: &str, foyer| Responsabilite { responsable: qui.into(), foyer, ..Responsabilite::default() };
    let mut b = Banc::new(
        &[lampe("hue:salon", 0, 254)],
        vec![nuit("Nuit d'Alice"), nuit("Nuit du foyer"), nuit("Nuit de Charlie"), nuit("Nuit adoptée par Alice")],
        vec![("Nuit d'Alice", resp("Alice", false)), ("Nuit du foyer", resp("Bob", true)), ("Nuit de Charlie", resp("Charlie", false)), ("Nuit adoptée par Alice", resp("Alice", true))],
    );
    let revoir = b.moteur.a_revoir();
    assert_eq!(revoir.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), vec!["Nuit de Charlie", "Nuit adoptée par Alice"]);
    assert!(revoir[0].1.contains("Charlie ne peut pas accorder ce mandat"));
    assert!(revoir[1].1.contains("seul un propriétaire"));
    let (alice, bob) = (b.m.hub.personne("Alice"), b.m.hub.personne("Bob"));
    b.m.hub.revoquer_personne(t(1_000), alice);
    b.m.hub.revoquer_personne(t(1_000), bob);
    b.moteur.lancer_nom(&mut b.m.hub, t(2_000), "Nuit d'Alice");
    b.moteur.lancer_nom(&mut b.m.hub, t(2_000), "Nuit du foyer");
    b.tourner(2_000, 4_000);
    assert_eq!(b.m.kit.envois.len(), 1, "seule la nuit du foyer éteint");
    assert!(b.moteur.a_revoir().iter().any(|(n, r)| n == "Nuit d'Alice" && r.contains("mandat refusé à l'exécution")));
    b.moteur.lancer_nom(&mut b.m.hub, t(5_000), "Nuit d'Alice");
    assert!(b.moteur.journal.last().is_some_and(|(_, a, x)| a == "Nuit d'Alice" && x.starts_with("à revoir")));
}

/// W3 : un bail ne s'ouvre qu'au succès de son ordre ; à son terme, il ne rétablit que s'il possède encore
/// l'état (sa commande est la dernière, la valeur la suit) : un humain qui a changé l'état entre-temps
/// l'emporte, par une commande comme à la main.
#[test]
fn un_bail_ne_retablit_que_ce_qu_il_possede() {
    let bail = |nom: &str, l: &str| routine(nom, vec![PasDef::Bail(format!("{l}/brightness"), vec![Arg::N(200)], 5)]);
    let mut b = Banc::new(
        &[lampe("hue:a", 0, 254), lampe("hue:b", 0, 254), lampe("hue:c", 0, 254), lampe("hue:charlie", 3, 254)],
        vec![bail("Bail A", "hue:a"), bail("Bail B", "hue:b"), bail("Bail C", "hue:c"), bail("Bail Charlie", "hue:charlie")],
        vec![],
    );
    let (tel, tab, ecran) = (compte(&b.m, "téléphone d'Alice"), compte(&b.m, "tablette du salon"), b.m.ecran);
    for (k, l) in ["hue:a", "hue:b", "hue:c"].iter().enumerate() {
        let r = b.m.req(tel, k as u64 + 1, &format!("{l}/brightness"), vec![Arg::N(50)], 1_000);
        b.m.hub.demander(t(1_000), r).expect("admise");
    }
    b.tourner(1_000, 2_000);
    for n in ["Bail A", "Bail B", "Bail C"] {
        b.moteur.lancer_nom(&mut b.m.hub, t(2_000), n);
    }
    b.tourner(2_000, 3_000);
    assert_eq!(b.moteur.baux.iter().filter(|x| x.etat == 1).count(), 3, "trois baux, chacun ouvert au succès de son ordre");
    // A : Alice le règle elle-même ; B : quelqu'un le règle au mur ; C : personne n'y touche.
    let r = b.m.req(tel, 10, "hue:a/brightness", vec![Arg::N(120)], 4_000);
    b.m.hub.demander(t(4_000), r).expect("admise");
    let p = b.m.hub.point("hue:b/brightness");
    b.m.monde.prevoir(T0 + 4_000, Quoi::Valeur { point: p, v: 80, mouvement: false, lu_ms: T0 + 4_000 });
    b.tourner(3_000, 10_000);
    let v = |b: &Banc, l: &str| b.m.hub.vifs[b.m.hub.point(&format!("{l}/brightness")).place()].valeur;
    assert_eq!((v(&b, "hue:a"), v(&b, "hue:b"), v(&b, "hue:c")), (120, 80, 50), "seul C est rétabli");
    assert_eq!(b.moteur.journal.iter().filter(|(_, _, x)| x.contains("changé de main")).count(), 2);
    // Un bail dont l'ordre est refusé ne s'ouvre pas : la tablette anonyme lance « Bail Charlie », la chambre
    // est retenue, le propriétaire refuse.
    b.moteur.lancer_par(&mut b.m.hub, t(11_000), "Bail Charlie", tab, 0);
    b.propager(11_000);
    let x = b.m.hub.demandes.refs().find(|&x| b.m.hub.demandes.fiches[x.case()].etat == etat::RETENUE).expect("retenue");
    let (o, rev) = (b.m.hub.ordre("hue:charlie/brightness"), b.m.hub.cartes[&x].revision);
    b.m.hub.decider(t(12_000), x, o, rev, false, ecran).expect("refusée");
    b.propager(12_000);
    assert!(b.moteur.journal.iter().any(|(_, a, x)| a == "Bail Charlie" && x.contains("bail non ouvert")));
    assert_eq!(b.moteur.baux.iter().filter(|x| x.etat == 1).count(), 0);
}

/// W4 : une exécution ne reprend que dans le programme où elle est née : réordonner les automatismes ne la
/// fait pas passer dans un autre ; en changer la logique l'arrête.
#[test]
fn une_execution_ne_change_pas_de_programme() {
    let arroser = routine("Arroser", vec![commander("hue:a/on", vec![Arg::N(1)]), commander("hue:a/brightness", vec![Arg::N(100)])]);
    let veille = routine("Veille", vec![commander("hue:b/on", vec![Arg::N(0)]), commander("hue:b/brightness", vec![Arg::N(10)])]);
    let mut b = Banc::new(&[lampe("hue:a", 0, 254), lampe("hue:b", 0, 254)], vec![arroser.clone(), veille.clone()], vec![]);
    b.moteur.lancer_nom(&mut b.m.hub, t(1_000), "Arroser");
    b.propager(1_000);
    // Réordonnées pendant que la première étape attend son issue.
    b.moteur.defs = vec![veille.clone(), arroser.clone()];
    b.moteur.compiler(&mut b.m.hub);
    b.tourner(1_000, 4_000);
    let v = |b: &Banc, n: &str| b.m.hub.vifs[b.m.hub.point(n).place()].valeur;
    assert_eq!((v(&b, "hue:a/brightness"), v(&b, "hue:b/brightness")), (100, 0), "la seconde étape est celle d'Arroser");
    // La logique change pendant qu'une exécution attend : elle s'arrête.
    b.moteur.lancer_nom(&mut b.m.hub, t(5_000), "Arroser");
    b.propager(5_000);
    let autre = Def { pas: vec![commander("hue:a/on", vec![Arg::N(1)]), commander("hue:a/brightness", vec![Arg::N(30)])], ..arroser };
    b.moteur.defs = vec![veille, autre];
    b.moteur.compiler(&mut b.m.hub);
    assert!(b.moteur.journal.iter().any(|(_, _, x)| x == "programme changé : arrêtée"));
    b.tourner(5_000, 8_000);
    assert_eq!(v(&b, "hue:a/brightness"), 100, "ni l'ancienne seconde étape, ni la nouvelle");
}

/// W5 : chaque transition d'une exécution passe au pair (`wf/ex/`) : l'actif tombe entre deux étapes, le
/// secours continue l'exécution là où elle attendait.
#[test]
fn une_execution_reprend_sur_le_secours_entre_deux_etapes() {
    let deux = routine("Deux temps", vec![commander("hue:a/on", vec![Arg::N(1)]), commander("hue:b/on", vec![Arg::N(1)])]);
    let mut b = Banc::new(&[lampe("hue:a", 0, 254), lampe("hue:b", 0, 254)], vec![deux], vec![]);
    b.m.rep("hue:a", Rep::Muet);
    b.attacher_copie(500);
    b.moteur.lancer_nom(&mut b.m.hub, t(1_000), "Deux temps");
    b.tourner(1_000, 1_500);
    assert_eq!(b.m.kit.envois.len(), 1, "hue:a/on est parti ; l'appareil n'a pas répondu");
    let (mut h, mut mo) = b.copie.take().expect("copie");
    assert_eq!(mo.ex.iter().filter(|x| x.etat == etat_ex::ISSUE).count(), 1, "le secours sait qu'une exécution attend");
    // L'actif tombe : la copie devient l'actif.
    h.promouvoir(t(2_000), true);
    mo.actif = true;
    for _ in 0..8 {
        for p in std::mem::take(&mut h.sorties) {
            donner(&mut mo, &mut h, t(2_000), &p.sortie);
        }
    }
    let ordres: Vec<String> = h.cmd.iter().map(|c| h.nom_ordre(c.ordre)).collect();
    assert_eq!(ordres, vec!["hue:b/on".to_string()], "la seconde étape part du secours");
}

/// W6 : deux automatismes qui se déclenchent l'un l'autre ne tournent pas sans fin : 20 départs par minute
/// et par automatisme, comme aujourd'hui ; une minute plus tard, ça repart ; une même transition d'une
/// demande, republiée, ne lance qu'une fois.
#[test]
fn deux_automatismes_ne_se_relancent_pas_sans_fin() {
    let reagit = |nom: &str, sur: &str, fait: &str| Def { nom: nom.into(), declencheur: DeclDef::Ordre(format!("{sur}/on"), 1 << issue::EN_COURS), pas: vec![commander(&format!("{fait}/on"), vec![Arg::N(1)])] };
    let vu = Def { nom: "Vu".into(), declencheur: DeclDef::Ordre("hue:z/on".into(), 1 << issue::EN_COURS | 1 << issue::CONFIRME), pas: vec![PasDef::Prevenir("vu".into())] };
    let mut b = Banc::new(&[lampe("hue:x", 0, 254), lampe("hue:y", 0, 254), lampe("hue:z", 0, 254)], vec![reagit("A", "hue:x", "hue:y"), reagit("B", "hue:y", "hue:x"), vu], vec![]);
    let ecran = b.m.ecran;
    let r = b.m.req(ecran, 1, "hue:x/on", vec![Arg::N(1)], 1_000);
    b.m.hub.demander(t(1_000), r).expect("admise");
    b.propager(1_000);
    assert_eq!((b.parts("A"), b.parts("B")), (20, 20));
    assert!(b.moteur.journal.iter().any(|(_, a, x)| a == "A" && x.starts_with("quota")));
    let r = b.m.req(ecran, 2, "hue:x/on", vec![Arg::N(1)], 62_000);
    b.m.hub.demander(t(62_000), r).expect("admise");
    b.propager(62_000);
    assert!(b.parts("A") > 20, "une minute plus tard");
    let r = b.m.req(ecran, 3, "hue:z/on", vec![Arg::N(1)], 70_000);
    let z = b.m.hub.demander(t(70_000), r).expect("admise");
    b.propager(70_000);
    assert_eq!(b.parts("Vu"), 1);
    let o = b.m.hub.ordre("hue:z/on");
    b.moteur.trace(&mut b.m.hub, t(70_000), z, o, issue::EN_COURS, false);
    assert_eq!(b.parts("Vu"), 1, "la même transition, republiée");
    b.moteur.trace(&mut b.m.hub, t(70_300), z, o, issue::CONFIRME, true);
    assert_eq!(b.parts("Vu"), 2, "une autre transition de la même demande");
}
