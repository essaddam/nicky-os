//! Les demandes : ce que la maison est en train de demander. Une fiche de 72 o par demande, dans
//! des cases réutilisées, comme les exécutions de la mémoire v2 (§ 8.4) : la génération écarte une
//! issue tardive destinée à l'occupante d'avant. Une demande retenue pour un humain est une demande
//! comme une autre, dans l'état `RETENUE`. Les reçus d'idempotence ont leur table, à côté : une
//! fenêtre par principal (le compte qui demande), avec sa plus haute séquence.
//!
//! v2 (revue Codex B3, B4) : la génération tient sur 32 bits et une case épuisée est condamnée ; les
//! reçus gardent la plus haute séquence de chaque principal, refusent les clés trop anciennes et les
//! conflits, et ne s'effacent jamais pour faire place à un autre appelant.

use crate::numeros::{Fnv, get16, get32, get64, put16, put32, put64, take};

/// Le numéro d'une demande : sa case (bits 0 à 15), l'époque de l'actif qui l'a prise (bits 16 à
/// 31 : son domaine de reprise) et sa génération (bits 32 à 63). 0 : aucune. Il voyage dans `cmd/` et
/// dans l'issue du module, et c'est le champ `demande` d'une exécution. Quatre milliards de
/// générations par case : le numéro entier n'est jamais redonné ; une case qui atteindrait la
/// dernière est condamnée (la règle du nettoyage). L'époque sépare deux actifs : le nouveau ne peut
/// pas redonner un numéro que l'ancien a pris sans le lui transmettre (revue B4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ref(pub u64);

impl Ref {
    pub fn new(case: u16, epoque: u16, generation: u32) -> Self {
        Self(u64::from(case) | u64::from(epoque) << 16 | u64::from(generation) << 32)
    }
    pub fn case(self) -> usize {
        (self.0 & 0xFFFF) as usize
    }
    pub fn epoque(self) -> u16 {
        (self.0 >> 16) as u16
    }
    pub fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

/// Au plus 65 536 cases : les demandes en cours (4 096 au plus) et les cases condamnées.

/// La dernière génération d'une case : la case qui la porte est condamnée quand elle se libère.
pub const GEN_MAX: u32 = u32::MAX;

pub mod etat {
    pub const LIBRE: u8 = 0;
    /// Retenue par la garde : un humain décide (10 min au plus).
    pub const RETENUE: u8 = 1;
    /// Remise au module (ou en attente de l'accusé du secours, pour un appui) : pas encore partie.
    pub const EN_FILE: u8 = 2;
    /// Partie vers l'appareil : son issue arrive avant l'échéance.
    pub const PARTIE: u8 = 3;
    /// Son issue est tombée (`accepte`, `envoye`, `incertain`) : la suite peut venir, dans `suivi_s`.
    pub const SUIVIE: u8 = 4;
    /// Un ordre de groupe qui attend ses filles : sans échéance à lui, borné par celles des filles.
    pub const MERE: u8 = 5;
}

/// Les neuf issues de la V7 (§ 5.6), plus `EN_COURS` pour la fiche d'un ordre.
pub mod issue {
    pub const CONFIRME: u8 = 1;
    pub const EXECUTE: u8 = 2;
    pub const ACCEPTE: u8 = 3;
    pub const ENVOYE: u8 = 4;
    pub const INCERTAIN: u8 = 5;
    pub const NON_ENVOYE: u8 = 6;
    pub const ECHEC: u8 = 7;
    pub const REFUSE: u8 = 8;
    pub const RETENUE: u8 = 9;
    pub const EN_COURS: u8 = 10;
    pub const NOMS: [&str; 11] =
        ["", "confirme", "execute", "accepte", "envoye", "incertain", "non_envoye", "echec", "refuse", "retenue", "en_cours"];
}

pub mod origine {
    pub const UI: u8 = 1;
    pub const MENAGE: u8 = 2;
    pub const API: u8 = 3;
    pub const MCP: u8 = 4;
    pub const ASSISTANT: u8 = 5;
    pub const AUTOMATISME: u8 = 6;
    pub const SCENE: u8 = 7;
    pub const SYSTEME: u8 = 8;
    pub fn agent(o: u8) -> bool {
        matches!(o, API | MCP | ASSISTANT)
    }
}

/// Ce qu'une demande porte en plus.
pub mod drapeau_d {
    /// Ses paramètres (au-delà du premier, ou un texte) sont au tiroir des paramètres.
    pub const PARAMS: u8 = 1;
    /// Un humain l'a approuvée : ses filles (un groupe) ne sont pas retenues de nouveau.
    pub const APPROUVEE: u8 = 2;
    /// Un texte privé l'accompagne, au tiroir privé (jamais sur `cmd/`, `ord/` ni au journal).
    pub const PRIVE: u8 = 4;
    /// Un groupe réduit au sous-ensemble permis de sa carte (v3) : ses filles sont les cibles de la carte.
    pub const SOUS_ENSEMBLE: u8 = 8;
}

/// Une demande : 72 o.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Demande {
    /// Le premier paramètre, au canon (0 pour un texte : il est au tiroir).
    pub valeur: i64,
    /// La séquence de l'appelant : avec son principal, la clé d'idempotence ; 0 pour une demande
    /// interne (le moteur, le hub, la fille d'un groupe), qui n'a pas de reçu.
    pub seq: u64,
    /// Horloge monotone (ms) : l'issue, la fin du suivi, ou celle de la retenue.
    pub echeance_ms: u64,
    /// Heure murale (ms) : son arrivée, pour le journal et l'écran.
    pub recue_ms: u64,
    /// La demande mère (un groupe) ; 0 : aucune.
    pub parent: Ref,
    /// Le numéro de l'ordre.
    pub ordre: u32,
    /// Qui : une personne, un agent (un rang), une exécution du moteur (case | génération).
    pub qui: u32,
    pub generation: u32,
    /// La forme sous laquelle elle a été acceptée : son interprétation ne change plus, même si
    /// l'ordre est redéclaré pendant qu'elle vit (revue I3).
    pub forme: u16,
    /// Le principal qui l'a demandée : le compte de la connexion, le moteur, le hub. Jamais déclaré
    /// par le client lui-même (revue B1).
    pub de: u16,
    pub etat: u8,
    pub issue: u8,
    pub origine: u8,
    pub raison: u8,
    /// Un ordre de groupe : ses cibles, et combien ont conclu, confirmé, exécuté (V7 G17).
    pub cibles: u8,
    pub conclues: u8,
    pub confirmees: u8,
    pub executees: u8,
    pub drapeaux: u8,
    /// Ce qu'on sait de la personne (`droits::preuve`) : établi par le serveur (v3).
    pub preuve: u8,
    /// L'époque de l'actif qui l'a prise (la moitié de son numéro).
    pub epoque: u16,
    /// La personne pour qui elle est faite (0 : inconnue) : ses droits sont engagés (v3).
    pub pour: u16,
    /// Le mandat d'un automatisme (son numéro + 1 ; 0 : aucun) : il borne ce qu'elle commande (v3).
    pub mandat: u16,
}

const _: () = assert!(std::mem::size_of::<Demande>() == 72);

/// Les cases des demandes, réutilisées.
#[derive(Clone, Default)]
pub struct Demandes {
    pub fiches: Vec<Demande>,
    libres: Vec<u32>,
    /// Les cases dont la génération est épuisée : jamais redonnées.
    pub condamnees: u32,
    pub en_cours: usize,
    pub pic: usize,
}

impl Demandes {
    /// Une case libre, à la génération suivante (jamais 0 : la demande 0 n'existe pas), à l'époque
    /// de l'actif qui la prend.
    pub fn prendre(&mut self, d: Demande, epoque: u16) -> Ref {
        let case = match self.libres.pop() {
            Some(c) => c as usize,
            None => {
                self.fiches.push(Demande::default());
                self.fiches.len() - 1
            }
        };
        let g = self.fiches[case].generation + 1;
        self.fiches[case] = Demande { generation: g, epoque, ..d };
        self.en_cours += 1;
        self.pic = self.pic.max(self.en_cours);
        Ref::new(case as u16, epoque, g)
    }

    /// Une copie applique la trace d'une demande : la même case, la même génération.
    pub fn placer(&mut self, r: Ref, d: Demande) {
        let case = r.case();
        if case >= self.fiches.len() {
            let avant = self.fiches.len();
            self.fiches.resize(case + 1, Demande::default());
            // Les cases sautées sont libres, à la génération 0, comme chez l'actif.
            for c in avant..case {
                self.libres.insert(0, c as u32);
            }
        }
        let nouvelle = self.fiches[case].etat == etat::LIBRE;
        self.libres.retain(|&c| c as usize != case);
        self.fiches[case] = Demande { generation: r.generation(), epoque: r.epoque(), ..d };
        if nouvelle {
            self.en_cours += 1;
            self.pic = self.pic.max(self.en_cours);
        }
    }

    /// La demande d'un numéro, si c'est toujours elle qui occupe la case.
    pub fn get(&self, r: Ref) -> Option<&Demande> {
        self.fiches.get(r.case()).filter(|d| d.etat != etat::LIBRE && d.generation == r.generation() && d.epoque == r.epoque())
    }

    pub fn get_mut(&mut self, r: Ref) -> Option<&mut Demande> {
        self.fiches.get_mut(r.case()).filter(|d| d.etat != etat::LIBRE && d.generation == r.generation() && d.epoque == r.epoque())
    }

    /// La demande est conclue : la case se libère, la génération reste ; une case à sa dernière
    /// génération est condamnée.
    pub fn liberer(&mut self, r: Ref) {
        if self.get(r).is_some() {
            let g = self.fiches[r.case()].generation;
            self.fiches[r.case()] = Demande { generation: g, ..Demande::default() };
            if g == GEN_MAX {
                self.condamnees += 1;
            } else {
                self.libres.push(r.case() as u32);
            }
            self.en_cours -= 1;
        }
    }

    pub fn refs(&self) -> impl Iterator<Item = Ref> + '_ {
        self.fiches.iter().enumerate().filter(|(_, d)| d.etat != etat::LIBRE).map(|(c, d)| Ref::new(c as u16, d.epoque, d.generation))
    }

    /// La demande d'un principal et de sa séquence, si elle vit encore.
    pub fn par_cle(&self, de: u16, seq: u64) -> Option<Ref> {
        self.refs().find(|&r| {
            let d = &self.fiches[r.case()];
            d.de == de && d.seq == seq
        })
    }

    /// Octets alloués : la taille réelle des fiches et de la liste des cases libres.
    pub fn octets(&self) -> usize {
        std::mem::size_of::<Demande>() * self.fiches.capacity() + 4 * self.libres.capacity()
    }

    /// La section « demandes » de l'état complet : les cases occupées et toutes les générations
    /// (une case libre garde la sienne, pour ne jamais redonner un numéro).
    pub fn ranger(&self, b: &mut Vec<u8>, vers_mur: &dyn Fn(u64) -> u64) {
        put32(b, self.fiches.len() as u32);
        put32(b, self.condamnees);
        for d in &self.fiches {
            put32(b, d.generation);
            b.push(d.etat);
            if d.etat == etat::LIBRE {
                continue;
            }
            put16(b, d.epoque);
            put64(b, d.valeur as u64);
            put64(b, d.seq);
            put64(b, if d.echeance_ms == u64::MAX { u64::MAX } else { vers_mur(d.echeance_ms) });
            put64(b, d.recue_ms);
            put64(b, d.parent.0);
            put32(b, d.ordre);
            put32(b, d.qui);
            put16(b, d.forme);
            put16(b, d.de);
            put16(b, d.pour);
            put16(b, d.mandat);
            b.extend_from_slice(&[d.issue, d.origine, d.raison, d.cibles, d.conclues, d.confirmees, d.executees, d.drapeaux, d.preuve]);
        }
    }

    pub fn relire(b: &mut &[u8], vers_mono: &dyn Fn(u64) -> u64) -> Option<Demandes> {
        let n = get32(b)? as usize;
        let mut t = Demandes { condamnees: get32(b)?, ..Demandes::default() };
        for c in 0..n {
            let generation = get32(b)?;
            let e = take(b, 1)?[0];
            if e == etat::LIBRE {
                t.fiches.push(Demande { generation, ..Demande::default() });
                if generation != GEN_MAX {
                    t.libres.push(c as u32);
                }
                continue;
            }
            let epoque = get16(b)?;
            let (valeur, seq) = (get64(b)? as i64, get64(b)?);
            let e_mur = get64(b)?;
            let echeance_ms = if e_mur == u64::MAX { u64::MAX } else { vers_mono(e_mur) };
            let (recue_ms, parent) = (get64(b)?, Ref(get64(b)?));
            let (ordre, qui) = (get32(b)?, get32(b)?);
            let (forme, de) = (get16(b)?, get16(b)?);
            let (pour, mandat) = (get16(b)?, get16(b)?);
            let q = take(b, 9)?;
            t.fiches.push(Demande {
                valeur,
                seq,
                echeance_ms,
                recue_ms,
                parent,
                ordre,
                qui,
                generation,
                forme,
                de,
                etat: e,
                issue: q[0],
                origine: q[1],
                raison: q[2],
                cibles: q[3],
                conclues: q[4],
                confirmees: q[5],
                executees: q[6],
                drapeaux: q[7],
                preuve: q[8],
                epoque,
                pour,
                mandat,
            });
            t.en_cours += 1;
        }
        t.libres.reverse();
        t.pic = t.en_cours;
        Some(t)
    }

    /// L'empreinte : cases, générations et contenu, échéances exclues (chaque machine a sa propre
    /// horloge monotone).
    pub fn empreinte(&self, h: &mut Fnv) {
        h.u64(u64::from(self.condamnees));
        for d in &self.fiches {
            h.u64(u64::from(d.generation) | u64::from(d.etat) << 32);
            if d.etat != etat::LIBRE {
                h.u64(d.valeur as u64);
                h.u64(d.seq);
                h.u64(d.recue_ms);
                h.u64(d.parent.0);
                h.u64(u64::from(d.ordre) | u64::from(d.qui) << 32);
                h.u64(u64::from(d.forme) | u64::from(d.de) << 16 | u64::from(d.epoque) << 32);
                h.octets(&[d.issue, d.origine, d.raison, d.cibles, d.conclues, d.confirmees, d.executees, d.drapeaux, d.preuve]);
                h.u64(u64::from(d.pour) | u64::from(d.mandat) << 16);
            }
        }
    }
}

/// Un reçu d'idempotence : une demande prise deux fois avec la même clé ne part qu'une fois, et la
/// seconde reçoit l'issue de la première. 32 o.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Recu {
    pub seq: u64,
    pub demande: Ref,
    /// L'empreinte des arguments, sur 64 bits, de leur encodage canonique (type, longueur, valeur) :
    /// la même clé avec un autre ordre ou d'autres arguments est un conflit, jamais une seconde
    /// demande (revue v3, M6 : `["ab","c"]` et `["a","bc"]` ne se confondent plus).
    pub args: u64,
    pub ordre: u32,
    pub issue: u8,
    pub raison: u8,
    pub _r: [u8; 2],
}

const _: () = assert!(std::mem::size_of::<Recu>() == 32);

/// 32 reçus par principal, comme aujourd'hui (`REQUEST_WINDOW`, occurrences.rs:134).
pub const RECU_FENETRE: usize = 32;

/// La fenêtre d'un principal : sa plus haute séquence, et ses 32 derniers reçus. 1 032 o.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fenetre {
    pub plus_haute: u64,
    pub recus: [Recu; RECU_FENETRE],
}

/// Ce que dit la table d'une clé qui arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Jamais vue : la demande est neuve.
    Neuve,
    /// Déjà prise, même ordre, mêmes arguments : la même demande, la même issue.
    Double(Recu),
    /// Déjà prise pour un autre ordre ou d'autres arguments : refusée.
    Conflit,
    /// Plus ancienne que la fenêtre : on ne peut plus savoir si elle est partie ; refusée.
    Perimee,
}

/// Les reçus : une fenêtre par principal, allouée à sa première demande, jamais cédée à un autre
/// (le nombre de principaux est borné par leur table, pas par celle des reçus).
#[derive(Clone, Default)]
pub struct Recus {
    pub fenetres: Vec<Option<Box<Fenetre>>>,
}

impl Recus {
    pub fn examiner(&self, de: u16, seq: u64, ordre: u32, args: u64) -> Verdict {
        let Some(f) = self.fenetres.get(usize::from(de)).and_then(|x| x.as_deref()) else { return Verdict::Neuve };
        if let Some(r) = f.recus.iter().find(|r| r.seq == seq) {
            return if r.ordre == ordre && r.args == args { Verdict::Double(*r) } else { Verdict::Conflit };
        }
        if seq.saturating_add(RECU_FENETRE as u64) <= f.plus_haute {
            return Verdict::Perimee;
        }
        Verdict::Neuve
    }

    /// Note un reçu neuf, ou l'issue nouvelle d'un reçu connu.
    pub fn noter(&mut self, de: u16, r: Recu) {
        let k = usize::from(de);
        if k >= self.fenetres.len() {
            self.fenetres.resize(k + 1, None);
        }
        let f = self.fenetres[k].get_or_insert_with(|| Box::new(Fenetre { plus_haute: 0, recus: [Recu::default(); RECU_FENETRE] }));
        f.plus_haute = f.plus_haute.max(r.seq);
        if let Some(x) = f.recus.iter_mut().find(|x| x.seq == r.seq) {
            *x = r;
            return;
        }
        // La place du plus ancien (une place vide a la séquence 0) : il serait « périmé » de toute façon.
        let vieux = f.recus.iter().enumerate().min_by_key(|(_, x)| x.seq).map_or(0, |(i, _)| i);
        f.recus[vieux] = r;
    }

    /// Le reçu d'une demande vivante, pour lui donner son issue.
    pub fn issue(&mut self, de: u16, seq: u64, issue: u8, raison: u8) {
        if let Some(Some(f)) = self.fenetres.get_mut(usize::from(de))
            && let Some(x) = f.recus.iter_mut().find(|x| x.seq == seq)
        {
            x.issue = issue;
            x.raison = raison;
        }
    }

    pub fn get(&self, de: u16, seq: u64) -> Option<Recu> {
        self.fenetres.get(usize::from(de)).and_then(|x| x.as_deref()).and_then(|f| f.recus.iter().find(|r| r.seq == seq).copied())
    }

    /// Octets alloués : un pointeur par principal, une fenêtre par principal qui a demandé.
    pub fn octets(&self) -> usize {
        8 * self.fenetres.capacity() + self.fenetres.iter().flatten().count() * std::mem::size_of::<Fenetre>()
    }

    pub fn ranger(&self, b: &mut Vec<u8>) {
        put32(b, self.fenetres.len() as u32);
        for f in &self.fenetres {
            let Some(f) = f else {
                b.push(0);
                continue;
            };
            b.push(1);
            put64(b, f.plus_haute);
            for r in &f.recus {
                put64(b, r.seq);
                put64(b, r.demande.0);
                put32(b, r.ordre);
                put64(b, r.args);
                b.extend_from_slice(&[r.issue, r.raison]);
            }
        }
    }

    pub fn relire(b: &mut &[u8]) -> Option<Recus> {
        let mut t = Recus::default();
        for _ in 0..get32(b)? {
            if take(b, 1)?[0] == 0 {
                t.fenetres.push(None);
                continue;
            }
            let mut f = Box::new(Fenetre { plus_haute: get64(b)?, recus: [Recu::default(); RECU_FENETRE] });
            for r in f.recus.iter_mut() {
                let (seq, demande) = (get64(b)?, Ref(get64(b)?));
                let (ordre, args) = (get32(b)?, get64(b)?);
                let q = take(b, 2)?;
                *r = Recu { seq, demande, ordre, args, issue: q[0], raison: q[1], _r: [0; 2] };
            }
            t.fenetres.push(Some(f));
        }
        Some(t)
    }

    pub fn empreinte(&self, h: &mut Fnv) {
        let mut b = Vec::new();
        self.ranger(&mut b);
        h.octets(&b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_case_reutilisee_change_de_generation() {
        let mut t = Demandes::default();
        let a = t.prendre(Demande { etat: etat::PARTIE, ordre: 7, ..Demande::default() }, 1);
        t.liberer(a);
        let b = t.prendre(Demande { etat: etat::PARTIE, ordre: 8, ..Demande::default() }, 1);
        assert_eq!(a.case(), b.case(), "la case sert de nouveau");
        assert_ne!(a, b, "le numéro entier, jamais");
        assert!(t.get(a).is_none(), "l'ancien numéro ne désigne plus rien");
        assert_eq!(t.get(b).unwrap().ordre, 8);
    }

    #[test]
    fn un_numero_d_une_autre_epoque_ne_designe_rien() {
        let mut t = Demandes::default();
        let a = t.prendre(Demande { etat: etat::PARTIE, ordre: 7, ..Demande::default() }, 1);
        let b = Ref::new(a.case() as u16, 2, a.generation());
        assert!(t.get(b).is_none(), "la même case, la même génération, prise par un autre actif : pas la même demande");
        assert_eq!(t.get(a).unwrap().ordre, 7);
    }

    #[test]
    fn une_case_epuisee_est_condamnee() {
        let mut t = Demandes::default();
        let a = t.prendre(Demande { etat: etat::PARTIE, ..Demande::default() }, 1);
        t.liberer(a);
        // La case 0 arrive à sa dernière génération (quatre milliards de réutilisations : 136 ans à une
        // demande par seconde sur la même case, 50 jours à mille) : elle sert une dernière fois, puis
        // plus jamais.
        t.fiches[0].generation = GEN_MAX - 1;
        let b = t.prendre(Demande { etat: etat::PARTIE, ..Demande::default() }, 1);
        assert_eq!((b.case(), b.generation()), (0, GEN_MAX));
        t.liberer(b);
        let c = t.prendre(Demande { etat: etat::PARTIE, ..Demande::default() }, 1);
        assert_eq!(c.case(), 1, "la case condamnée n'est jamais redonnée");
        assert_eq!(t.condamnees, 1);
        assert!(t.get(a).is_none() && t.get(b).is_none());
        let mut buf = Vec::new();
        t.ranger(&mut buf, &|x| x);
        let mut u = Demandes::relire(&mut buf.as_slice(), &|x| x).unwrap();
        u.liberer(c);
        let d = u.prendre(Demande { etat: etat::PARTIE, ..Demande::default() }, 1);
        assert_eq!(d.case(), 1, "relue, la case 0 reste condamnée");
    }

    #[test]
    fn un_appel_retente_retrouve_son_recu_et_un_vieux_rejeu_est_refuse() {
        let mut r = Recus::default();
        let recu = |seq: u64| Recu { seq, demande: Ref(seq + 100), ordre: 3, args: 9, issue: 1, ..Recu::default() };
        r.noter(4, recu(7));
        assert_eq!(r.examiner(4, 7, 3, 9), Verdict::Double(recu(7)));
        assert_eq!(r.examiner(4, 7, 3, 10), Verdict::Conflit, "la même clé, d'autres arguments");
        assert_eq!(r.examiner(4, 7, 5, 9), Verdict::Conflit, "la même clé, un autre ordre");
        assert_eq!(r.examiner(4, 8, 3, 9), Verdict::Neuve);
        assert_eq!(r.examiner(5, 7, 3, 9), Verdict::Neuve, "un autre principal, sa propre fenêtre");
        for s in 1..=33u64 {
            r.noter(4, recu(s));
        }
        assert_eq!(r.examiner(4, 1, 3, 9), Verdict::Perimee, "après 1 à 33, rejouer 1 ne repart pas");
        assert_eq!(r.examiner(4, 2, 3, 9), Verdict::Double(recu(2)));
        // 64 autres principaux : aucun ne prend la fenêtre du premier.
        for p in 10..74u16 {
            r.noter(p, recu(1));
        }
        assert_eq!(r.examiner(4, 33, 3, 9), Verdict::Double(recu(33)));
        let mut buf = Vec::new();
        r.ranger(&mut buf);
        let u = Recus::relire(&mut buf.as_slice()).unwrap();
        assert_eq!(u.examiner(4, 1, 3, 9), Verdict::Perimee, "la plus haute séquence survit à l'état complet");
    }

    #[test]
    fn la_table_relue_garde_les_generations() {
        let mut t = Demandes::default();
        let a = t.prendre(Demande { etat: etat::RETENUE, seq: 5, de: 2, ordre: 7, ..Demande::default() }, 1);
        let b = t.prendre(Demande { etat: etat::PARTIE, ordre: 8, ..Demande::default() }, 1);
        t.liberer(b);
        let mut buf = Vec::new();
        t.ranger(&mut buf, &|x| x);
        let mut u = Demandes::relire(&mut buf.as_slice(), &|x| x).unwrap();
        assert_eq!(u.get(a).unwrap().seq, 5);
        assert_eq!(u.par_cle(2, 5), Some(a));
        let c = u.prendre(Demande { etat: etat::PARTIE, ..Demande::default() }, 1);
        assert_ne!(c, b, "une demande relue ne redonne jamais un numéro");
    }
}
