//! Les droits sur les ordres (v3) : qui demande (le principal, la connexion), pour qui (la personne,
//! et la preuve qu'on a d'elle), quoi (la classe de l'ordre et les pièces qu'il atteint), et sous quel
//! mandat (un automatisme). Trois réponses : non, proposer (retenir pour un humain habilité), oui.
//!
//! Des tables de taille fixe ; une décision coûte quelques nanosecondes. Le hub décide à l'arrivée, à
//! l'approbation et à la remise au module ; le secours décide pareil, avec les mêmes tables.

/// La classe d'un ordre : ce qu'il engage. Elle est dans sa forme ; un geste sûr et un geste sensible
/// d'un même appareil (verrouiller, ouvrir) sont deux ordres, chacun sa classe.
pub mod classe {
    pub const CONFORT: u8 = 0;
    pub const MEDIA: u8 = 1;
    pub const CLIMAT: u8 = 2;
    /// Ouvrir une porte, un portail, un garage.
    pub const ACCES: u8 = 3;
    /// Armer l'alarme, changer son mode vers plus sûr.
    pub const ALARME: u8 = 4;
    /// Arrêter une machine au travail, allumer ce qui chauffe ou coupe.
    pub const MACHINE: u8 = 5;
    /// Caméras et micros.
    pub const VIE_PRIVEE: u8 = 6;
    /// Réglages, redémarrages, mises à jour.
    pub const SYSTEME: u8 = 7;
    pub const MESSAGE: u8 = 8;
    /// Désarmer : jamais par le chemin générique des ordres. Le code passe par le chemin privé de
    /// l'alarme (docs/components/alarme.md) ; aucune approbation ne l'ouvre.
    pub const DESARMER: u8 = 9;
    pub const N: usize = 10;
    pub const NOMS: [&str; N] = ["confort", "média", "climat", "accès", "alarme", "machine", "vie privée", "système", "message", "désarmer"];
    /// Les classes qui exigent une preuve personnelle, et que le secours ne laisse pas passer sans
    /// approbation tant que ses droits ne sont pas sûrs.
    pub const SENSIBLES: u16 = 1 << ACCES | 1 << ALARME | 1 << MACHINE | 1 << VIE_PRIVEE | 1 << SYSTEME | 1 << DESARMER;
    pub const TOUTES: u16 = (1 << N) - 1;
    pub const fn bit(c: u8) -> u16 {
        1 << c
    }
}

/// Le rôle d'une personne. `INCONNU` est le profil explicite de « quelqu'un » : une tablette partagée
/// que personne n'a déverrouillée, une voix que rien n'identifie.
pub mod role {
    pub const INCONNU: u8 = 0;
    pub const PROPRIETAIRE: u8 = 1;
    pub const ADULTE: u8 = 2;
    pub const ENFANT: u8 = 3;
    pub const INVITE: u8 = 4;
    /// Le ménage, un gardien : des pièces et une fenêtre.
    pub const SERVICE: u8 = 5;
    pub const N: usize = 6;
    pub const NOMS: [&str; N] = ["inconnu", "propriétaire", "adulte", "enfant", "invité", "service"];
}

/// Ce qu'on sait de la personne. Un indice (une voix reconnue) vaut sa personne pour le confort, ce
/// qui n'est pas sensible (le choix du propriétaire, v3 § 22) ; pour le sensible, sans preuve
/// personnelle, elle a au plus les droits d'une personne inconnue : là, un indice restreint, jamais il
/// n'étend.
pub mod preuve {
    pub const AUCUNE: u8 = 0;
    pub const INDICE: u8 = 1;
    /// Une session personnelle : un mot de passe, une appli personnelle, un PIN personnel tapé sur
    /// une tablette ; un mandat délivré par la personne à un agent.
    pub const PERSONNELLE: u8 = 2;
    pub const NOMS: [&str; 3] = ["aucune", "indice", "personnelle"];
}

pub mod niveau {
    pub const NON: u8 = 0;
    /// Retenir : un humain habilité décide.
    pub const PROPOSER: u8 = 1;
    pub const OUI: u8 = 2;
    pub const NOMS: [&str; 3] = ["non", "proposer", "oui"];
}

/// Une case de la table : le niveau (2 bits) et le droit d'approuver ce que d'autres proposent.
pub const APPROUVE: u8 = 4;

/// Le profil d'un principal : ce que la connexion permet, quelle que soit la personne. 32 o.
pub mod drapeau_p {
    /// Le hub, le moteur : ils nomment eux-mêmes leur autorité (une exécution, un mandat).
    pub const INTERNE: u8 = 1;
    /// Un écran ou une appli : peut montrer une carte et porter une décision, si la personne qui la
    /// prend a une preuve personnelle et le droit d'approuver.
    pub const DECIDE: u8 = 2;
    /// Un agent, un satellite, un script : rien dans une pièce protégée sans un humain (la garde
    /// d'aujourd'hui, « là où dort un enfant »).
    pub const GARDE_PIECES: u8 = 4;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Profil {
    /// Les pièces qu'il atteint (un bit par pièce, 64 au plus) ; `TOUTES_PIECES` : toutes.
    pub pieces: u64,
    /// La fin de son permis (heure murale, ms) : une invitation, une révocation.
    pub fin_mur_ms: u64,
    /// L'agent ou la personne qu'il nomme au journal (v2).
    pub qui: u32,
    /// La personne fixe de la connexion (une appli personnelle, un agent mandaté) ; 0 : aucune.
    pub personne: u16,
    /// Les classes qu'il commande, et celles qu'il peut seulement proposer.
    pub oui: u16,
    pub proposer: u16,
    pub origine: u8,
    /// La meilleure preuve que ce canal sait établir (un satellite : un indice).
    pub preuve_max: u8,
    pub drapeaux: u8,
    pub _r: [u8; 3],
}

pub const TOUTES_PIECES: u64 = u64::MAX;
pub const PIECES_MAX: u16 = 64;

pub fn pieces(ps: &[u16]) -> u64 {
    ps.iter().fold(0, |b, &p| {
        assert!(p < PIECES_MAX, "64 pièces au plus");
        b | 1 << p
    })
}

/// Une personne. 32 o.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Personne {
    pub pieces: u64,
    /// Sa fenêtre (heure murale, ms) : `[debut, fin)`.
    pub debut_mur_ms: u64,
    pub fin_mur_ms: u64,
    pub role: u8,
    pub drapeaux: u8,
    /// Les classes qui lui sont refusées en plus de son rôle (une exception personnelle).
    pub refuse: u16,
    pub _r: u32,
}

/// Une session personnelle sur un appareil partagé : le PIN de Charlie tapé sur la tablette du salon,
/// pour quelques minutes. 16 o.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Session {
    pub fin_mur_ms: u64,
    pub personne: u16,
    pub preuve: u8,
    pub _r: [u8; 5],
}

/// Le mandat d'un automatisme : ce que son graphe commande (le manifeste, calculé à la validation),
/// et qui en répond. 16 o.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Mandat {
    pub pieces: u64,
    pub classes: u16,
    /// Celui qui l'a validé, et dont les droits bornent le mandat tant qu'il n'est pas adopté.
    pub responsable: u16,
    pub drapeaux: u8,
    pub _r: [u8; 3],
}

pub mod drapeau_m {
    /// Adopté par le foyer : il survit au départ de son responsable (validé par un propriétaire).
    pub const FOYER: u8 = 1;
    /// Vivant : validé, pas encore remplacé.
    pub const VIVANT: u8 = 2;
}

const _: () = assert!(std::mem::size_of::<Profil>() == 32);
const _: () = assert!(std::mem::size_of::<Personne>() == 32);
const _: () = assert!(std::mem::size_of::<Session>() == 16);
const _: () = assert!(std::mem::size_of::<Mandat>() == 16);

/// La table rôles × classes : un octet par case (niveau | `APPROUVE`).
pub type Table = [[u8; classe::N]; role::N];

/// La table par défaut : une proposition, que le propriétaire règle (§ 22 de la v3).
pub fn table_par_defaut() -> Table {
    use classe::*;
    use niveau::*;
    let o = OUI;
    let p = PROPOSER;
    let n = NON;
    let a = OUI | APPROUVE;
    //            confort média climat accès alarme machine privée système message désarmer
    let inconnu = [o, o, o, p, p, p, n, n, o, n];
    let proprio = [a, a, a, a, a, a, a, a, a, n];
    let adulte = [a, a, a, a, a, a, o, p, o, n];
    let enfant = [o, o, p, p, n, p, n, n, o, n];
    let invite = [o, o, o, p, n, p, n, n, o, n];
    let service = [o, n, o, p, n, n, n, n, n, n];
    let mut t = [[0; N]; role::N];
    t[usize::from(role::INCONNU)] = inconnu;
    t[usize::from(role::PROPRIETAIRE)] = proprio;
    t[usize::from(role::ADULTE)] = adulte;
    t[usize::from(role::ENFANT)] = enfant;
    t[usize::from(role::INVITE)] = invite;
    t[usize::from(role::SERVICE)] = service;
    let _ = (CONFORT, DESARMER);
    t
}

/// Une autorité : tout ce qui décide d'une demande, établi par le serveur (jamais par le client).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Autorite<'a> {
    pub profil: Option<&'a Profil>,
    /// La personne, et ce qu'on en sait.
    pub personne: Option<&'a Personne>,
    pub preuve: u8,
    pub mandat: Option<&'a Mandat>,
    /// Le hub lui-même, ou un mandat adopté par le foyer : aucune personne ne le borne.
    pub systeme: bool,
}

/// Le contexte : les pièces protégées, et si les droits du secours qui vient de reprendre sont sûrs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Contexte {
    pub protegees: u64,
    pub droits_frais: bool,
}

/// La décision pour un ordre de classe `c`, qui atteint les pièces `ps`, maintenant.
pub fn decider(table: &Table, t_mur_ms: u64, a: &Autorite, c: u8, ps: u64, ctx: Contexte) -> u8 {
    use niveau::*;
    let b = classe::bit(c);
    // Le principal : vivant, ses pièces, ce que son canal permet.
    let mut n = match a.profil {
        Some(p) => {
            if t_mur_ms >= p.fin_mur_ms || ps & !p.pieces != 0 {
                return NON;
            }
            if p.oui & b != 0 {
                OUI
            } else if p.proposer & b != 0 {
                PROPOSER
            } else {
                NON
            }
        }
        None => NON,
    };
    // La personne : son rôle, ses pièces, sa fenêtre ; sans preuve personnelle, au plus les droits
    // d'une personne inconnue.
    let inconnu = table[usize::from(role::INCONNU)][usize::from(c)] & 3;
    let np = match a.personne {
        None if a.systeme => OUI,
        None => inconnu,
        Some(p) => {
            if t_mur_ms < p.debut_mur_ms || t_mur_ms >= p.fin_mur_ms || ps & !p.pieces != 0 {
                return NON;
            }
            let x = if p.refuse & b != 0 { NON } else { table[usize::from(p.role)][usize::from(c)] & 3 };
            // Une preuve personnelle : ses droits. Un indice : les siens pour le confort, et pour le
            // sensible, jamais plus qu'un inconnu.
            if a.preuve >= preuve::PERSONNELLE || (a.preuve == preuve::INDICE && b & classe::SENSIBLES == 0) { x } else { x.min(inconnu) }
        }
    };
    n = n.min(np);
    // Le contexte : une pièce protégée, sans preuve personnelle ou par un canal gardé ; un secours
    // qui a repris sans savoir si ses droits sont à jour.
    let garde = a.profil.is_some_and(|p| p.drapeaux & drapeau_p::GARDE_PIECES != 0);
    if ps & ctx.protegees != 0 && (garde || a.preuve < preuve::PERSONNELLE) {
        n = n.min(PROPOSER);
    }
    if !ctx.droits_frais && b & classe::SENSIBLES != 0 {
        n = n.min(PROPOSER);
    }
    // Le mandat d'un automatisme : il ne commande que son manifeste.
    if let Some(m) = a.mandat
        && (m.drapeaux & drapeau_m::VIVANT == 0 || m.classes & b == 0 || ps & !m.pieces != 0)
    {
        return NON;
    }
    if c == classe::DESARMER {
        return NON;
    }
    n
}

/// L'approbateur peut-il décider d'une retenue de classe `c`, sur les pièces `ps` ? Il lui faut un
/// écran (pas un agent, pas un satellite), une preuve personnelle, les pièces, et le droit
/// d'approuver la classe, ou, s'il est la personne pour qui l'agent a demandé, le droit de le faire
/// lui-même (confirmer sur son téléphone ce qu'il a demandé à l'assistant).
pub fn peut_approuver(table: &Table, t_mur_ms: u64, profil: &Profil, personne: Option<(u16, &Personne)>, preuve_: u8, c: u8, ps: u64, pour: u16) -> bool {
    if profil.drapeaux & drapeau_p::DECIDE == 0 || t_mur_ms >= profil.fin_mur_ms || ps & !profil.pieces != 0 || c == classe::DESARMER {
        return false;
    }
    let Some((id, p)) = personne else { return false };
    if preuve_ < preuve::PERSONNELLE || t_mur_ms < p.debut_mur_ms || t_mur_ms >= p.fin_mur_ms || ps & !p.pieces != 0 || p.refuse & classe::bit(c) != 0 {
        return false;
    }
    let x = table[usize::from(p.role)][usize::from(c)];
    x & APPROUVE != 0 || (id == pour && id != 0 && x & 3 == niveau::OUI)
}

/// Le mandat qu'un responsable peut accorder : chaque classe du manifeste doit être une classe qu'il
/// approuverait lui-même, sur toutes les pièces du manifeste.
pub fn peut_mandater(table: &Table, t_mur_ms: u64, p: &Personne, classes: u16, ps: u64) -> bool {
    if t_mur_ms < p.debut_mur_ms || t_mur_ms >= p.fin_mur_ms || ps & !p.pieces != 0 || classes & classe::bit(classe::DESARMER) != 0 {
        return false;
    }
    (0..classe::N as u8).filter(|&c| classes & classe::bit(c) != 0).all(|c| p.refuse & classe::bit(c) == 0 && table[usize::from(p.role)][usize::from(c)] & APPROUVE != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profil(oui: u16, proposer: u16, drapeaux: u8, preuve_max: u8) -> Profil {
        Profil { pieces: TOUTES_PIECES, fin_mur_ms: u64::MAX, oui, proposer, drapeaux, preuve_max, ..Profil::default() }
    }

    fn personne(role_: u8, ps: u64) -> Personne {
        Personne { pieces: ps, debut_mur_ms: 0, fin_mur_ms: u64::MAX, role: role_, ..Personne::default() }
    }

    /// Le corpus fixe des décisions (24 cas) : chaque ligne écrit la décision attendue (comme celui de
    /// la validation, revue I14) ; une table fausse ou une règle oubliée le fait échouer.
    #[test]
    fn les_decisions_suivent_le_corpus() {
        use classe::*;
        use niveau::*;
        let t = table_par_defaut();
        let ecran = profil(TOUTES & !bit(DESARMER), 0, drapeau_p::DECIDE, preuve::PERSONNELLE);
        let agent = profil(bit(CONFORT) | bit(MEDIA) | bit(CLIMAT) | bit(MESSAGE), bit(ACCES) | bit(ALARME) | bit(MACHINE) | bit(VIE_PRIVEE), drapeau_p::GARDE_PIECES, preuve::PERSONNELLE);
        let satellite = profil(bit(CONFORT) | bit(MEDIA) | bit(CLIMAT) | bit(MESSAGE), bit(ACCES) | bit(ALARME) | bit(MACHINE), drapeau_p::GARDE_PIECES, preuve::INDICE);
        let proprio = personne(role::PROPRIETAIRE, TOUTES_PIECES);
        let enfant = personne(role::ENFANT, pieces(&[3]));
        let invite = personne(role::INVITE, pieces(&[6]));
        let ctx = Contexte { protegees: pieces(&[3, 4]), droits_frais: true };
        let a = |p: &'static str| p;
        let cas: Vec<(&str, Autorite, u8, u64, u8)> = vec![
            (a("tablette sans personne : la lampe du salon"), Autorite { profil: Some(&ecran), personne: None, preuve: preuve::AUCUNE, mandat: None, systeme: false }, CONFORT, pieces(&[0]), OUI),
            (a("tablette sans personne : la porte"), Autorite { profil: Some(&ecran), personne: None, preuve: preuve::AUCUNE, mandat: None, systeme: false }, ACCES, pieces(&[5]), PROPOSER),
            (a("tablette sans personne : la chambre (protégée)"), Autorite { profil: Some(&ecran), personne: None, preuve: preuve::AUCUNE, mandat: None, systeme: false }, CONFORT, pieces(&[3]), PROPOSER),
            (a("tablette sans personne : une caméra"), Autorite { profil: Some(&ecran), personne: None, preuve: preuve::AUCUNE, mandat: None, systeme: false }, VIE_PRIVEE, pieces(&[0]), NON),
            (a("le propriétaire, PIN personnel : la porte"), Autorite { profil: Some(&ecran), personne: Some(&proprio), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, ACCES, pieces(&[5]), OUI),
            (a("le propriétaire, PIN personnel : la chambre"), Autorite { profil: Some(&ecran), personne: Some(&proprio), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, CONFORT, pieces(&[3]), OUI),
            (a("l'enfant, PIN personnel : sa chambre"), Autorite { profil: Some(&ecran), personne: Some(&enfant), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, CONFORT, pieces(&[3]), OUI),
            (a("l'enfant, PIN personnel : le salon (hors de ses pièces)"), Autorite { profil: Some(&ecran), personne: Some(&enfant), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, CONFORT, pieces(&[0]), NON),
            (a("l'enfant, PIN personnel : la machine de sa chambre"), Autorite { profil: Some(&ecran), personne: Some(&enfant), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, MACHINE, pieces(&[3]), PROPOSER),
            (a("voix inconnue : la lampe"), Autorite { profil: Some(&satellite), personne: None, preuve: preuve::AUCUNE, mandat: None, systeme: false }, CONFORT, pieces(&[0]), OUI),
            (a("voix inconnue : la porte"), Autorite { profil: Some(&satellite), personne: None, preuve: preuve::AUCUNE, mandat: None, systeme: false }, ACCES, pieces(&[5]), PROPOSER),
            (a("voix reconnue « propriétaire » (indice) : la porte, pas plus qu'inconnu"), Autorite { profil: Some(&satellite), personne: Some(&proprio), preuve: preuve::INDICE, mandat: None, systeme: false }, ACCES, pieces(&[5]), PROPOSER),
            (a("tablette, visage reconnu « propriétaire » (indice) : la porte, pas plus qu'inconnu"), Autorite { profil: Some(&ecran), personne: Some(&proprio), preuve: preuve::INDICE, mandat: None, systeme: false }, ACCES, pieces(&[5]), PROPOSER),
            (a("voix reconnue « propriétaire » (indice) : une caméra, pas plus qu'inconnu"), Autorite { profil: Some(&satellite), personne: Some(&proprio), preuve: preuve::INDICE, mandat: None, systeme: false }, VIE_PRIVEE, pieces(&[0]), NON),
            (a("voix reconnue « enfant » (indice) : le salon, restreint"), Autorite { profil: Some(&satellite), personne: Some(&enfant), preuve: preuve::INDICE, mandat: None, systeme: false }, CONFORT, pieces(&[0]), NON),
            (a("voix reconnue « enfant » (indice) : le climat de sa chambre, restreint"), Autorite { profil: Some(&satellite), personne: Some(&enfant), preuve: preuve::INDICE, mandat: None, systeme: false }, CLIMAT, pieces(&[3]), PROPOSER),
            (a("agent du propriétaire : la porte"), Autorite { profil: Some(&agent), personne: Some(&proprio), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, ACCES, pieces(&[5]), PROPOSER),
            (a("agent du propriétaire : la chambre (canal gardé)"), Autorite { profil: Some(&agent), personne: Some(&proprio), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, CONFORT, pieces(&[3]), PROPOSER),
            (a("agent du propriétaire : le salon"), Autorite { profil: Some(&agent), personne: Some(&proprio), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, CONFORT, pieces(&[0]), OUI),
            (a("agent : un réglage système"), Autorite { profil: Some(&agent), personne: Some(&proprio), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, SYSTEME, pieces(&[0]), NON),
            (a("invité : la lampe de sa pièce"), Autorite { profil: Some(&ecran), personne: Some(&invite), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, CONFORT, pieces(&[6]), OUI),
            (a("invité : la machine de sa pièce"), Autorite { profil: Some(&ecran), personne: Some(&invite), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, MACHINE, pieces(&[6]), PROPOSER),
            (a("invité : la porte d'entrée"), Autorite { profil: Some(&ecran), personne: Some(&invite), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, ACCES, pieces(&[5]), NON),
            (a("désarmer, même le propriétaire"), Autorite { profil: Some(&ecran), personne: Some(&proprio), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, DESARMER, pieces(&[5]), NON),
        ];
        let mut faux = Vec::new();
        for (nom, aut, c, ps, attendu) in &cas {
            let d = decider(&t, 1, aut, *c, *ps, ctx);
            if d != *attendu {
                faux.push(format!("{nom} : {} au lieu de {}", niveau::NOMS[usize::from(d)], niveau::NOMS[usize::from(*attendu)]));
            }
        }
        assert!(faux.is_empty(), "{faux:#?}");
        // Le secours qui a repris sans droits sûrs : rien de sensible sans un humain.
        let frais = Contexte { droits_frais: false, ..ctx };
        assert_eq!(decider(&t, 1, &Autorite { profil: Some(&ecran), personne: Some(&proprio), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, ACCES, pieces(&[5]), frais), PROPOSER);
        assert_eq!(decider(&t, 1, &Autorite { profil: Some(&ecran), personne: Some(&proprio), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, CONFORT, pieces(&[0]), frais), OUI);
        // Aucune table, aucun profil n'ouvre le désarmement, ni au hub (revue v3, D9).
        let mut ouverte = t;
        ouverte[usize::from(role::PROPRIETAIRE)][usize::from(DESARMER)] = OUI | APPROUVE;
        let tout = profil(TOUTES, 0, drapeau_p::DECIDE, preuve::PERSONNELLE);
        assert_eq!(decider(&ouverte, 1, &Autorite { profil: Some(&tout), personne: Some(&proprio), preuve: preuve::PERSONNELLE, mandat: None, systeme: false }, DESARMER, pieces(&[5]), ctx), NON);
        assert_eq!(decider(&ouverte, 1, &Autorite { profil: Some(&tout), personne: None, preuve: preuve::PERSONNELLE, mandat: None, systeme: true }, DESARMER, pieces(&[5]), ctx), NON);
        assert!(!peut_approuver(&ouverte, 1, &tout, Some((1, &proprio)), preuve::PERSONNELLE, DESARMER, pieces(&[5]), 1));
    }

    /// Le choix du propriétaire (v3 § 22) : une voix reconnue vaut sa personne pour le confort, jamais
    /// pour le sensible. Une maison qui refuse le média à une voix inconnue le laisse à la voix d'Alice.
    #[test]
    fn une_voix_reconnue_vaut_sa_personne_pour_le_confort() {
        use classe::*;
        use niveau::*;
        let mut t = table_par_defaut();
        t[usize::from(role::INCONNU)][usize::from(MEDIA)] = NON;
        let satellite = profil(bit(CONFORT) | bit(MEDIA) | bit(CLIMAT) | bit(MESSAGE), bit(ACCES) | bit(ALARME) | bit(MACHINE), drapeau_p::GARDE_PIECES, preuve::INDICE);
        let adulte = personne(role::ADULTE, TOUTES_PIECES);
        let ctx = Contexte { protegees: pieces(&[3]), droits_frais: true };
        let a = |p: Option<&Personne>, pv| decider(&t, 1, &Autorite { profil: Some(&satellite), personne: p, preuve: pv, mandat: None, systeme: false }, MEDIA, pieces(&[0]), ctx);
        assert_eq!(a(None, preuve::AUCUNE), NON, "une voix inconnue");
        assert_eq!(a(Some(&adulte), preuve::INDICE), OUI, "la voix reconnue d'Alice : son média");
        let sensible = decider(&t, 1, &Autorite { profil: Some(&satellite), personne: Some(&adulte), preuve: preuve::INDICE, mandat: None, systeme: false }, ACCES, pieces(&[5]), ctx);
        assert_eq!(sensible, PROPOSER, "le sensible : jamais plus qu'une voix inconnue");
        let chambre = decider(&t, 1, &Autorite { profil: Some(&satellite), personne: Some(&adulte), preuve: preuve::INDICE, mandat: None, systeme: false }, CONFORT, pieces(&[3]), ctx);
        assert_eq!(chambre, PROPOSER, "une chambre : un humain");
    }

    #[test]
    fn seul_un_habilite_approuve() {
        use classe::*;
        let t = table_par_defaut();
        let ecran = Profil { pieces: TOUTES_PIECES, fin_mur_ms: u64::MAX, oui: TOUTES, drapeaux: drapeau_p::DECIDE, preuve_max: preuve::PERSONNELLE, ..Profil::default() };
        let telephone = ecran;
        let agent = Profil { drapeaux: drapeau_p::GARDE_PIECES, ..ecran };
        let proprio = personne(role::PROPRIETAIRE, TOUTES_PIECES);
        let adulte = personne(role::ADULTE, TOUTES_PIECES);
        let enfant = personne(role::ENFANT, TOUTES_PIECES);
        let porte = pieces(&[5]);
        assert!(peut_approuver(&t, 1, &ecran, Some((1, &proprio)), preuve::PERSONNELLE, ACCES, porte, 9));
        assert!(!peut_approuver(&t, 1, &ecran, Some((1, &proprio)), preuve::INDICE, ACCES, porte, 9), "sans preuve personnelle");
        assert!(!peut_approuver(&t, 1, &ecran, None, preuve::AUCUNE, ACCES, porte, 9), "une tablette sans personne");
        assert!(!peut_approuver(&t, 1, &ecran, Some((3, &enfant)), preuve::PERSONNELLE, ACCES, porte, 9), "un enfant n'approuve pas une ouverture");
        assert!(!peut_approuver(&t, 1, &agent, Some((1, &proprio)), preuve::PERSONNELLE, ACCES, porte, 9), "un agent n'approuve pas");
        assert!(!peut_approuver(&t, 1, &ecran, Some((2, &adulte)), preuve::PERSONNELLE, SYSTEME, pieces(&[0]), 9), "un adulte n'approuve pas un réglage système");
        assert!(peut_approuver(&t, 1, &telephone, Some((2, &adulte)), preuve::PERSONNELLE, VIE_PRIVEE, pieces(&[0]), 2), "la personne confirme ce qu'elle a demandé à son agent");
        assert!(!peut_approuver(&t, 1, &telephone, Some((2, &adulte)), preuve::PERSONNELLE, VIE_PRIVEE, pieces(&[0]), 9), "mais pas pour un autre");
        assert!(!peut_approuver(&t, 1, &ecran, Some((1, &proprio)), preuve::PERSONNELLE, DESARMER, porte, 1), "désarmer n'est jamais approuvé");
    }

    #[test]
    fn un_mandat_ne_depasse_pas_son_responsable() {
        use classe::*;
        let t = table_par_defaut();
        let proprio = personne(role::PROPRIETAIRE, TOUTES_PIECES);
        let adulte = personne(role::ADULTE, TOUTES_PIECES);
        let enfant = personne(role::ENFANT, pieces(&[3]));
        assert!(peut_mandater(&t, 1, &proprio, bit(ACCES) | bit(CONFORT), pieces(&[0, 5])));
        assert!(!peut_mandater(&t, 1, &proprio, bit(DESARMER), pieces(&[5])), "un automatisme ne désarme jamais");
        assert!(!peut_mandater(&t, 1, &adulte, bit(SYSTEME), pieces(&[0])));
        assert!(!peut_mandater(&t, 1, &enfant, bit(CONFORT), pieces(&[3])), "un enfant ne valide pas d'automatisme");
    }
}
