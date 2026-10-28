//! Le hub réduit, v2 : les trois registres (appareils, valeurs, ordres), le catalogue des ordres, les
//! demandes, les reçus, les principaux et leurs permis, la garde, les issues et les sorties critiques
//! (au rang `s`, et au rang de leur pièce). La même structure sert à l'actif (il décide) et à la
//! copie de secours (elle applique les sorties).
//!
//! Les valeurs et les ordres sont séparés : un point de valeur n'est jamais un ordre, un ordre n'a
//! jamais de valeur. La forme d'un ordre cite une valeur de son appareil par sa clé (la valeur qui le
//! prouve, la valeur qu'un paramètre règle) ; c'est le seul lien.
//!
//! Ce que la revue Codex a changé, et où :
//! - B1 : le principal vient du sujet (`req/<client>`), jamais du contenu ; chaque cible est
//!   autorisée, filles d'un groupe comprises (`permis`).
//! - B2 : un appui attend l'accusé du secours avant de partir ; une reprise non sûre refuse les
//!   nouvelles tentatives ambiguës (`lancer`, `acquitter`, `demander`).
//! - B4 : les cartes, les paramètres et les textes privés ont chacun leur tiroir, clé par numéro.
//! - B5 : un texte privé ne voyage que par le canal privé (`Sortie::Prive`), jamais sur `cmd/` ni `ord/`.
//! - I1, I2, I3 : le permis, l'échéance et la forme sont revérifiés à chaque frontière (`decider`).
//! - I4 : seule une lecture commencée après l'envoi (ou après la reprise) confirme.
//! - I5, I6 : la fiche d'un ordre ne suit que sa dernière demande admise, et juge sa preuve.
//! - I7, I8, I9, I10 : un seul chemin d'envoi ; la mère compte ses filles par niveau de preuve, et
//!   sa progression est rejouée par la copie ; une cible dit de quel ordre de groupe elle dépend.
//! - I11 : une redéclaration garde les numéros par clé ; les listes d'un appareil sont des numéros.
//! - I12 : chaque plafond est vérifié avant toute mutation, avec une erreur.
//! - I13 : chaque sortie porte le rang de sa pièce : un écran filtré voit ses trous.
//!
//! Ce que la v3 ajoute (la question des droits, et la deuxième revue Codex) :
//! - trois identités par demande : le principal (la connexion), la personne (et la preuve qu'on en
//!   a), le mandat (un automatisme) ; une décision par classe d'ordre (`droits`), à l'arrivée, à
//!   l'approbation et à la remise au module (`niveau`, `remettre`) ;
//! - une session personnelle sur un appareil partagé ; un indice (la voix) ne fait que restreindre ;
//! - un approbateur habilité pour l'opération et ses pièces ; une carte a une révision ;
//! - la personne ne voyage jamais sur `ord/` : une sortie privée (`Sortie::Autorite`) la porte au
//!   pair ; les droits passent au secours (`Sortie::Droits`), par nom, jamais par position ;
//! - désarmer ne passe jamais par le chemin générique (`raison::DEDIE`).

use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::catalogue::{AUCUNE_CLE, Arg, Catalogue, Fiche, FormeDecl, drapeau, effet, nature, preuve, raison, retour};
use crate::droits::{self, Mandat, Personne, Profil, Session, Table, classe, drapeau_m, drapeau_p, niveau};
use crate::demandes::{Demande, Demandes, Recu, Recus, Ref, Verdict, drapeau_d, etat, issue, origine};
use crate::numeros::{Fnv, Numero, Registre, get16, get32, get64, put16, put32, put64, take};

/// L'échéance d'une demande : le hub attend 5 s (`CommandRequest::deadline` aujourd'hui).
pub const ECHEANCE_MS: u64 = 5_000;
/// Une retenue attend un humain 10 min (`APPROVAL_TTL_MS` aujourd'hui).
pub const RETENUE_MS: u64 = 10 * 60 * 1_000;
/// Une demande héritée (bascule, redémarrage, module mort) est suivie au moins 30 s par le hub.
pub const SUIVI_HERITE_S: u16 = 30;
/// Un appui attend l'accusé du secours au plus 200 ms ; au-delà, l'actif inscrit chez le témoin
/// que le secours a décroché, puis envoie.
pub const ATTENTE_SECOURS_MS: u64 = 200;
/// Les plafonds, vérifiés avant toute mutation.
pub const DEMANDES_MAX: usize = 4_096;
pub const RETENUES_MAX: usize = 50;
pub const CIBLES_MAX: usize = 64;
pub const PRINCIPAUX_MAX: usize = 256;
pub const PERSONNES_MAX: usize = 64;
/// Les sorties qui concernent tout le monde (formes, lot), et celles du seul pair (canal privé, baux).
pub const PIECE_GLOBALE: u16 = u16::MAX;
pub const PIECE_PRIVEE: u16 = u16::MAX - 1;
/// Une entrée de composition qui n'est pas une cible.
pub const SANS_SOURCE: u8 = 0xFF;

/// Les deux horloges, en millisecondes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct T {
    pub mono_ms: u64,
    pub mur_ms: u64,
}

impl T {
    /// Une heure murale, dans l'horloge monotone de cette machine.
    pub fn mono_de(self, mur: u64) -> u64 {
        if mur == u64::MAX {
            return u64::MAX;
        }
        self.mono_ms.saturating_add(mur.saturating_sub(self.mur_ms))
    }
    pub fn mur_de(self, mono: u64) -> u64 {
        if mono == u64::MAX {
            return u64::MAX;
        }
        self.mur_ms.saturating_add(mono.saturating_sub(self.mono_ms))
    }
}

/// Un appareil : sa liste (les numéros de ses valeurs, puis de ses ordres), sa composition, sa
/// pièce. 16 o.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct App {
    /// Début de sa liste dans `listes`.
    pub liste: u32,
    /// Début de sa composition dans `membres`.
    pub compo: u32,
    pub piece: u16,
    pub nb_points: u8,
    pub nb_ordres: u8,
    pub nb_compo: u8,
    pub genre: u8,
    pub _r: u16,
}

pub mod genre {
    /// Ses valeurs et ses ordres viennent d'un module.
    pub const PHYSIQUE: u8 = 0;
    /// Il assemble des valeurs et des ordres d'autres appareils, et peut en avoir à lui (une moyenne,
    /// un ordre de groupe).
    pub const VIRTUEL: u8 = 1;
}

/// Ce qu'une entrée de composition désigne.
pub mod sorte {
    pub const VALEUR: u8 = 1;
    pub const ORDRE: u8 = 2;
    pub const APPAREIL: u8 = 3;
}

/// Ce que l'appareil virtuel en fait.
pub mod role {
    /// Il le montre : une valeur ou un ordre d'un autre appareil, à sa place dans sa page.
    pub const AFFICHE: u8 = 1;
    /// Une de ses valeurs se calcule avec (une moyenne).
    pub const SOURCE: u8 = 2;
    /// Un de ses ordres de groupe le commande.
    pub const CIBLE: u8 = 3;
    /// Un appareil membre : la garde lit aussi sa pièce.
    pub const MEMBRE: u8 = 4;
}

/// Une entrée de composition : 8 o. Le numéro garde sa génération : une place libérée puis
/// réutilisée ne fait jamais entrer l'appareil neuf dans la composition de l'ancien.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Membre {
    pub numero: u32,
    /// Sa place dans la page de l'appareil virtuel.
    pub rang: u8,
    pub sorte: u8,
    pub role: u8,
    /// Pour une cible : le rang, parmi les ordres de l'appareil virtuel, de l'ordre de groupe qui la
    /// commande (`SANS_SOURCE` sinon).
    pub source: u8,
}

/// Ce que la mémoire v2 dit d'une valeur et qu'un écran affiche, réduit (`Def` fait 16 o dans la
/// v2) : son dictionnaire, sa nature, son unité, son échelle. 8 o.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct DefPoint {
    pub dico: u16,
    pub nature: u8,
    pub unite: u8,
    pub echelle: i8,
    pub _r: [u8; 3],
}

/// La valeur vive, réduite à ce que les ordres lisent (la mémoire v2 en garde 40 o).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vif {
    pub valeur: i64,
    pub change_ms: u64,
    pub mouvement: bool,
}

/// Une ligne du journal en mémoire (l'écran) : 48 o, des numéros et des codes ; les noms sont
/// résolus à l'affichage ; ce qui dure (`run/`, `hist/ops`) porte les noms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Ligne {
    pub instant_ms: u64,
    pub valeur: i64,
    pub demande: Ref,
    pub numero: u32,
    pub ordre: u32,
    pub qui: u32,
    pub duree_ms: u32,
    pub de: u16,
    pub origine: u8,
    pub issue: u8,
    pub raison: u8,
    /// 1 : une demande ; 2 : sa suite ; 3 : une décision humaine.
    pub action: u8,
    /// La personne de l'événement : celle de la demande, ou l'approbateur d'une décision (v3, M2).
    pub pour: u16,
}

const _: () = assert!(std::mem::size_of::<App>() == 16);
const _: () = assert!(std::mem::size_of::<Membre>() == 8);
const _: () = assert!(std::mem::size_of::<DefPoint>() == 8);
const _: () = assert!(std::mem::size_of::<Ligne>() == 48);

pub const JOURNAL: usize = 500;

/// Qui demande : un compte (un écran, l'appli, une tablette, un satellite, un agent, un script), le
/// moteur, le hub. Le serveur le déduit de la connexion ; un client ne le déclare jamais (revue B1).
/// Son profil (32 o) dit ce que le canal permet ; la personne vient d'une session personnelle, de la
/// personne fixe du compte, ou d'un indice (v3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    pub nom: String,
    pub p: Profil,
    /// La session personnelle en cours sur cet appareil (le PIN personnel tapé sur la tablette) : les
    /// nouvelles demandes prennent cette personne ; à sa fin, l'appareil redevient anonyme (revue v3, D1).
    pub session: Option<Session>,
}

impl std::ops::Deref for Principal {
    type Target = Profil;
    fn deref(&self) -> &Profil {
        &self.p
    }
}

impl std::ops::DerefMut for Principal {
    fn deref_mut(&mut self) -> &mut Profil {
        &mut self.p
    }
}

/// Les classes qu'un agent, un satellite ou un script commande d'eux-mêmes ; le reste, ils le proposent.
pub const CLASSES_AGENT: u16 = classe::bit(classe::CONFORT) | classe::bit(classe::MEDIA) | classe::bit(classe::CLIMAT) | classe::bit(classe::MESSAGE);
pub const PROPOSE_AGENT: u16 = classe::bit(classe::ACCES) | classe::bit(classe::ALARME) | classe::bit(classe::MACHINE) | classe::bit(classe::VIE_PRIVEE);
const TOUT_SAUF_DESARMER: u16 = classe::TOUTES & !classe::bit(classe::DESARMER);

impl Principal {
    fn profil(nom: &str, origine: u8, qui: u32, personne: u16, oui: u16, proposer: u16, drapeaux: u8, preuve_max: u8) -> Self {
        let p = Profil { pieces: droits::TOUTES_PIECES, fin_mur_ms: u64::MAX, qui, personne, oui, proposer, origine, preuve_max, drapeaux, _r: [0; 3] };
        Self { nom: nom.into(), p, session: None }
    }
    /// L'écran ou l'appli personnels de la personne `qui` : il décide d'une carte.
    pub fn humain(nom: &str, qui: u32) -> Self {
        Self::profil(nom, origine::UI, qui, qui as u16, TOUT_SAUF_DESARMER, 0, drapeau_p::DECIDE, droits::preuve::PERSONNELLE)
    }
    /// Une tablette partagée : personne tant qu'aucune session personnelle n'est ouverte.
    pub fn tablette(nom: &str) -> Self {
        Self::profil(nom, origine::UI, 0, 0, TOUT_SAUF_DESARMER, 0, drapeau_p::DECIDE, droits::preuve::PERSONNELLE)
    }
    /// Un satellite vocal : au mieux un indice de la personne qui parle.
    pub fn satellite(nom: &str) -> Self {
        Self::profil(nom, origine::ASSISTANT, 0, 0, CLASSES_AGENT, PROPOSE_AGENT & !classe::bit(classe::VIE_PRIVEE), drapeau_p::GARDE_PIECES, droits::preuve::INDICE)
    }
    /// Un agent ou un script ; `.pour(p)` : mandaté par la personne `p`.
    pub fn agent(nom: &str, origine: u8, qui: u32) -> Self {
        Self::profil(nom, origine, qui, 0, CLASSES_AGENT, PROPOSE_AGENT, drapeau_p::GARDE_PIECES, droits::preuve::PERSONNELLE)
    }
    pub fn interne(nom: &str, origine: u8) -> Self {
        Self::profil(nom, origine, 0, 0, TOUT_SAUF_DESARMER, 0, drapeau_p::INTERNE, droits::preuve::PERSONNELLE)
    }
    /// Le téléphone d'un invité : la personne `qui` (son rôle, ses pièces, sa fenêtre sont dans sa fiche).
    pub fn invite(nom: &str, qui: u32, pieces: Vec<u16>, fin_mur_ms: u64) -> Self {
        let mut p = Self::profil(nom, origine::UI, qui, qui as u16, TOUT_SAUF_DESARMER, 0, drapeau_p::DECIDE, droits::preuve::PERSONNELLE);
        p.p.pieces = droits::pieces(&pieces);
        p.p.fin_mur_ms = fin_mur_ms;
        p
    }
    pub fn pour(mut self, personne: u16) -> Self {
        self.p.personne = personne;
        self
    }
    pub fn est_interne(&self) -> bool {
        self.p.drapeaux & drapeau_p::INTERNE != 0
    }
}

/// L'autorité d'une demande du moteur, que seul un principal interne peut donner (revue v3, W1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AutoriteMoteur {
    /// Une personne l'a lancé (une routine, une scène) : ses droits et ceux de sa connexion bornent
    /// chaque ordre. Sinon, un déclencheur : le mandat et les droits de son responsable.
    pub appele: bool,
    /// La connexion de l'appelant.
    pub de: u16,
    pub pour: u16,
    pub preuve: u8,
    /// Le numéro du mandat : l'automatisme + 1.
    pub mandat: u16,
}

/// L'autorité d'une demande, telle que le hub la range : la connexion (`de`), la personne et sa
/// preuve, le mandat ; `systeme` : le hub lui-même, ou un mandat adopté par le foyer (aucune
/// personne ne le borne).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Aut {
    pub de: u16,
    pub pour: u16,
    pub preuve: u8,
    pub mandat: u16,
    pub systeme: bool,
}

/// Une entrée de composition déclarée : (sorte, nom, rôle, clé de l'ordre de groupe pour une cible).
pub type MembreDecl = (u8, String, u8, String);

/// Ce qu'un module déclare d'un appareil.
#[derive(Clone, Debug)]
pub struct AppDecl {
    pub nom: String,
    pub piece: u16,
    /// Les valeurs : leurs clés.
    pub points: Vec<String>,
    /// La nature, l'unité, l'échelle de chaque valeur (vide : inconnues).
    pub defs: Vec<DefPoint>,
    /// Les ordres : leurs clés et leur structure.
    pub ordres: Vec<(String, FormeDecl)>,
    /// Un appareil virtuel : ce qu'il emprunte, par les noms.
    pub membres: Vec<MembreDecl>,
}

impl AppDecl {
    pub fn physique(nom: &str, piece: u16, points: &[&str], ordres: Vec<(&str, FormeDecl)>) -> Self {
        Self {
            nom: nom.into(),
            piece,
            points: points.iter().map(|p| p.to_string()).collect(),
            defs: Vec::new(),
            ordres: ordres.into_iter().map(|(k, f)| (k.to_string(), f)).collect(),
            membres: Vec::new(),
        }
    }
}

/// Une demande, telle qu'elle arrive sur `req/<client>` (le nom de l'ordre) ou d'un client interne
/// (son numéro).
#[derive(Clone, Debug)]
pub struct Req {
    /// Le principal : déduit par le serveur du sujet `req/<client>` (l'ACL du courtier garantit qu'un
    /// compte ne publie que sur le sien), jamais du contenu.
    pub de: u16,
    /// Sa séquence : avec le principal, la clé d'idempotence de **toute** demande externe (≥ 1) ;
    /// 0 pour une demande interne.
    pub seq: u64,
    pub ordre: Cible,
    pub args: Vec<Arg>,
    /// L'échéance de l'appelant, en heure murale (0 : aucune).
    pub echeance_mur_ms: u64,
    /// La tentative (1 : la première) et l'heure murale de la première.
    pub tentative: u8,
    pub premiere_mur_ms: u64,
    /// Une exécution du moteur (case | génération) : seul un principal interne le donne.
    pub qui: u32,
    pub parent: Ref,
    /// Une personne que le canal croit reconnaître (une voix) : un indice, jamais plus (v3).
    pub indice: u16,
    /// L'époque de la session sur laquelle le client l'a émise (0 : la session en cours). Après une
    /// reprise non sûre, un appui émis avant la reprise est ambigu, même « première tentative »
    /// (revue v3, M7 : ni l'horloge ni le compteur du client ne prouvent qu'il n'est pas parti).
    pub emise_epoque: u32,
    /// L'autorité d'une exécution : seul un principal interne la donne.
    pub autorite: Option<AutoriteMoteur>,
}

impl Req {
    pub fn nouvelle(de: u16, seq: u64, ordre: Cible, args: Vec<Arg>, t: T) -> Self {
        Self { de, seq, ordre, args, echeance_mur_ms: 0, tentative: 1, premiere_mur_ms: t.mur_ms, qui: 0, parent: Ref(0), indice: 0, emise_epoque: 0, autorite: None }
    }
}

#[derive(Clone, Debug)]
pub enum Cible {
    Nom(String),
    Numero(Numero),
}

/// Un ordre remis à un module, sur `cmd/<module>/<appareil>/<ordre>` : le numéro de la demande, celui
/// de l'ordre, sa forme d'admission, les arguments (un texte privé n'y est qu'une longueur),
/// l'échéance, l'époque.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    pub d: Ref,
    pub ordre: Numero,
    pub appareil: u32,
    pub forme: u16,
    pub args: Vec<Arg>,
    pub echeance_ms: u64,
    pub epoque: u32,
}

/// L'issue rendue à qui l'a demandée, sur `res/<client>` : sa séquence, la demande, l'issue ; pour
/// un groupe, combien de cibles ont obéi sur combien.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Res {
    pub de: u16,
    pub seq: u64,
    pub d: Ref,
    pub issue: u8,
    pub raison: u8,
    pub suite: bool,
    pub n: u8,
    pub m: u8,
}

/// La carte d'une retenue : ce que l'humain voit et approuve (V7 : ce qu'il approuve ne se déguise
/// pas) ; la forme et les cibles qu'elle montre (revue I3 : une approbation se lie à elles).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Carte {
    pub texte: Box<str>,
    pub forme: u16,
    pub cibles: Box<[u32]>,
    /// Sa révision : la décision la nomme, une seule fois (revue v3, D4).
    pub revision: u32,
}

/// Une sortie critique : publiée au rang `s` et rejouée telle quelle par chaque copie.
#[derive(Clone, Debug, PartialEq)]
pub enum Sortie {
    /// `meta/forme/<f>` : une structure d'ordre, une fois.
    Forme { f: u16, decl: FormeDecl },
    /// `meta/<appareil>` : ses valeurs `(n, clé)`, leurs définitions, ses ordres `(n, clé, f)`, sa
    /// composition. Une redéclaration republie la liste entière : ce qui n'y est plus est retiré.
    Appareil { n: Numero, nom: String, piece: u16, genre: u8, points: Vec<(Numero, String)>, defs: Vec<DefPoint>, ordres: Vec<(Numero, String, u16)>, membres: Vec<Membre> },
    /// `val/` : une valeur, et l'heure murale où sa lecture a commencé.
    Valeur { point: Numero, valeur: i64, change_ms: u64, mouvement: bool, lu_ms: u64 },
    /// `ord/<appareil>/<ordre>` : la trace d'une demande, à chaque pas ; la fiche de la demande et
    /// celle de l'ordre après le pas, le reçu, la carte et les paramètres publics. Ni texte privé, ni
    /// contenu : une référence. L'échéance voyage en heure murale.
    Trace { d: Ref, demande: Demande, echeance_mur_ms: u64, ordre: Numero, fiche: Fiche, recu: Option<Recu>, carte: Option<Carte>, params: Option<Box<[Arg]>>, ligne: Ligne },
    /// Le canal privé, vers le seul pair (ACL du courtier) : le texte privé d'une demande vivante.
    Prive { d: Ref, texte: Box<str> },
    /// Le statut d'un appareil (retiré), avec ses valeurs et ses ordres.
    Statut { appareil: Numero, statut: u8 },
    /// `meta/lot` : les places libérées dans les trois registres.
    Lot { appareils: Vec<u32>, points: Vec<u32>, ordres: Vec<u32> },
    /// `wf/bail/<b>` : un bail du moteur (ouvert, ou fermé si `None`).
    Bail { b: u32, bail: Option<crate::moteur::Bail> },
    /// Un permis qui finit plus tôt que prévu (une invitation révoquée).
    Revocation { de: u16, fin_mur_ms: u64 },
    /// Les droits, au pair seulement (v3, revue M3) : les personnes et les profils par leur nom, la
    /// table, les mandats, la révision.
    Droits { rev: u32, personnes: Vec<(String, Personne)>, profils: Vec<(String, Profil)>, table: Box<Table>, mandats: Vec<Mandat> },
    /// Une session personnelle ouverte ou fermée sur un appareil, au pair seulement.
    Session { de: u16, session: Option<Session> },
    /// L'identité d'une trace, au pair seulement : `ord/` ne la porte jamais (revue v3, D8).
    Autorite { d: Ref, ordre: Numero, pour: u16, preuve: u8, fiche_pour: u16, fiche_preuve: u8, ligne_pour: u16 },
    /// `wf/ex/<c>` : une exécution du moteur, au pair seulement (revue v3, W5).
    Execution { c: u32, ex: Option<crate::moteur::Execution> },
}

/// Une sortie publiée : l'époque de l'actif qui la publie, son rang `s`, sa pièce et le rang de
/// cette pièce (`sp`) : un écran qui ne voit que certaines pièces vérifie la continuité de chacune
/// (revue I13) ; une époque nouvelle (une bascule, un redémarrage) fait se recharger copies et
/// écrans par l'état complet.
#[derive(Clone, Debug, PartialEq)]
pub struct Publiee {
    pub e: u32,
    pub s: u64,
    pub piece: u16,
    pub sp: u64,
    pub sortie: Sortie,
}

#[derive(Clone)]
pub struct Hub {
    pub actif: bool,
    pub epoque: u32,
    pub appareils: Registre,
    pub points: Registre,
    pub ordres: Registre,
    pub apps: Vec<App>,
    /// Les listes des appareils, bout à bout ; une redéclaration en ajoute une, l'ancienne meurt
    /// jusqu'au lot.
    pub listes: Vec<u32>,
    pub listes_mortes: usize,
    /// Les compositions des appareils virtuels, bout à bout.
    pub membres: Vec<Membre>,
    pub membres_morts: usize,
    pub vifs: Vec<Vif>,
    /// Par place de valeur : la place de son appareil, sa clé, sa définition.
    pub app_de: Vec<u32>,
    pub cle_point: Vec<u16>,
    pub defs: Vec<DefPoint>,
    pub catalogue: Catalogue,
    pub demandes: Demandes,
    pub recus: Recus,
    /// Les paramètres publics d'une demande (au-delà du premier, ou un texte), par son numéro.
    pub params: HashMap<Ref, Box<[Arg]>>,
    /// La carte d'une retenue, par son numéro.
    pub cartes: HashMap<Ref, Carte>,
    /// Les textes privés, par numéro : chez l'actif et son pair, jamais ailleurs, effacés à la
    /// conclusion.
    pub prives: HashMap<Ref, Box<str>>,
    /// Les demandes héritées d'une bascule, d'un redémarrage ou d'un module mort : le hub les suit,
    /// et seule une lecture commencée après `depuis` (heure murale) les confirme.
    pub heritees: Vec<(Ref, u64)>,
    pub pieces_protegees: Vec<u16>,
    pub principaux: Vec<Principal>,
    pub journal: VecDeque<Ligne>,
    pub prochaine_ligne: u32,
    /// Le rang `s` de la dernière sortie, et celui de chaque pièce.
    pub rang: u64,
    pub rangs_pieces: BTreeMap<u16, u64>,
    pub sorties: Vec<Publiee>,
    pub cmd: Vec<Cmd>,
    pub res: Vec<Res>,
    pub notices: Vec<String>,
    /// Les demandes qu'un module doit retirer de sa voie si elles ne sont pas parties (un permis
    /// révoqué).
    pub annulations: Vec<Ref>,
    /// Le secours est en phase : un appui attend son accusé avant de partir.
    pub secours_en_phase: bool,
    /// Le rang le plus haut que le secours a appliqué.
    pub acquitte: u64,
    /// Les appuis qui attendent l'accusé du secours : (rang de leur trace, depuis (monotone), ordre).
    pub attente_secours: Vec<(u64, u64, Cmd)>,
    /// L'actif a inscrit chez le témoin que le secours a décroché (et envoie sans accusé).
    pub decroche: bool,
    /// Une réduction de droits publiée au rang `s`, qui attend l'accusé du secours depuis `depuis`
    /// (monotone) : sans lui au bout de 200 ms, l'actif inscrit le décrochage chez le témoin, et une
    /// reprise ensuite ne croit pas ses droits frais (revue v3, M3).
    pub droits_a_accuser: Option<(u64, u64)>,
    /// L'époque née de la dernière reprise non sûre (une bascule, un redémarrage après lesquels un
    /// appui a pu partir sans que cette mémoire le sache) ; 0 : aucune. Rangée dans l'instantané : une
    /// reprise sûre plus tard ne l'efface pas (revue v3, M7).
    pub epoque_non_sure: u32,
    /// Désactivé par une faute injectée, pour la preuve.
    pub controle_generation: bool,
    /// v3 : les personnes (la place 0 n'existe pas), leurs noms, la table rôles × classes, les
    /// mandats des automatismes, la révision des droits ; et si les droits de cette machine sont sûrs
    /// (faux après une reprise non sûre : rien de sensible sans un humain, revue v3 M3).
    pub personnes: Vec<Personne>,
    pub noms_personnes: Vec<String>,
    pub table: Table,
    pub mandats: Vec<Mandat>,
    pub droits_rev: u32,
    pub droits_frais: bool,
    pub revision: u32,
}

impl Default for Hub {
    fn default() -> Self {
        Self {
            actif: true,
            epoque: 1,
            appareils: Registre::default(),
            points: Registre::default(),
            ordres: Registre::default(),
            apps: Vec::new(),
            listes: Vec::new(),
            listes_mortes: 0,
            membres: Vec::new(),
            membres_morts: 0,
            vifs: Vec::new(),
            app_de: Vec::new(),
            cle_point: Vec::new(),
            defs: Vec::new(),
            catalogue: Catalogue::new(),
            demandes: Demandes::default(),
            recus: Recus::default(),
            params: HashMap::new(),
            cartes: HashMap::new(),
            prives: HashMap::new(),
            heritees: Vec::new(),
            pieces_protegees: Vec::new(),
            principaux: Vec::new(),
            journal: VecDeque::with_capacity(JOURNAL),
            prochaine_ligne: 1,
            rang: 0,
            rangs_pieces: BTreeMap::new(),
            sorties: Vec::new(),
            cmd: Vec::new(),
            res: Vec::new(),
            notices: Vec::new(),
            annulations: Vec::new(),
            secours_en_phase: false,
            acquitte: 0,
            attente_secours: Vec::new(),
            decroche: false,
            droits_a_accuser: None,
            epoque_non_sure: 0,
            controle_generation: true,
            personnes: vec![Personne::default()],
            noms_personnes: vec![String::new()],
            table: droits::table_par_defaut(),
            mandats: Vec::new(),
            droits_rev: 0,
            droits_frais: true,
            revision: 0,
        }
    }
}

/// L'empreinte d'arguments (les textes privés y comptent par leur contenu : deux messages différents
/// sous la même clé sont un conflit).
pub fn empreinte_args(args: &[Arg]) -> u64 {
    // L'encodage canonique : le nombre d'arguments, puis pour chacun son type, sa longueur et sa
    // valeur (revue v3, M6) ; une empreinte de 64 bits.
    let mut h = Fnv::default();
    h.u64(args.len() as u64);
    for a in args {
        match a {
            Arg::N(v) => {
                h.u64(0);
                h.u64(*v as u64);
            }
            Arg::T(t) => {
                h.u64(1);
                h.u64(t.len() as u64);
                h.octets(t.as_bytes());
            }
            Arg::P(n) => {
                h.u64(2);
                h.u64(u64::from(*n));
            }
        }
    }
    h.0
}

impl Hub {
    /// Un compte de plus ; au plafond, un refus (revue v3, M1), jamais une panne.
    pub fn ajouter_principal(&mut self, p: Principal) -> Result<u16, &'static str> {
        if self.principaux.len() >= PRINCIPAUX_MAX {
            return Err("plafond des principaux");
        }
        if p.p.personne != 0 && usize::from(p.p.personne) >= self.personnes.len() {
            return Err("personne inconnue");
        }
        self.principaux.push(p);
        Ok((self.principaux.len() - 1) as u16)
    }

    // ---- les droits (v3) -----------------------------------------------------------------------

    pub fn ajouter_personne(&mut self, nom: &str, p: Personne) -> Result<u16, &'static str> {
        if self.personnes.len() >= PERSONNES_MAX {
            return Err("plafond des personnes");
        }
        self.personnes.push(p);
        self.noms_personnes.push(nom.into());
        self.droits_rev += 1;
        Ok((self.personnes.len() - 1) as u16)
    }

    pub fn personne(&self, nom: &str) -> u16 {
        self.noms_personnes.iter().position(|n| n == nom).map_or(0, |k| k as u16)
    }

    /// Une session personnelle s'ouvre sur un appareil (un PIN personnel, pour quelques minutes) ;
    /// elle remplace la précédente.
    pub fn ouvrir_session(&mut self, de: u16, personne: u16, preuve_: u8, fin_mur_ms: u64) {
        let s = Session { fin_mur_ms, personne, preuve: preuve_, _r: [0; 5] };
        if let Some(p) = self.principaux.get_mut(usize::from(de)) {
            p.session = Some(s);
            self.sortir(Sortie::Session { de, session: Some(s) });
        }
    }

    pub fn fermer_session(&mut self, de: u16) {
        if let Some(p) = self.principaux.get_mut(usize::from(de))
            && p.session.take().is_some()
        {
            self.sortir(Sortie::Session { de, session: None });
        }
    }

    /// Les mandats des automatismes, posés à leur validation (le moteur les a calculés et vérifiés).
    /// Seul l'actif les publie ; une copie les reçoit par `Sortie::Droits`.
    pub fn mandater(&mut self, mandats: Vec<Mandat>) {
        if self.mandats != mandats {
            self.mandats = mandats;
            if self.actif {
                self.droits_rev += 1;
                self.publier_droits();
            }
        }
    }

    /// Les droits partent au pair (et jamais aux écrans).
    pub fn publier_droits(&mut self) {
        let personnes = self.noms_personnes.iter().cloned().zip(self.personnes.iter().copied()).collect();
        let profils = self.principaux.iter().map(|p| (p.nom.clone(), p.p)).collect();
        self.sortir(Sortie::Droits { rev: self.droits_rev, personnes, profils, table: Box::new(self.table), mandats: self.mandats.clone() });
    }

    /// Une personne perd ses droits maintenant : ses sessions se ferment, ses retenues tombent, ce
    /// qui attend l'accusé du secours ne part pas, ce qui est dans une voie en est retiré.
    pub fn revoquer_personne(&mut self, t: T, id: u16) {
        if let Some(p) = self.personnes.get_mut(usize::from(id)) {
            p.fin_mur_ms = p.fin_mur_ms.min(t.mur_ms);
        }
        self.droits_rev += 1;
        let tablettes: Vec<u16> = (0..self.principaux.len()).filter(|&k| self.principaux[k].session.is_some_and(|s| s.personne == id)).map(|k| k as u16).collect();
        for de in tablettes {
            self.fermer_session(de);
        }
        self.publier_droits();
        self.attendre_accuse_droits(t);
        self.fin_de_personne(t, id);
    }

    /// Une réduction de droits n'est acquise que lorsque le secours l'a accusée, comme un appui : la
    /// dernière révocation ne se perd pas dans une bascule qui se croit sûre (revue v3, M3).
    fn attendre_accuse_droits(&mut self, t: T) {
        if self.secours_en_phase {
            self.droits_a_accuser = Some((self.rang, t.mono_ms));
        }
    }

    fn fin_de_personne(&mut self, t: T, id: u16) {
        let retenues: Vec<Ref> = self.demandes.refs().filter(|&x| self.demandes.fiches[x.case()].pour == id && self.demandes.fiches[x.case()].etat == etat::RETENUE).collect();
        for x in retenues {
            self.conclure(t, x, issue::REFUSE, raison::DROITS, 3);
        }
        let attente: Vec<Ref> = self.attente_secours.iter().filter(|(_, _, c)| self.demandes.get(c.d).is_some_and(|d| d.pour == id)).map(|(_, _, c)| c.d).collect();
        self.attente_secours.retain(|(_, _, c)| !attente.contains(&c.d));
        for x in attente {
            self.conclure(t, x, issue::NON_ENVOYE, raison::DROITS, 1);
        }
        let en_file: Vec<Ref> = self.demandes.refs().filter(|&x| self.demandes.fiches[x.case()].pour == id && self.demandes.fiches[x.case()].etat == etat::EN_FILE).collect();
        self.annulations.extend(en_file);
    }

    /// La personne et la preuve d'une connexion, maintenant : une session personnelle vivante, la
    /// personne fixe du compte, ou un indice que le canal sait donner ; sinon personne.
    pub fn personne_de(&self, t: T, de: u16, indice: u16) -> (u16, u8) {
        let Some(p) = self.principaux.get(usize::from(de)) else { return (0, droits::preuve::AUCUNE) };
        if let Some(s) = p.session
            && t.mur_ms < s.fin_mur_ms
        {
            return (s.personne, s.preuve.min(p.preuve_max));
        }
        if p.personne != 0 {
            return (p.personne, p.preuve_max);
        }
        if indice != 0 && p.preuve_max >= droits::preuve::INDICE {
            return (indice, droits::preuve::INDICE);
        }
        (0, droits::preuve::AUCUNE)
    }

    /// L'autorité d'une demande qui arrive, établie par le serveur (jamais par le client).
    fn autorite_req(&self, t: T, r: &Req) -> Result<Aut, u8> {
        if r.parent != Ref(0) {
            // Une fille d'un groupe : l'autorité de sa mère.
            let m = self.demandes.get(r.parent).ok_or(raison::DROITS)?;
            return Ok(self.aut_de(m));
        }
        let p = self.principaux.get(usize::from(r.de)).ok_or(raison::DROITS)?;
        if p.est_interne() {
            let Some(a) = r.autorite else {
                return Ok(Aut { de: r.de, pour: 0, preuve: droits::preuve::PERSONNELLE, mandat: 0, systeme: true });
            };
            let m = self.mandats.get(usize::from(a.mandat).wrapping_sub(1)).ok_or(raison::DROITS)?;
            if a.appele {
                if self.principaux.get(usize::from(a.de)).is_none_or(|q| q.est_interne()) {
                    return Err(raison::DROITS);
                }
                return Ok(Aut { de: a.de, pour: a.pour, preuve: a.preuve, mandat: a.mandat, systeme: false });
            }
            let foyer = m.drapeaux & drapeau_m::FOYER != 0;
            return Ok(Aut { de: r.de, pour: if foyer { 0 } else { m.responsable }, preuve: droits::preuve::PERSONNELLE, mandat: a.mandat, systeme: foyer });
        }
        let (pour, preuve_) = self.personne_de(t, r.de, r.indice);
        Ok(Aut { de: r.de, pour, preuve: preuve_, mandat: 0, systeme: false })
    }

    /// L'autorité rangée dans une demande : la même décision à l'approbation et à la remise.
    pub fn aut_de(&self, d: &Demande) -> Aut {
        let interne = self.principaux.get(usize::from(d.de)).is_some_and(|p| p.est_interne());
        let foyer = d.mandat != 0 && self.mandats.get(usize::from(d.mandat) - 1).is_some_and(|m| m.drapeaux & drapeau_m::FOYER != 0);
        Aut { de: d.de, pour: d.pour, preuve: d.preuve, mandat: d.mandat, systeme: interne && (d.mandat == 0 || foyer) }
    }

    pub fn bits_pieces(&self, o: Numero) -> u64 {
        self.pieces_de(o).iter().fold(0u64, |b, &p| b | 1u64.checked_shl(u32::from(p)).unwrap_or(1 << 63))
    }

    fn contexte(&self) -> droits::Contexte {
        droits::Contexte { protegees: self.pieces_protegees.iter().fold(0u64, |b, &p| b | 1u64.checked_shl(u32::from(p)).unwrap_or(0)), droits_frais: self.droits_frais }
    }

    /// La décision pour une autorité et un ordre, maintenant : non, proposer, oui.
    pub fn niveau(&self, t: T, a: &Aut, o: Numero) -> u8 {
        self.niveau_pieces(t, a, o, self.bits_pieces(o))
    }

    fn niveau_pieces(&self, t: T, a: &Aut, o: Numero, ps: u64) -> u8 {
        let f = self.catalogue.fiches[o.place()].forme;
        let c = self.catalogue.formes[usize::from(f)].classe();
        let personne = if a.pour == 0 { None } else { self.personnes.get(usize::from(a.pour)) };
        if a.pour != 0 && personne.is_none() {
            return niveau::NON;
        }
        let aut = droits::Autorite {
            profil: self.principaux.get(usize::from(a.de)).map(|p| &p.p),
            personne,
            preuve: a.preuve,
            mandat: if a.mandat == 0 { None } else { self.mandats.get(usize::from(a.mandat) - 1) },
            systeme: a.systeme,
        };
        if a.mandat != 0 && aut.mandat.is_none() {
            return niveau::NON;
        }
        droits::decider(&self.table, t.mur_ms, &aut, c, ps, self.contexte())
    }

    /// Le niveau d'un ordre de groupe réduit à `cibles` : son cadre (la pièce de l'appareil virtuel) et
    /// chacune de ses cibles, le plus petit.
    fn niveau_cibles(&self, t: T, a: &Aut, o: Numero, cibles: &[u32]) -> u8 {
        let piece = self.apps[self.app_de_ordre(o)].piece;
        let cadre = self.niveau_pieces(t, a, o, 1u64.checked_shl(u32::from(piece)).unwrap_or(0));
        cibles.iter().fold(cadre, |n, &c| n.min(self.niveau(t, a, Numero(c))))
    }

    /// Le sous-ensemble permis d'un ordre de groupe dont une cible est interdite : les cibles permises
    /// et leur niveau ; aucun si son cadre est interdit, si aucune cible n'est permise, ou si toutes le
    /// sont (v3 § 22 : le choix est annoncé avant).
    fn sous_ensemble(&self, t: T, a: &Aut, o: Numero) -> Option<(u8, Vec<u32>)> {
        let toutes = self.cibles_de(o);
        let permises: Vec<u32> = toutes.iter().copied().filter(|&c| self.niveau(t, a, Numero(c)) != niveau::NON).collect();
        let n = self.niveau_cibles(t, a, o, &permises);
        (n != niveau::NON && !permises.is_empty() && permises.len() < toutes.len()).then_some((n, permises))
    }

    /// Qui choisit le sous-ensemble : le demandeur, sur sa connexion et pour la même personne, ou sa
    /// personne à un écran, avec une preuve personnelle. Il ne fait que renoncer à une part de sa demande.
    fn peut_confirmer(&self, t: T, decideur: u16, d: &Demande) -> bool {
        let (id, preuve_) = self.personne_de(t, decideur, 0);
        if decideur == d.de {
            return id == d.pour;
        }
        self.principaux.get(usize::from(decideur)).is_some_and(|p| p.drapeaux & drapeau_p::DECIDE != 0) && preuve_ >= droits::preuve::PERSONNELLE && id != 0 && id == d.pour
    }

    /// Le décideur peut-il trancher cette retenue ? Un écran, une personne avec une preuve
    /// personnelle, le droit d'approuver la classe sur toutes les pièces de l'ordre (revue v3, D3).
    pub fn peut_approuver(&self, t: T, decideur: u16, x: &Demande) -> bool {
        let Some(p) = self.principaux.get(usize::from(decideur)) else { return false };
        let (id, preuve_) = self.personne_de(t, decideur, 0);
        let personne = self.personnes.get(usize::from(id)).filter(|_| id != 0).map(|q| (id, q));
        let o = Numero(x.ordre);
        let c = self.catalogue.formes[usize::from(x.forme)].classe();
        // Un secours aux droits incertains : seul un propriétaire approuve ce qui est sensible ; les
        // droits d'un autre approbateur ont pu changer sans que cette machine le sache (revue v3, M3).
        if !self.droits_frais && classe::bit(c) & classe::SENSIBLES != 0 && personne.is_none_or(|(_, q)| q.role != droits::role::PROPRIETAIRE) {
            return false;
        }
        droits::peut_approuver(&self.table, t.mur_ms, &p.p, personne, preuve_, c, self.bits_pieces(o), x.pour)
    }

    // ---- les appareils, leurs listes -------------------------------------------------------------

    pub fn liste(&self, a: usize) -> &[u32] {
        let app = self.apps[a];
        let d = app.liste as usize;
        &self.listes[d..d + usize::from(app.nb_points) + usize::from(app.nb_ordres)]
    }

    pub fn points_de(&self, a: usize) -> &[u32] {
        &self.liste(a)[..usize::from(self.apps[a].nb_points)]
    }

    pub fn ordres_de(&self, a: usize) -> &[u32] {
        &self.liste(a)[usize::from(self.apps[a].nb_points)..]
    }

    /// La valeur d'un appareil qui porte cette clé.
    pub fn point_de_cle(&self, a: usize, cle: u16) -> Option<Numero> {
        if cle == AUCUNE_CLE || a >= self.apps.len() {
            return None;
        }
        self.points_de(a).iter().map(|&n| Numero(n)).find(|n| self.cle_point.get(n.place()) == Some(&cle))
    }

    /// Une valeur qu'un ordre de son appareil règle : quiconque peut commander cet ordre peut
    /// provoquer un déclencheur sur elle (v3 : le déclencheur est « ouvert »).
    pub fn reglee_par_un_ordre(&self, point: Numero) -> bool {
        let Some(&a) = self.app_de.get(point.place()) else { return false };
        let cle = self.cle_point[point.place()];
        self.ordres_de(a as usize).iter().any(|&o| {
            let f = self.catalogue.fiches[Numero(o).place()].forme;
            self.catalogue.params_de(f).iter().any(|q| q.regle == cle)
        })
    }

    /// La place de l'appareil d'un ordre.
    pub fn app_de_ordre(&self, o: Numero) -> usize {
        Numero(self.catalogue.appareils_o[o.place()]).place()
    }

    /// La valeur qui prouve l'ordre, ou que son premier paramètre règle, sous une forme donnée.
    pub fn point_de(&self, o: Numero, cle: u16) -> Option<Numero> {
        self.point_de_cle(self.app_de_ordre(o), cle)
    }

    /// La composition d'un appareil (vide pour un appareil physique).
    pub fn compo(&self, a: Numero) -> &[Membre] {
        let app = &self.apps[a.place()];
        &self.membres[app.compo as usize..app.compo as usize + usize::from(app.nb_compo)]
    }

    pub fn nom_ordre(&self, o: Numero) -> String {
        self.ordres.nom(o).unwrap_or("(ordre retiré)").to_string()
    }

    fn forme(&mut self, d: &FormeDecl) -> Result<u16, &'static str> {
        let avant = self.catalogue.nb_formes();
        let f = self.catalogue.forme(d)?;
        if self.catalogue.nb_formes() > avant {
            self.sortir(Sortie::Forme { f, decl: d.clone() });
        }
        Ok(f)
    }

    /// Les formes d'un groupe et d'une cible sont compatibles : même effet, mêmes paramètres, des
    /// bornes de groupe comprises dans celles de la cible.
    fn compatibles(&self, groupe: u16, cible: u16) -> bool {
        let (a, b) = (self.catalogue.formes[usize::from(groupe)], self.catalogue.formes[usize::from(cible)]);
        // La même classe : un ordre de groupe ne cache pas une cible plus sensible (v3).
        a.classe() == b.classe()
            && a.effet == b.effet
            && a.nb_params == b.nb_params
            && self.catalogue.params_de(groupe).iter().zip(self.catalogue.params_de(cible)).all(|(p, q)| {
                p.nature == q.nature && p.echelle == q.echelle && p.unite == q.unite && p.dico == q.dico && p.min >= q.min && p.max <= q.max && p.pas <= q.pas.max(p.pas)
            })
    }

    /// Un module déclare un appareil : ses valeurs et ses ordres gardent leur numéro par clé ; les
    /// nouveaux prennent une place ; ceux qui ne sont plus déclarés sont retirés. Les formes nouvelles
    /// partent d'abord (`meta/forme/`). Tout plafond et toute composition invalide sont refusés
    /// avant la moindre mutation de l'appareil.
    pub fn declarer(&mut self, t: T, a: &AppDecl) -> Result<Numero, &'static str> {
        if a.points.len() > 255 || a.ordres.len() > 255 || a.membres.len() > 255 {
            return Err("plus de 255 valeurs, ordres ou membres");
        }
        // Les droits tiennent les pièces sur 64 bits (v3).
        if a.piece >= droits::PIECES_MAX {
            return Err("64 pièces au plus");
        }
        // Une clé, une fois par registre : la valeur `on` et l'ordre `on` sont deux choses.
        let (mut vp, mut vo) = (std::collections::HashSet::new(), std::collections::HashSet::new());
        if !a.points.iter().all(|k| vp.insert(k)) || !a.ordres.iter().all(|(k, _)| vo.insert(k)) {
            return Err("clé en double");
        }
        let mut formes = Vec::with_capacity(a.ordres.len());
        for (_, d) in &a.ordres {
            formes.push(self.forme(d)?);
        }
        let membres = self.resoudre_membres(a, &formes)?;
        let na = match self.appareils.numero(&a.nom) {
            Some(n) => n,
            None => {
                let pa = self.appareils.choisir(1);
                let n = self.appareils.a_donner(pa);
                self.appareils.affecter(n, &a.nom)?;
                n
            }
        };
        let neufs = a.points.iter().filter(|k| self.points.numero(&format!("{}/{k}", a.nom)).is_none()).count();
        let mut pp = self.points.choisir(neufs as u32);
        let mut points = Vec::with_capacity(a.points.len());
        for k in &a.points {
            let nom = format!("{}/{k}", a.nom);
            let n = match self.points.numero(&nom) {
                Some(n) => {
                    self.points.revenu(n);
                    n
                }
                None => {
                    let n = self.points.a_donner(pp);
                    self.points.affecter(n, &nom)?;
                    pp += 1;
                    n
                }
            };
            points.push((n, k.clone()));
        }
        let neufs = a.ordres.iter().filter(|(k, _)| self.ordres.numero(&format!("{}/{k}", a.nom)).is_none()).count();
        let mut po = self.ordres.choisir(neufs as u32);
        let mut ordres = Vec::with_capacity(a.ordres.len());
        for ((k, _), f) in a.ordres.iter().zip(&formes) {
            let nom = format!("{}/{k}", a.nom);
            let n = match self.ordres.numero(&nom) {
                Some(n) => {
                    self.ordres.revenu(n);
                    n
                }
                None => {
                    let n = self.ordres.a_donner(po);
                    self.ordres.affecter(n, &nom)?;
                    po += 1;
                    n
                }
            };
            ordres.push((n, k.clone(), *f));
        }
        let g = if a.membres.is_empty() { genre::PHYSIQUE } else { genre::VIRTUEL };
        let s = Sortie::Appareil { n: na, nom: a.nom.clone(), piece: a.piece, genre: g, points, defs: a.defs.clone(), ordres, membres };
        self.appliquer_appareil(t, &s);
        self.sortir(s);
        Ok(na)
    }

    /// La composition, résolue une fois depuis les noms et vérifiée : une cible est l'ordre d'un
    /// appareil **physique** (pas de groupe de groupes), rattachée à un ordre de groupe de cet
    /// appareil, de forme compatible ; 64 cibles au plus par ordre de groupe.
    fn resoudre_membres(&mut self, a: &AppDecl, formes: &[u16]) -> Result<Vec<Membre>, &'static str> {
        let mut membres = Vec::with_capacity(a.membres.len());
        let mut par_source = [0usize; 256];
        for (rang, (sorte_, nom, role_, source)) in a.membres.iter().enumerate() {
            let n = match *sorte_ {
                sorte::VALEUR => self.points.numero(nom),
                sorte::ORDRE => self.ordres.numero(nom),
                _ => self.appareils.numero(nom),
            }
            .ok_or("membre inconnu")?;
            let mut src = SANS_SOURCE;
            if *role_ == role::CIBLE {
                if *sorte_ != sorte::ORDRE {
                    return Err("une cible est un ordre");
                }
                let k = a.ordres.iter().position(|(c, _)| c == source).ok_or("une cible sans son ordre de groupe")?;
                let fs = formes[k];
                if self.catalogue.formes[usize::from(fs)].drapeaux & drapeau::GROUPE == 0 {
                    return Err("la source d'une cible n'est pas un ordre de groupe");
                }
                let ac = self.app_de_ordre(n);
                if self.apps.get(ac).is_none_or(|x| x.genre != genre::PHYSIQUE) {
                    return Err("une cible est l'ordre d'un appareil physique");
                }
                if !self.compatibles(fs, self.catalogue.fiches[n.place()].forme) {
                    return Err("une cible de forme incompatible avec son ordre de groupe");
                }
                par_source[k] += 1;
                if par_source[k] > CIBLES_MAX {
                    return Err("plus de 64 cibles pour un ordre de groupe");
                }
                src = k as u8;
            }
            membres.push(Membre { numero: n.0, rang: rang as u8, sorte: *sorte_, role: *role_, source: src });
        }
        Ok(membres)
    }

    fn appliquer_appareil(&mut self, t: T, s: &Sortie) {
        let Sortie::Appareil { n, piece, genre: g, points, defs, ordres, membres, .. } = s else { return };
        let p = n.place();
        if p >= self.apps.len() {
            self.apps.resize(p + 1, App::default());
        }
        let ancienne = self.apps[p];
        let (anc_points, anc_ordres): (Vec<u32>, Vec<u32>) = if ancienne.nb_points as usize + ancienne.nb_ordres as usize > 0 {
            (self.points_de(p).to_vec(), self.ordres_de(p).to_vec())
        } else {
            (Vec::new(), Vec::new())
        };
        for &x in &anc_points {
            if !points.iter().any(|(m, _)| m.0 == x) {
                self.points.retirer(Numero(x), t.mur_ms);
            }
        }
        for &x in &anc_ordres {
            if !ordres.iter().any(|(m, _, _)| m.0 == x) {
                self.ordres.retirer(Numero(x), t.mur_ms);
            }
        }
        self.listes_mortes += anc_points.len() + anc_ordres.len();
        self.membres_morts += usize::from(ancienne.nb_compo);
        let debut = self.listes.len() as u32;
        self.listes.extend(points.iter().map(|(x, _)| x.0));
        self.listes.extend(ordres.iter().map(|(x, _, _)| x.0));
        let compo = self.membres.len() as u32;
        self.membres.extend_from_slice(membres);
        self.apps[p] = App { liste: debut, compo, piece: *piece, nb_points: points.len() as u8, nb_ordres: ordres.len() as u8, nb_compo: membres.len() as u8, genre: *g, _r: 0 };
        for (k, (np, cle)) in points.iter().enumerate() {
            let q = np.place();
            if q >= self.vifs.len() {
                self.vifs.resize(q + 1, Vif::default());
                self.app_de.resize(q + 1, u32::MAX);
                self.cle_point.resize(q + 1, AUCUNE_CLE);
                self.defs.resize(q + 1, DefPoint::default());
            }
            if !anc_points.contains(&np.0) {
                self.vifs[q] = Vif::default();
            }
            self.app_de[q] = p as u32;
            self.cle_point[q] = self.catalogue.cle(cle);
            self.defs[q] = defs.get(k).copied().unwrap_or_default();
        }
        for (no, _, f) in ordres {
            if anc_ordres.contains(&no.0) {
                if self.catalogue.fiches[no.place()].forme != *f {
                    self.catalogue.changer_forme(no.place(), *f);
                }
            } else {
                self.catalogue.poser(no.place(), n.0, *f);
            }
        }
    }

    /// Ce qu'une page montre d'un appareil : ses valeurs, ses ordres, puis ce qu'il emprunte, à son
    /// rang. Une référence inerte (un appareil retiré, une place libérée) est dite, jamais remplacée.
    pub fn vue(&self, a: Numero) -> Vec<(u8, Numero, String)> {
        let mut v = Vec::new();
        for &n in self.points_de(a.place()) {
            let n = Numero(n);
            v.push((sorte::VALEUR, n, self.points.nom(n).unwrap_or("").to_string()));
        }
        for &n in self.ordres_de(a.place()) {
            let n = Numero(n);
            v.push((sorte::ORDRE, n, self.ordres.nom(n).unwrap_or("").to_string()));
        }
        for m in self.compo(a) {
            if m.role != role::AFFICHE {
                continue;
            }
            let n = Numero(m.numero);
            let (vivant, nom) = match m.sorte {
                sorte::VALEUR => (self.points.vivant(n), self.points.nom(n)),
                sorte::ORDRE => (self.ordres.vivant(n), self.ordres.nom(n)),
                _ => (self.appareils.vivant(n), self.appareils.nom(n)),
            };
            let nom = match (vivant, nom) {
                (true, Some(x)) => x.to_string(),
                (false, Some(x)) => format!("{x} (retiré)"),
                _ => "(membre retiré)".to_string(),
            };
            v.push((m.sorte, n, nom));
        }
        v
    }

    /// Les appareils virtuels qui montrent, calculent avec ou commandent ce numéro.
    pub fn aussi_dans(&self, sorte_: u8, n: Numero) -> Vec<Numero> {
        (0..self.apps.len())
            .filter(|&p| self.apps[p].genre == genre::VIRTUEL && self.appareils.vivant(self.appareils.numero_de(p)))
            .filter(|&p| self.compo(self.appareils.numero_de(p)).iter().any(|m| m.sorte == sorte_ && m.numero == n.0))
            .map(|p| self.appareils.numero_de(p))
            .collect()
    }

    /// Les cibles vivantes d'un ordre de groupe : celles rattachées à lui (revue I10).
    pub fn cibles_de(&self, o: Numero) -> Vec<u32> {
        let a = self.app_de_ordre(o);
        let Some(k) = self.ordres_de(a).iter().position(|&x| x == o.0) else { return Vec::new() };
        self.compo(self.appareils.numero_de(a))
            .iter()
            .filter(|m| m.role == role::CIBLE && usize::from(m.source) == k && self.ordres.vivant(Numero(m.numero)))
            .map(|m| m.numero)
            .collect()
    }

    /// Toutes les pièces qu'un ordre atteint : celle de son appareil, et, pour un ordre de groupe,
    /// celles des appareils de ses cibles et de ses membres (la règle d'aujourd'hui, `rooms_of`).
    pub fn pieces_de(&self, o: Numero) -> Vec<u16> {
        let a = self.app_de_ordre(o);
        let mut v = vec![self.apps[a].piece];
        for c in self.cibles_de(o) {
            v.push(self.apps[self.app_de_ordre(Numero(c))].piece);
        }
        for m in self.compo(self.appareils.numero_de(a)) {
            if m.sorte == sorte::APPAREIL && m.role == role::MEMBRE && self.appareils.vivant(Numero(m.numero)) {
                v.push(self.apps[Numero(m.numero).place()].piece);
            }
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    // ---- les principaux et leurs permis ----------------------------------------------------------

    /// L'échéance d'une demande : 5 s, bornée par celle de l'appelant et par la fin de son permis et
    /// de celui de sa personne.
    fn echeance(&self, t: T, a: &Aut, appelant_mur_ms: u64) -> u64 {
        let fin = self.principaux.get(usize::from(a.de)).map_or(u64::MAX, |p| p.fin_mur_ms).min(self.personnes.get(usize::from(a.pour)).filter(|_| a.pour != 0).map_or(u64::MAX, |p| p.fin_mur_ms));
        let mut e = t.mono_ms + ECHEANCE_MS;
        if appelant_mur_ms != 0 {
            e = e.min(t.mono_de(appelant_mur_ms));
        }
        e.min(t.mono_de(fin))
    }

    /// Un permis révoqué maintenant : ses retenues sont refusées, ce qui n'est pas parti est retiré
    /// des voies, ses abonnements sont coupés par le courtier (la sortie le dit).
    pub fn revoquer(&mut self, t: T, de: u16) {
        if let Some(p) = self.principaux.get_mut(usize::from(de)) {
            p.fin_mur_ms = p.fin_mur_ms.min(t.mur_ms);
        }
        self.sortir(Sortie::Revocation { de, fin_mur_ms: t.mur_ms });
        self.attendre_accuse_droits(t);
        self.fin_de_permis(t, de);
    }

    fn fin_de_permis(&mut self, t: T, de: u16) {
        let retenues: Vec<Ref> = self.demandes.refs().filter(|&x| self.demandes.fiches[x.case()].de == de && self.demandes.fiches[x.case()].etat == etat::RETENUE).collect();
        for x in retenues {
            self.conclure(t, x, issue::REFUSE, raison::DROITS, 3);
        }
        let attente: Vec<Ref> = self.attente_secours.iter().filter(|(_, _, c)| self.demandes.get(c.d).is_some_and(|d| d.de == de)).map(|(_, _, c)| c.d).collect();
        self.attente_secours.retain(|(_, _, c)| !attente.contains(&c.d));
        for x in attente {
            self.conclure(t, x, issue::NON_ENVOYE, raison::DROITS, 1);
        }
        let en_file: Vec<Ref> = self.demandes.refs().filter(|&x| self.demandes.fiches[x.case()].de == de && self.demandes.fiches[x.case()].etat == etat::EN_FILE).collect();
        self.annulations.extend(en_file);
    }

    // ---- les demandes ------------------------------------------------------------------------

    /// Une demande arrive. Refusée tout de suite (inconnue, retirée, échue, hors des droits, hors de sa
    /// forme, une clé en conflit ou périmée, une tentative ambiguë, un plafond) ; sinon elle prend une
    /// case : retenue pour un humain, ou lancée.
    pub fn demander(&mut self, t: T, r: Req) -> Result<Ref, (u8, u8)> {
        // Une fille de groupe naît du répartiteur, jamais d'une demande reçue : qui se dirait fille
        // d'une demande vivante hériterait de son autorité (v3).
        if r.parent != Ref(0) {
            return self.refuser(t, &Req { parent: Ref(0), ..r }, Numero(0), issue::REFUSE, raison::DROITS);
        }
        self.admettre(t, r, false)
    }

    fn admettre(&mut self, t: T, r: Req, approuvee: bool) -> Result<Ref, (u8, u8)> {
        let Some(p) = self.principaux.get(usize::from(r.de)).cloned() else {
            return self.refuser(t, &r, Numero(0), issue::REFUSE, raison::DROITS);
        };
        let interne = p.est_interne() || r.parent != Ref(0);
        if !interne && r.seq == 0 {
            return self.refuser(t, &r, Numero(0), issue::REFUSE, raison::NOMBRE);
        }
        let o = match &r.ordre {
            Cible::Nom(n) => self.ordres.numero(n),
            Cible::Numero(n) => Some(*n),
        };
        let Some(o) = o.filter(|&o| self.ordres.nom(o).is_some()) else {
            return self.refuser(t, &r, Numero(0), issue::REFUSE, raison::INCONNU);
        };
        if !self.ordres.vivant(o) {
            return self.refuser(t, &r, o, issue::REFUSE, raison::RETIRE);
        }
        if r.echeance_mur_ms != 0 && r.echeance_mur_ms <= t.mur_ms {
            return self.refuser(t, &r, o, issue::REFUSE, raison::ECHUE);
        }
        let f = self.catalogue.fiches[o.place()].forme;
        let forme = self.catalogue.formes[usize::from(f)];
        // Désarmer ne passe jamais par le chemin générique : le code va au chemin privé de l'alarme ;
        // ni retenu, ni approuvable (revue v3, D9).
        if forme.classe() == classe::DESARMER {
            return self.refuser(t, &r, o, issue::REFUSE, raison::DEDIE);
        }
        // Les droits : le principal, la personne et sa preuve, le mandat (v3).
        let aut = match self.autorite_req(t, &r) {
            Ok(a) => a,
            Err(x) => return self.refuser(t, &r, o, issue::REFUSE, x),
        };
        let mut n = self.niveau(t, &aut, o);
        // Un groupe dont une cible est interdite : ni refusé en bloc, ni exécuté en partie d'office ; le
        // sous-ensemble permis est annoncé au demandeur, qui le confirme (le choix du propriétaire, v3).
        // Seulement à une personne qui demande : un automatisme qui a perdu une cible est refusé, et mis
        // à revoir, comme pour toute perte de droits (W2).
        let mut sous_ensemble = None;
        if n == niveau::NON && forme.drapeaux & drapeau::GROUPE != 0 && !interne && !approuvee
            && let Some((ns, permises)) = self.sous_ensemble(t, &aut, o)
        {
            n = ns;
            sous_ensemble = Some(permises);
        }
        if n == niveau::NON {
            return self.refuser(t, &r, o, issue::REFUSE, raison::DROITS);
        }
        if let Err(x) = self.catalogue.valider(f, &r.args) {
            return self.refuser(t, &r, o, issue::REFUSE, x);
        }
        let h = empreinte_args(&r.args);
        if !interne {
            match self.recus.examiner(r.de, r.seq, o.0, h) {
                Verdict::Double(x) => {
                    // La même demande, la même issue : elle ne part pas deux fois.
                    let (n, m) = self.demandes.get(x.demande).map_or((0, 0), |d| (d.confirmees + d.executees, d.cibles));
                    self.res.push(Res { de: r.de, seq: r.seq, d: x.demande, issue: x.issue, raison: x.raison, suite: false, n, m });
                    return Ok(x.demande);
                }
                Verdict::Conflit => return self.refuser(t, &r, o, issue::REFUSE, raison::CONFLIT),
                Verdict::Perimee => return self.refuser(t, &r, o, issue::REFUSE, raison::PERIMEE),
                Verdict::Neuve => {}
            }
            // Un appui émis avant une reprise non sûre, dont la clé est inconnue ici : il a pu partir
            // avant ; on ne le refait pas (revue B2), même s'il se dit « première tentative » (revue v3,
            // M7) : c'est l'époque de la session qui le date, pas le client ; une demande qui ne la dit
            // pas (0) est d'une session inconnue, donc d'avant.
            if self.epoque_non_sure != 0
                && r.emise_epoque < self.epoque_non_sure
                && forme.effet == effet::APPUI
                && forme.drapeaux & drapeau::PROPRIO_HUB == 0
            {
                return self.refuser(t, &r, o, issue::INCERTAIN, raison::AMBIGUE);
            }
        }
        // « Proposer » : un humain habilité décide (la garde, généralisée par classe et par canal). Un
        // sous-ensemble : d'abord le demandeur.
        let retenue = !approuvee && (n == niveau::PROPOSER || sous_ensemble.is_some());
        let total = if forme.drapeaux & drapeau::GROUPE != 0 { self.cibles_de(o).len() } else { 0 };
        let cibles = match &sous_ensemble {
            Some(s) => s.clone(),
            None if forme.drapeaux & drapeau::GROUPE != 0 => self.cibles_de(o),
            None => Vec::new(),
        };
        let besoin = 1 + cibles.len();
        if self.demandes.en_cours + besoin > DEMANDES_MAX {
            return self.refuser(t, &r, o, issue::REFUSE, raison::SATURE);
        }
        if retenue && self.demandes.refs().filter(|&x| self.demandes.fiches[x.case()].etat == etat::RETENUE).count() >= RETENUES_MAX {
            return self.refuser(t, &r, o, issue::REFUSE, raison::SATURE);
        }
        // Les arguments : le premier au canon dans la fiche ; le reste au tiroir ; un texte privé au
        // tiroir privé, remplacé par sa longueur partout ailleurs.
        let mut publics = r.args.clone();
        let mut prive = None;
        if forme.drapeaux & drapeau::TEXTE_PRIVE != 0 {
            for (a, q) in publics.iter_mut().zip(self.catalogue.params_de(f)) {
                if q.nature == nature::TEXTE
                    && let Arg::T(x) = a
                {
                    prive = Some(x.clone());
                    *a = Arg::P(x.len() as u32);
                }
            }
        }
        let valeur = match publics.first() {
            Some(Arg::N(v)) => *v,
            _ => 0,
        };
        let au_tiroir = publics.len() > 1 || publics.iter().any(|a| !matches!(a, Arg::N(_)));
        let mut d = Demande {
            valeur,
            seq: if interne { 0 } else { r.seq },
            recue_ms: t.mur_ms,
            parent: r.parent,
            ordre: o.0,
            qui: if p.est_interne() || r.parent != Ref(0) { r.qui } else { p.qui },
            forme: f,
            de: aut.de,
            pour: aut.pour,
            preuve: aut.preuve,
            mandat: aut.mandat,
            origine: p.origine,
            cibles: 1,
            drapeaux: if approuvee { drapeau_d::APPROUVEE } else { 0 }
                | if au_tiroir { drapeau_d::PARAMS } else { 0 }
                | if prive.is_some() { drapeau_d::PRIVE } else { 0 }
                | if sous_ensemble.is_some() { drapeau_d::SOUS_ENSEMBLE } else { 0 },
            ..Demande::default()
        };
        if retenue {
            d.etat = etat::RETENUE;
            d.issue = issue::RETENUE;
            d.raison = if sous_ensemble.is_some() { raison::SOUS_ENSEMBLE } else { raison::GARDE };
            d.echeance_ms = t.mono_ms + RETENUE_MS;
        } else {
            d.etat = etat::EN_FILE;
            d.echeance_ms = self.echeance(t, &aut, r.echeance_mur_ms);
        }
        let x = self.demandes.prendre(d, self.epoque as u16);
        if au_tiroir {
            self.params.insert(x, publics.into_boxed_slice());
        }
        if let Some(texte) = &prive {
            self.prives.insert(x, texte.clone());
        }
        if !interne {
            self.recus.noter(r.de, Recu { seq: r.seq, demande: x, ordre: o.0, args: h, issue: d.issue, raison: d.raison, _r: [0; 2] });
        }
        // La fiche de l'ordre : cette demande est sa dernière commande (l'ordre d'admission décide).
        {
            let fi = &mut self.catalogue.fiches[o.place()];
            fi.valeur = valeur;
            fi.instant_ms = t.mur_ms;
            fi.demande = x;
            fi.qui = d.qui;
            fi.pour = d.pour;
            fi.preuve_pour = d.preuve;
            fi.origine = d.origine;
            fi.issue = if retenue { issue::RETENUE } else { issue::EN_COURS };
            fi.changes = 0;
            fi.preuve = 0;
            fi.actives += 1;
        }
        let carte = retenue.then(|| {
            let pieces: Vec<String> = self.pieces_de(o).iter().map(u16::to_string).collect();
            self.revision += 1;
            let texte = match &sous_ensemble {
                Some(s) => format!("{} · {} cibles permises sur {} : confirmer le sous-ensemble", self.nom_ordre(o), s.len(), total),
                None => format!("{} · pièce {}", self.nom_ordre(o), pieces.join("+")),
            };
            Carte { texte: texte.into_boxed_str(), forme: f, cibles: cibles.clone().into_boxed_slice(), revision: self.revision }
        });
        if let Some(c) = &carte {
            self.cartes.insert(x, c.clone());
        }
        if !interne && retenue {
            let (k, m) = if sous_ensemble.is_some() { (cibles.len() as u8, total as u8) } else { (0, 0) };
            self.res.push(Res { de: r.de, seq: r.seq, d: x, issue: issue::RETENUE, raison: d.raison, suite: false, n: k, m });
        }
        self.tracer(t, x, o, 1, carte, true, None);
        if let Some(texte) = prive {
            self.sortir(Sortie::Prive { d: x, texte });
        }
        if !retenue {
            self.lancer(t, x);
        }
        Ok(x)
    }

    /// Le seul chemin d'envoi, après la garde, pour une demande admise comme pour une demande
    /// approuvée (revue I7) : un groupe se répartit ; un ordre du hub s'exécute ; un appui attend
    /// l'accusé du secours ; le reste part au module.
    fn lancer(&mut self, t: T, x: Ref) {
        let d = *self.demandes.get(x).expect("demande");
        let o = Numero(d.ordre);
        let forme = self.catalogue.formes[usize::from(d.forme)];
        if self.catalogue.fiches[o.place()].demande == x {
            self.catalogue.fiches[o.place()].changes = 0;
        }
        if forme.drapeaux & drapeau::GROUPE != 0 {
            self.repartir(t, x);
            return;
        }
        if forme.drapeaux & drapeau::PROPRIO_HUB != 0 {
            // Le hub l'exécute lui-même, sans `cmd/` (V7 G18).
            self.conclure(t, x, if forme.retour == retour::ETAT { issue::CONFIRME } else { issue::EXECUTE }, 0, 1);
            return;
        }
        let c = Cmd { d: x, ordre: o, appareil: self.app_de_ordre(o) as u32, forme: d.forme, args: self.args_publics(&d, x), echeance_ms: d.echeance_ms, epoque: self.epoque };
        if forme.effet == effet::APPUI && self.secours_en_phase {
            // Un appui ne part qu'une fois que le secours sait qu'il va partir (revue B2).
            self.attente_secours.push((self.rang, t.mono_ms, c));
        } else {
            self.cmd.push(c);
        }
    }

    /// Le secours a appliqué jusqu'au rang `s` : les appuis dont la trace est en deçà partent.
    pub fn acquitter(&mut self, s: u64) {
        self.acquitte = self.acquitte.max(s);
        if self.droits_a_accuser.is_some_and(|(r, _)| r <= self.acquitte) {
            self.droits_a_accuser = None;
        }
        let (prets, reste): (Vec<_>, Vec<_>) = std::mem::take(&mut self.attente_secours).into_iter().partition(|(r, _, _)| *r <= self.acquitte);
        self.attente_secours = reste;
        self.cmd.extend(prets.into_iter().map(|(_, _, c)| c));
    }

    /// Un ordre de groupe : la mère connaît toutes ses cibles avant la première fille ; elle n'a pas
    /// d'échéance à elle (revue I9) ; chaque fille est une demande ordinaire du même principal, dont
    /// les droits sont vérifiés (revue B1) et qui n'est pas retenue de nouveau si la mère est approuvée.
    fn repartir(&mut self, t: T, x: Ref) {
        let d = *self.demandes.get(x).expect("demande");
        let o = Numero(d.ordre);
        let cibles: Vec<u32> = match self.cartes.get(&x) {
            Some(c) if d.drapeaux & (drapeau_d::APPROUVEE | drapeau_d::SOUS_ENSEMBLE) != 0 => c.cibles.to_vec(),
            _ => self.cibles_de(o),
        };
        {
            let m = self.demandes.get_mut(x).expect("demande");
            m.etat = etat::MERE;
            m.echeance_ms = u64::MAX;
            m.cibles = cibles.len() as u8;
            m.conclues = 0;
            m.confirmees = 0;
            m.executees = 0;
        }
        self.tracer(t, x, o, 1, None, false, None);
        if cibles.is_empty() {
            self.conclure(t, x, issue::ECHEC, raison::PARTIEL, 1);
            return;
        }
        let args = self.args_complets(&d, x);
        for c in cibles {
            let fille = Req { de: d.de, seq: 0, ordre: Cible::Numero(Numero(c)), args: args.clone(), echeance_mur_ms: 0, tentative: 1, premiere_mur_ms: t.mur_ms, qui: d.qui, parent: x, indice: 0, emise_epoque: 0, autorite: None };
            if let Err((i, _)) = self.admettre(t, fille, d.drapeaux & drapeau_d::APPROUVEE != 0) {
                self.fille_conclue(t, x, i);
            }
        }
    }

    /// Une fille vient de conclure : la mère compte, par niveau de preuve, et conclut quand toutes ont
    /// conclu : `confirme` si toutes ont été confirmées, `execute` si toutes ont au moins été
    /// exécutées, sinon `echec`, cause `partiel`, n/m (revue I8).
    fn fille_conclue(&mut self, t: T, mere: Ref, i: u8) {
        let Some(m) = self.demandes.get_mut(mere) else { return };
        compter_fille(m, i);
        if self.actif && m.conclues >= m.cibles {
            let (i, r) = issue_de_mere(m);
            self.conclure(t, mere, i, r, 1);
        }
    }

    /// Les arguments publics d'une demande, tels qu'ils voyagent (un texte privé n'y est qu'une longueur).
    pub fn args_publics(&self, d: &Demande, x: Ref) -> Vec<Arg> {
        match self.params.get(&x) {
            Some(a) => a.to_vec(),
            None if self.catalogue.params_de(d.forme).is_empty() => Vec::new(),
            None => vec![Arg::N(d.valeur)],
        }
    }

    /// Les arguments complets, textes privés compris (pour le hub seul : une fille, une revalidation).
    fn args_complets(&self, d: &Demande, x: Ref) -> Vec<Arg> {
        let mut a = self.args_publics(d, x);
        if let Some(p) = self.prives.get(&x) {
            for v in a.iter_mut() {
                if matches!(v, Arg::P(_)) {
                    *v = Arg::T(p.clone());
                }
            }
        }
        a
    }

    /// Le texte privé d'une demande, pour le module de son appareil (le canal privé) : à personne
    /// d'autre.
    pub fn contenu_prive(&self, module_appareils: &[u32], x: Ref) -> Option<&str> {
        let d = self.demandes.get(x)?;
        let a = self.app_de_ordre(Numero(d.ordre)) as u32;
        module_appareils.contains(&a).then(|| self.prives.get(&x).map(|s| &**s)).flatten()
    }

    fn refuser(&mut self, t: T, r: &Req, o: Numero, i: u8, x: u8) -> Result<Ref, (u8, u8)> {
        let valeur = match r.args.first() {
            Some(Arg::N(v)) => *v,
            _ => 0,
        };
        let qui = self.principaux.get(usize::from(r.de)).map_or(0, |p| p.qui);
        let origine_ = self.principaux.get(usize::from(r.de)).map_or(0, |p| p.origine);
        let pour = self.autorite_req(t, r).map_or(0, |a| a.pour);
        self.ligne(Ligne { instant_ms: t.mur_ms, valeur, ordre: o.0, qui, de: r.de, origine: origine_, issue: i, raison: x, action: 1, pour, ..Ligne::default() });
        if r.seq != 0 {
            self.res.push(Res { de: r.de, seq: r.seq, d: Ref(0), issue: i, raison: x, suite: false, n: 0, m: 0 });
        }
        Err((i, x))
    }

    /// Un humain décide d'une retenue, depuis un écran, en nommant l'ordre et la révision de la carte
    /// qu'il a vues. Il doit être habilité pour cette opération et ses pièces (revue v3, D3) ; la
    /// carte est celle qu'il a vue, une seule fois (D4) ; tout est revérifié à cet instant : l'échéance,
    /// les droits de celui qui demandait, la forme et les cibles (revue I1, I2, I3).
    pub fn decider(&mut self, t: T, x: Ref, ordre_vu: Numero, revision_vue: u32, oui: bool, decideur: u16) -> Result<Ref, &'static str> {
        let d = *self.demandes.get(x).ok_or("demande inconnue ou déjà décidée")?;
        if d.etat != etat::RETENUE {
            return Err("pas une retenue");
        }
        if d.ordre != ordre_vu.0 {
            return Err("cette demande porte sur un autre ordre que celui montré");
        }
        if self.cartes.get(&x).is_none_or(|c| c.revision != revision_vue) {
            return Err("la carte montrée n'est plus la bonne");
        }
        // Un sous-ensemble à confirmer : le demandeur choisit, pas un approbateur (v3 § 22).
        let a_confirmer = d.raison == raison::SOUS_ENSEMBLE;
        if a_confirmer {
            if !self.peut_confirmer(t, decideur, &d) {
                return Err("seul le demandeur choisit le sous-ensemble");
            }
        } else if !self.peut_approuver(t, decideur, &d) {
            return Err("ce décideur n'est pas habilité pour cette opération");
        }
        let (pour_dec, _) = self.personne_de(t, decideur, 0);
        let qui_dec = self.principaux.get(usize::from(decideur)).map_or(0, |p| p.qui);
        let acteur = Some((decideur, pour_dec, qui_dec));
        if t.mono_ms >= d.echeance_ms {
            self.conclure_par(t, x, issue::REFUSE, raison::EXPIREE, 3, acteur);
            return Err("retenue expirée");
        }
        let o = Numero(d.ordre);
        if !oui {
            self.conclure_par(t, x, issue::REFUSE, raison::REFUSEE, 3, acteur);
            return Ok(x);
        }
        if !self.ordres.vivant(o) {
            self.conclure_par(t, x, issue::REFUSE, raison::RETIRE, 3, acteur);
            return Ok(x);
        }
        let carte = self.cartes.get(&x).cloned().expect("carte");
        let reduit = d.drapeaux & drapeau_d::SOUS_ENSEMBLE != 0;
        let niv = if reduit { self.niveau_cibles(t, &self.aut_de(&d), o, &carte.cibles) } else { self.niveau(t, &self.aut_de(&d), o) };
        if niv == niveau::NON {
            self.conclure_par(t, x, issue::REFUSE, raison::DROITS, 3, acteur);
            return Ok(x);
        }
        // La structure ou les cibles ont changé depuis la carte : l'approbation ne vaut pas pour ce
        // qu'elle n'a pas montré. Revalidée : refusée, ou une nouvelle carte, d'une révision nouvelle.
        // Un sous-ensemble garde les cibles de sa carte tant que chacune est encore une cible du groupe.
        let f = self.catalogue.fiches[o.place()].forme;
        let toutes = if self.catalogue.formes[usize::from(f)].drapeaux & drapeau::GROUPE != 0 { self.cibles_de(o) } else { Vec::new() };
        let cibles = if reduit && carte.cibles.iter().all(|c| toutes.contains(c)) { carte.cibles.to_vec() } else { toutes };
        if f != d.forme || *carte.cibles != *cibles {
            let args = self.args_complets(&d, x);
            if let Err(r) = self.catalogue.valider(f, &args) {
                self.conclure_par(t, x, issue::REFUSE, r, 3, acteur);
                return Ok(x);
            }
            // Un sous-ensemble dont le groupe a changé : le sous-ensemble permis du moment, à choisir de
            // nouveau ; s'il n'y en a plus (rien, ou tout est permis), le demandeur redemande.
            let (raison_, cibles) = if reduit {
                match self.sous_ensemble(t, &self.aut_de(&d), o) {
                    Some((_, permises)) => (raison::SOUS_ENSEMBLE, permises),
                    None => {
                        self.conclure_par(t, x, issue::REFUSE, raison::DROITS, 3, acteur);
                        return Ok(x);
                    }
                }
            } else {
                (raison::FORME_CHANGEE, cibles)
            };
            {
                let m = self.demandes.get_mut(x).expect("demande");
                m.forme = f;
                m.raison = raison_;
                m.echeance_ms = t.mono_ms + RETENUE_MS;
            }
            self.revision += 1;
            let nouvelle = Carte { texte: carte.texte.clone(), forme: f, cibles: cibles.into_boxed_slice(), revision: self.revision };
            self.cartes.insert(x, nouvelle.clone());
            if d.seq != 0 {
                self.res.push(Res { de: d.de, seq: d.seq, d: x, issue: issue::RETENUE, raison: raison_, suite: false, n: 0, m: 0 });
            }
            self.tracer(t, x, o, 3, Some(nouvelle), false, acteur);
            return Ok(x);
        }
        // Le sous-ensemble confirmé, mais une de ses cibles se propose (une chambre, un canal gardé) : un
        // humain habilité l'approuve ensuite, sur une carte neuve. Confirmer n'est pas approuver.
        if a_confirmer && niv == niveau::PROPOSER {
            self.demandes.get_mut(x).expect("demande").raison = raison::GARDE;
            self.revision += 1;
            let nouvelle = Carte { revision: self.revision, ..carte };
            self.cartes.insert(x, nouvelle.clone());
            if d.seq != 0 {
                self.res.push(Res { de: d.de, seq: d.seq, d: x, issue: issue::RETENUE, raison: raison::GARDE, suite: false, n: 0, m: 0 });
            }
            self.tracer(t, x, o, 3, Some(nouvelle), false, acteur);
            return Ok(x);
        }
        {
            let e = self.echeance(t, &self.aut_de(&d), 0);
            let m = self.demandes.get_mut(x).expect("demande");
            m.etat = etat::EN_FILE;
            m.issue = 0;
            m.raison = 0;
            m.echeance_ms = e;
            if !a_confirmer {
                m.drapeaux |= drapeau_d::APPROUVEE;
            }
        }
        // Consommée : la même révision ne décide plus rien.
        if let Some(c) = self.cartes.get_mut(&x) {
            c.revision = 0;
        }
        if self.catalogue.fiches[o.place()].demande == x {
            self.catalogue.fiches[o.place()].issue = issue::EN_COURS;
        }
        self.tracer(t, x, o, 3, None, false, acteur);
        self.lancer(t, x);
        Ok(x)
    }

    /// Les ordres remis aux modules (`cmd/`), au dernier point avant l'envoi : les droits de chacun
    /// sont revérifiés là (revue v3, D6) ; un permis révoqué entre l'admission et la remise ne part pas.
    pub fn remettre(&mut self, t: T) -> Vec<Cmd> {
        let mut v = Vec::new();
        for c in std::mem::take(&mut self.cmd) {
            let Some(d) = self.demandes.get(c.d).copied() else { continue };
            if self.niveau(t, &self.aut_de(&d), c.ordre) == niveau::NON {
                self.conclure(t, c.d, issue::NON_ENVOYE, raison::DROITS, 1);
                continue;
            }
            v.push(c);
        }
        v
    }

    /// L'issue d'une demande, rendue par son module (`in/<module>/$issue`, avec le numéro `d`).
    pub fn issue_module(&mut self, t: T, x: Ref, i: u8, r: u8) {
        let Some(d) = self.demandes.get(x).copied() else {
            self.notices.push(format!("issue tardive écartée (demande {}, génération {})", x.case(), x.generation()));
            return;
        };
        if self.controle_generation && d.etat != etat::EN_FILE {
            self.notices.push(format!("issue en double écartée (demande {})", x.case()));
            return;
        }
        self.issue_puis_suivi(t, x, i, r);
    }

    fn issue_puis_suivi(&mut self, t: T, x: Ref, i: u8, r: u8) {
        let d = *self.demandes.get(x).expect("demande");
        let o = Numero(d.ordre);
        let suivi_s = self.catalogue.formes[usize::from(d.forme)].suivi_s;
        let herite = self.heritees.iter().any(|h| h.0 == x);
        let suivi = if herite { suivi_s.max(SUIVI_HERITE_S) } else { suivi_s };
        if matches!(i, issue::ACCEPTE | issue::ENVOYE | issue::INCERTAIN) && suivi > 0 {
            {
                let m = self.demandes.get_mut(x).expect("demande");
                m.etat = etat::SUIVIE;
                m.issue = i;
                m.raison = r;
                m.echeance_ms = t.mono_ms + u64::from(suivi) * 1_000;
            }
            if d.seq != 0 {
                self.recus.issue(d.de, d.seq, i, r);
                self.res.push(Res { de: d.de, seq: d.seq, d: x, issue: i, raison: r, suite: false, n: 0, m: 0 });
            }
            if self.catalogue.fiches[o.place()].demande == x {
                self.catalogue.fiches[o.place()].issue = i;
            }
            self.tracer(t, x, o, 1, None, false, None);
        } else {
            self.conclure(t, x, i, r, 1);
        }
    }

    /// La suite d'une demande suivie, une fois, dans sa fenêtre (V7 G1).
    pub fn suite_module(&mut self, t: T, x: Ref, i: u8, r: u8) {
        match self.demandes.get(x) {
            Some(d) if d.etat == etat::SUIVIE => self.conclure(t, x, i, r, 2),
            _ => self.notices.push(format!("suite tardive écartée (demande {})", x.case())),
        }
    }

    /// Une demande se conclut : son issue, son reçu, sa trace, la fiche de l'ordre (si c'est encore
    /// sa dernière commande) ; la case se libère, ses tiroirs aussi.
    fn conclure(&mut self, t: T, x: Ref, i: u8, r: u8, action: u8) {
        self.conclure_par(t, x, i, r, action, None);
    }

    /// Conclure, avec l'acteur de l'événement s'il n'est pas le demandeur (un approbateur) :
    /// (principal, personne, qui).
    fn conclure_par(&mut self, t: T, x: Ref, i: u8, r: u8, action: u8, acteur: Option<(u16, u16, u32)>) {
        let d = {
            let m = self.demandes.get_mut(x).expect("demande");
            m.issue = i;
            m.raison = r;
            m.etat = etat::LIBRE;
            *m
        };
        let o = Numero(d.ordre);
        if d.seq != 0 {
            self.recus.issue(d.de, d.seq, i, r);
            let (n, m) = if d.cibles > 1 || self.catalogue.formes[usize::from(d.forme)].drapeaux & drapeau::GROUPE != 0 { (d.confirmees + d.executees, d.cibles) } else { (0, 0) };
            self.res.push(Res { de: d.de, seq: d.seq, d: x, issue: i, raison: r, suite: action == 2, n, m });
        }
        {
            let fi = &mut self.catalogue.fiches[o.place()];
            fi.actives -= 1;
            if fi.demande == x {
                fi.issue = i;
                fi.instant_ms = t.mur_ms;
                fi.preuve = if i == issue::CONFIRME && fi.changes >= 1 { preuve::CHANGEMENT | if fi.changes == 1 { preuve::PREMIER } else { 0 } } else { 0 };
            }
        }
        self.tracer(t, x, o, action, None, false, acteur);
        self.demandes.fiches[x.case()].etat = etat::SUIVIE; // le temps de libérer proprement
        self.demandes.liberer(x);
        self.heritees.retain(|h| h.0 != x);
        self.params.remove(&x);
        self.cartes.remove(&x);
        self.prives.remove(&x);
        if d.parent != Ref(0) {
            self.fille_conclue(t, d.parent, i);
        }
    }

    /// La trace d'une demande sur `ord/` : la fiche de la demande et celle de l'ordre, après ce pas.
    fn tracer(&mut self, t: T, x: Ref, o: Numero, action: u8, carte: Option<Carte>, avec_params: bool, acteur: Option<(u16, u16, u32)>) {
        let d = self.demandes.fiches[x.case()];
        let i = if matches!(d.etat, etat::LIBRE | etat::SUIVIE | etat::RETENUE) { d.issue } else { issue::EN_COURS };
        let a_jour = !self.controle_generation || self.ordres.nom(o).is_some();
        if a_jour && self.catalogue.fiches[o.place()].demande == x {
            self.catalogue.fiches[o.place()].instant_ms = t.mur_ms;
        }
        let fiche = if a_jour { self.catalogue.fiches[o.place()] } else { Fiche::default() };
        let recu = if d.seq != 0 { self.recus.get(d.de, d.seq) } else { None };
        let params = if avec_params { self.params.get(&x).cloned() } else { None };
        let (de, pour, qui) = acteur.unwrap_or((d.de, d.pour, d.qui));
        let ligne = Ligne {
            instant_ms: t.mur_ms,
            valeur: d.valeur,
            demande: x,
            ordre: o.0,
            qui,
            duree_ms: (t.mur_ms.saturating_sub(d.recue_ms)) as u32,
            de,
            origine: if acteur.is_some() { origine::UI } else { d.origine },
            issue: i,
            raison: d.raison,
            action,
            pour,
            ..Ligne::default()
        };
        let ligne = self.ligne(ligne);
        let echeance_mur_ms = t.mur_de(d.echeance_ms);
        // `ord/` ne porte jamais la personne (revue v3, D8) : la trace publique la masque ; le pair la
        // reçoit par une sortie privée, juste après.
        let publique = Demande { echeance_ms: 0, pour: 0, preuve: 0, ..d };
        let fiche_pub = Fiche { pour: 0, preuve_pour: 0, ..fiche };
        self.sortir(Sortie::Trace { d: x, demande: publique, echeance_mur_ms, ordre: o, fiche: fiche_pub, recu, carte, params, ligne: Ligne { pour: 0, ..ligne } });
        if d.pour != 0 || fiche.pour != 0 || ligne.pour != 0 {
            self.sortir(Sortie::Autorite { d: x, ordre: o, pour: d.pour, preuve: d.preuve, fiche_pour: fiche.pour, fiche_preuve: fiche.preuve_pour, ligne_pour: ligne.pour });
        }
    }

    /// Un propriétaire, à un écran, avec une preuve personnelle, confirme les droits du secours qui a
    /// repris sans savoir s'ils étaient à jour (revue v3, M3).
    pub fn confirmer_droits(&mut self, t: T, de: u16) -> Result<(), &'static str> {
        let (id, preuve_) = self.personne_de(t, de, 0);
        let p = self.principaux.get(usize::from(de)).ok_or("principal inconnu")?;
        let ok = p.drapeaux & drapeau_p::DECIDE != 0 && preuve_ >= droits::preuve::PERSONNELLE && self.personnes.get(usize::from(id)).is_some_and(|q| id != 0 && q.role == droits::role::PROPRIETAIRE);
        if !ok {
            return Err("seul un propriétaire confirme les droits");
        }
        self.droits_frais = true;
        Ok(())
    }

    fn ligne(&mut self, mut l: Ligne) -> Ligne {
        l.numero = self.prochaine_ligne;
        self.prochaine_ligne += 1;
        if self.journal.len() == JOURNAL {
            self.journal.pop_front();
        }
        self.journal.push_back(l);
        l
    }

    fn piece_de_sortie(&self, s: &Sortie) -> u16 {
        let piece_app = |a: usize| self.apps.get(a).map_or(PIECE_GLOBALE, |x| x.piece);
        match s {
            Sortie::Appareil { piece, .. } => *piece,
            Sortie::Valeur { point, .. } => self.app_de.get(point.place()).map_or(PIECE_GLOBALE, |&a| piece_app(a as usize)),
            Sortie::Trace { ordre, .. } => piece_app(self.app_de_ordre(*ordre)),
            Sortie::Statut { appareil, .. } => piece_app(appareil.place()),
            Sortie::Prive { .. } | Sortie::Bail { .. } | Sortie::Droits { .. } | Sortie::Session { .. } | Sortie::Autorite { .. } | Sortie::Execution { .. } => PIECE_PRIVEE,
            Sortie::Forme { .. } | Sortie::Lot { .. } | Sortie::Revocation { .. } => PIECE_GLOBALE,
        }
    }

    /// Publie une sortie critique : le rang suivant, et le rang suivant de sa pièce.
    pub fn sortir(&mut self, s: Sortie) {
        let piece = self.piece_de_sortie(&s);
        self.rang += 1;
        let c = self.rangs_pieces.entry(piece).or_insert(0);
        *c += 1;
        let sp = *c;
        self.sorties.push(Publiee { e: self.epoque, s: self.rang, piece, sp, sortie: s });
    }

    /// Le superviseur dit qu'un module est mort : ce qu'il avait pris et pas rendu a pu partir.
    /// « Incertain », cause « module » (V7 G20), jamais « non envoyé », jamais renvoyé.
    pub fn module_mort(&mut self, t: T, appareils: &[u32]) {
        let refs: Vec<Ref> = self
            .demandes
            .refs()
            .filter(|&x| {
                let d = &self.demandes.fiches[x.case()];
                d.etat == etat::EN_FILE && appareils.contains(&(self.app_de_ordre(Numero(d.ordre)) as u32))
            })
            .collect();
        for x in refs {
            self.heritees.push((x, t.mur_ms));
            self.issue_puis_suivi(t, x, issue::INCERTAIN, raison::MODULE);
        }
    }

    // ---- les valeurs -------------------------------------------------------------------------

    /// Une valeur arrive d'un module (`in/`), puis part sur `val/`, avec l'heure où sa lecture a
    /// commencé. Elle tient la preuve des ordres qu'elle concerne (revue I5) ; le hub ne confirme
    /// lui-même que les demandes héritées, et seulement par une lecture commencée après leur reprise
    /// (revue I4).
    pub fn valeur(&mut self, t: T, point: Numero, v: i64, mouvement: bool, lu_ms: u64) {
        if !self.points.vivant(point) {
            return;
        }
        self.noter_valeur(point, v, t.mur_ms, mouvement);
        let s = Sortie::Valeur { point, valeur: v, change_ms: self.vifs[point.place()].change_ms, mouvement, lu_ms };
        self.sortir(s);
        for (h, depuis) in self.heritees.clone() {
            let Some(d) = self.demandes.get(h).copied() else { continue };
            if d.etat != etat::SUIVIE || lu_ms < depuis {
                continue;
            }
            let o = Numero(d.ordre);
            let cle = self.catalogue.formes[usize::from(d.forme)].confirme;
            if self.point_de(o, cle) == Some(point) && self.catalogue.confirme_par(d.forme, d.valeur, v, mouvement) {
                self.conclure(t, h, issue::CONFIRME, 0, 2);
            }
        }
    }

    /// La valeur vive, et la preuve des ordres de son appareil qui la citent : un changement compte
    /// pendant qu'une commande envoyée vit ; après la preuve, il dit « modifiée depuis ». Rejoué tel
    /// quel par chaque copie.
    fn noter_valeur(&mut self, point: Numero, v: i64, mur_ms: u64, mouvement: bool) {
        let x = &mut self.vifs[point.place()];
        let change = x.valeur != v;
        if change {
            x.change_ms = mur_ms;
        }
        x.valeur = v;
        x.mouvement = mouvement;
        if change {
            let cle = self.cle_point[point.place()];
            let a = self.app_de[point.place()] as usize;
            let ordres: Vec<u32> = self.ordres_de(a).to_vec();
            for o in ordres {
                let fi = self.catalogue.fiches[Numero(o).place()];
                if self.catalogue.formes[usize::from(fi.forme)].confirme != cle || fi.issue == 0 {
                    continue;
                }
                let vivante = self.demandes.get(fi.demande).is_some_and(|d| d.etat != etat::RETENUE);
                let f = &mut self.catalogue.fiches[Numero(o).place()];
                if vivante {
                    f.changes = f.changes.saturating_add(1);
                } else if f.preuve & preuve::CHANGEMENT != 0 || f.issue == issue::CONFIRME {
                    f.preuve |= preuve::MODIFIEE;
                }
            }
        }
    }

    // ---- les échéances -----------------------------------------------------------------------

    /// La prochaine échéance du hub, en heure murale : le réveil au plus proche (mémoire v2 § 6).
    pub fn prochaine(&self, t: T) -> Option<u64> {
        let demandes = self.demandes.refs().map(|x| self.demandes.fiches[x.case()].echeance_ms).filter(|&e| e != u64::MAX).map(|m| t.mur_de(m));
        let secours = self.attente_secours.iter().map(|(_, depuis, _)| t.mur_de(depuis + ATTENTE_SECOURS_MS)).chain(self.droits_a_accuser.map(|(_, depuis)| t.mur_de(depuis + ATTENTE_SECOURS_MS)));
        // La fin d'un permis ou d'une personne dont une demande vit encore est une échéance : le hub
        // s'y réveille de lui-même, sans battement (revue v3, D7).
        let fins = self.demandes.refs().flat_map(|x| {
            let d = &self.demandes.fiches[x.case()];
            let fp = self.principaux.get(usize::from(d.de)).map_or(u64::MAX, |p| p.fin_mur_ms);
            let fq = if d.pour == 0 { u64::MAX } else { self.personnes.get(usize::from(d.pour)).map_or(u64::MAX, |p| p.fin_mur_ms) };
            [fp, fq]
        });
        let fins = fins.filter(|&f| f != u64::MAX && f > t.mur_ms);
        demandes.chain(secours).chain(fins).min()
    }

    pub fn tic(&mut self, t: T) {
        // Le secours ne répond plus : l'actif l'inscrit chez le témoin, puis envoie sans accusé.
        let droits_sans_accuse = self.droits_a_accuser.is_some_and(|(_, depuis)| t.mono_ms >= depuis + ATTENTE_SECOURS_MS);
        if droits_sans_accuse || self.attente_secours.iter().any(|(_, depuis, _)| t.mono_ms >= depuis + ATTENTE_SECOURS_MS) {
            self.droits_a_accuser = None;
            self.decroche = true;
            self.secours_en_phase = false;
            self.notices.push("le secours a décroché : inscrit chez le témoin, les appuis partent sans accusé".into());
            let prets = std::mem::take(&mut self.attente_secours);
            self.cmd.extend(prets.into_iter().map(|(_, _, c)| c));
        }
        let echues: Vec<Ref> = self.demandes.refs().filter(|&x| self.demandes.fiches[x.case()].echeance_ms <= t.mono_ms).collect();
        for x in echues {
            let Some(d) = self.demandes.get(x).copied() else { continue };
            match d.etat {
                etat::EN_FILE => self.issue_puis_suivi(t, x, issue::INCERTAIN, raison::SANS_REPONSE),
                etat::SUIVIE => {
                    // Le hub conclut lui-même : `envoye` reste `envoye`, `incertain` reste `incertain`.
                    self.notices.push(format!("{} : aucune suite dans la fenêtre de suivi", self.nom_ordre(Numero(d.ordre))));
                    self.conclure(t, x, d.issue, d.raison, 2);
                }
                etat::RETENUE => self.conclure(t, x, issue::REFUSE, raison::EXPIREE, 3),
                _ => {}
            }
        }
        // Un permis arrivé à sa fin : ses retenues tombent, ce qui n'est pas parti est retiré.
        let finis: Vec<u16> = (0..self.principaux.len())
            .filter(|&k| self.principaux[k].fin_mur_ms <= t.mur_ms && self.demandes.refs().any(|x| self.demandes.fiches[x.case()].de == k as u16))
            .map(|k| k as u16)
            .collect();
        for de in finis {
            self.fin_de_permis(t, de);
        }
        let personnes: Vec<u16> = (1..self.personnes.len())
            .filter(|&k| self.personnes[k].fin_mur_ms <= t.mur_ms && self.demandes.refs().any(|x| self.demandes.fiches[x.case()].pour == k as u16))
            .map(|k| k as u16)
            .collect();
        for p in personnes {
            self.fin_de_personne(t, p);
        }
    }

    // ---- la haute disponibilité --------------------------------------------------------------

    /// La copie devient l'actif. Ce qui était remis à un module de l'ancien actif a pu partir :
    /// « incertain », cause « bascule », jamais renvoyé ; le hub le suit lui-même (il connaît la
    /// forme), par des lectures commencées après la reprise. `sure` : le témoin ne dit pas que le
    /// secours avait décroché, donc aucun appui n'est parti sans que cette mémoire le sache.
    pub fn promouvoir(&mut self, t: T, sure: bool) {
        self.actif = true;
        self.epoque += 1;
        if !sure {
            self.epoque_non_sure = self.epoque;
        }
        // Une reprise non sûre : les droits copiés ont pu manquer une révocation ; rien de sensible
        // sans un humain, jusqu'à ce qu'un propriétaire les confirme (revue v3, M3).
        self.droits_frais = sure;
        self.secours_en_phase = false;
        self.decroche = false;
        let refs: Vec<Ref> = self.demandes.refs().collect();
        for x in refs {
            let Some(d) = self.demandes.get(x).copied() else { continue };
            match d.etat {
                etat::EN_FILE => {
                    self.heritees.push((x, t.mur_ms));
                    self.issue_puis_suivi(t, x, issue::INCERTAIN, raison::BASCULE);
                }
                etat::SUIVIE => {
                    self.heritees.push((x, t.mur_ms));
                    let fin = t.mono_ms + u64::from(SUIVI_HERITE_S) * 1_000;
                    let m = self.demandes.get_mut(x).expect("demande");
                    m.echeance_ms = m.echeance_ms.max(fin);
                }
                _ => {}
            }
        }
        // Une mère dont toutes les filles avaient conclu chez l'ancien actif conclut ici.
        let meres: Vec<Ref> = self.demandes.refs().filter(|&x| self.demandes.fiches[x.case()].etat == etat::MERE && self.demandes.fiches[x.case()].conclues >= self.demandes.fiches[x.case()].cibles).collect();
        for m in meres {
            self.fille_conclue_sans_compter(t, m);
        }
    }

    fn fille_conclue_sans_compter(&mut self, t: T, mere: Ref) {
        let (i, r) = issue_de_mere(self.demandes.get(mere).expect("mère"));
        self.conclure(t, mere, i, r, 1);
    }

    /// L'arrêt figé : les demandes remises ont eu leur seconde ; au-delà, « incertain ».
    pub fn arreter(&mut self, t: T) {
        let suivies: Vec<Ref> = self.demandes.refs().filter(|&x| self.demandes.fiches[x.case()].etat == etat::SUIVIE).collect();
        for x in suivies {
            if !self.heritees.iter().any(|h| h.0 == x) {
                self.heritees.push((x, t.mur_ms));
            }
        }
        // Un appui qui attendait encore l'accusé n'est jamais parti : il ne partira pas.
        for (_, _, c) in std::mem::take(&mut self.attente_secours) {
            if self.demandes.get(c.d).is_some() {
                self.conclure(t, c.d, issue::NON_ENVOYE, raison::ARRET, 1);
            }
        }
        let refs: Vec<Ref> = self.demandes.refs().filter(|&x| self.demandes.fiches[x.case()].etat == etat::EN_FILE).collect();
        for x in refs {
            if self.demandes.get(x).is_none() {
                continue;
            }
            self.heritees.push((x, t.mur_ms));
            self.issue_puis_suivi(t, x, issue::INCERTAIN, raison::ARRET);
        }
    }

    /// Le hub redémarre sur son instantané : ce qu'il suit ne se confirme que par des lectures
    /// faites après le redémarrage. `sure` : l'instantané est celui de l'arrêt figé (rien n'est parti
    /// après lui) ; après une panne et un instantané plus ancien, la reprise n'est pas sûre.
    pub fn redemarre(&mut self, t: T, sure: bool) {
        self.actif = true;
        self.epoque += 1;
        if !sure {
            self.epoque_non_sure = self.epoque;
        }
        self.droits_frais = sure;
        for h in &mut self.heritees {
            h.1 = t.mur_ms;
        }
    }

    /// Une copie applique une sortie, dans l'ordre de `s`, à son heure `t`. Un trou est une erreur :
    /// la copie se refait par l'état complet.
    pub fn appliquer(&mut self, p: &Publiee, t: T) -> Result<(), &'static str> {
        if p.e != self.epoque {
            return Err("époque nouvelle : se refaire par l'état complet");
        }
        if p.s != self.rang + 1 {
            return Err("trou dans le flux critique");
        }
        self.rang = p.s;
        self.rangs_pieces.insert(p.piece, p.sp);
        match &p.sortie {
            Sortie::Forme { f, decl } => self.catalogue.appliquer_forme(*f, decl)?,
            Sortie::Appareil { n, nom, points, ordres, .. } => {
                if self.appareils.nom(*n).is_none() {
                    self.appareils.affecter(*n, nom)?;
                }
                for (np, cle) in points {
                    let nom_p = format!("{nom}/{cle}");
                    if self.points.nom(*np) == Some(nom_p.as_str()) {
                        self.points.revenu(*np);
                    } else {
                        self.points.affecter(*np, &nom_p)?;
                    }
                }
                for (no, cle, _) in ordres {
                    let nom_o = format!("{nom}/{cle}");
                    if self.ordres.nom(*no) == Some(nom_o.as_str()) {
                        self.ordres.revenu(*no);
                    } else {
                        self.ordres.affecter(*no, &nom_o)?;
                    }
                }
                self.appliquer_appareil(t, &p.sortie);
            }
            Sortie::Valeur { point, valeur, mouvement, change_ms, .. } => {
                if self.points.vivant(*point) {
                    self.noter_valeur(*point, *valeur, *change_ms, *mouvement);
                    self.vifs[point.place()].change_ms = *change_ms;
                }
            }
            Sortie::Trace { d, demande, echeance_mur_ms, ordre, fiche, recu, carte, params, ligne } => {
                let demande = &Demande { echeance_ms: t.mono_de(*echeance_mur_ms), ..*demande };
                if demande.etat == etat::LIBRE {
                    if self.demandes.get(*d).is_none() {
                        self.demandes.placer(*d, Demande { etat: etat::SUIVIE, ..*demande });
                    }
                    self.demandes.fiches[d.case()].etat = etat::SUIVIE;
                    self.demandes.liberer(*d);
                    self.params.remove(d);
                    self.cartes.remove(d);
                    self.prives.remove(d);
                    self.heritees.retain(|h| h.0 != *d);
                    // La progression d'un groupe, rejouée comme chez l'actif (revue I8).
                    if demande.parent != Ref(0)
                        && let Some(m) = self.demandes.get_mut(demande.parent)
                    {
                        compter_fille(m, demande.issue);
                    }
                } else {
                    // Une mère garde ses compteurs : ceux de la copie, rejoués, valent ceux de l'actif.
                    let compteurs = self.demandes.get(*d).filter(|m| m.etat == etat::MERE && demande.etat == etat::MERE).map(|m| (m.conclues, m.confirmees, m.executees));
                    self.demandes.placer(*d, *demande);
                    if let Some((c, k, e)) = compteurs {
                        let m = self.demandes.get_mut(*d).expect("mère");
                        (m.conclues, m.confirmees, m.executees) = (c, k, e);
                    }
                    if let Some(c) = carte {
                        self.cartes.insert(*d, c.clone());
                    }
                    if let Some(a) = params {
                        self.params.insert(*d, a.clone());
                    }
                }
                if *fiche != Fiche::default() && self.ordres.nom(*ordre).is_some() {
                    self.catalogue.fiches[ordre.place()] = *fiche;
                }
                if let Some(r) = recu {
                    self.recus.noter(demande.de, *r);
                }
                self.ligne(Ligne { numero: 0, ..*ligne });
                self.prochaine_ligne = ligne.numero + 1;
                if let Some(l) = self.journal.back_mut() {
                    l.numero = ligne.numero;
                }
            }
            Sortie::Prive { d, texte } => {
                self.prives.insert(*d, texte.clone());
            }
            Sortie::Statut { appareil, statut: st } => self.appliquer_statut(*appareil, *st, 0),
            Sortie::Lot { appareils, points, ordres } => self.appliquer_lot(appareils, points, ordres),
            Sortie::Bail { .. } | Sortie::Execution { .. } => {}
            Sortie::Revocation { de, fin_mur_ms } => {
                if let Some(p) = self.principaux.get_mut(usize::from(*de)) {
                    p.fin_mur_ms = p.fin_mur_ms.min(*fin_mur_ms);
                }
            }
            Sortie::Droits { rev, personnes, profils, table, mandats } => self.appliquer_droits(*rev, personnes, profils, table, mandats),
            Sortie::Session { de, session } => {
                if let Some(p) = self.principaux.get_mut(usize::from(*de)) {
                    p.session = *session;
                }
            }
            Sortie::Autorite { d, ordre, pour, preuve: pv, fiche_pour, fiche_preuve, ligne_pour } => {
                if let Some(m) = self.demandes.get_mut(*d) {
                    m.pour = *pour;
                    m.preuve = *pv;
                } else if let Some(m) = self.demandes.fiches.get_mut(d.case()) {
                    // La demande vient de conclure : la case libre garde ce qu'en avait l'actif.
                    m.pour = *pour;
                    m.preuve = *pv;
                }
                if self.ordres.nom(*ordre).is_some() {
                    let f = &mut self.catalogue.fiches[ordre.place()];
                    f.pour = *fiche_pour;
                    f.preuve_pour = *fiche_preuve;
                }
                if let Some(l) = self.journal.back_mut() {
                    l.pour = *ligne_pour;
                }
            }
        }
        Ok(())
    }

    /// Une copie applique les droits de l'actif : les personnes et les profils par leur **nom**,
    /// jamais par leur position (revue v3, M1).
    pub fn appliquer_droits(&mut self, rev: u32, personnes: &[(String, Personne)], profils: &[(String, Profil)], table: &Table, mandats: &[Mandat]) {
        // Les numéros sont ceux de l'actif : les personnes dans son ordre, les comptes alignés sur le
        // sien, par leur nom ; la configuration propre de la copie ne fait que fournir les comptes.
        self.noms_personnes = personnes.iter().map(|(n, _)| n.clone()).collect();
        self.personnes = personnes.iter().map(|(_, x)| *x).collect();
        let noms: Vec<String> = profils.iter().map(|(n, _)| n.clone()).collect();
        self.aligner_principaux(&noms);
        for (p, (_, x)) in self.principaux.iter_mut().zip(profils) {
            p.p = *x;
        }
        self.table = *table;
        self.mandats = mandats.to_vec();
        self.droits_rev = rev;
    }

    /// Les comptes dans l'ordre de `noms` (celui de l'instantané, ou de l'actif) : un numéro de compte
    /// ne change jamais. Un nom que la configuration n'a plus devient une tombe sans aucun droit ; un
    /// compte nouveau prend un numéro après les autres (revue v3, M1).
    fn aligner_principaux(&mut self, noms: &[String]) {
        let mut reste = std::mem::take(&mut self.principaux);
        let mut v: Vec<Principal> = noms
            .iter()
            .map(|n| match reste.iter().position(|p| &p.nom == n) {
                Some(k) => reste.remove(k),
                None => Principal { nom: n.clone(), p: Profil { fin_mur_ms: 0, ..Profil::default() }, session: None },
            })
            .collect();
        v.append(&mut reste);
        self.principaux = v;
    }

    // ---- le nettoyage ------------------------------------------------------------------------

    /// Un humain retire un appareil : ses valeurs et ses ordres ne sont plus ni lus ni commandés ; ses
    /// retenues sont terminées ; les demandes déjà remises vont à leur issue.
    pub fn retirer(&mut self, t: T, a: Numero) {
        self.appliquer_statut(a, crate::numeros::statut::RETIRE, t.mur_ms);
        let retenues: Vec<Ref> = self
            .demandes
            .refs()
            .filter(|&x| {
                let d = &self.demandes.fiches[x.case()];
                d.etat == etat::RETENUE && self.app_de_ordre(Numero(d.ordre)) == a.place()
            })
            .collect();
        for x in retenues {
            self.conclure(t, x, issue::REFUSE, raison::RETIRE, 3);
        }
        self.sortir(Sortie::Statut { appareil: a, statut: crate::numeros::statut::RETIRE });
    }

    fn appliquer_statut(&mut self, a: Numero, st: u8, depuis_ms: u64) {
        if st != crate::numeros::statut::RETIRE {
            return;
        }
        self.appareils.retirer(a, depuis_ms);
        for n in self.points_de(a.place()).to_vec() {
            self.points.retirer(Numero(n), depuis_ms);
        }
        for n in self.ordres_de(a.place()).to_vec() {
            self.ordres.retirer(Numero(n), depuis_ms);
        }
    }

    /// Le lot : les places retirées depuis le délai de grâce sont libérées dans les trois registres ;
    /// fiches et valeurs remises à zéro ; les listes et compositions mortes sont tassées.
    pub fn lot(&mut self, t: T, grace_ms: u64) {
        let appareils = self.appareils.liberables(t.mur_ms, grace_ms);
        let points = self.points.liberables(t.mur_ms, grace_ms);
        let ordres = self.ordres.liberables(t.mur_ms, grace_ms);
        self.appliquer_lot(&appareils, &points, &ordres);
        self.sortir(Sortie::Lot { appareils, points, ordres });
    }

    fn appliquer_lot(&mut self, appareils: &[u32], points: &[u32], ordres: &[u32]) {
        self.appareils.liberer(appareils);
        self.points.liberer(points);
        self.ordres.liberer(ordres);
        for &p in points {
            self.vifs[p as usize] = Vif::default();
            self.app_de[p as usize] = u32::MAX;
            self.cle_point[p as usize] = AUCUNE_CLE;
            self.defs[p as usize] = DefPoint::default();
        }
        for &p in ordres {
            self.catalogue.effacer(p as usize);
        }
        for &p in appareils {
            self.apps[p as usize] = App::default();
        }
        // Tasser les listes et les compositions : chaque copie le fait au même rang, à l'identique.
        let mut listes = Vec::with_capacity(self.listes.len() - self.listes_mortes);
        let mut membres = Vec::with_capacity(self.membres.len() - self.membres_morts);
        for a in 0..self.apps.len() {
            let app = self.apps[a];
            let n = usize::from(app.nb_points) + usize::from(app.nb_ordres);
            let d = listes.len() as u32;
            listes.extend_from_slice(&self.listes[app.liste as usize..app.liste as usize + n]);
            let c = membres.len() as u32;
            membres.extend_from_slice(&self.membres[app.compo as usize..app.compo as usize + usize::from(app.nb_compo)]);
            self.apps[a].liste = d;
            self.apps[a].compo = c;
        }
        self.listes = listes;
        self.membres = membres;
        self.listes_mortes = 0;
        self.membres_morts = 0;
    }

    // ---- l'état complet, l'instantané --------------------------------------------------------

    /// Toute la mémoire des ordres (et ce qu'elle cite), au rang courant, pour le pair et pour le
    /// disque du hub ; les échéances en heure murale : l'horloge monotone ne survit pas à la machine.
    pub fn ranger(&self, t: T) -> Vec<u8> {
        let mut b = Vec::new();
        self.appareils.ranger(&mut b);
        self.points.ranger(&mut b);
        self.ordres.ranger(&mut b);
        put32(&mut b, self.apps.len() as u32);
        for a in &self.apps {
            put32(&mut b, a.liste);
            put32(&mut b, a.compo);
            put16(&mut b, a.piece);
            b.extend_from_slice(&[a.nb_points, a.nb_ordres, a.nb_compo, a.genre]);
        }
        put32(&mut b, self.listes.len() as u32);
        for &x in &self.listes {
            put32(&mut b, x);
        }
        put32(&mut b, self.listes_mortes as u32);
        put32(&mut b, self.membres.len() as u32);
        for m in &self.membres {
            put32(&mut b, m.numero);
            b.extend_from_slice(&[m.rang, m.sorte, m.role, m.source]);
        }
        put32(&mut b, self.membres_morts as u32);
        put32(&mut b, self.vifs.len() as u32);
        for (k, v) in self.vifs.iter().enumerate() {
            put64(&mut b, v.valeur as u64);
            put64(&mut b, v.change_ms);
            b.push(u8::from(v.mouvement));
            put32(&mut b, self.app_de[k]);
            put16(&mut b, self.cle_point[k]);
            let d = self.defs[k];
            put16(&mut b, d.dico);
            b.extend_from_slice(&[d.nature, d.unite, d.echelle as u8]);
        }
        self.catalogue.ranger(&mut b);
        self.demandes.ranger(&mut b, &|e| t.mur_de(e));
        self.recus.ranger(&mut b);
        ranger_args(&mut b, &self.params);
        let mut cartes: Vec<(&Ref, &Carte)> = self.cartes.iter().collect();
        cartes.sort_by_key(|x| x.0);
        put32(&mut b, cartes.len() as u32);
        for (k, c) in cartes {
            put64(&mut b, k.0);
            put32(&mut b, c.texte.len() as u32);
            b.extend_from_slice(c.texte.as_bytes());
            put16(&mut b, c.forme);
            put32(&mut b, c.revision);
            put32(&mut b, c.cibles.len() as u32);
            for &x in c.cibles.iter() {
                put32(&mut b, x);
            }
        }
        let mut prives: Vec<(&Ref, &Box<str>)> = self.prives.iter().collect();
        prives.sort_by_key(|x| x.0);
        put32(&mut b, prives.len() as u32);
        for (k, s) in prives {
            put64(&mut b, k.0);
            put32(&mut b, s.len() as u32);
            b.extend_from_slice(s.as_bytes());
        }
        put32(&mut b, self.heritees.len() as u32);
        for h in &self.heritees {
            put64(&mut b, h.0.0);
            put64(&mut b, h.1);
        }
        // Les droits, par nom (revue v3, M1) : la fin et la session de chaque compte, les personnes,
        // la table, les mandats, la révision ; si cette machine les sait sûrs.
        put32(&mut b, self.principaux.len() as u32);
        for p in &self.principaux {
            put16(&mut b, p.nom.len() as u16);
            b.extend_from_slice(p.nom.as_bytes());
            put64(&mut b, p.fin_mur_ms);
            match p.session {
                Some(s) => {
                    b.push(1);
                    put64(&mut b, s.fin_mur_ms);
                    put16(&mut b, s.personne);
                    b.push(s.preuve);
                }
                None => b.push(0),
            }
        }
        put32(&mut b, self.personnes.len() as u32);
        for (nom, x) in self.noms_personnes.iter().zip(&self.personnes) {
            put16(&mut b, nom.len() as u16);
            b.extend_from_slice(nom.as_bytes());
            put64(&mut b, x.pieces);
            put64(&mut b, x.debut_mur_ms);
            put64(&mut b, x.fin_mur_ms);
            b.extend_from_slice(&[x.role, x.drapeaux]);
            put16(&mut b, x.refuse);
        }
        for ligne in &self.table {
            b.extend_from_slice(ligne);
        }
        put32(&mut b, self.mandats.len() as u32);
        for m in &self.mandats {
            put64(&mut b, m.pieces);
            put16(&mut b, m.classes);
            put16(&mut b, m.responsable);
            b.push(m.drapeaux);
        }
        put32(&mut b, self.droits_rev);
        b.push(u8::from(self.droits_frais));
        put32(&mut b, self.epoque_non_sure);
        put32(&mut b, self.revision);
        put32(&mut b, self.epoque);
        put32(&mut b, self.prochaine_ligne);
        put64(&mut b, self.rang);
        put32(&mut b, self.rangs_pieces.len() as u32);
        for (k, v) in &self.rangs_pieces {
            put16(&mut b, *k);
            put64(&mut b, *v);
        }
        b
    }

    /// Relit un état complet ou un instantané. Les principaux (les comptes, les invitations) viennent
    /// de la configuration ; l'instantané n'en garde que la fin des permis (une révocation).
    pub fn relire(mut b: &[u8], t: T, principaux: Vec<Principal>) -> Option<Hub> {
        let b = &mut b;
        let mut h = Hub { actif: false, principaux, ..Hub::default() };
        h.appareils = Registre::relire(b)?;
        h.points = Registre::relire(b)?;
        h.ordres = Registre::relire(b)?;
        for _ in 0..get32(b)? {
            let (liste, compo, piece) = (get32(b)?, get32(b)?, get16(b)?);
            let q = take(b, 4)?;
            h.apps.push(App { liste, compo, piece, nb_points: q[0], nb_ordres: q[1], nb_compo: q[2], genre: q[3], _r: 0 });
        }
        for _ in 0..get32(b)? {
            h.listes.push(get32(b)?);
        }
        h.listes_mortes = get32(b)? as usize;
        for _ in 0..get32(b)? {
            let numero = get32(b)?;
            let q = take(b, 4)?;
            h.membres.push(Membre { numero, rang: q[0], sorte: q[1], role: q[2], source: q[3] });
        }
        h.membres_morts = get32(b)? as usize;
        for _ in 0..get32(b)? {
            let (valeur, change_ms) = (get64(b)? as i64, get64(b)?);
            let mouvement = take(b, 1)?[0] == 1;
            h.vifs.push(Vif { valeur, change_ms, mouvement });
            h.app_de.push(get32(b)?);
            h.cle_point.push(get16(b)?);
            let dico = get16(b)?;
            let q = take(b, 3)?;
            h.defs.push(DefPoint { dico, nature: q[0], unite: q[1], echelle: q[2] as i8, _r: [0; 3] });
        }
        h.catalogue = Catalogue::relire(b)?;
        h.demandes = Demandes::relire(b, &|e| t.mono_de(e))?;
        h.recus = Recus::relire(b)?;
        h.params = relire_args(b)?;
        for _ in 0..get32(b)? {
            let k = Ref(get64(b)?);
            let l = get32(b)? as usize;
            let texte: Box<str> = std::str::from_utf8(take(b, l)?).ok()?.into();
            let forme = get16(b)?;
            let revision = get32(b)?;
            let mut cibles = Vec::new();
            for _ in 0..get32(b)? {
                cibles.push(get32(b)?);
            }
            h.cartes.insert(k, Carte { texte, forme, cibles: cibles.into_boxed_slice(), revision });
        }
        for _ in 0..get32(b)? {
            let k = Ref(get64(b)?);
            let l = get32(b)? as usize;
            h.prives.insert(k, std::str::from_utf8(take(b, l)?).ok()?.into());
        }
        for _ in 0..get32(b)? {
            h.heritees.push((Ref(get64(b)?), get64(b)?));
        }
        // Les comptes, par leur nom (revue v3, M1) : chacun reprend le numéro que l'instantané lui
        // donnait, même si la configuration a été réordonnée ; demandes, reçus et journal le citent.
        let mut noms = Vec::new();
        let mut etats = Vec::new();
        for _ in 0..get32(b)? {
            let l = get16(b)? as usize;
            noms.push(std::str::from_utf8(take(b, l)?).ok()?.to_string());
            let fin = get64(b)?;
            let session = match take(b, 1)?[0] {
                1 => {
                    let f = get64(b)?;
                    let personne = get16(b)?;
                    Some(Session { fin_mur_ms: f, personne, preuve: take(b, 1)?[0], _r: [0; 5] })
                }
                _ => None,
            };
            etats.push((fin, session));
        }
        h.aligner_principaux(&noms);
        for (p, (fin, session)) in h.principaux.iter_mut().zip(etats) {
            p.fin_mur_ms = p.fin_mur_ms.min(fin);
            p.session = session;
        }
        h.personnes.clear();
        h.noms_personnes.clear();
        for _ in 0..get32(b)? {
            let l = get16(b)? as usize;
            h.noms_personnes.push(std::str::from_utf8(take(b, l)?).ok()?.to_string());
            let (pieces, debut_mur_ms, fin_mur_ms) = (get64(b)?, get64(b)?, get64(b)?);
            let q = take(b, 2)?;
            let refuse = get16(b)?;
            h.personnes.push(Personne { pieces, debut_mur_ms, fin_mur_ms, role: q[0], drapeaux: q[1], refuse, _r: 0 });
        }
        for ligne in h.table.iter_mut() {
            let n = ligne.len();
            ligne.copy_from_slice(take(b, n)?);
        }
        for _ in 0..get32(b)? {
            let (pieces, classes, responsable) = (get64(b)?, get16(b)?, get16(b)?);
            h.mandats.push(Mandat { pieces, classes, responsable, drapeaux: take(b, 1)?[0], _r: [0; 3] });
        }
        h.droits_rev = get32(b)?;
        h.droits_frais = take(b, 1)?[0] == 1;
        h.epoque_non_sure = get32(b)?;
        h.revision = get32(b)?;
        h.epoque = get32(b)?;
        h.prochaine_ligne = get32(b)?;
        h.rang = get64(b)?;
        for _ in 0..get32(b)? {
            let k = get16(b)?;
            h.rangs_pieces.insert(k, get64(b)?);
        }
        Some(h)
    }

    /// L'état d'un **client** (un écran, l'appli, un site), filtré par ses droits : ce qu'il charge
    /// par MQTT (`etat/demande` → `etat/reponse/<client>`, en morceaux de 4 Kio), puis il suit le flux
    /// de ses pièces. Le même contenu que `/api/snapshot` (revue I17) : les appareils qu'il voit,
    /// physiques et virtuels, leur disponibilité, leurs valeurs (numéro, clé, nature, unité, échelle,
    /// dictionnaire, valeur, changé), leurs ordres (numéro, clé, forme, dernière commande, en cours),
    /// leur composition, les retenues qu'il voit ; une fois chacun : les formes, les dictionnaires, les
    /// libellés. En tête : l'époque, le rang `s`, et le rang de chacune de ses pièces (revue I13).
    /// `lecteur` : la personne qui regarde (0 : personne) ; elle voit lesquelles des dernières
    /// commandes sont les siennes, jamais qui a fait les autres (revue v3, D8).
    pub fn ranger_client(&self, voit: &dyn Fn(u16) -> bool, libelles: &dyn Fn(&str) -> String, lecteur: u16) -> Vec<u8> {
        let c = self.contenu_client(voit);
        let mut b = Vec::with_capacity(16 + 96 * c.apps.len());
        put32(&mut b, self.epoque);
        put64(&mut b, self.rang);
        put16(&mut b, c.pieces.len() as u16);
        for (k, v) in &c.pieces {
            put16(&mut b, *k);
            put64(&mut b, *v);
        }
        put16(&mut b, c.formes.len() as u16);
        for &f in &c.formes {
            let d = self.catalogue.decl(f);
            put16(&mut b, f);
            b.extend_from_slice(&[d.effet, d.retour, d.drapeaux]);
            put16(&mut b, self.catalogue.formes[usize::from(f)].confirme);
            put16(&mut b, d.suivi_s);
            put16(&mut b, d.tolerance);
            b.push(d.params.len() as u8);
            for q in self.catalogue.params_de(f) {
                put32(&mut b, q.min as u32);
                put32(&mut b, q.max as u32);
                put16(&mut b, q.pas);
                put16(&mut b, q.dico);
                put16(&mut b, q.regle);
                b.extend_from_slice(&[q.nature, q.echelle as u8, q.unite]);
            }
            b.push(d.attendus.len() as u8);
            for &x in &d.attendus {
                put32(&mut b, x as u32);
            }
            let s = d.schema.unwrap_or_default();
            put16(&mut b, s.len() as u16);
            b.extend_from_slice(s.as_bytes());
        }
        put16(&mut b, c.dicos.len() as u16);
        for &d in &c.dicos {
            put16(&mut b, d);
            let codes = self.catalogue.codes(d);
            put16(&mut b, codes.len() as u16);
            for (code, nom) in codes {
                put32(&mut b, *code as u32);
                b.push(nom.len() as u8);
                b.extend_from_slice(nom.as_bytes());
            }
        }
        put16(&mut b, c.cles.len() as u16);
        for &k in &c.cles {
            put16(&mut b, k);
            let nom = self.catalogue.nom_cle(k);
            b.push(nom.len() as u8);
            b.extend_from_slice(nom.as_bytes());
            let l = libelles(nom);
            b.push(l.len() as u8);
            b.extend_from_slice(l.as_bytes());
        }
        put32(&mut b, c.apps.len() as u32);
        for &p in &c.apps {
            let na = self.appareils.numero_de(p);
            let nom = self.appareils.nom(na).unwrap_or("");
            let a = self.apps[p];
            put32(&mut b, na.0);
            b.extend_from_slice(&[a.genre, a.nb_points, a.nb_ordres, a.nb_compo, u8::from(self.appareils.vivant(na))]);
            put16(&mut b, a.piece);
            b.push(nom.len() as u8);
            b.extend_from_slice(nom.as_bytes());
            for &pn in self.points_de(p) {
                let q = Numero(pn).place();
                put32(&mut b, pn);
                put16(&mut b, self.cle_point[q]);
                let d = self.defs[q];
                put16(&mut b, d.dico);
                b.extend_from_slice(&[d.nature, d.unite, d.echelle as u8]);
                let v = self.vifs[q];
                put64(&mut b, v.valeur as u64);
                put64(&mut b, v.change_ms);
            }
            for &on in self.ordres_de(p) {
                let f = self.catalogue.fiches[Numero(on).place()];
                let cle = self.ordres.nom(Numero(on)).unwrap_or("").rsplit('/').next().unwrap_or("");
                put32(&mut b, on);
                b.push(cle.len() as u8);
                b.extend_from_slice(cle.as_bytes());
                put16(&mut b, f.forme);
                put64(&mut b, f.valeur as u64);
                put64(&mut b, f.instant_ms);
                put16(&mut b, f.actives);
                b.extend_from_slice(&[f.issue, f.origine, f.preuve, u8::from(lecteur != 0 && f.pour == lecteur)]);
            }
            for m in self.compo(na) {
                put32(&mut b, m.numero);
                b.extend_from_slice(&[m.rang, m.sorte, m.role, m.source]);
            }
        }
        put16(&mut b, c.retenues.len() as u16);
        for x in &c.retenues {
            let carte = &self.cartes[x];
            put64(&mut b, x.0);
            put16(&mut b, carte.forme);
            b.push(carte.texte.len() as u8);
            b.extend_from_slice(carte.texte.as_bytes());
        }
        b
    }

    /// Le même contenu en JSON : ce qui sépare le gain du partage (formes, dictionnaires, libellés une
    /// fois) de celui de l'encodage binaire.
    pub fn ranger_client_json(&self, voit: &dyn Fn(u16) -> bool, libelles: &dyn Fn(&str) -> String, lecteur: u16) -> String {
        use std::fmt::Write as _;
        let c = self.contenu_client(voit);
        let mut s = format!("{{\"e\":{},\"s\":{},\"pieces\":{{", self.epoque, self.rang);
        s.push_str(&c.pieces.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect::<Vec<_>>().join(","));
        s.push_str("},\"formes\":[");
        s.push_str(&c.formes.iter().map(|&f| self.catalogue.json_forme(f)).collect::<Vec<_>>().join(","));
        s.push_str("],\"dicos\":{");
        s.push_str(
            &c.dicos
                .iter()
                .map(|&d| format!("\"{d}\":{{{}}}", self.catalogue.codes(d).iter().map(|(k, n)| format!("\"{k}\":\"{n}\"")).collect::<Vec<_>>().join(",")))
                .collect::<Vec<_>>()
                .join(","),
        );
        s.push_str("},\"cles\":{");
        s.push_str(&c.cles.iter().map(|&k| format!("\"{k}\":[\"{}\",\"{}\"]", self.catalogue.nom_cle(k), libelles(self.catalogue.nom_cle(k)))).collect::<Vec<_>>().join(","));
        s.push_str("},\"appareils\":[");
        for (i, &p) in c.apps.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let na = self.appareils.numero_de(p);
            let a = self.apps[p];
            let _ = write!(s, "{{\"n\":{},\"nom\":\"{}\",\"piece\":{},\"genre\":{},\"en_ligne\":{},\"valeurs\":[", na.0, self.appareils.nom(na).unwrap_or(""), a.piece, a.genre, self.appareils.vivant(na));
            s.push_str(
                &self
                    .points_de(p)
                    .iter()
                    .map(|&pn| {
                        let q = Numero(pn).place();
                        let d = self.defs[q];
                        let v = self.vifs[q];
                        format!("[{pn},{},{},{},{},{},{},{}]", self.cle_point[q], d.nature, d.unite, d.echelle, d.dico, v.valeur, v.change_ms)
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            );
            s.push_str("],\"ordres\":[");
            s.push_str(
                &self
                    .ordres_de(p)
                    .iter()
                    .map(|&on| {
                        let f = self.catalogue.fiches[Numero(on).place()];
                        let cle = self.ordres.nom(Numero(on)).unwrap_or("").rsplit('/').next().unwrap_or("");
                        format!("[{on},\"{cle}\",{},{},{},{},{},{},{},{}]", f.forme, f.valeur, f.instant_ms, f.issue, f.origine, f.actives, f.preuve, u8::from(lecteur != 0 && f.pour == lecteur))
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            );
            s.push_str("],\"compo\":[");
            s.push_str(&self.compo(na).iter().map(|m| format!("[{},{},{},{},{}]", m.numero, m.rang, m.sorte, m.role, m.source)).collect::<Vec<_>>().join(","));
            s.push_str("]}");
        }
        s.push_str("],\"retenues\":[");
        s.push_str(&c.retenues.iter().map(|x| format!("[{},{},\"{}\"]", x.0, self.cartes[x].forme, self.cartes[x].texte)).collect::<Vec<_>>().join(","));
        s.push_str("]}");
        s
    }

    fn contenu_client(&self, voit: &dyn Fn(u16) -> bool) -> ContenuClient {
        let apps: Vec<usize> = (0..self.apps.len())
            .filter(|&p| self.appareils.nom(self.appareils.numero_de(p)).is_some() && voit(self.apps[p].piece))
            .collect();
        let mut formes = std::collections::BTreeSet::new();
        let mut dicos = std::collections::BTreeSet::new();
        let mut cles = std::collections::BTreeSet::new();
        for &p in &apps {
            for &on in self.ordres_de(p) {
                let f = self.catalogue.fiches[Numero(on).place()].forme;
                formes.insert(f);
                for q in self.catalogue.params_de(f) {
                    if q.nature == nature::ENUM {
                        dicos.insert(q.dico);
                    }
                    if q.regle != AUCUNE_CLE {
                        cles.insert(q.regle);
                    }
                }
                let c = self.catalogue.formes[usize::from(f)].confirme;
                if c != AUCUNE_CLE {
                    cles.insert(c);
                }
            }
            for &pn in self.points_de(p) {
                let q = Numero(pn).place();
                cles.insert(self.cle_point[q]);
                if self.defs[q].nature == nature::ENUM {
                    dicos.insert(self.defs[q].dico);
                }
            }
        }
        let mut pieces: Vec<(u16, u64)> = self.rangs_pieces.iter().filter(|(k, _)| **k == PIECE_GLOBALE || (**k != PIECE_PRIVEE && voit(**k))).map(|(k, v)| (*k, *v)).collect();
        pieces.sort();
        let retenues: Vec<Ref> = {
            let mut v: Vec<Ref> = self.cartes.keys().copied().filter(|x| self.demandes.get(*x).is_some_and(|d| voit(self.apps[self.app_de_ordre(Numero(d.ordre))].piece))).collect();
            v.sort();
            v
        };
        ContenuClient { apps, formes: formes.into_iter().collect(), dicos: dicos.into_iter().collect(), cles: cles.into_iter().filter(|&k| k != AUCUNE_CLE).collect(), pieces, retenues }
    }

    /// L'empreinte exacte de la mémoire des ordres (échéances monotones exclues : chaque machine a
    /// son horloge).
    pub fn empreinte(&self) -> u64 {
        let mut h = Fnv::default();
        self.appareils.empreinte(&mut h);
        self.points.empreinte(&mut h);
        self.ordres.empreinte(&mut h);
        for a in &self.apps {
            h.u64(u64::from(a.liste) | u64::from(a.compo) << 32);
            h.u64(u64::from(a.nb_points) | u64::from(a.nb_ordres) << 8 | u64::from(a.piece) << 16 | u64::from(a.nb_compo) << 32 | u64::from(a.genre) << 40);
        }
        for &x in &self.listes {
            h.u64(u64::from(x));
        }
        for m in &self.membres {
            h.u64(u64::from(m.numero) | u64::from(m.rang) << 32 | u64::from(m.sorte) << 40 | u64::from(m.role) << 48 | u64::from(m.source) << 56);
        }
        for (k, v) in self.vifs.iter().enumerate() {
            h.u64(v.valeur as u64);
            h.u64(v.change_ms);
            h.u64(u64::from(self.cle_point[k]) | u64::from(self.app_de[k]) << 16);
        }
        self.catalogue.empreinte(&mut h);
        self.demandes.empreinte(&mut h);
        self.recus.empreinte(&mut h);
        let mut b = Vec::new();
        ranger_args(&mut b, &self.params);
        h.octets(&b);
        let mut cartes: Vec<(&Ref, &Carte)> = self.cartes.iter().collect();
        cartes.sort_by_key(|x| x.0);
        for (k, c) in cartes {
            h.u64(k.0);
            h.octets(c.texte.as_bytes());
            h.u64(u64::from(c.forme) | u64::from(c.revision) << 16);
            for &x in c.cibles.iter() {
                h.u64(u64::from(x));
            }
        }
        let mut prives: Vec<(&Ref, &Box<str>)> = self.prives.iter().collect();
        prives.sort_by_key(|x| x.0);
        for (k, s) in prives {
            h.u64(k.0);
            h.octets(s.as_bytes());
        }
        for (k, v) in &self.rangs_pieces {
            h.u64(u64::from(*k) << 48 ^ *v);
        }
        h.u64(self.rang);
        // Les droits : la même politique chez l'actif et chez chaque copie.
        for (nom, x) in self.noms_personnes.iter().zip(&self.personnes) {
            h.octets(nom.as_bytes());
            h.u64(x.pieces ^ x.fin_mur_ms.rotate_left(17) ^ u64::from(x.role) << 56 ^ u64::from(x.refuse) << 40);
        }
        for p in &self.principaux {
            h.u64(p.p.pieces ^ p.p.fin_mur_ms.rotate_left(9) ^ u64::from(p.p.oui) << 48 ^ u64::from(p.p.proposer) << 32);
            h.u64(p.session.map_or(0, |s| s.fin_mur_ms ^ u64::from(s.personne) << 48));
        }
        for m in &self.mandats {
            h.u64(m.pieces ^ u64::from(m.classes) << 48 ^ u64::from(m.responsable) << 32 ^ u64::from(m.drapeaux) << 24);
        }
        h.u64(u64::from(self.droits_rev));
        h.0
    }

    /// Octets des tableaux de la mémoire des ordres, à leur taille réelle (`size_of` × capacité) ;
    /// les tables de hachage et les tiroirs sont mesurés par le banc avec le compteur d'allocations.
    pub fn octets_tableaux_ordres(&self) -> usize {
        self.catalogue.octets_tableaux() + self.demandes.octets() + self.recus.octets() + std::mem::size_of::<Ligne>() * self.journal.capacity()
    }

    /// Octets des appareils, de leurs listes et de leurs compositions, et des colonnes par valeur.
    pub fn octets_appareils(&self) -> usize {
        std::mem::size_of::<App>() * self.apps.capacity()
            + 4 * self.listes.capacity()
            + std::mem::size_of::<Membre>() * self.membres.capacity()
            + 4 * self.app_de.capacity()
            + 2 * self.cle_point.capacity()
            + std::mem::size_of::<DefPoint>() * self.defs.capacity()
    }

    /// La taille d'une sortie, en octets binaires (ce que le pont transporte : HA v2 § 6), en-tête
    /// compris (s, pièce, sp).
    pub fn octets_sortie(p: &Publiee) -> usize {
        4 + 8 + 2 + 8
            + match &p.sortie {
                Sortie::Forme { decl, .. } => 16 + 20 * decl.params.len() + 4 * decl.attendus.len() + decl.confirme.len(),
                Sortie::Appareil { nom, points, ordres, membres, defs, .. } => {
                    nom.len() + 9 + points.iter().map(|(_, c)| 6 + c.len()).sum::<usize>() + ordres.iter().map(|(_, c, _)| 8 + c.len()).sum::<usize>() + 8 * membres.len() + 8 * defs.len()
                }
                Sortie::Valeur { .. } => 29,
                Sortie::Trace { recu, carte, params, .. } => {
                    std::mem::size_of::<Demande>()
                        + 40
                        + 20
                        + 48
                        + recu.map_or(0, |_| 32)
                        + carte.as_ref().map_or(0, |c| c.texte.len() + 6 + 4 * c.cibles.len())
                        + params.as_ref().map_or(0, |a| a.iter().map(|x| if let Arg::T(t) = x { t.len() + 2 } else { 9 }).sum())
                }
                Sortie::Prive { texte, .. } => 12 + texte.len(),
                Sortie::Statut { .. } => 5,
                Sortie::Lot { appareils, points, ordres } => 12 + 4 * (appareils.len() + points.len() + ordres.len()),
                Sortie::Bail { .. } => 44,
                Sortie::Revocation { .. } => 10,
                Sortie::Droits { personnes, profils, mandats, .. } => {
                    8 + personnes.iter().map(|(n, _)| n.len() + 2 + std::mem::size_of::<Personne>()).sum::<usize>()
                        + profils.iter().map(|(n, _)| n.len() + 2 + std::mem::size_of::<Profil>()).sum::<usize>()
                        + std::mem::size_of::<Table>()
                        + std::mem::size_of::<Mandat>() * mandats.len()
                }
                Sortie::Session { .. } => 3 + std::mem::size_of::<Session>(),
                Sortie::Autorite { .. } => 8 + 4 + 2 + 1 + 2 + 1 + 2,
                Sortie::Execution { .. } => 4 + 48,
            }
    }

    /// Le reste : trouve un ordre par son nom.
    pub fn ordre(&self, nom: &str) -> Numero {
        self.ordres.numero(nom).unwrap_or_else(|| panic!("ordre inconnu : {nom}"))
    }
    pub fn point(&self, nom: &str) -> Numero {
        self.points.numero(nom).unwrap_or_else(|| panic!("point inconnu : {nom}"))
    }

    /// Le texte d'une issue, pour l'écran et le journal.
    pub fn dire(&self, l: &Ligne) -> String {
        let o = Numero(l.ordre);
        format!(
            "{} · {}{}",
            self.ordres.nom(o).unwrap_or("(ordre retiré)"),
            issue::NOMS[usize::from(l.issue)],
            if l.raison != 0 { format!(" ({})", raison::NOMS[usize::from(l.raison)]) } else { String::new() }
        )
    }

    pub fn effet_appui(&self, f: u16) -> bool {
        self.catalogue.formes[usize::from(f)].effet == effet::APPUI
    }
}

/// L'issue d'une mère dont toutes les filles ont conclu : la preuve la plus faible de ses filles.
fn issue_de_mere(m: &Demande) -> (u8, u8) {
    if m.confirmees == m.cibles {
        (issue::CONFIRME, 0)
    } else if m.confirmees + m.executees == m.cibles {
        (issue::EXECUTE, 0)
    } else {
        (issue::ECHEC, raison::PARTIEL)
    }
}

/// Une fille conclue : la mère compte, par niveau de preuve (la même règle chez l'actif et la copie).
fn compter_fille(m: &mut Demande, i: u8) {
    m.conclues = m.conclues.saturating_add(1);
    match i {
        issue::CONFIRME => m.confirmees = m.confirmees.saturating_add(1),
        issue::EXECUTE => m.executees = m.executees.saturating_add(1),
        _ => {}
    }
}

struct ContenuClient {
    apps: Vec<usize>,
    formes: Vec<u16>,
    dicos: Vec<u16>,
    cles: Vec<u16>,
    pieces: Vec<(u16, u64)>,
    retenues: Vec<Ref>,
}

fn ranger_args(b: &mut Vec<u8>, params: &HashMap<Ref, Box<[Arg]>>) {
    let mut v: Vec<(&Ref, &Box<[Arg]>)> = params.iter().collect();
    v.sort_by_key(|x| x.0);
    put32(b, v.len() as u32);
    for (k, a) in v {
        put64(b, k.0);
        b.push(a.len() as u8);
        for x in a.iter() {
            match x {
                Arg::N(n) => {
                    b.push(0);
                    put64(b, *n as u64);
                }
                Arg::T(t) => {
                    b.push(1);
                    put32(b, t.len() as u32);
                    b.extend_from_slice(t.as_bytes());
                }
                Arg::P(n) => {
                    b.push(2);
                    put32(b, *n);
                }
            }
        }
    }
}

fn relire_args(b: &mut &[u8]) -> Option<HashMap<Ref, Box<[Arg]>>> {
    let mut m = HashMap::new();
    for _ in 0..get32(b)? {
        let k = Ref(get64(b)?);
        let n = take(b, 1)?[0];
        let mut v = Vec::with_capacity(usize::from(n));
        for _ in 0..n {
            v.push(match take(b, 1)?[0] {
                0 => Arg::N(get64(b)? as i64),
                1 => {
                    let l = get32(b)? as usize;
                    Arg::T(std::str::from_utf8(take(b, l)?).ok()?.into())
                }
                _ => Arg::P(get32(b)?),
            });
        }
        m.insert(k, v.into_boxed_slice());
    }
    Some(m)
}
