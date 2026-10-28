//! Le numéro d'un point ou d'un appareil, et le registre qui le donne.
//!
//! Un numéro est une **place** (bits 0 à 23 : la fiche dans les colonnes, comme dans la mémoire v2)
//! plus la **génération** de cette place (bits 24 à 30). Le bit de poids fort reste réservé aux
//! marques de cause (mémoire v2 § 8.4). Une place libérée par un lot sert de nouveau, à la
//! génération suivante ; le numéro entier, lui, n'est jamais redonné. C'est la forme des
//! identifiants d'entité des moteurs de jeu (EnTT : 20 bits d'entité et 12 de version dans un
//! `u32` ; Bevy : un index et une génération).
//!
//! Le cycle d'une place : en service → absent (l'intégration ne la donne plus, tout est gardé) →
//! retiré (un humain l'a décidé ; plus lue ni commandée) → libre (un lot : nom effacé, génération
//! suivante) → de nouveau en service, sous un autre nom. Une place à sa dernière génération est
//! condamnée par le lot qui la libère.

use std::hash::BuildHasher;

/// Place (bits 0 à 23) et génération (bits 24 à 30) ; le bit 31 reste aux marques de cause.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Numero(pub u32);

pub const BITS_PLACE: u32 = 24;
pub const MASQUE_PLACE: u32 = (1 << BITS_PLACE) - 1;
/// La dernière génération : une place qui la porte est condamnée par le lot qui la libère.
pub const GEN_MAX: u8 = 0x7F;

impl Numero {
    pub fn new(place: u32, generation: u8) -> Self {
        debug_assert!(place <= MASQUE_PLACE && generation <= GEN_MAX);
        Self(place | (u32::from(generation) << BITS_PLACE))
    }
    pub fn place(self) -> usize {
        (self.0 & MASQUE_PLACE) as usize
    }
    pub fn generation(self) -> u8 {
        ((self.0 >> BITS_PLACE) & u32::from(GEN_MAX)) as u8
    }
}

pub mod statut {
    /// Déclaré, suivi, lu et commandé.
    pub const EN_SERVICE: u8 = 0;
    /// L'intégration ne le donne plus (catalogue relu, pont redémarré) : tout est gardé, il peut revenir.
    pub const ABSENT: u8 = 1;
    /// Retiré par un humain : plus lu, plus commandé ; son nom reste jusqu'au lot.
    pub const RETIRE: u8 = 2;
    /// Libérée par un lot : sans nom, prête à servir, à la génération suivante.
    pub const LIBRE: u8 = 3;
    /// Génération épuisée : la place ne sert plus jamais.
    pub const CONDAMNE: u8 = 4;
}
use statut::{ABSENT, CONDAMNE, EN_SERVICE, LIBRE, RETIRE};

/// Une place retirée qui attend un lot, et depuis quand (heure murale, ms).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retrait {
    pub place: u32,
    pub depuis_ms: u64,
}

/// Le registre : qui occupe chaque place, à quelle génération, et les places libres. Les noms sont
/// rangés bout à bout dans un tiroir ; la table de hachage ne garde que des places.
#[derive(Clone)]
pub struct Registre {
    pub generation: Vec<u8>,
    pub statut: Vec<u8>,
    debut: Vec<u32>,
    long: Vec<u16>,
    texte: String,
    /// Octets du tiroir qui n'appartiennent plus à personne (noms effacés ou déplacés).
    mort: usize,
    table: hashbrown::HashTable<u32>,
    hacheur: std::hash::RandomState,
    /// Les places libres, en plages (début, longueur) triées, jamais voisines.
    pub libres: Vec<(u32, u32)>,
    pub retraits: Vec<Retrait>,
}

impl Default for Registre {
    fn default() -> Self {
        Self {
            generation: Vec::new(),
            statut: Vec::new(),
            debut: Vec::new(),
            long: Vec::new(),
            texte: String::new(),
            mort: 0,
            table: hashbrown::HashTable::new(),
            hacheur: std::hash::RandomState::new(),
            libres: Vec::new(),
            retraits: Vec::new(),
        }
    }
}

impl Registre {
    /// Nombre de places, libres comprises.
    pub fn len(&self) -> usize {
        self.generation.len()
    }

    fn nom_place(&self, p: usize) -> &str {
        if self.long[p] == 0 {
            return "";
        }
        let d = self.debut[p] as usize;
        &self.texte[d..d + self.long[p] as usize]
    }

    /// Le numéro actuel d'une place.
    pub fn numero_de(&self, p: usize) -> Numero {
        Numero::new(p as u32, self.generation[p])
    }

    /// Le numéro d'un nom connu (en service, absent ou retiré).
    pub fn numero(&self, nom: &str) -> Option<Numero> {
        let h = self.hacheur.hash_one(nom);
        self.table.find(h, |&p| self.nom_place(p as usize) == nom).map(|&p| self.numero_de(p as usize))
    }

    /// Le nom d'un numéro tant qu'il désigne encore sa place ; `None` dès qu'un lot l'a libérée.
    pub fn nom(&self, n: Numero) -> Option<&str> {
        let p = n.place();
        (p < self.len() && self.generation[p] == n.generation() && self.statut[p] <= RETIRE).then(|| self.nom_place(p))
    }

    /// Lu et commandé : en service ou absent, à sa génération. Toute autre référence est inerte.
    pub fn vivant(&self, n: Numero) -> bool {
        let p = n.place();
        p < self.len() && self.generation[p] == n.generation() && self.statut[p] <= ABSENT
    }

    /// Places qui portent un nom (en service, absentes, retirées).
    pub fn nommes(&self) -> usize {
        self.statut.iter().filter(|&&s| s <= RETIRE).count()
    }

    /// Où ranger `k` places qui se suivent : la première plage libre assez longue ; sinon la plage du
    /// bout, prolongée ; sinon au bout.
    pub fn choisir(&self, k: u32) -> u32 {
        if let Some(&(d, _)) = self.libres.iter().find(|&&(_, l)| l >= k) {
            return d;
        }
        match self.libres.last() {
            Some(&(d, l)) if (d + l) as usize == self.len() => d,
            _ => self.len() as u32,
        }
    }

    /// Le numéro que portera la place `p` si on la donne maintenant.
    pub fn a_donner(&self, p: u32) -> Numero {
        Numero::new(p, self.generation.get(p as usize).copied().unwrap_or(0))
    }

    /// Donne la place `n` à `nom`. L'actif l'appelle après `choisir` ; une copie, en appliquant `meta/`.
    /// Une place qui n'est pas libre à cette génération est refusée : deux mémoires ont divergé.
    pub fn affecter(&mut self, n: Numero, nom: &str) -> Result<(), &'static str> {
        let p = n.place();
        if p > self.len() {
            return Err("place au-delà du bout");
        }
        if nom.is_empty() || nom.len() > usize::from(u16::MAX) {
            return Err("nom vide ou trop long");
        }
        if self.numero(nom).is_some() {
            return Err("nom déjà connu");
        }
        if p == self.len() {
            if n.generation() != 0 {
                return Err("place neuve à une génération non nulle");
            }
            self.generation.push(0);
            self.statut.push(LIBRE);
            self.debut.push(0);
            self.long.push(0);
        } else {
            if self.statut[p] != LIBRE || self.generation[p] != n.generation() {
                return Err("place occupée, ou d'une autre génération");
            }
            self.oter_libre(p as u32);
        }
        self.debut[p] = self.texte.len() as u32;
        self.long[p] = nom.len() as u16;
        self.texte.push_str(nom);
        self.statut[p] = EN_SERVICE;
        let h = self.hacheur.hash_one(nom);
        let (texte, debut, long, hacheur) = (&self.texte, &self.debut, &self.long, &self.hacheur);
        self.table.insert_unique(h, p as u32, |&q| {
            let d = debut[q as usize] as usize;
            hacheur.hash_one(&texte[d..d + long[q as usize] as usize])
        });
        Ok(())
    }

    /// L'intégration ne le donne plus : tout est gardé.
    pub fn absent(&mut self, n: Numero) {
        if self.vivant(n) {
            self.statut[n.place()] = ABSENT;
        }
    }

    /// Il revient (absent, ou retiré mais pas encore libéré) : même place, même numéro.
    pub fn revenu(&mut self, n: Numero) {
        let p = n.place();
        if self.nom(n).is_some() && self.statut[p] != EN_SERVICE {
            if self.statut[p] == RETIRE {
                self.retraits.retain(|r| r.place as usize != p);
            }
            self.statut[p] = EN_SERVICE;
        }
    }

    /// Un humain le retire : plus lu, plus commandé ; la place attend un lot.
    pub fn retirer(&mut self, n: Numero, depuis_ms: u64) {
        if self.vivant(n) {
            self.statut[n.place()] = RETIRE;
            self.retraits.push(Retrait { place: n.place() as u32, depuis_ms });
        }
    }

    /// Les places qu'un lot peut libérer : retirées depuis au moins `grace_ms`.
    pub fn liberables(&self, maintenant_ms: u64, grace_ms: u64) -> Vec<u32> {
        self.retraits
            .iter()
            .filter(|r| r.depuis_ms.saturating_add(grace_ms) <= maintenant_ms)
            .map(|r| r.place)
            .collect()
    }

    /// Le lot : libère ces places d'un coup. Chacune perd son nom et passe à la génération suivante ;
    /// une place à sa dernière génération est condamnée. Rejoué tel quel par chaque copie.
    pub fn liberer(&mut self, places: &[u32]) {
        let mut neuves = Vec::with_capacity(places.len());
        for &p in places {
            let i = p as usize;
            if self.statut[i] != RETIRE {
                continue;
            }
            let h = self.hacheur.hash_one(self.nom_place(i));
            if let Ok(e) = self.table.find_entry(h, |&q| q == p) {
                e.remove();
            }
            self.mort += usize::from(self.long[i]);
            self.long[i] = 0;
            if self.generation[i] == GEN_MAX {
                self.statut[i] = CONDAMNE;
            } else {
                self.generation[i] += 1;
                self.statut[i] = LIBRE;
                neuves.push(p);
            }
        }
        let st = &self.statut;
        self.retraits.retain(|r| st[r.place as usize] == RETIRE);
        neuves.sort_unstable();
        self.fondre(&neuves);
        self.tasser_noms_si_besoin();
    }

    /// Le déménagement : chaque nom passe d'une place à une place libre plus basse (à sa génération) ;
    /// la place quittée est libérée comme par un lot. Rejoué tel quel par chaque copie.
    pub fn deplacer(&mut self, mouvements: &[(Numero, Numero)]) -> Result<(), &'static str> {
        for &(de, vers) in mouvements {
            let (a, b) = (de.place(), vers.place());
            if self.nom(de).is_none() {
                return Err("déménagement d'une place sans nom");
            }
            if b >= self.len() || self.statut[b] != LIBRE || self.generation[b] != vers.generation() {
                return Err("déménagement vers une place qui n'est pas libre");
            }
            let h = self.hacheur.hash_one(self.nom_place(a));
            if let Some(q) = self.table.find_mut(h, |&q| q as usize == a) {
                *q = b as u32;
            }
            self.debut[b] = self.debut[a];
            self.long[b] = self.long[a];
            self.statut[b] = self.statut[a];
            if self.statut[a] == RETIRE {
                if let Some(r) = self.retraits.iter_mut().find(|r| r.place as usize == a) {
                    r.place = b as u32;
                }
            }
            self.long[a] = 0;
            if self.generation[a] == GEN_MAX {
                self.statut[a] = CONDAMNE;
            } else {
                self.generation[a] += 1;
                self.statut[a] = LIBRE;
            }
        }
        self.libres.clear();
        for p in 0..self.len() {
            if self.statut[p] == LIBRE {
                match self.libres.last_mut() {
                    Some((d, l)) if (*d + *l) as usize == p => *l += 1,
                    _ => self.libres.push((p as u32, 1)),
                }
            }
        }
        Ok(())
    }

    /// Le bout utile : au-delà, toutes les places sont libres ; les colonnes s'y arrêtent.
    pub fn fin_utile(&self) -> usize {
        match self.libres.last() {
            Some(&(d, l)) if (d + l) as usize == self.len() => d as usize,
            _ => self.len(),
        }
    }

    /// Fusionne des places libres (triées) dans les plages, en recollant les voisines.
    fn fondre(&mut self, places: &[u32]) {
        let mut plages: Vec<(u32, u32)> = Vec::new();
        for &p in places {
            match plages.last_mut() {
                Some((d, l)) if *d + *l == p => *l += 1,
                _ => plages.push((p, 1)),
            }
        }
        let mut tout: Vec<(u32, u32)> = Vec::with_capacity(self.libres.len() + plages.len());
        let (mut a, mut b) = (self.libres.iter().copied().peekable(), plages.into_iter().peekable());
        loop {
            let x = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) => {
                    if x.0 < y.0 {
                        a.next()
                    } else {
                        b.next()
                    }
                }
                (Some(_), None) => a.next(),
                (None, Some(_)) => b.next(),
                (None, None) => break,
            };
            let x = x.expect("une plage");
            match tout.last_mut() {
                Some((d, l)) if *d + *l == x.0 => *l += x.1,
                _ => tout.push(x),
            }
        }
        self.libres = tout;
    }

    /// Retire la place `p` de sa plage libre.
    fn oter_libre(&mut self, p: u32) {
        let i = self.libres.partition_point(|&(d, _)| d <= p) - 1;
        let (d, l) = self.libres[i];
        debug_assert!(d <= p && p < d + l);
        let (avant, apres) = (p - d, d + l - p - 1);
        match (avant > 0, apres > 0) {
            (true, true) => {
                self.libres[i].1 = avant;
                self.libres.insert(i + 1, (p + 1, apres));
            }
            (true, false) => self.libres[i].1 = avant,
            (false, true) => self.libres[i] = (p + 1, apres),
            (false, false) => {
                self.libres.remove(i);
            }
        }
    }

    /// Les noms effacés laissent des trous dans le tiroir : passé la moitié, on le récrit.
    fn tasser_noms_si_besoin(&mut self) {
        if self.mort <= 4_096 || 2 * self.mort <= self.texte.len() {
            return;
        }
        let mut texte = String::with_capacity(self.texte.len() - self.mort);
        for p in 0..self.len() {
            if self.long[p] > 0 {
                let d = self.debut[p] as usize;
                let nom = &self.texte[d..d + self.long[p] as usize];
                self.debut[p] = texte.len() as u32;
                texte.push_str(nom);
            } else {
                self.debut[p] = 0;
            }
        }
        self.texte = texte;
        self.mort = 0;
    }

    /// Octets occupés (capacités), table comprise.
    pub fn octets(&self) -> usize {
        self.generation.capacity()
            + self.statut.capacity()
            + 4 * self.debut.capacity()
            + 2 * self.long.capacity()
            + self.texte.capacity()
            + 5 * self.table.capacity()
            + 8 * self.libres.capacity()
            + 16 * self.retraits.capacity()
    }

    /// Octets des noms encore gardés : ce qu'un instantané emporte.
    pub fn octets_noms(&self) -> usize {
        self.texte.len() - self.mort
    }

    /// La section « annuaire » de l'état complet et de l'instantané : places, générations, statuts,
    /// noms, plages libres, retraits.
    pub fn ranger(&self, b: &mut Vec<u8>) {
        put32(b, self.len() as u32);
        b.extend_from_slice(&self.generation);
        b.extend_from_slice(&self.statut);
        for p in 0..self.len() {
            let nom = self.nom_place(p).as_bytes();
            put16(b, nom.len() as u16);
            b.extend_from_slice(nom);
        }
        put32(b, self.libres.len() as u32);
        for &(d, l) in &self.libres {
            put32(b, d);
            put32(b, l);
        }
        put32(b, self.retraits.len() as u32);
        for r in &self.retraits {
            put32(b, r.place);
            put64(b, r.depuis_ms);
        }
    }

    pub fn relire(b: &mut &[u8]) -> Option<Registre> {
        let n = get32(b)? as usize;
        let mut r = Registre { generation: take(b, n)?.to_vec(), statut: take(b, n)?.to_vec(), ..Registre::default() };
        r.debut.reserve(n);
        r.long.reserve(n);
        for p in 0..n {
            let l = get16(b)?;
            let nom = std::str::from_utf8(take(b, usize::from(l))?).ok()?;
            r.debut.push(r.texte.len() as u32);
            r.long.push(l);
            r.texte.push_str(nom);
            if l > 0 {
                let h = r.hacheur.hash_one(nom);
                let (texte, debut, long, hacheur) = (&r.texte, &r.debut, &r.long, &r.hacheur);
                r.table.insert_unique(h, p as u32, |&q| {
                    let d = debut[q as usize] as usize;
                    hacheur.hash_one(&texte[d..d + long[q as usize] as usize])
                });
            }
        }
        for _ in 0..get32(b)? {
            r.libres.push((get32(b)?, get32(b)?));
        }
        for _ in 0..get32(b)? {
            r.retraits.push(Retrait { place: get32(b)?, depuis_ms: get64(b)? });
        }
        Some(r)
    }

    /// L'empreinte exacte (places, générations, statuts, noms, plages, retraits).
    pub fn empreinte(&self, h: &mut Fnv) {
        h.u64(self.len() as u64);
        h.octets(&self.generation);
        h.octets(&self.statut);
        for p in 0..self.len() {
            h.u64(u64::from(self.long[p]));
            h.octets(self.nom_place(p).as_bytes());
        }
        for &(d, l) in &self.libres {
            h.u64(u64::from(d) << 32 | u64::from(l));
        }
        for r in &self.retraits {
            h.u64(u64::from(r.place));
            h.u64(r.depuis_ms);
        }
    }
}

// --- Octets, et une empreinte qui ne dépend que du contenu --------------------------------------

pub fn put16(b: &mut Vec<u8>, x: u16) {
    b.extend_from_slice(&x.to_le_bytes());
}
pub fn put32(b: &mut Vec<u8>, x: u32) {
    b.extend_from_slice(&x.to_le_bytes());
}
pub fn put64(b: &mut Vec<u8>, x: u64) {
    b.extend_from_slice(&x.to_le_bytes());
}
pub fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    if b.len() < n {
        return None;
    }
    let (t, r) = b.split_at(n);
    *b = r;
    Some(t)
}
pub fn get16(b: &mut &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes(take(b, 2)?.try_into().ok()?))
}
pub fn get32(b: &mut &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(take(b, 4)?.try_into().ok()?))
}
pub fn get64(b: &mut &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(take(b, 8)?.try_into().ok()?))
}

/// FNV-1a sur 64 bits : assez pour comparer deux mémoires dans un test.
pub struct Fnv(pub u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    pub fn octets(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= u64::from(x);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    pub fn u64(&mut self, x: u64) {
        self.octets(&x.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trois() -> Registre {
        let mut r = Registre::default();
        for nom in ["a/x", "b/x", "c/x"] {
            let n = r.a_donner(r.choisir(1));
            r.affecter(n, nom).unwrap();
        }
        r
    }

    #[test]
    fn une_place_liberee_sert_a_la_generation_suivante() {
        let mut r = trois();
        let b = r.numero("b/x").unwrap();
        r.retirer(b, 0);
        assert_eq!(r.liberables(1_000, 2_000), Vec::<u32>::new(), "le délai de grâce n'est pas passé");
        r.liberer(&r.liberables(5_000, 2_000));
        assert_eq!(r.numero("b/x"), None);
        assert_eq!(r.nom(b), None, "l'ancien numéro ne désigne plus rien");
        let p = r.choisir(1);
        assert_eq!(p as usize, b.place(), "la place libre sert d'abord");
        let d = r.a_donner(p);
        assert_eq!(d.generation(), 1);
        assert_ne!(d, b, "le numéro entier n'est jamais redonné");
        r.affecter(d, "d/x").unwrap();
        assert_eq!(r.nom(b), None);
        assert_eq!(r.nom(d), Some("d/x"));
    }

    #[test]
    fn une_generation_epuisee_condamne_la_place() {
        let mut r = Registre::default();
        let mut vus = std::collections::HashSet::new();
        for k in 0..200u32 {
            let p = r.choisir(1);
            let n = r.a_donner(p);
            assert!(vus.insert(n), "numéro redonné");
            r.affecter(n, &format!("n{k}/x")).unwrap();
            r.retirer(n, 0);
            r.liberer(&[n.place() as u32]);
        }
        assert_eq!(r.statut[0], statut::CONDAMNE);
        assert!(r.len() > 1, "après 128 générations, la place 0 ne sert plus");
    }

    #[test]
    fn une_copie_refuse_une_place_qui_n_est_pas_libre() {
        let mut r = trois();
        let a = r.numero("a/x").unwrap();
        assert!(r.affecter(a, "z/x").is_err(), "place occupée");
        r.retirer(a, 0);
        r.liberer(&[a.place() as u32]);
        assert!(r.affecter(a, "z/x").is_err(), "ancienne génération");
        assert!(r.affecter(r.a_donner(a.place() as u32), "z/x").is_ok());
    }

    #[test]
    fn les_plages_se_recollent_et_le_bout_se_rogne() {
        let mut r = Registre::default();
        for k in 0..10u32 {
            r.affecter(r.a_donner(r.choisir(1)), &format!("p{k}")).unwrap();
        }
        for k in [3u32, 4, 8, 9] {
            r.retirer(r.numero_de(k as usize), 0);
        }
        r.liberer(&r.liberables(0, 0));
        assert_eq!(r.libres, vec![(3, 2), (8, 2)]);
        assert_eq!(r.fin_utile(), 8);
        assert_eq!(r.choisir(3), 8, "la plage du bout se prolonge");
        assert_eq!(r.choisir(2), 3);
    }

    #[test]
    fn le_tiroir_des_noms_se_tasse_sans_rien_perdre() {
        let mut r = Registre::default();
        for k in 0..400u32 {
            r.affecter(r.a_donner(r.choisir(1)), &format!("appareil-{k:04}/temperature")).unwrap();
        }
        let avant = r.texte.len();
        for k in (0..400u32).filter(|k| k % 4 != 0) {
            r.retirer(r.numero_de(k as usize), 0);
        }
        r.liberer(&r.liberables(0, 0));
        assert!(r.texte.len() < avant / 2, "le tiroir a été récrit");
        for k in 0..400u32 {
            let nom = format!("appareil-{k:04}/temperature");
            assert_eq!(r.numero(&nom).is_some(), k % 4 == 0, "{nom}");
        }
        let mut b = Vec::new();
        r.ranger(&mut b);
        let s = Registre::relire(&mut b.as_slice()).unwrap();
        assert_eq!(s.numero("appareil-0396/temperature"), r.numero("appareil-0396/temperature"));
        let n = r.a_donner(r.choisir(1));
        r.affecter(n, "neuf/temperature").unwrap();
        assert_eq!(r.nom(n), Some("neuf/temperature"));
    }

    #[test]
    fn l_annuaire_relu_est_identique() {
        let mut r = trois();
        r.retirer(r.numero("a/x").unwrap(), 7);
        r.retirer(r.numero("c/x").unwrap(), 9);
        r.liberer(&[0]);
        let mut b = Vec::new();
        r.ranger(&mut b);
        let s = Registre::relire(&mut b.as_slice()).unwrap();
        let (mut h1, mut h2) = (Fnv::default(), Fnv::default());
        r.empreinte(&mut h1);
        s.empreinte(&mut h2);
        assert_eq!(h1.0, h2.0);
        assert_eq!(s.numero("b/x"), r.numero("b/x"));
        assert_eq!(s.numero("a/x"), None);
    }
}
