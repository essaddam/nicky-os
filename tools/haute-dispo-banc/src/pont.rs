//! Le pont MQTT entre l'actif et le secours, joué sur deux vrais courtiers `rumqttd` 0.20 (le
//! courtier de la V7), dans un seul processus, avec un relais TCP qui imite le lien.
//!
//! Côté actif : le hub publie `val/`, `vu/` et `wf/` (l'état des exécutions), tous dans le même rang
//! `s`. Il garde en mémoire une **file** de ses dernières sorties (v2 : bornée, aucun disque). Il
//! répond à deux demandes :
//! - `moli/v1/repl/demande {de}` : un tronçon de la file, une fenêtre de 32 tronçons de 64 sorties à
//!   la fois, en binaire ; si `de` est sorti de la file, la réponse le dit (« hors file ») ;
//! - `moli/v1/etat/demande` : l'**état complet** au rang `s0` (valeurs, santé, exécutions), construit
//!   en mémoire, envoyé par morceaux de 4 Kio.
//!
//! Côté secours : le **pont à reprise** (un client `rumqttc`) s'abonne aux sorties de l'actif à
//! travers le relais, les remet dans l'ordre de `s`, et les publie sur le courtier du secours. Un
//! trou se comble par la file de l'actif ; au-delà, par l'état complet. Les abonnés locaux du secours
//! (ici la réplique) font de même auprès de la mémoire du secours.
//!
//! Pour comparer : le pont de `rumqttd` lui-même (un filtre, session propre codée en dur, aucun
//! tampon), et la v1 (une file sans limite, comme un journal sur disque).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use rumqttd::local::{LinkRx, LinkTx};
use rumqttd::{Broker, Config, Notification};
use serde_json::json;

use crate::copie::{rapports, temps_actif};
use crate::flux::{Actif, OCTETS_BINAIRE, Replique, Sortie, empreinte_moteur, etat_complet};
use crate::maison::generer;
use crate::outils::{Temps, resultat};
use crate::relais::{Lien, relayer};

/// Sorties par tronçon, tronçons par fenêtre : 64 × 46 o ≈ 2,9 Kio (sous les 4 Kio de la V7) ;
/// une fenêtre ≈ 94 Kio, sous le journal du filtre des tronçons (1 Mio × 4) : jamais de saut.
pub const PAR_TRONCON: usize = 64;
pub const TRONCONS_PAR_FENETRE: u64 = 32;
const EN_TETE: usize = 16;
/// Données par morceau d'état complet : 4 Kio moins l'en-tête.
const PAR_MORCEAU: usize = 4_096 - EN_TETE;
/// La fin de fenêtre qui dit « ce rang est sorti de la file : demande l'état complet ».
const HORS_FILE: u64 = u64::MAX;
/// Un rang qui manque n'est demandé qu'après ce délai : l'autre filtre le livre peut-être.
const GRACE: Duration = Duration::from_millis(200);
/// Le hub publie 2 s après le démarrage des ponts.
const DEPART: Duration = Duration::from_secs(2);
/// Une demande sans avance pendant ce délai est refaite (un tronçon ou un morceau perdu).
const RELANCE: Duration = Duration::from_secs(2);
/// La file en mémoire de l'actif, proposée en v2 : 10 000 sorties (≈ 0,45 Mio en binaire, une heure
/// et demie de la maison à sa cadence moyenne).
pub const FILE_V2: u64 = 10_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pont {
    /// Le pont à reprise proposé.
    Reprise,
    /// Le pont de rumqttd 0.20, tel quel.
    Rumqttd,
}

#[derive(Clone)]
pub struct Options {
    pub nom: &'static str,
    /// Messages par seconde publiés par le hub ; 0 : tout est déjà publié (le secours revient).
    pub debit_msgs: f64,
    pub messages: usize,
    /// Coupures du lien : (début, fin) en secondes depuis le départ.
    pub coupures: Vec<(f64, f64)>,
    pub delai_ms: u32,
    pub gigue_ms: u32,
    /// Octets par seconde du lien ; 0 : sans limite.
    pub debit_lien: u64,
    pub pont: Pont,
    pub port: u16,
    /// La file en mémoire de l'actif, en sorties ; `u64::MAX` : sans limite (la v1, un journal).
    pub file: u64,
}

#[derive(Default)]
pub struct Mesure {
    pub messages: u64,
    pub recus: u64,
    pub perdus: u64,
    pub identique: bool,
    pub lat_vif: Vec<f64>,
    pub via_file: u64,
    pub demandes_pont: u64,
    pub demandes_locales: u64,
    pub connexions: u64,
    pub rattrapages_ms: Vec<f64>,
    pub duree_s: f64,
    pub octets_retour: u64,
    pub ecritures_retour: u64,
    pub octets_aller: u64,
    pub tampon_max: u64,
    pub pauses: u64,
    pub executions: u64,
    pub etats_pont: u64,
    pub etats_locaux: u64,
    pub octets_etat: u64,
    /// La mémoire que tient le pont du secours est celle de l'actif (pont à reprise seulement).
    pub memoire_secours_identique: bool,
}

/// Ce que le rattrapeur demande.
#[derive(Debug, PartialEq, Eq)]
enum Demande {
    /// Un tronçon de la file, à partir de ce rang.
    Troncon(u64),
    /// L'état complet.
    Etat,
}

/// Un état complet en cours de réception.
struct EtatRecu {
    s0: u64,
    morceaux: Vec<Option<Vec<u8>>>,
    recus: usize,
}

/// Remettre dans l'ordre de `s`, et dire quand demander un tronçon ou l'état complet.
#[derive(Default)]
struct Rattrapeur {
    dernier: u64,
    tampon: BTreeMap<u64, Sortie>,
    /// Une demande de tronçon en cours, et l'instant de sa dernière avance.
    fenetre: Option<Instant>,
    /// Une demande d'état complet en cours, depuis quand.
    etat_demande: Option<Instant>,
    etat: Option<EtatRecu>,
    /// Un trou vu et pas encore demandé : depuis quand.
    trou: Option<Instant>,
    /// Le bout annoncé par le battement de la mémoire en amont.
    bout_vu: u64,
    demandes: u64,
    tampon_max: usize,
}

impl Rattrapeur {
    /// Une sortie reçue (en direct ou d'un tronçon) ; celles qui deviennent prêtes, dans l'ordre.
    fn recevoir(&mut self, so: Sortie, prets: &mut Vec<Sortie>) {
        if so.s <= self.dernier || self.tampon.contains_key(&so.s) {
            return;
        }
        if so.s != self.dernier + 1 {
            if self.tampon.len() < 50_000 {
                self.tampon.insert(so.s, so);
                self.tampon_max = self.tampon_max.max(self.tampon.len());
            }
            self.trou.get_or_insert_with(Instant::now);
            return;
        }
        prets.push(so);
        self.dernier = so.s;
        self.vider(prets);
        if self.fenetre.is_some() {
            self.fenetre = Some(Instant::now());
        }
    }

    fn vider(&mut self, prets: &mut Vec<Sortie>) {
        while let Some(x) = self.tampon.remove(&(self.dernier + 1)) {
            prets.push(x);
            self.dernier = x.s;
        }
        self.trou = if self.tampon.is_empty() && self.bout_vu <= self.dernier { None } else { Some(Instant::now()) };
    }

    /// Le battement de la mémoire en amont : son bout. Plus loin que nous, c'est un trou, même quand
    /// plus rien ne passe (la dernière sortie perdue).
    fn bout(&mut self, s: u64) {
        self.bout_vu = self.bout_vu.max(s);
        if s > self.dernier {
            self.trou.get_or_insert_with(Instant::now);
        }
    }

    fn demander(&mut self) -> Demande {
        self.fenetre = Some(Instant::now());
        self.trou = None;
        self.demandes += 1;
        Demande::Troncon(self.dernier + 1)
    }

    fn demander_etat(&mut self) -> Demande {
        self.fenetre = None;
        self.trou = None;
        self.etat = None;
        self.etat_demande = Some(Instant::now());
        self.demandes += 1;
        Demande::Etat
    }

    /// Le dernier tronçon d'une fenêtre est arrivé : la suivante, si l'on est encore en retard ;
    /// l'état complet, si le rang demandé est sorti de la file.
    fn fin_fenetre(&mut self, fin_journal: u64, fin_fenetre: u64) -> Option<Demande> {
        if fin_fenetre == HORS_FILE {
            return (self.etat_demande.is_none()).then(|| self.demander_etat());
        }
        if self.dernier < fin_fenetre {
            return None;
        }
        let retard = fin_journal.max(self.bout_vu).max(self.tampon.keys().next_back().copied().unwrap_or(0));
        if fin_fenetre < self.dernier + 1 && fin_journal <= self.dernier && self.dernier < retard {
            // La file n'a pas ce qu'il manque (pas de file, ou pas encore) : relance plus tard.
            return None;
        }
        if self.dernier < retard {
            Some(self.demander())
        } else {
            self.fenetre = None;
            None
        }
    }

    /// Un morceau d'état complet ; rend l'état entier quand le dernier morceau arrive.
    fn morceau(&mut self, s0: u64, k: usize, n: usize, donnees: &[u8]) -> Option<(u64, Vec<u8>)> {
        if k >= n || self.etat_demande.is_none() {
            return None;
        }
        let e = self.etat.get_or_insert_with(|| EtatRecu { s0, morceaux: vec![None; n], recus: 0 });
        if e.s0 != s0 || e.morceaux.len() != n {
            *e = EtatRecu { s0, morceaux: vec![None; n], recus: 0 };
        }
        if e.morceaux[k].is_none() {
            e.morceaux[k] = Some(donnees.to_vec());
            e.recus += 1;
        }
        if e.recus < n {
            return None;
        }
        let e = self.etat.take().unwrap();
        Some((e.s0, e.morceaux.into_iter().flatten().flatten().collect()))
    }

    /// L'état complet au rang `s0` est appliqué : ce qui était gardé au-delà devient prêt.
    fn apres_etat(&mut self, s0: u64, prets: &mut Vec<Sortie>) {
        self.etat_demande = None;
        self.fenetre = None;
        if s0 > self.dernier {
            self.dernier = s0;
        }
        self.tampon.retain(|&s, _| s > s0);
        self.vider(prets);
    }

    /// Tous les 100 ms : un trou qui dure plus que la grâce, une demande sans avance.
    fn tic(&mut self) -> Option<Demande> {
        if let Some(t) = self.etat_demande {
            return (t.elapsed() > RELANCE).then(|| self.demander_etat());
        }
        match (self.fenetre, self.trou) {
            (Some(t), _) if t.elapsed() > RELANCE => Some(self.demander()),
            (None, Some(t)) if t.elapsed() > GRACE => Some(self.demander()),
            _ => None,
        }
    }
}

/// Ce que les tâches partagent.
struct Partage {
    t0: Instant,
    n: u64,
    envoi_ns: Vec<AtomicU64>,
    via: Vec<AtomicU8>,
    /// La dernière sortie publiée par l'actif.
    publie: AtomicU64,
    /// La dernière sortie remise par le pont au courtier du secours.
    copie: AtomicU64,
    /// Le premier rang que la mémoire du secours a reçu un par un (après un état complet).
    copie_debut: AtomicU64,
    applique: AtomicU64,
    /// Les sorties distinctes que la réplique a reçues (appliquées ou en attente d'un trou).
    recus: AtomicU64,
    /// Les pauses du routeur pour la réplique (sa file a débordé).
    pauses: AtomicU64,
    fini: AtomicBool,
    via_file: AtomicU64,
    demandes_pont: AtomicU64,
    demandes_locales: AtomicU64,
    connexions: AtomicU64,
    tampon_max: AtomicU64,
    etats_pont: AtomicU64,
    etats_locaux: AtomicU64,
    octets_etat: AtomicU64,
    cible: AtomicU64,
    retour_ns: AtomicU64,
    rattrapages_ms: Mutex<Vec<f64>>,
    lat_vif_us: Mutex<Vec<f64>>,
}

impl Partage {
    fn ns(&self) -> u64 {
        self.t0.elapsed().as_nanos() as u64
    }
}

/// L'heure de cette machine pour appliquer une sortie (les échéances monotones sont locales).
fn temps(p: &Partage) -> Temps {
    let ns = p.ns();
    Temps { mono: 1_000 + (ns / 1_000_000_000) as u32, mur_ms: crate::copie::DEBUT_MS + ns / 1_000_000 }
}

fn config(port: u16, pont: Option<u16>) -> Config {
    let connexions = json!({"connection_timeout_ms": 5000, "max_payload_size": 4_194_304, "max_inflight_count": 100, "dynamic_filters": true});
    let mut c = json!({
        "id": 0,
        // Le réglage de la V7 (§ 6.4) : 64 Kio × 4 par filtre ; les tronçons et l'état, 1 Mio × 4.
        "router": {"id": 0, "max_connections": 64, "max_outgoing_packet_count": 200,
                   "max_segment_size": 65_536, "max_segment_count": 4,
                   "custom_segment": {"moli/v1/repl/troncon/#": {"max_segment_size": 1_048_576, "max_segment_count": 4},
                                      "moli/v1/etat/reponse": {"max_segment_size": 1_048_576, "max_segment_count": 4}}},
        "v4": {"1": {"name": "v4-1", "listen": format!("127.0.0.1:{port}"), "next_connection_delay_ms": 1, "connections": connexions}}
    });
    if let Some(relais) = pont {
        // Un seul filtre possible : le pont de rumqttd ne sait pas en prendre plusieurs.
        c["bridge"] = json!({"name": "pont-rumqttd", "addr": format!("127.0.0.1:{relais}"), "qos": 0,
                             "sub_path": "moli/v1/#", "reconnection_delay": 1, "ping_delay": 5, "connections": connexions});
    }
    serde_json::from_value(c).expect("configuration rumqttd")
}

fn courtier(config: Config, liens: &[&str]) -> Vec<(LinkTx, LinkRx)> {
    let mut b = Broker::new(config);
    let l = liens.iter().map(|n| b.link(n).expect("lien local")).collect();
    std::thread::Builder::new().name("courtier".into()).spawn(move || b.start()).expect("fil du courtier");
    l
}

fn troncon(sorties: &[Sortie], fin_journal: u64, fin_fenetre: u64, b: &mut Vec<u8>) {
    b.clear();
    b.extend_from_slice(&fin_journal.to_le_bytes());
    b.extend_from_slice(&fin_fenetre.to_le_bytes());
    for so in sorties {
        so.ranger(b);
    }
}

/// Lit un tronçon : (bout de la file, fin de la fenêtre, sorties).
fn lire_troncon(charge: &[u8]) -> (u64, u64, impl Iterator<Item = Sortie> + '_) {
    let fin_journal = u64::from_le_bytes(charge[0..8].try_into().unwrap());
    let fin_fenetre = u64::from_le_bytes(charge[8..16].try_into().unwrap());
    (fin_journal, fin_fenetre, charge[EN_TETE..].chunks_exact(OCTETS_BINAIRE).map(Sortie::relire))
}

/// Lit un morceau d'état complet : (s0, rang du morceau, nombre de morceaux, données).
fn lire_morceau(charge: &[u8]) -> Option<(u64, usize, usize, &[u8])> {
    let s0 = u64::from_le_bytes(charge.get(0..8)?.try_into().ok()?);
    let k = u32::from_le_bytes(charge.get(8..12)?.try_into().ok()?) as usize;
    let n = u32::from_le_bytes(charge.get(12..16)?.try_into().ok()?) as usize;
    Some((s0, k, n, &charge[EN_TETE..]))
}

/// D'où une mémoire tire son état complet.
enum Source {
    /// L'actif : sa mémoire est la vérité ; le banc la refait depuis ses sorties jusqu'au bout publié.
    Actif(Box<Replique>),
    /// Le secours : la mémoire que tient le pont (les sorties remises, l'état complet reçu).
    Pont(Arc<Mutex<Replique>>),
}

/// La mémoire d'une machine, vue des autres : sa file des dernières sorties, et son état complet.
/// `bout` dit jusqu'où elle va ; `debut` d'où sa file commence (après un état complet reçu).
#[allow(clippy::too_many_arguments)]
async fn memoire(
    mut tx: LinkTx,
    mut rx: LinkRx,
    sorties: Arc<Vec<Sortie>>,
    mut source: Source,
    bout: impl Fn() -> u64,
    debut: impl Fn() -> u64,
    file: u64,
    p: Arc<Partage>,
) {
    tx.subscribe("moli/v1/repl/demande").expect("abonnement");
    tx.subscribe("moli/v1/etat/demande").expect("abonnement");
    let mut charge = Vec::with_capacity(EN_TETE + PAR_TRONCON * OCTETS_BINAIRE);
    let t = Temps { mono: 1_000, mur_ms: crate::copie::DEBUT_MS };
    loop {
        // `next` rend parfois `None` sur un réveil vide : ce n'est pas la fin du lien.
        let n = match rx.next().await {
            Ok(Some(n)) => n,
            Ok(None) => continue,
            Err(_) => break,
        };
        let f = match n {
            Notification::Forward(f) => f,
            // Le routeur met en pause un lien dont la file déborde ; il ne le reprend que sur ce signal.
            Notification::Unschedule => {
                let _ = rx.wake().await;
                continue;
            }
            _ => continue,
        };
        let fin = bout();
        if f.publish.topic.ends_with(b"etat/demande") {
            // L'état complet, construit en mémoire, envoyé par morceaux de 4 Kio.
            let (s0, b) = match &mut source {
                Source::Actif(miroir) => {
                    while miroir.dernier < fin {
                        let so = sorties[miroir.dernier as usize];
                        miroir.recevoir(so, t);
                    }
                    let b = etat_complet(&miroir.etat, &miroir.moteur, miroir.dernier);
                    p.octets_etat.fetch_add(b.len() as u64, Relaxed);
                    (miroir.dernier, b)
                }
                Source::Pont(r) => {
                    let r = r.lock().unwrap();
                    (r.dernier, etat_complet(&r.etat, &r.moteur, r.dernier))
                }
            };
            let n = b.len().div_ceil(PAR_MORCEAU);
            for (k, morceau) in b.chunks(PAR_MORCEAU).enumerate() {
                charge.clear();
                charge.extend_from_slice(&s0.to_le_bytes());
                charge.extend_from_slice(&(k as u32).to_le_bytes());
                charge.extend_from_slice(&(n as u32).to_le_bytes());
                charge.extend_from_slice(morceau);
                let _ = tx.publish("moli/v1/etat/reponse", charge.clone());
            }
            continue;
        }
        let Ok(d) = serde_json::from_slice::<serde_json::Value>(&f.publish.payload) else { continue };
        let de = d["de"].as_u64().unwrap_or(1).max(1);
        let premier = debut().max(if file == u64::MAX { 1 } else { fin.saturating_sub(file - 1).max(1) });
        if de < premier {
            // Le rang demandé est sorti de la file : il faut l'état complet.
            troncon(&[], fin, HORS_FILE, &mut charge);
            let _ = tx.publish("moli/v1/repl/troncon/reponse", charge.clone());
            continue;
        }
        if de > fin {
            // Rien à donner : un tronçon vide le dit.
            troncon(&[], fin, de - 1, &mut charge);
            let _ = tx.publish("moli/v1/repl/troncon/reponse", charge.clone());
            continue;
        }
        let fin_fenetre = fin.min(de - 1 + TRONCONS_PAR_FENETRE * PAR_TRONCON as u64);
        for debut in (de..=fin_fenetre).step_by(PAR_TRONCON) {
            let jusqu_a = (debut + PAR_TRONCON as u64 - 1).min(fin_fenetre);
            troncon(&sorties[(debut - 1) as usize..jusqu_a as usize], fin, fin_fenetre, &mut charge);
            let _ = tx.publish("moli/v1/repl/troncon/reponse", charge.clone());
        }
    }
}

/// Le battement d'une mémoire : son bout, toutes les 500 ms (`moli/v1/repl/bout`). Une copie qui le
/// voit plus loin qu'elle sait qu'il lui manque la fin, même quand plus rien ne passe.
async fn battement(mut tx: LinkTx, _rx: LinkRx, bout: impl Fn() -> u64, p: Arc<Partage>) {
    let mut tic = tokio::time::interval(Duration::from_millis(500));
    while !p.fini.load(Relaxed) {
        tic.tick().await;
        let s = bout();
        if s > 0 {
            let _ = tx.publish("moli/v1/repl/bout", format!("{{\"s\":{s}}}"));
        }
    }
}

fn lire_bout(charge: &[u8]) -> Option<u64> {
    serde_json::from_slice::<serde_json::Value>(charge).ok()?["s"].as_u64()
}

/// Ce que la boucle du client MQTT remonte au pont.
enum Remontee {
    Connecte,
    Message(rumqttc::Publish),
    Tombe(String),
}

/// La boucle du client MQTT, dans sa propre tâche : on ne l'interrompt jamais. L'abandonner au milieu
/// d'une reconnexion lente laisse une connexion à moitié ouverte chez le courtier de l'actif ; à force,
/// il atteint `max_connections` et refuse tout le monde (vu sur le lien distant du banc).
async fn boucle_client(mut ev: rumqttc::EventLoop, tx: tokio::sync::mpsc::UnboundedSender<Remontee>) {
    loop {
        let r = match ev.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => Remontee::Connecte,
            Ok(Event::Incoming(Packet::Publish(m))) => Remontee::Message(m),
            Ok(_) => continue,
            Err(e) => {
                let _ = tx.send(Remontee::Tombe(e.to_string()));
                // Le lien est tombé : session propre, rien n'attend au courtier ; on retente.
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        if tx.send(r).is_err() {
            return;
        }
    }
}

/// Le pont à reprise, côté secours. Il tient la mémoire du secours (`memoire_b`) : il y applique les
/// sorties qu'il remet, et l'état complet qu'il reçoit.
async fn pont_a_reprise(port_relais: u16, mut tx_b: LinkTx, noms: Arc<Vec<String>>, memoire_b: Arc<Mutex<Replique>>, p: Arc<Partage>) {
    let mut o = MqttOptions::new("pont-secours", "127.0.0.1", port_relais);
    o.set_keep_alive(Duration::from_secs(5));
    o.set_clean_session(true);
    o.set_max_packet_size(4 << 20, 4 << 20);
    let (client, ev) = AsyncClient::new(o, 4096);
    let (tx_ev, mut rx_ev) = tokio::sync::mpsc::unbounded_channel();
    let boucle = tokio::spawn(boucle_client(ev, tx_ev));
    let diag = std::env::var_os("HD_DIAG").is_some();
    let mut r = Rattrapeur::default();
    let mut prets = Vec::new();
    let mut charge = Vec::with_capacity(128);
    let mut tic = tokio::time::interval(Duration::from_millis(100));
    let mut accuse = Instant::now();
    loop {
        let mut demande = None;
        tokio::select! {
            e = rx_ev.recv() => match e {
                Some(Remontee::Connecte) => {
                    p.connexions.fetch_add(1, Relaxed);
                    if diag {
                        eprintln!("pont : connecté à {:.1} s (dernier {})", p.t0.elapsed().as_secs_f64(), r.dernier);
                    }
                    for f in ["moli/v1/val/#", "moli/v1/vu/#", "moli/v1/wf/#", "moli/v1/repl/troncon/#", "moli/v1/repl/bout", "moli/v1/etat/reponse"] {
                        let _ = client.try_subscribe(f, QoS::AtMostOnce);
                    }
                    // Ce qui a été publié pendant l'absence est perdu pour le courtier : on le demande.
                    demande = Some(r.demander());
                }
                Some(Remontee::Message(m)) => {
                    if m.topic.starts_with("moli/v1/repl/troncon/") {
                        let (fin_journal, fin_fenetre, sorties) = lire_troncon(&m.payload);
                        for so in sorties {
                            if so.s > r.dernier && !r.tampon.contains_key(&so.s) {
                                p.via[(so.s - 1) as usize].store(1, Relaxed);
                                p.via_file.fetch_add(1, Relaxed);
                            }
                            r.recevoir(so, &mut prets);
                        }
                        demande = r.fin_fenetre(fin_journal, fin_fenetre);
                    } else if m.topic == "moli/v1/repl/bout" {
                        if let Some(s) = lire_bout(&m.payload) {
                            r.bout(s);
                        }
                    } else if m.topic == "moli/v1/etat/reponse" {
                        if let Some((s0, k, n, donnees)) = lire_morceau(&m.payload) {
                            if let Some((s0, etat)) = r.morceau(s0, k, n, donnees) {
                                // La mémoire du secours prend l'état complet : elle est au rang s0 ; sa
                                // file ne commence qu'après.
                                let pris = s0 > r.dernier && memoire_b.lock().unwrap().prendre_etat(&etat, temps(&p)).is_some();
                                if pris {
                                    p.etats_pont.fetch_add(1, Relaxed);
                                    p.copie_debut.store(s0 + 1, Relaxed);
                                    p.copie.fetch_max(s0, Relaxed);
                                    r.apres_etat(s0, &mut prets);
                                    if diag {
                                        eprintln!("pont : état complet au rang {s0} à {:.1} s", p.t0.elapsed().as_secs_f64());
                                    }
                                } else {
                                    r.etat_demande = None;
                                }
                            }
                        }
                    } else if let Some(so) = Sortie::lire(&m.payload) {
                        r.recevoir(so, &mut prets);
                    }
                }
                Some(Remontee::Tombe(e)) => {
                    if diag {
                        eprintln!("pont : tombé à {:.1} s : {e}", p.t0.elapsed().as_secs_f64());
                    }
                    r.fenetre = None;
                    r.etat_demande = None;
                }
                None => return,
            },
            _ = tic.tick() => {
                if p.fini.load(Relaxed) {
                    boucle.abort();
                    return;
                }
                demande = r.tic();
                if accuse.elapsed() > Duration::from_millis(500) {
                    accuse = Instant::now();
                    let _ = client.try_publish("moli/v1/repl/accuse/secours", QoS::AtMostOnce, false, format!("{{\"s\":{}}}", r.dernier));
                }
            }
        }
        // Remettre au courtier du secours, dans l'ordre ; la mémoire du secours avance avec.
        if !prets.is_empty() {
            let t = temps(&p);
            let mut mb = memoire_b.lock().unwrap();
            for so in &prets {
                mb.recevoir(*so, t);
            }
        }
        for so in prets.drain(..) {
            p.copie.store(so.s, Relaxed);
            so.charge(&mut charge);
            let _ = tx_b.publish(so.sujet(&noms[so.i as usize % noms.len()]), charge.clone());
        }
        p.tampon_max.fetch_max(r.tampon_max as u64, Relaxed);
        match demande {
            Some(Demande::Troncon(de)) => {
                p.demandes_pont.fetch_add(1, Relaxed);
                let _ = client.try_publish("moli/v1/repl/demande", QoS::AtMostOnce, false, format!("{{\"de\":{de}}}"));
            }
            Some(Demande::Etat) => {
                p.demandes_pont.fetch_add(1, Relaxed);
                let _ = client.try_publish("moli/v1/etat/demande", QoS::AtMostOnce, false, "{}");
            }
            None => {}
        }
    }
}

/// La réplique, abonnée au courtier du secours comme n'importe quel abonné local ; ses trous, elle
/// les comble par la mémoire du secours (sa file, puis son état complet).
async fn replique(mut tx: LinkTx, mut rx: LinkRx, mut rep: Replique, p: Arc<Partage>) -> Replique {
    for f in ["moli/v1/val/#", "moli/v1/vu/#", "moli/v1/wf/#", "moli/v1/repl/troncon/#", "moli/v1/repl/bout", "moli/v1/etat/reponse"] {
        tx.subscribe(f).expect("abonnement");
    }
    let mut r = Rattrapeur::default();
    let mut prets = Vec::new();
    let mut lat = Vec::new();
    let mut tic = tokio::time::interval(Duration::from_millis(100));
    loop {
        let mut demande = None;
        let t = temps(&p);
        tokio::select! {
            n = rx.next() => {
                let n = match n {
                    Ok(Some(n)) => n,
                    Ok(None) => continue,
                    Err(_) => break,
                };
                let f = match n {
                    Notification::Forward(f) => f,
                    Notification::Unschedule => {
                        p.pauses.fetch_add(1, Relaxed);
                        let _ = rx.wake().await;
                        continue;
                    }
                    _ => continue,
                };
                if f.publish.topic.starts_with(b"moli/v1/repl/troncon/") {
                    let (fin_journal, fin_fenetre, sorties) = lire_troncon(&f.publish.payload);
                    for so in sorties {
                        if so.s > r.dernier && !r.tampon.contains_key(&so.s) {
                            p.via[(so.s - 1) as usize].store(1, Relaxed);
                        }
                        r.recevoir(so, &mut prets);
                    }
                    demande = r.fin_fenetre(fin_journal, fin_fenetre);
                } else if &f.publish.topic[..] == b"moli/v1/repl/bout" {
                    if let Some(s) = lire_bout(&f.publish.payload) {
                        r.bout(s);
                    }
                } else if &f.publish.topic[..] == b"moli/v1/etat/reponse" {
                    if let Some((s0, k, n, donnees)) = lire_morceau(&f.publish.payload) {
                        if let Some((s0, etat)) = r.morceau(s0, k, n, donnees) {
                            // Un état plus ancien que ce que l'on tient déjà ne sert à rien.
                            if s0 > r.dernier && rep.prendre_etat(&etat, t).is_some() {
                                p.etats_locaux.fetch_add(1, Relaxed);
                                r.apres_etat(s0, &mut prets);
                            } else {
                                r.etat_demande = None;
                            }
                        }
                    }
                } else if let Some(so) = Sortie::lire(&f.publish.payload) {
                    r.recevoir(so, &mut prets);
                }
            }
            _ = tic.tick() => demande = r.tic(),
        }
        let maintenant = p.ns();
        for so in prets.drain(..) {
            let t = Temps { mono: 1_000 + (maintenant / 1_000_000_000) as u32, mur_ms: so.r_ms };
            rep.recevoir(so, t);
            if p.via[(so.s - 1) as usize].load(Relaxed) == 0 {
                let e = p.envoi_ns[(so.s - 1) as usize].load(Relaxed);
                if e != 0 {
                    lat.push(maintenant.saturating_sub(e) as f64 / 1_000.0);
                }
            }
        }
        p.applique.store(rep.dernier, Relaxed);
        p.recus.store(r.dernier + r.tampon.len() as u64, Relaxed);
        let cible = p.cible.load(Relaxed);
        if cible != 0 && rep.dernier >= cible {
            p.cible.store(0, Relaxed);
            let ms = maintenant.saturating_sub(p.retour_ns.load(Relaxed)) as f64 / 1e6;
            p.rattrapages_ms.lock().unwrap().push(ms);
        }
        match demande {
            Some(Demande::Troncon(de)) => {
                p.demandes_locales.fetch_add(1, Relaxed);
                let _ = tx.publish("moli/v1/repl/demande", format!("{{\"de\":{de}}}"));
            }
            Some(Demande::Etat) => {
                p.demandes_locales.fetch_add(1, Relaxed);
                let _ = tx.publish("moli/v1/etat/demande", "{}");
            }
            None => {}
        }
        if rep.dernier >= p.n || p.fini.load(Relaxed) {
            break;
        }
    }
    *p.lat_vif_us.lock().unwrap() = lat;
    rep
}

/// Les sorties de la maison : les rapports des modules, et une exécution qui démarre sur un point
/// lu par une règle sur sept (elles partagent le rang `s`).
fn sorties_de(actif: &mut Actif, m: &crate::maison::Maison, messages: usize) -> Vec<Sortie> {
    let heures = messages.div_ceil(6_000).max(1) as u64;
    let rs = rapports(m, heures * 3_600, 7);
    let mut sorties = Vec::with_capacity(messages);
    for r in &rs {
        if sorties.len() >= messages {
            break;
        }
        let t = temps_actif(r.t_ms);
        let so = actif.rapport(r.n, r.valeur, t);
        sorties.push(so);
        if !so.vu && so.i % 7 == 0 && sorties.len() < messages {
            sorties.push(actif.execution(so.i % crate::flux::CASES as u32, so.i, so.c_ms, t));
        }
    }
    assert_eq!(sorties.len(), messages, "pas assez de rapports");
    sorties
}

pub fn jouer(o: &Options) -> Mesure {
    let m = generer(1);
    let mut actif = Actif::new(&m);
    let sorties: Arc<Vec<Sortie>> = Arc::new(sorties_de(&mut actif, &m, o.messages));
    let executions = sorties.iter().filter(|s| s.wf).count() as u64;
    let noms: Arc<Vec<String>> = Arc::new(m.points.iter().map(|p| p.nom.clone()).collect());
    let n = sorties.len() as u64;
    let p = Arc::new(Partage {
        t0: Instant::now(),
        n,
        envoi_ns: (0..n).map(|_| AtomicU64::new(0)).collect(),
        via: (0..n).map(|_| AtomicU8::new(0)).collect(),
        publie: AtomicU64::new(0),
        copie: AtomicU64::new(0),
        copie_debut: AtomicU64::new(1),
        applique: AtomicU64::new(0),
        recus: AtomicU64::new(0),
        pauses: AtomicU64::new(0),
        fini: AtomicBool::new(false),
        via_file: AtomicU64::new(0),
        demandes_pont: AtomicU64::new(0),
        demandes_locales: AtomicU64::new(0),
        connexions: AtomicU64::new(0),
        tampon_max: AtomicU64::new(0),
        etats_pont: AtomicU64::new(0),
        etats_locaux: AtomicU64::new(0),
        octets_etat: AtomicU64::new(0),
        cible: AtomicU64::new(0),
        retour_ns: AtomicU64::new(0),
        rattrapages_ms: Mutex::new(Vec::new()),
        lat_vif_us: Mutex::new(Vec::new()),
    });
    let (pa, pb, pr) = (o.port, o.port + 1, o.port + 2);
    let mut la = courtier(config(pa, None), &["hub", "memoire", "battement"]).into_iter();
    let (mut hub_tx, _hub_rx) = la.next().unwrap();
    let (ma_tx, ma_rx) = la.next().unwrap();
    let (ba_tx, ba_rx) = la.next().unwrap();
    let mut lb = courtier(config(pb, (o.pont == Pont::Rumqttd).then_some(pr)), &["pont", "replique", "memoire", "battement"]).into_iter();
    let (pont_tx, _pont_rx) = lb.next().unwrap();
    let (rtx, rrx) = lb.next().unwrap();
    let (mb_tx, mb_rx) = lb.next().unwrap();
    let (bb_tx, bb_rx) = lb.next().unwrap();
    let lien = Arc::new(Lien::default());
    lien.regler(o.delai_ms, o.gigue_ms, o.debit_lien);
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let (rep0, miroir_a) = (Replique::new(&m), Box::new(Replique::new(&m)));
    let memoire_b = Arc::new(Mutex::new(Replique::new(&m)));
    let actif_empreinte = empreinte_moteur(&actif.etat, &actif.moteur);
    let o2 = o.clone();
    let (rep, duree) = {
        let (p, lien, memoire_b) = (p.clone(), lien.clone(), memoire_b.clone());
        rt.block_on(async move {
            std::thread::sleep(Duration::from_millis(300)); // les écouteurs des courtiers s'ouvrent
            let ecoute = tokio::net::TcpListener::bind(("127.0.0.1", pr)).await.expect("relais");
            tokio::spawn(relayer(ecoute, ([127, 0, 0, 1], pa).into(), lien.clone()));
            let (pj, pk, pb_) = (p.clone(), p.clone(), p.clone());
            tokio::spawn(memoire(ma_tx, ma_rx, sorties.clone(), Source::Actif(miroir_a), move || pj.publie.load(Relaxed), || 1, o2.file, pk));
            tokio::spawn(battement(ba_tx, ba_rx, move || pb_.publie.load(Relaxed), p.clone()));
            if o2.pont == Pont::Reprise {
                // La mémoire du secours ne se tient qu'avec le pont à reprise : celui de rumqttd n'en tient pas.
                let (pj, pd, pk, pb_) = (p.clone(), p.clone(), p.clone(), p.clone());
                let source = Source::Pont(memoire_b.clone());
                tokio::spawn(memoire(mb_tx, mb_rx, sorties.clone(), source, move || pj.copie.load(Relaxed), move || pd.copie_debut.load(Relaxed), o2.file, pk));
                tokio::spawn(battement(bb_tx, bb_rx, move || pb_.copie.load(Relaxed), p.clone()));
            }
            let fin_replique = tokio::spawn(replique(rtx, rrx, rep0, p.clone()));
            if o2.debit_msgs == 0.0 {
                // Le secours revient après une absence : l'actif a tout publié pendant ce temps.
                p.publie.store(n, Relaxed);
                p.cible.store(n, Relaxed);
                p.retour_ns.store(p.ns(), Relaxed);
            }
            if o2.pont == Pont::Reprise {
                tokio::spawn(pont_a_reprise(pr, pont_tx, noms.clone(), memoire_b.clone(), p.clone()));
            }
            // Les ponts se connectent pendant 2 s avant que le hub publie : les pertes mesurées sont
            // celles des coupures. Les coupures du lien, à leur heure (depuis le départ du hub).
            let depart = p.t0 + DEPART;
            let (l2, p2, coupures) = (lien.clone(), p.clone(), o2.coupures.clone());
            tokio::spawn(async move {
                for (a, b) in coupures {
                    tokio::time::sleep_until((depart + Duration::from_secs_f64(a)).into()).await;
                    l2.couper();
                    tokio::time::sleep_until((depart + Duration::from_secs_f64(b)).into()).await;
                    l2.retablir();
                    p2.retour_ns.store(p2.ns(), Relaxed);
                    p2.cible.store(p2.publie.load(Relaxed).max(1), Relaxed);
                }
            });
            // Le hub publie, au rythme demandé.
            if o2.debit_msgs > 0.0 {
                let periode = Duration::from_secs_f64(1.0 / o2.debit_msgs);
                let mut charge = Vec::new();
                for (k, so) in sorties.iter().enumerate() {
                    tokio::time::sleep_until((depart + periode * k as u32).into()).await;
                    so.charge(&mut charge);
                    p.envoi_ns[k].store(p.ns(), Relaxed);
                    p.publie.store(so.s, Relaxed);
                    let _ = hub_tx.publish(so.sujet(&noms[so.i as usize % noms.len()]), charge.clone());
                }
            }
            let attente = Duration::from_secs(if o2.debit_msgs == 0.0 { 300 } else { 30 });
            let rep = tokio::time::timeout(attente, fin_replique).await;
            p.fini.store(true, Relaxed);
            (rep.ok().and_then(Result::ok), p.t0.elapsed().as_secs_f64())
        })
    };
    if std::env::var_os("HD_DIAG").is_some() {
        eprintln!(
            "[{}] publié {} · copie {} · réplique {} · états {}/{} · durée {:.1} s",
            o.nom,
            p.publie.load(Relaxed),
            p.copie.load(Relaxed),
            p.applique.load(Relaxed),
            p.etats_pont.load(Relaxed),
            p.etats_locaux.load(Relaxed),
            duree
        );
    }
    rt.shutdown_timeout(Duration::from_millis(500));
    let mut lat = std::mem::take(&mut *p.lat_vif_us.lock().unwrap());
    lat.sort_by(f64::total_cmp);
    let recus = rep.as_ref().map_or(p.applique.load(Relaxed), |r| r.dernier);
    Mesure {
        messages: n,
        recus,
        perdus: n - p.recus.load(Relaxed).min(n),
        identique: rep.as_ref().is_some_and(|r| r.dernier == n && empreinte_moteur(&r.etat, &r.moteur) == actif_empreinte),
        lat_vif: lat,
        via_file: p.via_file.load(Relaxed),
        demandes_pont: p.demandes_pont.load(Relaxed),
        demandes_locales: p.demandes_locales.load(Relaxed),
        connexions: p.connexions.load(Relaxed),
        rattrapages_ms: p.rattrapages_ms.lock().unwrap().clone(),
        duree_s: duree,
        octets_retour: lien.octets_retour.load(Relaxed),
        ecritures_retour: lien.ecritures_retour.load(Relaxed),
        octets_aller: lien.octets_aller.load(Relaxed),
        tampon_max: p.tampon_max.load(Relaxed),
        pauses: p.pauses.load(Relaxed),
        executions,
        etats_pont: p.etats_pont.load(Relaxed),
        etats_locaux: p.etats_locaux.load(Relaxed),
        octets_etat: p.octets_etat.load(Relaxed),
        memoire_secours_identique: {
            let mb = memoire_b.lock().unwrap();
            mb.dernier == n && empreinte_moteur(&mb.etat, &mb.moteur) == actif_empreinte
        },
    }
}

fn centile(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v[((v.len() as f64 - 1.0) * q).round() as usize]
}

fn publier(sec: &str, echelle: u32, cas: &str, m: &Mesure) {
    let r = |cle: &str, v: f64, u: &str| resultat(sec, echelle, &format!("{cas}_{cle}"), v, u);
    r("messages", m.messages as f64, "messages");
    r("executions", m.executions as f64, "messages");
    r("appliques", m.recus as f64, "messages");
    r("perdus", m.perdus as f64, "messages");
    r("identique", f64::from(u8::from(m.identique)), "booléen");
    r("memoire_secours_identique", f64::from(u8::from(m.memoire_secours_identique)), "booléen");
    r("lat_p50", centile(&m.lat_vif, 0.5) / 1_000.0, "ms");
    r("lat_p99", centile(&m.lat_vif, 0.99) / 1_000.0, "ms");
    r("lat_max", centile(&m.lat_vif, 1.0) / 1_000.0, "ms");
    r("via_file", m.via_file as f64, "messages");
    r("etats_pont", m.etats_pont as f64, "états");
    r("etats_locaux", m.etats_locaux as f64, "états");
    r("octets_etat", m.octets_etat as f64, "o");
    r("demandes_pont", m.demandes_pont as f64, "demandes");
    r("demandes_locales", m.demandes_locales as f64, "demandes");
    r("connexions", m.connexions as f64, "connexions");
    for (k, x) in m.rattrapages_ms.iter().enumerate() {
        r(&format!("rattrapage_{}", k + 1), *x, "ms");
    }
    r("rattrapage_max", m.rattrapages_ms.iter().copied().fold(0.0, f64::max), "ms");
    r("duree", m.duree_s, "s");
    r("octets_retour", m.octets_retour as f64, "o");
    r("octets_aller", m.octets_aller as f64, "o");
    r("octets_par_message", m.octets_retour as f64 / m.messages as f64, "o");
    r("ecritures_retour", m.ecritures_retour as f64, "écritures");
    r("tampon_max", m.tampon_max as f64, "messages");
    r("pauses_routeur", m.pauses as f64, "pauses");
}

fn cas(nom: &str) -> bool {
    std::env::var("HD_CAS").map_or(true, |c| c == nom)
}

/// Le réseau local : régime, coupures courtes (dans la file), coupure longue (au-delà : l'état
/// complet), et le pont de rumqttd sous les coupures courtes.
pub fn banc_local(echelle: u32) {
    let sec = "pont";
    let base = Options { nom: "", debit_msgs: 200.0, messages: 6_000, coupures: vec![], delai_ms: 0, gigue_ms: 0, debit_lien: 0, pont: Pont::Reprise, port: 18_830, file: FILE_V2 };
    // 1. Régime : 200 messages/s (la pointe des deux maisons est de 239/s, V7 § 6.2), 30 s.
    if cas("regime") {
        let m = jouer(&Options { nom: "regime", ..base.clone() });
        publier(sec, echelle, "regime", &m);
    }
    // 2. Deux coupures (5 s et 10 s) pendant 40 s à 200 messages/s : dans la file.
    let coupures = vec![(10.0, 15.0), (25.0, 35.0)];
    if cas("coupures") {
        let m = jouer(&Options { nom: "coupures", messages: 8_000, coupures: coupures.clone(), port: 18_840, ..base.clone() });
        publier(sec, echelle, "coupures", &m);
    }
    // 3. Une coupure de 60 s à 200 messages/s : 12 000 sorties, plus que la file. L'état complet.
    if cas("longue") {
        let m = jouer(&Options { nom: "longue", messages: 16_000, coupures: vec![(10.0, 70.0)], port: 18_920, ..base.clone() });
        publier(sec, echelle, "longue", &m);
    }
    // 4. Le pont de rumqttd, sous les coupures courtes.
    if cas("rumqttd") {
        let m = jouer(&Options { nom: "rumqttd", messages: 8_000, coupures, pont: Pont::Rumqttd, port: 18_850, ..base });
        publier(sec, echelle, "rumqttd", &m);
    }
}

/// Le secours distant : un lien de 40 ms ± 20 ms à 1 Mbit/s, qui tombe 3 s toutes les 15 s ; puis
/// la cadence moyenne de la maison sur le même lien.
pub fn banc_distant(echelle: u32) {
    let sec = "distant";
    let coupures: Vec<(f64, f64)> = (1..=4).map(|k| (f64::from(k) * 15.0, f64::from(k) * 15.0 + 3.0)).collect();
    let lien = Options { nom: "", debit_msgs: 20.0, messages: 1_500, coupures, delai_ms: 40, gigue_ms: 20, debit_lien: 125_000, pont: Pont::Reprise, port: 18_860, file: FILE_V2 };
    if cas("flappant") {
        let m = jouer(&Options { nom: "flappant", ..lien.clone() });
        publier(sec, echelle, "flappant", &m);
    }
    if cas("cadence") {
        let m = jouer(&Options { nom: "cadence", debit_msgs: 2.0, messages: 120, coupures: vec![], port: 18_870, ..lien });
        publier(sec, echelle, "cadence_maison", &m);
    }
}

/// Le secours revient après une absence. `file` sans limite (la v1 : rejouer un journal) ou bornée
/// (la v2 : l'état complet), sur le réseau local, à 10 Mbit/s, à 1 Mbit/s.
fn retour(sec: &str, echelle: u32, file: u64, base_port: u16) {
    for (k, (nom, messages, debit)) in [
        ("heure_local", 6_882, 0),
        ("jour_local", 165_000, 0),
        ("jour_10mbit", 165_000, 1_250_000),
        ("jour_1mbit", 165_000, 125_000),
    ]
    .into_iter()
    .enumerate()
    {
        if !cas(nom) {
            continue;
        }
        let delai = if debit == 0 { 0 } else { 40 };
        let m = jouer(&Options { nom, debit_msgs: 0.0, messages, coupures: vec![], delai_ms: delai, gigue_ms: 0, debit_lien: debit, pont: Pont::Reprise, port: base_port + 10 * k as u16, file });
        publier(sec, echelle, nom, &m);
    }
}

/// La v1 : le secours rejoue tout ce qu'il a manqué (une file sans limite, comme un journal).
pub fn banc_rattrapage(echelle: u32) {
    retour("rattrapage", echelle, u64::MAX, 18_880);
}

/// La v2 : la file en mémoire est bornée ; au-delà, l'état complet.
pub fn banc_resync(echelle: u32) {
    retour("resync", echelle, FILE_V2, 18_940);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn so(s: u64) -> Sortie {
        Sortie { s, ..Sortie::default() }
    }

    #[test]
    fn le_rattrapeur_remet_dans_l_ordre_et_attend_la_grace() {
        let mut r = Rattrapeur::default();
        let mut prets = Vec::new();
        r.recevoir(so(1), &mut prets);
        r.recevoir(so(3), &mut prets); // val/ et wf/ ne sont pas dans le même journal du courtier
        assert_eq!(r.tic(), None, "dans la grâce, aucun trou n'est demandé");
        r.recevoir(so(2), &mut prets);
        assert_eq!(prets.iter().map(|x| x.s).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(r.trou.is_none());
        r.recevoir(so(5), &mut prets);
        std::thread::sleep(GRACE + Duration::from_millis(20));
        assert_eq!(r.tic(), Some(Demande::Troncon(4)), "après la grâce, le trou est demandé");
        assert_eq!(r.tic(), None, "une seule fois");
        // Le tronçon arrive : la fenêtre est finie, plus rien à demander.
        r.recevoir(so(4), &mut prets);
        assert_eq!(r.fin_fenetre(5, 4), None);
        assert_eq!(r.dernier, 5);
        assert!(r.fenetre.is_none());
    }

    #[test]
    fn une_fenetre_finie_en_retard_demande_la_suivante() {
        let mut r = Rattrapeur::default();
        let mut prets = Vec::new();
        assert_eq!(r.demander(), Demande::Troncon(1));
        for s in 1..=2048 {
            r.recevoir(so(s), &mut prets);
        }
        assert_eq!(r.fin_fenetre(10_000, 2048), Some(Demande::Troncon(2049)));
        // Une file qui n'a rien (le pont de rumqttd n'en tient pas) : on ne boucle pas.
        let mut v = Rattrapeur::default();
        v.recevoir(so(7), &mut prets);
        v.demander();
        assert_eq!(v.fin_fenetre(0, 0), None);
    }

    #[test]
    fn hors_file_l_etat_complet_puis_la_suite() {
        let mut r = Rattrapeur::default();
        let mut prets = Vec::new();
        r.recevoir(so(1), &mut prets);
        for s in 900..=905 {
            r.recevoir(so(s), &mut prets);
        }
        r.demander();
        // La file de l'actif ne remonte plus jusqu'à 2 : l'état complet, une seule fois.
        assert_eq!(r.fin_fenetre(905, HORS_FILE), Some(Demande::Etat));
        assert_eq!(r.fin_fenetre(905, HORS_FILE), None);
        // Il arrive en trois morceaux, dans le désordre ; il vaut le rang 902.
        assert_eq!(r.morceau(902, 2, 3, b"c"), None);
        assert_eq!(r.morceau(902, 0, 3, b"a"), None);
        assert_eq!(r.morceau(902, 1, 3, b"b"), Some((902, b"abc".to_vec())));
        prets.clear();
        r.apres_etat(902, &mut prets);
        assert_eq!(prets.iter().map(|x| x.s).collect::<Vec<_>>(), vec![903, 904, 905]);
        assert_eq!(r.dernier, 905);
        assert!(r.etat_demande.is_none());
    }
}
