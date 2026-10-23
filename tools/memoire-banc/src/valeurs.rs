//! La mémoire des valeurs : la forme d'aujourd'hui (recopiée de moli-core et moli-runtime), et la
//! forme proposée, où le numéro d'un point est sa place.

use std::collections::{BTreeMap, HashMap};
use std::hint::black_box;
use std::sync::{Arc, RwLock};

use crate::maison::{Maison, Nature};
use crate::outils::{Alea, ns, poids, resultat};

// ---------------------------------------------------------------------------------------------
// Aujourd'hui : moli_core::{Value, Sample}, moli-runtime States, le miroir du moteur.
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(Arc<str>),
}

#[derive(Clone, Debug)]
pub struct Sample {
    pub value: Value,
    pub ts: u64,
}

/// `HashMap<DeviceId, BTreeMap<Arc<str>, Sample>>` (state_cache.rs:15), derrière le verrou du hub.
pub type States = HashMap<Arc<str>, BTreeMap<Arc<str>, Sample>>;

pub fn valeur_aujourdhui(n: usize, nature: Nature) -> Value {
    match nature {
        Nature::Mesure => Value::Float(18.0 + (n % 70) as f64 / 10.0),
        Nature::Binaire => Value::Bool(n % 2 == 0),
        // Les énumérations sont en texte aujourd'hui.
        Nature::Enum => Value::Text(Arc::from(["auto", "manuel", "absent"][n % 3])),
        Nature::Texte => Value::Text(Arc::from("Lecture : France Inter")),
        Nature::Vide => Value::Null,
    }
}

pub fn states_aujourdhui(m: &Maison) -> States {
    let mut s = States::new();
    for (n, p) in m.points.iter().enumerate() {
        let (d, k) = p.nom.split_once('/').unwrap();
        s.entry(Arc::from(d))
            .or_default()
            .insert(Arc::from(k), Sample { value: valeur_aujourdhui(n, p.nature), ts: 1_792_675_624_123 });
    }
    s
}

/// `Hub::state` (hub.rs:779) : `PointId::split` alloue un `DeviceId`, puis le verrou, deux
/// recherches et une copie du relevé.
pub fn hub_state(s: &RwLock<States>, point: &str) -> Option<Sample> {
    let (d, k) = point.split_once('/')?;
    let device: Arc<str> = Arc::from(d);
    s.read().unwrap().get(&device)?.get(k).cloned()
}

/// Le miroir du moteur d'automatismes : `HashMap<String, Value>` (engine.rs, `last`).
pub fn miroir_aujourdhui(m: &Maison) -> HashMap<String, Value> {
    m.points.iter().enumerate().map(|(n, p)| (p.nom.clone(), valeur_aujourdhui(n, p.nature))).collect()
}

// ---------------------------------------------------------------------------------------------
// Proposé : l'annuaire (nom → numéro) au bord, puis des fiches de taille fixe rangées par numéro.
// ---------------------------------------------------------------------------------------------

pub mod qualite {
    pub const BONNE: u8 = 0;
    pub const INCERTAINE: u8 = 1;
    pub const MAUVAISE: u8 = 2;
}
pub mod vie {
    pub const VIVANT: u8 = 0;
    pub const MUET: u8 = 1;
}

/// La valeur vive d'un point (§ 11.2 de la V7) : 40 octets, aucune allocation.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Vif {
    pub valeur: i64,
    pub precedente: i64,
    pub observe_ms: u64,
    pub change_ms: u64,
    pub recu_s: u32,
    pub genres: u8,
    pub qualite: u8,
}

/// La variante à 32 octets : deux fiches par ligne de cache, aucune à cheval.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Vif32 {
    pub valeur: i64,
    pub precedente: i64,
    pub change_ms: u64,
    pub recu_s: u32,
    pub observe_ecart_ms: i16,
    pub genres: u8,
    pub qualite: u8,
}

/// Ce qu'une condition lit, seul dans sa colonne : 16 octets, quatre par ligne de cache.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Chaud {
    pub valeur: i64,
    pub change_s: u32,
    pub genre: u8,
    pub qualite: u8,
    pub _r: u16,
}

/// La définition d'un point (§ 11.4.4 de la V7) : 16 octets.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Def {
    pub appareil: u32,
    pub libelle: u32,
    pub cle: u16,
    pub dico: u16,
    pub nature: u8,
    pub unite: u8,
    pub echelle: i8,
    pub sens: u8,
}

/// La santé d'un appareil : 20 octets.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Sante {
    pub contact_s: u32,
    pub change_s: u32,
    pub rythme_s: u32,
    pub appris_s: u32,
    pub pertes: u8,
    pub vie: u8,
}

/// Le seul endroit où un nom devient un numéro. Chaque nom une fois, bout à bout ; la table de
/// hachage ne garde que des numéros. Un numéro n'est jamais réutilisé.
#[derive(Clone)]
pub struct Annuaire {
    texte: String,
    fins: Vec<u32>,
    table: hashbrown::HashTable<u32>,
    hacheur: std::hash::RandomState,
}

impl Default for Annuaire {
    fn default() -> Self {
        Self { texte: String::new(), fins: Vec::new(), table: hashbrown::HashTable::new(), hacheur: std::hash::RandomState::new() }
    }
}

impl Annuaire {
    pub fn nom(&self, n: u32) -> &str {
        let debut = if n == 0 { 0 } else { self.fins[n as usize - 1] as usize };
        &self.texte[debut..self.fins[n as usize] as usize]
    }
    pub fn numero(&self, nom: &str) -> Option<u32> {
        use std::hash::BuildHasher;
        let h = self.hacheur.hash_one(nom);
        self.table.find(h, |&n| self.nom(n) == nom).copied()
    }
    /// Le prochain numéro, au bout : la place suivante dans toutes les colonnes.
    pub fn attribuer(&mut self, nom: &str) -> u32 {
        use std::hash::BuildHasher;
        if let Some(n) = self.numero(nom) {
            return n;
        }
        let n = u32::try_from(self.fins.len()).expect("4 milliards de points");
        self.texte.push_str(nom);
        self.fins.push(self.texte.len() as u32);
        let h = self.hacheur.hash_one(nom);
        let (texte, fins, hacheur) = (&self.texte, &self.fins, &self.hacheur);
        let nom_de = |k: u32| {
            let debut = if k == 0 { 0 } else { fins[k as usize - 1] as usize };
            &texte[debut..fins[k as usize] as usize]
        };
        self.table.insert_unique(h, n, |&k| hacheur.hash_one(nom_de(k)));
        n
    }
    pub fn len(&self) -> usize {
        self.fins.len()
    }
    /// Au démarrage, l'annuaire relu a sa taille exacte : on rend la marge d'agrandissement.
    pub fn serrer(&mut self) {
        use std::hash::BuildHasher;
        self.texte.shrink_to_fit();
        self.fins.shrink_to_fit();
        let (texte, fins, hacheur) = (&self.texte, &self.fins, &self.hacheur);
        self.table.shrink_to_fit(|&k| {
            let debut = if k == 0 { 0 } else { fins[k as usize - 1] as usize };
            hacheur.hash_one(&texte[debut..fins[k as usize] as usize])
        });
    }
}

/// L'état vif du hub : une colonne par sorte de fiche, toutes indexées par le même numéro.
#[derive(Clone)]
pub struct Etat {
    pub annuaire: Annuaire,
    pub defs: Vec<Def>,
    pub vifs: Vec<Vif>,
    pub sante: Vec<Sante>,
    /// Par appareil : rythme × (pertes + 1), calculé une fois (0 : jamais muet).
    pub delai: Vec<u32>,
    /// Par appareil : contact + délai, écrit à chaque nouvelle (u32::MAX : aucune échéance).
    pub echeance: Vec<u32>,
    /// Par point : la fraîcheur demandée (`fraicheur_s` de la méta), 0 si aucune.
    pub fraicheur: Vec<u32>,
    /// Par point : reçu + fraîcheur (u32::MAX : aucune).
    pub perime: Vec<u32>,
    /// Le tiroir des textes : la fiche porte le rang, le texte vit ici.
    pub textes: Vec<Box<str>>,
}

impl Etat {
    pub fn depuis(m: &Maison) -> Self {
        let mut e = Etat {
            annuaire: Annuaire::default(),
            defs: Vec::with_capacity(m.points.len()),
            vifs: Vec::with_capacity(m.points.len()),
            sante: Vec::with_capacity(m.appareils.len()),
            delai: Vec::with_capacity(m.appareils.len()),
            echeance: Vec::with_capacity(m.appareils.len()),
            fraicheur: Vec::with_capacity(m.points.len()),
            perime: Vec::with_capacity(m.points.len()),
            textes: Vec::new(),
        };
        for a in &m.appareils {
            let r = a.classe.rythme_s();
            e.sante.push(Sante { rythme_s: r, pertes: a.classe.pertes(), ..Sante::default() });
            let delai = r.saturating_mul(u32::from(a.classe.pertes()) + 1);
            e.delai.push(delai);
            e.echeance.push(if delai == 0 { u32::MAX } else { delai });
        }
        for (n, p) in m.points.iter().enumerate() {
            let numero = e.annuaire.attribuer(&p.nom);
            debug_assert_eq!(numero as usize, n);
            e.defs.push(Def { appareil: p.appareil, nature: p.nature as u8, ..Def::default() });
            let valeur = match p.nature {
                Nature::Texte => {
                    e.textes.push("Lecture : France Inter".into());
                    (e.textes.len() - 1) as i64
                }
                Nature::Mesure => 1_800 + (n % 700) as i64,
                _ => (n % 3) as i64,
            };
            e.vifs.push(Vif { valeur, ..Vif::default() });
            // Une mesure sur deux demande sa fraîcheur (« une température de moins de 10 min »).
            let f = if p.nature == Nature::Mesure && n % 2 == 0 { 600 } else { 0 };
            e.fraicheur.push(f);
            e.perime.push(if f == 0 { u32::MAX } else { f });
        }
        e.annuaire.serrer();
        e.textes.shrink_to_fit();
        e
    }

    /// Une nouvelle d'un point : en place, sans allocation. Rend vrai si la valeur a changé.
    /// Les instants montrés sont en heure murale (ms) ; les échéances, en secondes monotones.
    #[inline]
    pub fn rapport(&mut self, n: u32, valeur: i64, mur_ms: u64, mono_s: u32) -> bool {
        let s = (mur_ms / 1_000) as u32;
        let i = n as usize;
        let v = &mut self.vifs[i];
        let change = v.valeur != valeur;
        if change {
            v.precedente = v.valeur;
            v.valeur = valeur;
            v.change_ms = mur_ms;
        }
        v.observe_ms = mur_ms;
        v.recu_s = s;
        v.qualite = qualite::BONNE;
        let d = self.defs[i].appareil as usize;
        let sante = &mut self.sante[d];
        sante.contact_s = s;
        sante.vie = vie::VIVANT;
        if self.delai[d] != 0 {
            self.echeance[d] = mono_s.saturating_add(self.delai[d]);
        }
        if self.fraicheur[i] != 0 {
            self.perime[i] = mono_s.saturating_add(self.fraicheur[i]);
        }
        change
    }

    /// Au redémarrage : chaque appareil et chaque fraîcheur reçoivent un délai complet, le hub
    /// n'écoutait pas. L'horloge monotone repart de zéro : les échéances se recalculent.
    pub fn grace(&mut self, mono_s: u32) {
        for d in 0..self.sante.len() {
            if self.delai[d] != 0 {
                self.echeance[d] = mono_s.saturating_add(self.delai[d]);
            }
        }
        for n in 0..self.fraicheur.len() {
            if self.fraicheur[n] != 0 {
                self.perime[n] = mono_s.saturating_add(self.fraicheur[n]);
            }
        }
    }

    /// Les points d'un appareil (leurs numéros se suivent à la déclaration).
    pub fn points_de(&self, d: u32) -> impl Iterator<Item = u32> + '_ {
        self.defs.iter().enumerate().filter(move |(_, x)| x.appareil == d).map(|(n, _)| n as u32)
    }

    /// Ce qu'une condition lit. La qualité effective tient compte de l'appareil : un appareil
    /// muet rend toutes ses valeurs mauvaises sans réécrire une seule fiche.
    #[inline]
    pub fn lire(&self, n: u32) -> (i64, u8) {
        let i = n as usize;
        let v = &self.vifs[i];
        let muet = self.sante[self.defs[i].appareil as usize].vie == vie::MUET;
        (v.valeur, if muet { qualite::MAUVAISE } else { v.qualite })
    }
}

// ---------------------------------------------------------------------------------------------
// Le banc : retrouver une valeur, l'écrire, copier tout l'état.
// ---------------------------------------------------------------------------------------------

pub fn banc(echelle: u32) {
    let m = crate::maison::generer(echelle);
    let nb = m.points.len();
    let sec = "valeurs";
    resultat(sec, echelle, "points", nb as f64, "");
    resultat(sec, echelle, "appareils", m.appareils.len() as f64, "");
    let noms_moyens = m.points.iter().map(|p| p.nom.len()).sum::<usize>() as f64 / nb as f64;
    resultat(sec, echelle, "longueur_moyenne_nom", noms_moyens, "car");

    // Les tailles, par la règle du compilateur.
    use std::mem::size_of;
    for (cle, t) in [
        ("taille_vif", size_of::<Vif>()),
        ("taille_vif32", size_of::<Vif32>()),
        ("taille_chaud", size_of::<Chaud>()),
        ("taille_def", size_of::<Def>()),
        ("taille_sante", size_of::<Sante>()),
        ("taille_value_aujourdhui", size_of::<Value>()),
        ("taille_sample_aujourdhui", size_of::<Sample>()),
    ] {
        resultat(sec, echelle, cle, t as f64, "o");
    }

    // La mémoire, octets demandés (allocateur compté). Aujourd'hui : seulement jusqu'à ×10.
    let aujourdhui = echelle <= 10;
    let (states, o_states) = poids(|| if aujourdhui { states_aujourdhui(&m) } else { States::new() });
    let (miroir, o_miroir) = poids(|| if aujourdhui { miroir_aujourdhui(&m) } else { HashMap::new() });
    if aujourdhui {
        resultat(sec, echelle, "memoire_states_aujourdhui", o_states as f64, "o");
        resultat(sec, echelle, "memoire_miroir_moteur_aujourdhui", o_miroir as f64, "o");
    }
    let (mut etat, o_etat) = poids(|| Etat::depuis(&m));
    resultat(sec, echelle, "memoire_etat_propose", o_etat as f64, "o");
    let o_annuaire = {
        let (a, o) = poids(|| {
            let mut a = Annuaire::default();
            for p in &m.points {
                a.attribuer(&p.nom);
            }
            a.serrer();
            a
        });
        black_box(a.len());
        o
    };
    resultat(sec, echelle, "memoire_annuaire_points", o_annuaire as f64, "o");
    let colonnes = nb * (size_of::<Def>() + size_of::<Vif>() + 8)
        + m.appareils.len() * (size_of::<Sante>() + 8);
    resultat(sec, echelle, "memoire_colonnes_seules", colonnes as f64, "o");

    // Lectures dans un ordre tiré au hasard (une condition lit un point quelconque).
    let mut a = Alea::new(7);
    const N: usize = 4_096;
    let ordre: Vec<u32> = (0..N).map(|_| a.sous(nb) as u32).collect();
    let noms: Vec<&str> = ordre.iter().map(|&n| m.points[n as usize].nom.as_str()).collect();
    let verrou = RwLock::new(states);
    if aujourdhui {
        let t = ns(N, 31, || {
            let mut x = 0u64;
            for nom in &noms {
                x += u64::from(hub_state(&verrou, nom).is_some());
            }
            x
        });
        resultat(sec, echelle, "lire_hub_state_aujourdhui", t, "ns");
        let t = ns(N, 31, || {
            let mut x = 0u64;
            for nom in &noms {
                x += u64::from(miroir.get(*nom).is_some());
            }
            x
        });
        resultat(sec, echelle, "lire_miroir_moteur_aujourdhui", t, "ns");
    }
    drop(verrou);
    drop(miroir);
    let t = ns(N, 31, || {
        let mut x = 0u64;
        for nom in &noms {
            x += u64::from(etat.annuaire.numero(nom).unwrap_or(0));
        }
        x
    });
    resultat(sec, echelle, "annuaire_nom_vers_numero", t, "ns");
    let t = ns(N, 101, || {
        let mut x = 0i64;
        for &n in &ordre {
            x = x.wrapping_add(etat.vifs[n as usize].valeur);
        }
        x as u64
    });
    resultat(sec, echelle, "lire_par_numero_vif40", t, "ns");
    let vif32: Vec<Vif32> = etat.vifs.iter().map(|v| Vif32 { valeur: v.valeur, ..Vif32::default() }).collect();
    let t = ns(N, 101, || {
        let mut x = 0i64;
        for &n in &ordre {
            x = x.wrapping_add(vif32[n as usize].valeur);
        }
        x as u64
    });
    resultat(sec, echelle, "lire_par_numero_vif32", t, "ns");
    let chaud: Vec<Chaud> = etat.vifs.iter().map(|v| Chaud { valeur: v.valeur, ..Chaud::default() }).collect();
    let t = ns(N, 101, || {
        let mut x = 0i64;
        for &n in &ordre {
            x = x.wrapping_add(chaud[n as usize].valeur);
        }
        x as u64
    });
    resultat(sec, echelle, "lire_par_numero_chaud16", t, "ns");
    let t = ns(N, 101, || {
        let mut x = 0i64;
        for &n in &ordre {
            let (v, q) = etat.lire(n);
            x = x.wrapping_add(v + i64::from(q));
        }
        x as u64
    });
    resultat(sec, echelle, "lire_par_numero_qualite_effective", t, "ns");
    // Lire derrière un verrou (un lecteur d'un autre fil, sans rival).
    let partage = RwLock::new(chaud.clone());
    let t = ns(N, 101, || {
        let mut x = 0i64;
        for &n in &ordre {
            x = x.wrapping_add(partage.read().unwrap()[n as usize].valeur);
        }
        x as u64
    });
    resultat(sec, echelle, "lire_par_numero_derriere_verrou", t, "ns");

    // Écrire une nouvelle (valeur, instants, santé, échéances), en place.
    let mut horloge = 1_792_675_624_123u64;
    let t = ns(N, 101, || {
        let mut x = 0u64;
        for &n in &ordre {
            horloge += 250;
            x += u64::from(etat.rapport(n, (horloge % 900) as i64, horloge, (horloge / 1_000) as u32));
        }
        x
    });
    resultat(sec, echelle, "ecrire_une_nouvelle", t, "ns");

    // Copier tout l'état vif (un instantané pour un écran, l'API ou le disque).
    let t = ns(1, 31, || {
        let c = etat.vifs.clone();
        let s = etat.sante.clone();
        (c.len() + s.len()) as u64
    });
    resultat(sec, echelle, "copier_vifs_et_sante", t / 1_000.0, "us");
    let t = ns(1, 31, || {
        let c = chaud.clone();
        c.len() as u64
    });
    resultat(sec, echelle, "copier_colonne_chaude", t / 1_000.0, "us");
    // L'instantané binaire : toutes les colonnes bout à bout, octet pour octet.
    let t = ns(1, 31, || {
        let mut sortie: Vec<u8> = Vec::with_capacity(colonnes);
        for v in &etat.vifs {
            sortie.extend_from_slice(&v.valeur.to_le_bytes());
            sortie.extend_from_slice(&v.precedente.to_le_bytes());
            sortie.extend_from_slice(&v.observe_ms.to_le_bytes());
            sortie.extend_from_slice(&v.change_ms.to_le_bytes());
            sortie.extend_from_slice(&v.recu_s.to_le_bytes());
            sortie.push(v.genres);
            sortie.push(v.qualite);
        }
        sortie.len() as u64
    });
    resultat(sec, echelle, "instantane_binaire_vifs", t / 1_000.0, "us");
    black_box(&etat.textes);
}
