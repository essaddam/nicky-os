//! Le compteur d'octets, le chronomètre, le hasard reproductible, la sortie des résultats.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

/// L'allocateur du système (mallocng sous musl, comme la production), sous un compteur des octets
/// demandés et encore vivants.
pub struct Compteur;

static VIVANTS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Compteur {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            VIVANTS.fetch_add(l.size(), Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        VIVANTS.fetch_sub(l.size(), Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            VIVANTS.fetch_add(n, Relaxed);
            VIVANTS.fetch_sub(l.size(), Relaxed);
        }
        q
    }
}

/// Octets demandés et encore vivants.
pub fn vivants() -> usize {
    VIVANTS.load(Relaxed)
}

/// Octets vivants ajoutés par `f`, et son résultat (gardé vivant).
pub fn poids<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let avant = vivants();
    let t = f();
    (t, vivants().saturating_sub(avant))
}

/// Médiane, en nanosecondes par opération, de `tours` passages de `f`, qui fait `ops` opérations.
pub fn ns(ops: usize, tours: usize, mut f: impl FnMut() -> u64) -> f64 {
    let mut t = Vec::with_capacity(tours);
    black_box(f());
    for _ in 0..tours {
        let d = Instant::now();
        black_box(f());
        t.push(d.elapsed().as_nanos() as f64 / ops as f64);
    }
    t.sort_by(f64::total_cmp);
    t[tours / 2]
}

/// Les deux horloges : la monotone (secondes depuis le démarrage, jamais de saut) pour les durées,
/// la murale (ms, peut sauter : NTP, réglage) pour le calendrier et ce qu'on montre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Temps {
    pub mono: u32,
    pub mur_ms: u64,
}

impl Temps {
    pub fn mur_s(self) -> u32 {
        (self.mur_ms / 1_000) as u32
    }
}

/// splitmix64 : le même tirage à chaque lancement.
pub struct Alea(u64);

impl Alea {
    pub fn new(graine: u64) -> Self {
        Self(graine)
    }
    pub fn suivant(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn sous(&mut self, n: usize) -> usize {
        (self.suivant() % n as u64) as usize
    }
}

/// Mémoire résidente du processus (Linux), en Kio.
pub fn rss_kio() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|l| l.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Une ligne de résultat, lue par le script de synthèse : RESULTAT|section|échelle|clé|valeur|unité.
pub fn resultat(section: &str, echelle: u32, cle: &str, valeur: f64, unite: &str) {
    println!("RESULTAT|{section}|{echelle}|{cle}|{valeur:.3}|{unite}");
}
