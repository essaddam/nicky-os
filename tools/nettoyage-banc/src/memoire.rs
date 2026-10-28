//! La mémoire des valeurs de la v2, rangée par place, avec le registre générationnel ; les sorties
//! critiques que l'actif publie dans le rang `s` (`meta/`, `val/`, `meta/lot`,
//! `meta/demenagement`) et qu'une copie applique telles quelles ; l'état complet ; deux empreintes.

use std::collections::HashMap;

use crate::maison::{Classe, Nature};
use crate::numeros::{Fnv, Numero, Registre, get32, get64, put32, put64, statut, take};
use crate::outils::Temps;
use crate::valeurs::{Def, Sante, Vif, qualite, vie};

/// Un appareil et ses points, avec leurs numéros : `meta/`.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub appareil: Numero,
    pub nom: String,
    pub classe: Classe,
    pub points: Vec<(Numero, String, Nature)>,
}

/// Les places libérées par un lot : `meta/lot`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lot {
    pub points: Vec<u32>,
    pub appareils: Vec<u32>,
}

impl Lot {
    pub fn est_vide(&self) -> bool {
        self.points.is_empty() && self.appareils.is_empty()
    }
    /// Le message : un en-tête, puis les places en plages (début, longueur), 8 o chacune.
    pub fn octets(&self) -> usize {
        16 + 8 * (plages(&self.points) + plages(&self.appareils))
    }
}

fn plages(places: &[u32]) -> usize {
    let mut v = places.to_vec();
    v.sort_unstable();
    v.windows(2).filter(|w| w[1] != w[0] + 1).count() + usize::from(!v.is_empty())
}

/// Les mouvements d'un déménagement (ancien numéro → nouveau) : `meta/demenagement`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Demenagement {
    pub points: Vec<(Numero, Numero)>,
    pub appareils: Vec<(Numero, Numero)>,
}

impl Demenagement {
    /// Le message : un en-tête, puis 8 o par mouvement (en morceaux de 4 Kio au plus, V7 § 6.4).
    pub fn octets(&self) -> usize {
        16 + 8 * (self.points.len() + self.appareils.len())
    }
    /// Ancien numéro de point → nouveau.
    pub fn table_points(&self) -> HashMap<u32, Numero> {
        self.points.iter().map(|&(a, b)| (a.0, b)).collect()
    }
}

/// Une sortie critique de l'actif, dans l'ordre du rang `s`.
#[derive(Clone, Debug, PartialEq)]
pub enum Sortie {
    Declaration(Declaration),
    Valeur { n: Numero, v: i64, t: Temps },
    /// Un appareil absent, revenu ou retiré, ses points avec lui.
    Statut { a: Numero, statut: u8, t_ms: u64 },
    Lot(Lot),
    Demenagement(Demenagement),
}

/// La mémoire d'une machine : registres, colonnes par place (fiches de la v2), et pour chaque
/// appareil la plage de ses points.
#[derive(Clone, Default)]
pub struct Memoire {
    pub points: Registre,
    pub appareils: Registre,
    pub defs: Vec<Def>,
    pub vifs: Vec<Vif>,
    pub perime: Vec<u32>,
    pub fraicheur: Vec<u32>,
    pub sante: Vec<Sante>,
    pub echeance: Vec<u32>,
    pub delai: Vec<u32>,
    pub plage: Vec<(u32, u32)>,
}

const JAMAIS: u32 = u32::MAX;

impl Memoire {
    /// L'actif choisit les numéros d'un appareil neuf : une place d'appareil, et des places de points
    /// qui se suivent (la première plage libre assez longue). Rien n'est encore appliqué.
    pub fn preparer(&self, nom: &str, cles: &[(&str, Nature)], classe: Classe) -> Declaration {
        let a = self.appareils.a_donner(self.appareils.choisir(1));
        let d = self.points.choisir(cles.len() as u32);
        let points = cles
            .iter()
            .enumerate()
            .map(|(k, &(cle, nature))| (self.points.a_donner(d + k as u32), format!("{nom}/{cle}"), nature))
            .collect();
        Declaration { appareil: a, nom: nom.to_owned(), classe, points }
    }

    /// Applique une sortie critique. L'actif passe par ici comme ses copies : même code, même ordre.
    pub fn appliquer(&mut self, so: &Sortie) -> Result<(), &'static str> {
        match so {
            Sortie::Declaration(d) => self.declarer(d),
            Sortie::Valeur { n, v, t } => {
                self.rapport(*n, *v, *t);
                Ok(())
            }
            Sortie::Statut { a, statut, t_ms } => {
                self.changer_statut(*a, *statut, *t_ms);
                Ok(())
            }
            Sortie::Lot(l) => {
                self.appliquer_lot(l);
                Ok(())
            }
            Sortie::Demenagement(dm) => self.appliquer_demenagement(dm),
        }
    }

    fn declarer(&mut self, d: &Declaration) -> Result<(), &'static str> {
        self.appareils.affecter(d.appareil, &d.nom)?;
        let a = d.appareil.place();
        if self.sante.len() <= a {
            self.sante.resize(a + 1, Sante::default());
            self.echeance.resize(a + 1, JAMAIS);
            self.delai.resize(a + 1, 0);
            self.plage.resize(a + 1, (0, 0));
        }
        let r = d.classe.rythme_s();
        let delai = r.saturating_mul(u32::from(d.classe.pertes()) + 1);
        self.sante[a] = Sante { rythme_s: r, pertes: d.classe.pertes(), ..Sante::default() };
        self.delai[a] = delai;
        self.echeance[a] = JAMAIS;
        let premier = d.points.first().map_or(0, |p| p.0.place() as u32);
        self.plage[a] = (premier, d.points.len() as u32);
        for (k, (n, nom, nature)) in d.points.iter().enumerate() {
            if n.place() != premier as usize + k {
                return Err("les points d'un appareil se suivent");
            }
            self.points.affecter(*n, nom)?;
            let p = n.place();
            if self.vifs.len() <= p {
                self.defs.resize(p + 1, Def::default());
                self.vifs.resize(p + 1, Vif::default());
                self.perime.resize(p + 1, JAMAIS);
                self.fraicheur.resize(p + 1, 0);
            }
            self.defs[p] = Def { appareil: d.appareil.0, nature: *nature as u8, ..Def::default() };
            self.vifs[p] = Vif { qualite: qualite::INCERTAINE, ..Vif::default() };
            self.fraicheur[p] = if *nature == Nature::Mesure { 600 } else { 0 };
            self.perime[p] = JAMAIS;
        }
        Ok(())
    }

    /// Une nouvelle d'un point en service : en place, comme `Etat::rapport` de la v2. Une nouvelle pour
    /// un numéro inerte (retiré, libéré, d'une autre génération) est ignorée.
    pub fn rapport(&mut self, n: Numero, valeur: i64, t: Temps) -> bool {
        if !self.points.vivant(n) {
            return false;
        }
        let p = n.place();
        let v = &mut self.vifs[p];
        let change = v.valeur != valeur;
        if change {
            v.precedente = v.valeur;
            v.valeur = valeur;
            v.change_ms = t.mur_ms;
        }
        v.observe_ms = t.mur_ms;
        v.recu_s = t.mur_s();
        v.qualite = qualite::BONNE;
        let a = Numero(self.defs[p].appareil).place();
        self.sante[a].contact_s = t.mur_s();
        self.sante[a].vie = vie::VIVANT;
        if self.delai[a] != 0 {
            self.echeance[a] = t.mono.saturating_add(self.delai[a]);
        }
        if self.fraicheur[p] != 0 {
            self.perime[p] = t.mono.saturating_add(self.fraicheur[p]);
        }
        change
    }

    /// Ce qu'une condition lit : `None` pour un numéro inerte ; une valeur mauvaise si l'appareil est
    /// absent ou muet (« rien ne part sur une valeur qu'on ne connaît plus »).
    pub fn lire(&self, n: Numero) -> Option<(i64, u8)> {
        if !self.points.vivant(n) {
            return None;
        }
        let p = n.place();
        let a = Numero(self.defs[p].appareil);
        let mauvaise = self.points.statut[p] == statut::ABSENT
            || self.appareils.statut[a.place()] == statut::ABSENT
            || self.sante[a.place()].vie == vie::MUET;
        Some((self.vifs[p].valeur, if mauvaise { qualite::MAUVAISE } else { self.vifs[p].qualite }))
    }

    /// Les points d'un appareil, à leur numéro actuel.
    pub fn points_de(&self, a: Numero) -> Vec<Numero> {
        let (d, k) = self.plage[a.place()];
        (d..d.saturating_add(k))
            .filter(|&p| (p as usize) < self.defs.len() && self.defs[p as usize].appareil == a.0)
            .map(|p| self.points.numero_de(p as usize))
            .filter(|&n| self.points.nom(n).is_some())
            .collect()
    }

    fn changer_statut(&mut self, a: Numero, s: u8, t_ms: u64) {
        if self.appareils.nom(a).is_none() {
            return;
        }
        let points = self.points_de(a);
        match s {
            statut::ABSENT => {
                self.appareils.absent(a);
                points.iter().for_each(|&n| self.points.absent(n));
                self.echeance[a.place()] = JAMAIS;
            }
            statut::EN_SERVICE => {
                self.appareils.revenu(a);
                points.iter().for_each(|&n| self.points.revenu(n));
            }
            statut::RETIRE => {
                self.appareils.retirer(a, t_ms);
                for &n in &points {
                    self.points.retirer(n, t_ms);
                    self.perime[n.place()] = JAMAIS;
                }
                self.echeance[a.place()] = JAMAIS;
            }
            _ => {}
        }
    }

    /// L'actif choisit le lot : les places retirées depuis au moins `grace_ms`. Un appareil ne part
    /// qu'avec tous ses points.
    pub fn choisir_lot(&self, maintenant_ms: u64, grace_ms: u64) -> Lot {
        let points = self.points.liberables(maintenant_ms, grace_ms);
        let mut part = vec![false; self.points.len()];
        for &p in &points {
            part[p as usize] = true;
        }
        let appareils = self
            .appareils
            .liberables(maintenant_ms, grace_ms)
            .into_iter()
            .filter(|&a| {
                let (d, k) = self.plage[a as usize];
                (d..d + k).all(|p| part[p as usize] || self.points.statut[p as usize] != statut::RETIRE)
            })
            .collect();
        Lot { points, appareils }
    }

    fn appliquer_lot(&mut self, lot: &Lot) {
        for &p in &lot.points {
            let i = p as usize;
            if self.points.statut[i] == statut::RETIRE {
                self.defs[i] = Def::default();
                self.vifs[i] = Vif::default();
                self.perime[i] = JAMAIS;
                self.fraicheur[i] = 0;
            }
        }
        self.points.liberer(&lot.points);
        for &a in &lot.appareils {
            let i = a as usize;
            if self.appareils.statut[i] == statut::RETIRE {
                self.sante[i] = Sante::default();
                self.echeance[i] = JAMAIS;
                self.delai[i] = 0;
                self.plage[i] = (0, 0);
            }
        }
        self.appareils.liberer(&lot.appareils);
        self.rogner();
    }

    /// Le déménagement, choisi par l'actif : appareils et blocs de points descendent, dans l'ordre,
    /// vers les places libres les plus basses. Garde l'ordre, donc les points d'un appareil ensemble.
    pub fn demenager(&self) -> Demenagement {
        let mut dm = Demenagement::default();
        // Appareils : une place chacun.
        let (mut gens, mut st) = (self.appareils.generation.clone(), self.appareils.statut.clone());
        let mut ecrire = 0usize;
        for p in 0..st.len() {
            if st[p] > statut::RETIRE {
                continue;
            }
            while st[ecrire] == statut::CONDAMNE {
                ecrire += 1;
            }
            if ecrire < p {
                dm.appareils.push((Numero::new(p as u32, gens[p]), Numero::new(ecrire as u32, gens[ecrire])));
                st[ecrire] = st[p];
                vider(&mut gens, &mut st, p);
            }
            ecrire += 1;
        }
        // Points : par appareil, d'un bloc, dans l'ordre de leur première place.
        let (mut gens, mut st) = (self.points.generation.clone(), self.points.statut.clone());
        let mut blocs: Vec<(u32, u32)> = (0..self.appareils.len())
            .filter(|&a| self.appareils.statut[a] <= statut::RETIRE)
            .map(|a| self.plage[a])
            .collect();
        blocs.sort_unstable();
        let mut ecrire = 0u32;
        for (d, k) in blocs {
            let mut t = ecrire;
            while let Some(c) = (t..t + k).find(|&q| (q as usize) < st.len() && st[q as usize] == statut::CONDAMNE) {
                t = c + 1;
            }
            if t < d {
                for i in 0..k {
                    let (a, b) = ((d + i) as usize, (t + i) as usize);
                    if st[a] <= statut::RETIRE {
                        dm.points.push((Numero::new(a as u32, gens[a]), Numero::new(b as u32, gens[b])));
                        st[b] = st[a];
                        vider(&mut gens, &mut st, a);
                    }
                }
                ecrire = t + k;
            } else {
                ecrire = d + k;
            }
        }
        dm
    }

    fn appliquer_demenagement(&mut self, dm: &Demenagement) -> Result<(), &'static str> {
        self.appareils.deplacer(&dm.appareils)?;
        let mut app: HashMap<u32, u32> = HashMap::with_capacity(dm.appareils.len());
        for &(a, b) in &dm.appareils {
            let (i, j) = (a.place(), b.place());
            self.sante[j] = self.sante[i];
            self.echeance[j] = self.echeance[i];
            self.delai[j] = self.delai[i];
            self.plage[j] = self.plage[i];
            self.sante[i] = Sante::default();
            self.echeance[i] = JAMAIS;
            self.delai[i] = 0;
            self.plage[i] = (0, 0);
            app.insert(a.0, b.0);
        }
        self.points.deplacer(&dm.points)?;
        for &(a, b) in &dm.points {
            let (i, j) = (a.place(), b.place());
            self.defs[j] = self.defs[i];
            self.vifs[j] = self.vifs[i];
            self.perime[j] = self.perime[i];
            self.fraicheur[j] = self.fraicheur[i];
            self.defs[i] = Def::default();
            self.vifs[i] = Vif::default();
            self.perime[i] = JAMAIS;
            self.fraicheur[i] = 0;
        }
        // Chaque point suit son appareil ; chaque appareil retrouve la plage de ses points.
        for a in 0..self.plage.len() {
            if self.appareils.statut[a] <= statut::RETIRE {
                self.plage[a] = (u32::MAX, 0);
            }
        }
        for p in 0..self.points.len().min(self.defs.len()) {
            if self.points.statut[p] > statut::RETIRE {
                continue;
            }
            let def = &mut self.defs[p];
            if let Some(&b) = app.get(&def.appareil) {
                def.appareil = b;
            }
            let pl = &mut self.plage[Numero(def.appareil).place()];
            if pl.0 == u32::MAX {
                *pl = (p as u32, 1);
            } else {
                pl.1 = p as u32 + 1 - pl.0;
            }
        }
        self.rogner();
        Ok(())
    }

    /// Les colonnes s'arrêtent au bout utile, et rendent leur mémoire quand il en reste beaucoup.
    fn rogner(&mut self) {
        let fp = self.points.fin_utile().min(self.vifs.len());
        self.defs.truncate(fp);
        self.vifs.truncate(fp);
        self.perime.truncate(fp);
        self.fraicheur.truncate(fp);
        if self.vifs.capacity() > fp + fp / 4 + 64 {
            self.defs.shrink_to_fit();
            self.vifs.shrink_to_fit();
            self.perime.shrink_to_fit();
            self.fraicheur.shrink_to_fit();
        }
        let fa = self.appareils.fin_utile().min(self.sante.len());
        self.sante.truncate(fa);
        self.echeance.truncate(fa);
        self.delai.truncate(fa);
        self.plage.truncate(fa);
        if self.sante.capacity() > fa + fa / 4 + 64 {
            self.sante.shrink_to_fit();
            self.echeance.shrink_to_fit();
            self.delai.shrink_to_fit();
            self.plage.shrink_to_fit();
        }
    }

    /// Octets des colonnes et des registres (capacités).
    pub fn octets(&self) -> usize {
        use std::mem::size_of;
        self.defs.capacity() * size_of::<Def>()
            + self.vifs.capacity() * size_of::<Vif>()
            + 4 * (self.perime.capacity() + self.fraicheur.capacity())
            + self.sante.capacity() * size_of::<Sante>()
            + 4 * (self.echeance.capacity() + self.delai.capacity())
            + 8 * self.plage.capacity()
            + self.points.octets()
            + self.appareils.octets()
    }

    /// L'état complet (et l'instantané, même format) : les deux annuaires, puis les fiches des seules
    /// places qui portent un nom. Une place libre n'y coûte que ses 4 octets d'annuaire : l'état complet
    /// suit ce qui vit, dès le lot, sans attendre un déménagement.
    pub fn ranger(&self) -> Vec<u8> {
        let mut b = Vec::new();
        self.points.ranger(&mut b);
        self.appareils.ranger(&mut b);
        let nommes = |r: &Registre, n: usize| (0..n).filter(|&p| r.statut[p] <= statut::RETIRE).collect::<Vec<_>>();
        let pts = nommes(&self.points, self.vifs.len());
        put32(&mut b, self.vifs.len() as u32);
        put32(&mut b, pts.len() as u32);
        for p in pts {
            let (d, v) = (&self.defs[p], &self.vifs[p]);
            put32(&mut b, p as u32);
            put32(&mut b, d.appareil);
            put32(&mut b, d.libelle);
            b.extend_from_slice(&d.cle.to_le_bytes());
            b.extend_from_slice(&d.dico.to_le_bytes());
            b.extend_from_slice(&[d.nature, d.unite, d.echelle as u8, d.sens]);
            put64(&mut b, v.valeur as u64);
            put64(&mut b, v.precedente as u64);
            put64(&mut b, v.observe_ms);
            put64(&mut b, v.change_ms);
            put32(&mut b, v.recu_s);
            b.extend_from_slice(&[v.genres, v.qualite]);
            put32(&mut b, self.perime[p]);
            put32(&mut b, self.fraicheur[p]);
        }
        let apps = nommes(&self.appareils, self.sante.len());
        put32(&mut b, self.sante.len() as u32);
        put32(&mut b, apps.len() as u32);
        for a in apps {
            let s = &self.sante[a];
            put32(&mut b, a as u32);
            for x in [s.contact_s, s.change_s, s.rythme_s, s.appris_s, self.echeance[a], self.delai[a], self.plage[a].0, self.plage[a].1] {
                put32(&mut b, x);
            }
            b.extend_from_slice(&[s.pertes, s.vie]);
        }
        b
    }

    pub fn relire(octets: &[u8]) -> Option<Memoire> {
        let b = &mut &octets[..];
        let mut m = Memoire { points: Registre::relire(b)?, appareils: Registre::relire(b)?, ..Memoire::default() };
        let n = get32(b)? as usize;
        m.defs = vec![Def::default(); n];
        m.vifs = vec![Vif::default(); n];
        m.perime = vec![JAMAIS; n];
        m.fraicheur = vec![0; n];
        for _ in 0..get32(b)? {
            let p = get32(b)? as usize;
            if p >= n {
                return None;
            }
            let appareil = get32(b)?;
            let libelle = get32(b)?;
            let x = take(b, 8)?;
            m.defs[p] = Def {
                appareil,
                libelle,
                cle: u16::from_le_bytes([x[0], x[1]]),
                dico: u16::from_le_bytes([x[2], x[3]]),
                nature: x[4],
                unite: x[5],
                echelle: x[6] as i8,
                sens: x[7],
            };
            let (valeur, precedente, observe_ms, change_ms) = (get64(b)? as i64, get64(b)? as i64, get64(b)?, get64(b)?);
            let recu_s = get32(b)?;
            let g = take(b, 2)?;
            m.vifs[p] = Vif { valeur, precedente, observe_ms, change_ms, recu_s, genres: g[0], qualite: g[1] };
            m.perime[p] = get32(b)?;
            m.fraicheur[p] = get32(b)?;
        }
        let n = get32(b)? as usize;
        m.sante = vec![Sante::default(); n];
        m.echeance = vec![JAMAIS; n];
        m.delai = vec![0; n];
        m.plage = vec![(0, 0); n];
        for _ in 0..get32(b)? {
            let a = get32(b)? as usize;
            if a >= n {
                return None;
            }
            let (contact_s, change_s, rythme_s, appris_s) = (get32(b)?, get32(b)?, get32(b)?, get32(b)?);
            let (echeance, delai, d, k) = (get32(b)?, get32(b)?, get32(b)?, get32(b)?);
            let x = take(b, 2)?;
            m.sante[a] = Sante { contact_s, change_s, rythme_s, appris_s, pertes: x[0], vie: x[1] };
            m.echeance[a] = echeance;
            m.delai[a] = delai;
            m.plage[a] = (d, k);
        }
        b.is_empty().then_some(m)
    }

    /// L'empreinte exacte : registres et fiches, place par place. Deux copies conformes l'ont égale.
    pub fn empreinte(&self) -> u64 {
        let mut h = Fnv::default();
        self.points.empreinte(&mut h);
        self.appareils.empreinte(&mut h);
        h.octets(&self.ranger());
        h.0
    }

    /// L'empreinte par les noms : ce que la maison est, sans les numéros. Un déménagement ne la change
    /// pas ; un numéro mal suivi, si.
    pub fn empreinte_noms(&self) -> u64 {
        let mut h = Fnv::default();
        let mut pts: Vec<(&str, usize)> =
            (0..self.points.len()).filter_map(|p| self.points.nom(self.points.numero_de(p)).map(|nom| (nom, p))).collect();
        pts.sort_unstable();
        for (nom, p) in pts {
            let (d, v) = (&self.defs[p], &self.vifs[p]);
            h.octets(nom.as_bytes());
            h.octets(&[self.points.statut[p], d.nature, v.genres, v.qualite]);
            for x in [v.valeur as u64, v.precedente as u64, v.observe_ms, v.change_ms] {
                h.u64(x);
            }
            h.u64(u64::from(v.recu_s) << 32 | u64::from(self.perime[p]));
            h.octets(self.appareils.nom(Numero(d.appareil)).unwrap_or("?").as_bytes());
        }
        let mut apps: Vec<(&str, usize)> = (0..self.appareils.len())
            .filter_map(|a| self.appareils.nom(self.appareils.numero_de(a)).map(|nom| (nom, a)))
            .collect();
        apps.sort_unstable();
        for (nom, a) in apps {
            let s = &self.sante[a];
            h.octets(nom.as_bytes());
            h.octets(&[self.appareils.statut[a], s.pertes, s.vie]);
            h.u64(u64::from(s.contact_s) << 32 | u64::from(self.echeance[a]));
        }
        h.0
    }
}

/// Une place quittée par un déménagement : libérée comme par un lot (génération suivante).
fn vider(gens: &mut [u8], st: &mut [u8], p: usize) {
    if gens[p] == crate::numeros::GEN_MAX {
        st[p] = statut::CONDAMNE;
    } else {
        gens[p] += 1;
        st[p] = statut::LIBRE;
    }
}
