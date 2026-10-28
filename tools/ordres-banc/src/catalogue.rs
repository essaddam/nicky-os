//! Le catalogue des ordres : ce que la maison sait demander à chaque appareil.
//!
//! Un **ordre** n'est pas une valeur. Une valeur dit ce que l'appareil est (« la lampe est
//! allumée ») ; un ordre dit ce que la maison peut lui demander (« allume-toi »). Ils ont chacun leur
//! registre, leurs numéros et leurs fiches. Le seul lien est une référence : la forme d'un ordre dit
//! quelle valeur de l'appareil le prouve (son retour) et quelle valeur chaque paramètre règle, **par
//! la clé de la valeur** (`on`, `brightness`), jamais par son rang : une redéclaration qui ajoute,
//! retire ou réordonne des valeurs ne déplace aucune référence (revue I11).
//!
//! - **L'ordre** a son numéro (place + génération, comme un point : le registre du nettoyage) et sa
//!   fiche de 40 o (`Fiche`) : sa forme, son appareil, ce qu'on lui a demandé en dernier, combien de
//!   demandes vivent, et ce qui prouve la dernière.
//! - **La structure de l'ordre** est sa **forme** : 24 o (`Forme`), plus 20 o par paramètre
//!   (`Param`) et 4 o par valeur attendue. Une forme est partagée par tous les ordres qui ont la
//!   même structure. Elle ne change jamais : une structure nouvelle est une forme nouvelle.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};

use crate::numeros::{Fnv, get16, get32, get64, put16, put32, put64, take};

pub mod nature {
    pub const BINAIRE: u8 = 1;
    pub const MESURE: u8 = 2;
    pub const ENUM: u8 = 3;
    pub const TEXTE: u8 = 4;
    /// Un objet court (V7 G12) : JSON plat de 3 Kio au plus, au schéma déclaré (des clés et des types
    /// simples) ; un objet imbriqué est refusé explicitement (revue I14).
    pub const OBJET: u8 = 5;
}

pub mod effet {
    /// Une consigne : la dernière en attente gagne (régler, positionner). La rejouer ne change rien.
    pub const CONSIGNE: u8 = 1;
    /// Un appui : chacun compte, dans l'ordre (volume +, stop, démarrer). Le rejouer le refait.
    pub const APPUI: u8 = 2;
}

/// Ce que l'appareil dit en retour (V7 § 5.6) : l'issue ne dépasse jamais ce que le retour permet.
pub mod retour {
    /// Une valeur de l'appareil revient : celle de `confirme`, parmi les attendues.
    pub const ETAT: u8 = 1;
    /// Le pont dit « émis », rien de plus (un volet RTS) : `execute` au mieux.
    pub const PONT: u8 = 2;
    /// Un accusé tout de suite, la valeur plus tard (un aspirateur, un cloud) : `accepte`, puis la suite.
    pub const ACCUSE: u8 = 3;
    /// Rien : l'issue s'arrête à `envoye`.
    pub const AUCUN: u8 = 4;
}

pub mod drapeau {
    /// Une valeur en mouvement ne confirme qu'à l'arrêt (un volet io).
    pub const A_L_ARRET: u8 = 1;
    /// L'ordre part vers l'appareil même hors ligne (Wake-on-LAN).
    pub const REVEILLABLE: u8 = 2;
    /// Ouvre une porte : un agent demande, un humain décide (la garde).
    pub const OUVRE: u8 = 4;
    /// Arrête une machine au travail : idem.
    pub const CONTROLE: u8 = 8;
    /// Le texte ne voyage ni sur `cmd/`, ni sur `ord/`, ni au journal : une référence seulement ; le
    /// contenu passe par le canal privé, au module de l'appareil et au secours (V7 G7, revue B5).
    pub const TEXTE_PRIVE: u8 = 16;
    /// Le hub l'exécute lui-même : aides, minuteries, compteurs, scènes (V7 G18).
    pub const PROPRIO_HUB: u8 = 32;
    /// La valeur attendue est le premier paramètre, à la tolérance près (régler 128 : 128 ± 2).
    pub const ATTENDU_PARAM: u8 = 64;
    /// Un ordre de groupe : le hub le répartit sur les ordres cibles de l'appareil virtuel, une
    /// demande fille par cible ; l'issue dit combien ont obéi (V7 G17).
    pub const GROUPE: u8 = 128;
}

/// Aucune clé : pas de valeur qui confirme, pas de valeur réglée.
pub const AUCUNE_CLE: u16 = u16::MAX;

/// Un paramètre d'ordre, au canon : 20 o. Les bornes sont celles de l'**ordre** (V7 G24), à part de
/// la validité d'une lecture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Param {
    pub min: i32,
    pub max: i32,
    /// Pas, au canon (0 : aucun) ; longueur maximale en octets pour un texte ou un objet.
    pub pas: u16,
    /// Le dictionnaire des codes d'une énumération (partagé, comme `Def.dico` de la mémoire v2).
    pub dico: u16,
    /// La valeur de l'appareil que ce paramètre règle, par sa clé (`AUCUNE_CLE`).
    pub regle: u16,
    pub nature: u8,
    /// Valeur réelle = canon × 10^echelle.
    pub echelle: i8,
    pub unite: u8,
    pub _r: [u8; 3],
}

/// La forme d'un ordre : sa structure, partagée. 24 o, plus ses paramètres et ses attendus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Forme {
    /// Début de ses paramètres dans `params`.
    pub params: u32,
    /// Début de ses valeurs attendues dans `attendus` (codes, ou constantes).
    pub attendus: u32,
    /// La valeur dont l'observation prouve l'ordre, par sa clé (`AUCUNE_CLE`).
    pub confirme: u16,
    /// La fenêtre de suivi après l'échéance, 180 s au plus (V7 G1).
    pub suivi_s: u16,
    /// La tolérance de la confirmation, au canon.
    pub tolerance: u16,
    /// Le schéma d'un objet court, par son rang dans le tiroir des schémas (0 : aucun).
    pub schema: u16,
    pub nb_params: u8,
    pub nb_attendus: u8,
    pub effet: u8,
    pub retour: u8,
    pub drapeaux: u8,
    /// `[0]` : la classe déclarée (`CLASSE_AUTO` : déduite des drapeaux).
    pub _r: [u8; 3],
}

impl Forme {
    /// La classe de l'ordre, pour ses droits (v3).
    pub fn classe(&self) -> u8 {
        use crate::droits::classe;
        match self._r[0] {
            CLASSE_AUTO if self.drapeaux & drapeau::OUVRE != 0 => classe::ACCES,
            CLASSE_AUTO if self.drapeaux & drapeau::CONTROLE != 0 => classe::MACHINE,
            CLASSE_AUTO if self.drapeaux & drapeau::TEXTE_PRIVE != 0 => classe::MESSAGE,
            CLASSE_AUTO => classe::CONFORT,
            c => c,
        }
    }
}

/// Ce qu'un module déclare pour un paramètre.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ParamDecl {
    pub nature: u8,
    pub min: i32,
    pub max: i32,
    pub pas: u16,
    pub dico: u16,
    pub echelle: i8,
    pub unite: u8,
    /// La clé de la valeur réglée ("" : aucune).
    pub regle: String,
}

/// Ce qu'un module déclare pour la structure d'un ordre. Deux ordres de même déclaration partagent
/// leur forme.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FormeDecl {
    pub effet: u8,
    pub retour: u8,
    /// La clé de la valeur qui prouve l'ordre ("" : aucune).
    pub confirme: String,
    pub drapeaux: u8,
    pub suivi_s: u16,
    pub tolerance: u16,
    pub params: Vec<ParamDecl>,
    pub attendus: Vec<i32>,
    pub schema: Option<String>,
    /// La classe de l'ordre (`droits::classe`) ; `CLASSE_AUTO` : déduite de ses drapeaux (ouvre :
    /// accès ; contrôle : machine ; texte privé : message ; sinon confort).
    pub classe: u8,
}

/// La classe d'une forme qui ne la déclare pas : déduite de ses drapeaux.
pub const CLASSE_AUTO: u8 = u8::MAX;

impl FormeDecl {
    pub fn sans_param(effet: u8, retour: u8) -> Self {
        Self { effet, retour, confirme: String::new(), drapeaux: 0, suivi_s: 0, tolerance: 0, params: Vec::new(), attendus: Vec::new(), schema: None, classe: CLASSE_AUTO }
    }
    /// « Régler » une valeur : un paramètre, et la valeur de la même clé le confirme.
    pub fn regler(p: ParamDecl, cle: &str, tolerance: u16) -> Self {
        Self {
            effet: effet::CONSIGNE,
            retour: retour::ETAT,
            confirme: cle.to_string(),
            drapeaux: drapeau::ATTENDU_PARAM,
            suivi_s: 0,
            tolerance,
            params: vec![ParamDecl { regle: cle.to_string(), ..p }],
            attendus: Vec::new(),
            schema: None,
            classe: CLASSE_AUTO,
        }
    }
}

pub fn binaire() -> ParamDecl {
    ParamDecl { nature: nature::BINAIRE, min: 0, max: 1, pas: 1, dico: 0, echelle: 0, unite: 0, regle: String::new() }
}
pub fn mesure(min: i32, max: i32, pas: u16, echelle: i8, unite: u8) -> ParamDecl {
    ParamDecl { nature: nature::MESURE, min, max, pas, dico: 0, echelle, unite, regle: String::new() }
}
pub fn code(dico: u16, min: i32, max: i32) -> ParamDecl {
    ParamDecl { nature: nature::ENUM, min, max, pas: 1, dico, echelle: 0, unite: 0, regle: String::new() }
}
pub fn texte(octets: u16) -> ParamDecl {
    ParamDecl { nature: nature::TEXTE, min: 0, max: 0, pas: octets, dico: 0, echelle: 0, unite: 0, regle: String::new() }
}
pub fn objet(octets: u16) -> ParamDecl {
    ParamDecl { nature: nature::OBJET, min: 0, max: 0, pas: octets, dico: 0, echelle: 0, unite: 0, regle: String::new() }
}

/// Ce que la fiche dit de la preuve de la dernière commande (revue I5).
pub mod preuve {
    /// La dernière commande a été prouvée par un **changement** de la valeur (pas par la lecture
    /// d'une valeur déjà là).
    pub const CHANGEMENT: u8 = 1;
    /// Ce changement était le premier de la valeur depuis l'envoi.
    pub const PREMIER: u8 = 2;
    /// La valeur a changé depuis la preuve (un geste à la main, une autre application).
    pub const MODIFIEE: u8 = 4;
}

/// Ce qu'on a demandé en dernier à un ordre, et ce qu'il est : 40 o, une fiche par ordre.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Fiche {
    /// Le premier paramètre de la dernière commande, au canon.
    pub valeur: i64,
    /// Le dernier pas de la dernière commande (heure murale, ms).
    pub instant_ms: u64,
    /// La dernière commande : la dernière demande **admise** pour cet ordre. Une issue tardive d'une
    /// demande plus ancienne ne la remplace jamais (revue I6).
    pub demande: crate::demandes::Ref,
    /// L'acteur technique de la dernière commande : une exécution du moteur, un agent (un rang).
    pub qui: u32,
    pub forme: u16,
    /// La personne pour qui elle a été faite (0 : personne connue) ; jamais publiée sur `ord/` (v3).
    pub pour: u16,
    /// Combien de demandes de cet ordre vivent (retenues, en file, suivies) : « en cours », à part
    /// de l'issue de la dernière. Sur 16 bits : le plafond de 4 096 demandes tient (revue v3, M5).
    pub actives: u16,
    /// L'issue de la dernière commande ; `EN_COURS` tant qu'elle n'est pas tombée ; 0 : jamais.
    pub issue: u8,
    pub origine: u8,
    /// Combien de changements de la valeur qui prouve, depuis l'envoi de la dernière commande.
    pub changes: u8,
    /// `preuve::*`.
    pub preuve: u8,
    /// Ce qu'on savait de la personne (`droits::preuve`).
    pub preuve_pour: u8,
    pub _r: u8,
}

/// Un argument d'ordre, tel qu'il arrive.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Arg {
    N(i64),
    T(Box<str>),
    /// Un texte privé, remplacé par sa longueur : le contenu est au tiroir privé (revue B5).
    P(u32),
}

/// Les raisons d'un refus (V7 G16 : un vocabulaire fermé).
pub mod raison {
    pub const INCONNU: u8 = 1;
    pub const BORNES: u8 = 2;
    pub const NOMBRE: u8 = 3;
    pub const RETIRE: u8 = 4;
    pub const GARDE: u8 = 5;
    pub const REMPLACE: u8 = 6;
    pub const VOIE_PLEINE: u8 = 7;
    pub const SANS_REPONSE: u8 = 8;
    pub const MODULE: u8 = 9;
    pub const BASCULE: u8 = 10;
    pub const ARRET: u8 = 11;
    pub const PARTIEL: u8 = 12;
    pub const EXPIREE: u8 = 13;
    pub const REFUSEE: u8 = 14;
    pub const APPAREIL: u8 = 15;
    pub const ABSENT: u8 = 16;
    /// Hors des droits du principal : une pièce qu'il ne voit pas, un permis échu ou révoqué.
    pub const DROITS: u8 = 17;
    /// Arrivée après l'échéance de l'appelant.
    pub const ECHUE: u8 = 18;
    /// Une nouvelle tentative d'un appui dont la première a pu partir avant une reprise non sûre.
    pub const AMBIGUE: u8 = 19;
    /// La même clé d'idempotence, pour un autre ordre ou d'autres arguments.
    pub const CONFLIT: u8 = 20;
    /// Une clé d'idempotence plus ancienne que la fenêtre de son principal.
    pub const PERIMEE: u8 = 21;
    /// Un plafond atteint : demandes en cours, retenues, cibles d'un groupe.
    pub const SATURE: u8 = 22;
    /// La structure de l'ordre a changé depuis la carte : une nouvelle carte est présentée.
    pub const FORME_CHANGEE: u8 = 23;
    /// Ce que la forme déclare et que le banc ne sait pas valider (un objet imbriqué) : refusé.
    pub const NON_PRIS: u8 = 24;
    /// Désarmer : jamais par le chemin générique ; le chemin privé de l'alarme, avec son code (v3).
    pub const DEDIE: u8 = 25;
    /// Un groupe dont une cible est interdite : le sous-ensemble permis est annoncé, et le demandeur le
    /// confirme avant que rien ne parte (le choix du propriétaire, v3 § 22).
    pub const SOUS_ENSEMBLE: u8 = 26;
    pub const NOMS: [&str; 27] = [
        "", "inconnu", "bornes", "nombre", "retire", "garde", "remplace", "voie_pleine", "sans_reponse", "module", "bascule", "arret",
        "partiel", "expiree", "refusee", "appareil", "absent", "droits", "echue", "ambigue", "conflit", "perimee", "sature",
        "forme_changee", "non_pris", "dedie", "sous_ensemble",
    ];
}

/// Le catalogue : les formes partagées, les clés des valeurs, les dictionnaires, et une fiche par
/// ordre, rangée à la place de son numéro.
#[derive(Clone, Default)]
pub struct Catalogue {
    /// La forme 0 n'existe pas : une fiche libre la porte.
    pub formes: Vec<Forme>,
    pub params: Vec<Param>,
    pub attendus: Vec<i32>,
    pub schemas: Vec<Box<str>>,
    /// Combien de fiches portent chaque forme.
    pub refs: Vec<u32>,
    /// Déclaration → forme, pour partager.
    index: hashbrown::HashTable<u16>,
    hacheur: std::hash::RandomState,
    /// Les clés des valeurs (`on`, `brightness`…), une fois : une forme les cite par leur numéro.
    pub cles: Vec<Box<str>>,
    index_cles: HashMap<Box<str>, u16>,
    /// Les dictionnaires des énumérations : (code, nom). Le 0 est vide.
    pub dicos: Vec<Vec<(i32, Box<str>)>>,
    /// Une fiche par place d'ordre.
    pub fiches: Vec<Fiche>,
    /// Par place d'ordre : la place de son appareil (une colonne, comme pour les valeurs).
    pub appareils_o: Vec<u32>,
}

const _: () = assert!(std::mem::size_of::<Param>() == 20);
const _: () = assert!(std::mem::size_of::<Forme>() == 24);
const _: () = assert!(std::mem::size_of::<Fiche>() == 40);

fn cle_h(d: &FormeDecl, h: &std::hash::RandomState) -> u64 {
    let mut s = h.build_hasher();
    d.hash(&mut s);
    s.finish()
}

/// Le plafond des formes : un numéro de forme tient sur 16 bits, la forme 0 n'existe pas.
pub const FORMES_MAX: usize = u16::MAX as usize;
/// Au plus 16 paramètres et 32 valeurs attendues par forme.
pub const PARAMS_MAX: usize = 16;
pub const ATTENDUS_MAX: usize = 32;

impl Catalogue {
    pub fn new() -> Self {
        let mut c = Self::default();
        c.formes.push(Forme::default());
        c.refs.push(0);
        c.schemas.push("".into());
        c.dicos.push(Vec::new());
        c
    }

    pub fn nb_formes(&self) -> usize {
        self.formes.len() - 1
    }

    /// Le numéro d'une clé, donné à sa première apparition (dans l'ordre des sorties : chaque copie
    /// donne les mêmes numéros en appliquant les mêmes sorties).
    pub fn cle(&mut self, nom: &str) -> u16 {
        if nom.is_empty() {
            return AUCUNE_CLE;
        }
        if let Some(&k) = self.index_cles.get(nom) {
            return k;
        }
        let k = self.cles.len() as u16;
        self.cles.push(nom.into());
        self.index_cles.insert(nom.into(), k);
        k
    }

    pub fn cle_existante(&self, nom: &str) -> Option<u16> {
        self.index_cles.get(nom).copied()
    }

    pub fn nom_cle(&self, k: u16) -> &str {
        if k == AUCUNE_CLE { "" } else { &self.cles[usize::from(k)] }
    }

    /// La déclaration qu'une forme représente (ce que publie `meta/forme/<f>`).
    pub fn decl(&self, f: u16) -> FormeDecl {
        let x = self.formes[usize::from(f)];
        let p = x.params as usize;
        let a = x.attendus as usize;
        FormeDecl {
            effet: x.effet,
            retour: x.retour,
            confirme: self.nom_cle(x.confirme).to_string(),
            drapeaux: x.drapeaux,
            suivi_s: x.suivi_s,
            tolerance: x.tolerance,
            params: self.params[p..p + usize::from(x.nb_params)]
                .iter()
                .map(|q| ParamDecl { nature: q.nature, min: q.min, max: q.max, pas: q.pas, dico: q.dico, echelle: q.echelle, unite: q.unite, regle: self.nom_cle(q.regle).to_string() })
                .collect(),
            attendus: self.attendus[a..a + usize::from(x.nb_attendus)].to_vec(),
            schema: (x.schema != 0).then(|| self.schemas[usize::from(x.schema)].to_string()),
            classe: x._r[0],
        }
    }

    /// La forme d'une déclaration : celle qui existe déjà, ou une nouvelle (jamais modifiée ensuite).
    /// Refusée si elle dépasse un plafond ou déclare ce que la validation ne sait pas vérifier.
    pub fn forme(&mut self, d: &FormeDecl) -> Result<u16, &'static str> {
        let h = cle_h(d, &self.hacheur);
        if let Some(&f) = self.index.find(h, |&f| self.decl(f) == *d) {
            return Ok(f);
        }
        if self.formes.len() > FORMES_MAX {
            return Err("plus de place pour une forme");
        }
        if d.params.len() > PARAMS_MAX || d.attendus.len() > ATTENDUS_MAX {
            return Err("trop de paramètres ou de valeurs attendues");
        }
        if let Some(s) = &d.schema
            && lire_schema(s).is_none()
        {
            return Err("schéma d'objet non pris en charge (un objet plat de types simples)");
        }
        if d.params.iter().any(|p| p.nature == nature::ENUM && usize::from(p.dico) >= self.dicos.len()) {
            return Err("dictionnaire inconnu");
        }
        // Un texte privé par forme (revue v3, M8) : le tiroir privé n'en garde qu'un.
        if d.drapeaux & drapeau::TEXTE_PRIVE != 0 && d.params.iter().filter(|p| p.nature == nature::TEXTE).count() > 1 {
            return Err("un seul texte privé par forme");
        }
        if d.classe != CLASSE_AUTO && usize::from(d.classe) >= crate::droits::classe::N {
            return Err("classe inconnue");
        }
        Ok(self.creer(d, h))
    }

    /// Une copie applique `meta/forme/<f>` : la forme prend exactement ce numéro.
    pub fn appliquer_forme(&mut self, f: u16, d: &FormeDecl) -> Result<(), &'static str> {
        if usize::from(f) < self.formes.len() {
            return if self.decl(f) == *d { Ok(()) } else { Err("une forme connue a changé de structure") };
        }
        if usize::from(f) != self.formes.len() {
            return Err("forme hors de rang");
        }
        let h = cle_h(d, &self.hacheur);
        self.creer(d, h);
        Ok(())
    }

    fn creer(&mut self, d: &FormeDecl, h: u64) -> u16 {
        let f = self.formes.len() as u16;
        let schema = match &d.schema {
            Some(s) => {
                self.schemas.push(s.as_str().into());
                (self.schemas.len() - 1) as u16
            }
            None => 0,
        };
        let confirme = self.cle(&d.confirme);
        let regles: Vec<u16> = d.params.iter().map(|p| self.cle(&p.regle)).collect();
        self.formes.push(Forme {
            params: self.params.len() as u32,
            attendus: self.attendus.len() as u32,
            confirme,
            suivi_s: d.suivi_s,
            tolerance: d.tolerance,
            schema,
            nb_params: d.params.len() as u8,
            nb_attendus: d.attendus.len() as u8,
            effet: d.effet,
            retour: d.retour,
            drapeaux: d.drapeaux,
            _r: [d.classe, 0, 0],
        });
        for (p, regle) in d.params.iter().zip(regles) {
            self.params.push(Param { min: p.min, max: p.max, pas: p.pas, dico: p.dico, regle, nature: p.nature, echelle: p.echelle, unite: p.unite, _r: [0; 3] });
        }
        self.attendus.extend_from_slice(&d.attendus);
        self.refs.push(0);
        let decls: Vec<FormeDecl> = (0..self.formes.len()).map(|g| if g == 0 { FormeDecl::sans_param(0, 0) } else { self.decl(g as u16) }).collect();
        let hacheur = &self.hacheur;
        self.index.insert_unique(h, f, |&g| cle_h(&decls[usize::from(g)], hacheur));
        f
    }

    /// La fiche de la place `p` prend l'ordre d'un appareil, avec sa forme ; tout le reste à zéro
    /// (une place réutilisée ne garde rien de l'ordre d'avant).
    pub fn poser(&mut self, p: usize, appareil: u32, f: u16) {
        if p >= self.fiches.len() {
            self.fiches.resize(p + 1, Fiche::default());
            self.appareils_o.resize(p + 1, 0);
        }
        let ancienne = self.fiches[p].forme;
        if ancienne != 0 {
            self.refs[usize::from(ancienne)] -= 1;
        }
        self.fiches[p] = Fiche { forme: f, ..Fiche::default() };
        self.appareils_o[p] = appareil;
        self.refs[usize::from(f)] += 1;
    }

    /// Une redéclaration change la forme d'un ordre connu ; sa dernière commande reste.
    pub fn changer_forme(&mut self, p: usize, f: u16) {
        let ancienne = self.fiches[p].forme;
        if ancienne != 0 {
            self.refs[usize::from(ancienne)] -= 1;
        }
        self.fiches[p].forme = f;
        self.refs[usize::from(f)] += 1;
    }

    /// Le lot libère la place : la fiche est remise à zéro.
    pub fn effacer(&mut self, p: usize) {
        if p < self.fiches.len() {
            let f = self.fiches[p].forme;
            if f != 0 {
                self.refs[usize::from(f)] -= 1;
            }
            self.fiches[p] = Fiche::default();
        }
    }

    pub fn params_de(&self, f: u16) -> &[Param] {
        let x = &self.formes[usize::from(f)];
        &self.params[x.params as usize..x.params as usize + usize::from(x.nb_params)]
    }

    pub fn attendus_de(&self, f: u16) -> &[i32] {
        let x = &self.formes[usize::from(f)];
        &self.attendus[x.attendus as usize..x.attendus as usize + usize::from(x.nb_attendus)]
    }

    pub fn codes(&self, d: u16) -> &[(i32, Box<str>)] {
        self.dicos.get(usize::from(d)).map_or(&[], |v| v.as_slice())
    }

    /// Valider des arguments contre la forme : la même fonction chez le hub, l'écran et l'agent.
    pub fn valider(&self, f: u16, args: &[Arg]) -> Result<(), u8> {
        if f == 0 || usize::from(f) >= self.formes.len() {
            return Err(raison::INCONNU);
        }
        let schema = self.formes[usize::from(f)].schema;
        valider_params(self.params_de(f), args, &|d, v| self.codes(d).iter().any(|x| x.0 == v), (schema != 0).then(|| &*self.schemas[usize::from(schema)]))
    }
}

/// Valider des arguments contre une forme reçue par `meta/forme/<f>` (un écran, un agent), avec les
/// dictionnaires reçus par `meta/dico/<d>`.
pub fn valider_decl(d: &FormeDecl, args: &[Arg], contient: &dyn Fn(u16, i32) -> bool) -> Result<(), u8> {
    let ps: Vec<Param> = d
        .params
        .iter()
        .map(|p| Param { min: p.min, max: p.max, pas: p.pas, dico: p.dico, regle: 0, nature: p.nature, echelle: p.echelle, unite: p.unite, _r: [0; 3] })
        .collect();
    valider_params(&ps, args, contient, d.schema.as_deref())
}

fn valider_params(ps: &[Param], args: &[Arg], contient: &dyn Fn(u16, i32) -> bool, schema: Option<&str>) -> Result<(), u8> {
    if args.len() != ps.len() {
        return Err(raison::NOMBRE);
    }
    for (p, a) in ps.iter().zip(args) {
        match (p.nature, a) {
            (nature::TEXTE, Arg::T(t)) => {
                if t.len() > usize::from(p.pas) {
                    return Err(raison::BORNES);
                }
            }
            (nature::OBJET, Arg::T(t)) => {
                if t.len() > usize::from(p.pas) {
                    return Err(raison::BORNES);
                }
                let Some(sch) = schema.and_then(lire_schema) else { return Err(raison::NON_PRIS) };
                if !objet_conforme(t, &sch) {
                    return Err(raison::BORNES);
                }
            }
            (_, Arg::P(_)) => return Err(raison::BORNES),
            (nature::TEXTE | nature::OBJET, Arg::N(_)) | (_, Arg::T(_)) => return Err(raison::BORNES),
            (_, Arg::N(v)) => {
                let v = *v;
                if v < i64::from(p.min) || v > i64::from(p.max) {
                    return Err(raison::BORNES);
                }
                if p.pas > 1 && (v - i64::from(p.min)) % i64::from(p.pas) != 0 {
                    return Err(raison::BORNES);
                }
                // Une énumération : le code doit être au dictionnaire, pas seulement entre les bornes.
                if p.nature == nature::ENUM && !contient(p.dico, v as i32) {
                    return Err(raison::BORNES);
                }
            }
        }
    }
    Ok(())
}

/// Un type de valeur d'un objet plat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeSimple {
    Texte,
    Entier,
    Booleen,
}

/// Le schéma d'un objet court : `{"cle":"string|integer|boolean",…}`, plat. Tout autre schéma
/// (imbriqué, tableau, nombre réel) n'est pas pris en charge : la forme est refusée.
pub fn lire_schema(s: &str) -> Option<Vec<(String, TypeSimple)>> {
    let paires = lire_objet(s)?;
    paires
        .into_iter()
        .map(|(k, v)| match v {
            Simple::T(t) => match t.as_str() {
                "string" => Some((k, TypeSimple::Texte)),
                "integer" => Some((k, TypeSimple::Entier)),
                "boolean" => Some((k, TypeSimple::Booleen)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// L'objet est-il conforme au schéma : seulement des clés déclarées, chacune une fois, au bon type.
pub fn objet_conforme(t: &str, schema: &[(String, TypeSimple)]) -> bool {
    let Some(paires) = lire_objet(t) else { return false };
    let mut vues = std::collections::HashSet::new();
    paires.iter().all(|(k, v)| {
        vues.insert(k.clone())
            && schema.iter().any(|(c, ty)| {
                c == k
                    && matches!((ty, v), (TypeSimple::Texte, Simple::T(_)) | (TypeSimple::Entier, Simple::N(_)) | (TypeSimple::Booleen, Simple::B(_)))
            })
    })
}

#[derive(Clone, Debug, PartialEq)]
enum Simple {
    T(String),
    N(i64),
    B(bool),
}

/// Un objet JSON plat : `{ "clé": "texte" | entier | true | false, … }`. `None` pour tout le reste.
fn lire_objet(s: &str) -> Option<Vec<(String, Simple)>> {
    let b = s.trim().as_bytes();
    let mut i = 0;
    let blanc = |i: &mut usize| {
        while *i < b.len() && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
    };
    let chaine = |i: &mut usize| -> Option<String> {
        if b.get(*i) != Some(&b'"') {
            return None;
        }
        *i += 1;
        let mut out = Vec::new();
        while *i < b.len() {
            match b[*i] {
                b'"' => {
                    *i += 1;
                    return String::from_utf8(out).ok();
                }
                b'\\' => {
                    let c = *b.get(*i + 1)?;
                    if !matches!(c, b'"' | b'\\' | b'/') {
                        return None;
                    }
                    out.push(c);
                    *i += 2;
                }
                c if c < 0x20 => return None,
                c => {
                    out.push(c);
                    *i += 1;
                }
            }
        }
        None
    };
    blanc(&mut i);
    if b.get(i) != Some(&b'{') {
        return None;
    }
    i += 1;
    let mut v = Vec::new();
    blanc(&mut i);
    if b.get(i) == Some(&b'}') {
        return (i + 1 == b.len()).then_some(v);
    }
    loop {
        blanc(&mut i);
        let k = chaine(&mut i)?;
        blanc(&mut i);
        if b.get(i) != Some(&b':') {
            return None;
        }
        i += 1;
        blanc(&mut i);
        let x = match b.get(i)? {
            b'"' => Simple::T(chaine(&mut i)?),
            b't' if b[i..].starts_with(b"true") => {
                i += 4;
                Simple::B(true)
            }
            b'f' if b[i..].starts_with(b"false") => {
                i += 5;
                Simple::B(false)
            }
            c if *c == b'-' || c.is_ascii_digit() => {
                let d = i;
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                Simple::N(std::str::from_utf8(&b[d..i]).ok()?.parse().ok()?)
            }
            _ => return None,
        };
        v.push((k, x));
        blanc(&mut i);
        match b.get(i)? {
            b',' => i += 1,
            b'}' => return (i + 1 == b.len()).then_some(v),
            _ => return None,
        }
    }
}

impl Catalogue {
    /// Ce que la valeur `v` de la valeur qui confirme dit de l'ordre : prouvé, ou pas encore. La
    /// forme est celle de la demande (sa forme d'admission), pas celle d'aujourd'hui.
    pub fn confirme_par(&self, f: u16, premier: i64, v: i64, en_mouvement: bool) -> bool {
        let x = &self.formes[usize::from(f)];
        if x.retour != retour::ETAT && x.retour != retour::ACCUSE {
            return false;
        }
        if x.drapeaux & drapeau::A_L_ARRET != 0 && en_mouvement {
            return false;
        }
        if x.drapeaux & drapeau::ATTENDU_PARAM != 0 {
            return (v - premier).abs() <= i64::from(x.tolerance);
        }
        self.attendus_de(f).iter().any(|&a| i64::from(a) == v)
    }

    /// Le schéma d'outil qu'un agent reçoit pour un ordre (JSON Schema, en unités humaines).
    pub fn schema_outil(&self, f: u16) -> String {
        let x = &self.formes[usize::from(f)];
        let mut s = String::from("{\"type\":\"object\",\"properties\":{");
        for (k, p) in self.params_de(f).iter().enumerate() {
            if k > 0 {
                s.push(',');
            }
            s.push_str(&format!("\"p{k}\":"));
            match p.nature {
                nature::BINAIRE => s.push_str("{\"type\":\"boolean\"}"),
                nature::MESURE => s.push_str(&format!(
                    "{{\"type\":\"number\",\"minimum\":{},\"maximum\":{},\"multipleOf\":{},\"x-unite\":{}}}",
                    humain(p.min, p.echelle),
                    humain(p.max, p.echelle),
                    humain(i32::from(p.pas.max(1)), p.echelle),
                    p.unite
                )),
                nature::ENUM => {
                    let noms: Vec<String> = self.codes(p.dico).iter().filter(|(c, _)| *c >= p.min && *c <= p.max).map(|(_, n)| format!("\"{n}\"")).collect();
                    s.push_str(&format!("{{\"enum\":[{}]}}", noms.join(",")));
                }
                nature::OBJET => s.push_str(&format!("{{\"type\":\"object\",\"x-schema\":{}}}", self.schemas[usize::from(x.schema)])),
                _ => s.push_str(&format!("{{\"type\":\"string\",\"maxLength\":{}}}", p.pas)),
            }
        }
        s.push_str(&format!(
            "}},\"x-effet\":\"{}\",\"x-retour\":\"{}\",\"x-suivi_s\":{}}}",
            if x.effet == effet::APPUI { "appui" } else { "consigne" },
            ["", "etat", "pont", "accuse", "aucun"][usize::from(x.retour)],
            x.suivi_s
        ));
        s
    }

    /// `meta/forme/<f>` : la forme telle qu'elle voyage, une fois (JSON compact), schéma compris.
    pub fn json_forme(&self, f: u16) -> String {
        let d = self.decl(f);
        let mut s = format!("{{\"f\":{f},\"e\":{},\"r\":{},\"c\":\"{}\",\"d\":{},\"s\":{},\"t\":{},\"p\":[", d.effet, d.retour, d.confirme, d.drapeaux, d.suivi_s, d.tolerance);
        for (k, p) in d.params.iter().enumerate() {
            if k > 0 {
                s.push(',');
            }
            s.push_str(&format!("[{},{},{},{},{},{},{},\"{}\"]", p.nature, p.min, p.max, p.pas, p.dico, p.echelle, p.unite, p.regle));
        }
        s.push_str("],\"a\":[");
        s.push_str(&d.attendus.iter().map(i32::to_string).collect::<Vec<_>>().join(","));
        s.push(']');
        if let Some(sc) = &d.schema {
            s.push_str(&format!(",\"o\":{sc}"));
        }
        s.push('}');
        s
    }

    /// Octets alloués (tailles réelles : `size_of` et capacités ; la table de partage est mesurée
    /// par le banc avec le compteur d'allocations).
    pub fn octets_tableaux(&self) -> usize {
        std::mem::size_of::<Forme>() * self.formes.capacity()
            + std::mem::size_of::<Param>() * self.params.capacity()
            + 4 * self.attendus.capacity()
            + 4 * self.refs.capacity()
            + std::mem::size_of::<Fiche>() * self.fiches.capacity()
            + 4 * self.appareils_o.capacity()
    }

    /// La section « catalogue » de l'état complet et de l'instantané : clés, dictionnaires, formes,
    /// fiches.
    pub fn ranger(&self, b: &mut Vec<u8>) {
        put32(b, self.cles.len() as u32);
        for k in &self.cles {
            put16(b, k.len() as u16);
            b.extend_from_slice(k.as_bytes());
        }
        put32(b, self.dicos.len() as u32);
        for d in &self.dicos {
            put32(b, d.len() as u32);
            for (c, n) in d {
                put32(b, *c as u32);
                put16(b, n.len() as u16);
                b.extend_from_slice(n.as_bytes());
            }
        }
        put32(b, self.formes.len() as u32);
        for f in 1..self.formes.len() {
            let x = self.formes[f];
            let d = self.decl(f as u16);
            put16(b, u16::from(d.effet) | u16::from(d.retour) << 8);
            put16(b, x.confirme);
            put16(b, u16::from(d.drapeaux) | u16::from(d.classe) << 8);
            put16(b, d.suivi_s);
            put16(b, d.tolerance);
            put16(b, d.params.len() as u16);
            for q in self.params_de(f as u16) {
                put32(b, q.min as u32);
                put32(b, q.max as u32);
                put16(b, q.pas);
                put16(b, q.dico);
                put16(b, q.regle);
                b.extend_from_slice(&[q.nature, q.echelle as u8, q.unite]);
            }
            put16(b, d.attendus.len() as u16);
            for &a in &d.attendus {
                put32(b, a as u32);
            }
            let s = d.schema.unwrap_or_default();
            put16(b, s.len() as u16);
            b.extend_from_slice(s.as_bytes());
        }
        put32(b, self.fiches.len() as u32);
        for (x, a) in self.fiches.iter().zip(&self.appareils_o) {
            put64(b, x.valeur as u64);
            put64(b, x.instant_ms);
            put64(b, x.demande.0);
            put32(b, *a);
            put32(b, x.qui);
            put16(b, x.forme);
            put16(b, x.pour);
            put16(b, x.actives);
            b.extend_from_slice(&[x.issue, x.origine, x.changes, x.preuve, x.preuve_pour]);
        }
    }

    pub fn relire(b: &mut &[u8]) -> Option<Catalogue> {
        let mut c = Catalogue::new();
        for _ in 0..get32(b)? {
            let l = get16(b)?;
            let s = std::str::from_utf8(take(b, usize::from(l))?).ok()?;
            c.cle(s);
        }
        c.dicos.clear();
        for _ in 0..get32(b)? {
            let mut d = Vec::new();
            for _ in 0..get32(b)? {
                let code = get32(b)? as i32;
                let l = get16(b)?;
                d.push((code, std::str::from_utf8(take(b, usize::from(l))?).ok()?.into()));
            }
            c.dicos.push(d);
        }
        let nf = get32(b)? as usize;
        for _ in 1..nf {
            let er = get16(b)?;
            let confirme = get16(b)?;
            let dc = get16(b)?;
            let (drapeaux, classe) = (dc as u8, (dc >> 8) as u8);
            let (suivi_s, tolerance) = (get16(b)?, get16(b)?);
            let np = get16(b)?;
            let mut params = Vec::with_capacity(usize::from(np));
            for _ in 0..np {
                let (min, max) = (get32(b)? as i32, get32(b)? as i32);
                let (pas, dico, regle) = (get16(b)?, get16(b)?, get16(b)?);
                let q = take(b, 3)?;
                params.push(ParamDecl { nature: q[0], min, max, pas, dico, echelle: q[1] as i8, unite: q[2], regle: c.nom_cle(regle).to_string() });
            }
            let na = get16(b)?;
            let mut attendus = Vec::with_capacity(usize::from(na));
            for _ in 0..na {
                attendus.push(get32(b)? as i32);
            }
            let ls = get16(b)?;
            let s = std::str::from_utf8(take(b, usize::from(ls))?).ok()?;
            let d = FormeDecl {
                effet: er as u8,
                retour: (er >> 8) as u8,
                confirme: c.nom_cle(confirme).to_string(),
                drapeaux,
                suivi_s,
                tolerance,
                params,
                attendus,
                schema: (!s.is_empty()).then(|| s.to_string()),
                classe,
            };
            let h = cle_h(&d, &c.hacheur);
            c.creer(&d, h);
        }
        let n = get32(b)? as usize;
        for _ in 0..n {
            let valeur = get64(b)? as i64;
            let instant_ms = get64(b)?;
            let demande = crate::demandes::Ref(get64(b)?);
            let (appareil, qui) = (get32(b)?, get32(b)?);
            let (forme, pour, actives) = (get16(b)?, get16(b)?, get16(b)?);
            let q = take(b, 5)?;
            c.fiches.push(Fiche { valeur, instant_ms, demande, qui, forme, pour, actives, issue: q[0], origine: q[1], changes: q[2], preuve: q[3], preuve_pour: q[4], _r: 0 });
            c.appareils_o.push(appareil);
            if forme != 0 {
                c.refs[usize::from(forme)] += 1;
            }
        }
        Some(c)
    }

    pub fn empreinte(&self, h: &mut Fnv) {
        h.u64(self.formes.len() as u64);
        let mut b = Vec::new();
        self.ranger(&mut b);
        h.octets(&b);
        for &r in &self.refs {
            h.u64(u64::from(r));
        }
    }
}

/// Une borne au canon, en unités humaines (`50` à l'échelle −1 : `5`).
pub fn humain(v: i32, echelle: i8) -> String {
    if echelle >= 0 {
        return (i64::from(v) * 10i64.pow(echelle as u32)).to_string();
    }
    let d = 10i64.pow(u32::from(echelle.unsigned_abs()));
    let v = i64::from(v);
    if v % d == 0 { (v / d).to_string() } else { format!("{}", v as f64 / d as f64) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lampe() -> FormeDecl {
        FormeDecl::regler(mesure(0, 254, 1, 0, 0), "brightness", 2)
    }

    #[test]
    fn deux_ordres_de_meme_structure_partagent_leur_forme() {
        let mut c = Catalogue::new();
        let a = c.forme(&lampe()).unwrap();
        let b = c.forme(&lampe()).unwrap();
        assert_eq!(a, b);
        let mut autre = lampe();
        autre.params[0].max = 100;
        assert_ne!(c.forme(&autre).unwrap(), a, "une borne différente fait une autre forme");
        let mut attendu = FormeDecl::sans_param(effet::APPUI, retour::ACCUSE);
        attendu.attendus = vec![5, 6];
        let x = c.forme(&attendu).unwrap();
        attendu.attendus = vec![5, 7];
        assert_ne!(c.forme(&attendu).unwrap(), x, "d'autres valeurs attendues font une autre forme");
        assert_eq!(c.nb_formes(), 4);
        assert_eq!(c.formes[usize::from(a)].confirme, c.cle_existante("brightness").unwrap(), "la forme cite la valeur par sa clé");
    }

    /// Un corpus fixe : chaque décision attendue est écrite ici, pas déduite d'une autre machine qui
    /// appellerait la même fonction (revue I14).
    #[test]
    fn la_validation_suit_la_forme_sur_un_corpus_fixe() {
        let mut c = Catalogue::new();
        c.dicos.push(vec![(1, "eco".into()), (3, "confort".into())]);
        let consigne = c.forme(&FormeDecl::regler(mesure(50, 300, 5, -1, 1), "temperature", 0)).unwrap();
        let mode = c.forme(&FormeDecl { params: vec![code(1, 1, 3)], ..FormeDecl::sans_param(effet::CONSIGNE, retour::PONT) }).unwrap();
        let t = c.forme(&FormeDecl { params: vec![texte(16)], ..FormeDecl::sans_param(effet::APPUI, retour::PONT) }).unwrap();
        let o = c
            .forme(&FormeDecl { params: vec![objet(200)], schema: Some("{\"uri\":\"string\",\"volume\":\"integer\"}".into()), ..FormeDecl::sans_param(effet::APPUI, retour::ACCUSE) })
            .unwrap();
        let corpus: Vec<(u16, Vec<Arg>, Result<(), u8>)> = vec![
            (consigne, vec![Arg::N(215)], Ok(())),
            (consigne, vec![Arg::N(301)], Err(raison::BORNES)),
            (consigne, vec![Arg::N(212)], Err(raison::BORNES)),
            (consigne, vec![], Err(raison::NOMBRE)),
            (consigne, vec![Arg::T("215".into())], Err(raison::BORNES)),
            (mode, vec![Arg::N(1)], Ok(())),
            (mode, vec![Arg::N(3)], Ok(())),
            (mode, vec![Arg::N(2)], Err(raison::BORNES)),
            (t, vec![Arg::T("bonjour".into())], Ok(())),
            (t, vec![Arg::T("un texte bien trop long".into())], Err(raison::BORNES)),
            (o, vec![Arg::T("{\"uri\":\"x-sonos://a\",\"volume\":20}".into())], Ok(())),
            (o, vec![Arg::T("{\"uri\":\"a\"}".into())], Ok(())),
            (o, vec![Arg::T("{\"uri\":\"a\",\"volume\":\"fort\"}".into())], Err(raison::BORNES)),
            (o, vec![Arg::T("{\"uri\":\"a\",\"autre\":1}".into())], Err(raison::BORNES)),
            (o, vec![Arg::T("{\"uri\":\"a\",\"uri\":\"b\"}".into())], Err(raison::BORNES)),
            (o, vec![Arg::T("pas du json".into())], Err(raison::BORNES)),
            (o, vec![Arg::T("{\"uri\":{\"x\":1}}".into())], Err(raison::BORNES)),
        ];
        for (f, args, attendu) in corpus {
            assert_eq!(c.valider(f, &args), attendu, "forme {f}, {args:?}");
            let d = c.decl(f);
            assert_eq!(valider_decl(&d, &args, &|k, v| c.codes(k).iter().any(|x| x.0 == v)), attendu, "l'écran, forme {f}, {args:?}");
        }
        assert!(c.forme(&FormeDecl { params: vec![objet(200)], schema: Some("{\"a\":{\"b\":\"string\"}}".into()), ..FormeDecl::sans_param(effet::APPUI, retour::ACCUSE) }).is_err(), "un schéma imbriqué est refusé à la déclaration");
        assert!(c.json_forme(o).contains("\"o\":{\"uri\":\"string\""), "la forme publiée porte son schéma");
        assert!(c.schema_outil(consigne).contains("\"minimum\":5,\"maximum\":30"), "l'agent lit 5–30, pas 50–300");
    }

    #[test]
    fn le_catalogue_relu_est_identique() {
        let mut c = Catalogue::new();
        c.dicos.push(vec![(1, "eco".into())]);
        let f = c.forme(&lampe()).unwrap();
        let g = c.forme(&FormeDecl { schema: Some("{\"uri\":\"string\"}".into()), ..FormeDecl::sans_param(effet::APPUI, retour::ACCUSE) }).unwrap();
        c.poser(0, 7, f);
        c.poser(3, 7, g);
        c.fiches[0].valeur = 128;
        c.fiches[0].preuve = preuve::CHANGEMENT;
        let mut b = Vec::new();
        c.ranger(&mut b);
        let mut r = Catalogue::relire(&mut b.as_slice()).unwrap();
        let (mut h1, mut h2) = (Fnv::default(), Fnv::default());
        c.empreinte(&mut h1);
        r.empreinte(&mut h2);
        assert_eq!(h1.0, h2.0);
        assert_eq!(r.forme(&lampe()).unwrap(), f, "la forme relue garde son numéro");
        assert_eq!(r.codes(1).len(), 1, "les dictionnaires sont dans l'instantané");
    }
}
