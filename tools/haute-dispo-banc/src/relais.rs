//! Un relais TCP qui imite le lien entre les deux machines : coupure, délai, gigue, débit. Il compte
//! les octets et les écritures dans chaque sens.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc};
use tokio::time::{Duration, Instant, sleep_until};

#[derive(Default)]
pub struct Lien {
    pub coupe: AtomicBool,
    pub generation: AtomicU64,
    pub coupure: Notify,
    pub delai_ms: AtomicU32,
    pub gigue_ms: AtomicU32,
    /// Octets par seconde dans chaque sens ; 0 : sans limite.
    pub debit: AtomicU64,
    /// Secours → actif (demandes, accusés).
    pub octets_aller: AtomicU64,
    /// Actif → secours (le flux, les tronçons).
    pub octets_retour: AtomicU64,
    pub ecritures_retour: AtomicU64,
}

impl Lien {
    /// Coupe le lien : les connexions ouvertes tombent, les nouvelles sont refusées.
    pub fn couper(&self) {
        self.coupe.store(true, Relaxed);
        self.generation.fetch_add(1, Relaxed);
        self.coupure.notify_waiters();
    }
    pub fn retablir(&self) {
        self.coupe.store(false, Relaxed);
    }
    pub fn regler(&self, delai_ms: u32, gigue_ms: u32, debit_octets_s: u64) {
        self.delai_ms.store(delai_ms, Relaxed);
        self.gigue_ms.store(gigue_ms, Relaxed);
        self.debit.store(debit_octets_s, Relaxed);
    }
}

/// Relaie `ecoute` vers `cible` jusqu'à la fin du programme.
pub async fn relayer(ecoute: TcpListener, cible: SocketAddr, lien: Arc<Lien>) {
    let diag = std::env::var_os("HD_DIAG").is_some();
    let t0 = Instant::now();
    loop {
        let Ok((client, _)) = ecoute.accept().await else { continue };
        if lien.coupe.load(Relaxed) {
            drop(client);
            continue;
        }
        let serveur = TcpStream::connect(cible).await;
        if diag {
            eprintln!("relais : connexion à {:.1} s, vers l'actif : {}", t0.elapsed().as_secs_f64(), if serveur.is_ok() { "ouverte" } else { "refusée" });
        }
        let Ok(serveur) = serveur else { continue };
        let _ = client.set_nodelay(true);
        let _ = serveur.set_nodelay(true);
        let g = lien.generation.load(Relaxed);
        let (cr, cw) = client.into_split();
        let (sr, sw) = serveur.into_split();
        tokio::spawn(pompe(cr, sw, lien.clone(), g, false));
        tokio::spawn(pompe(sr, cw, lien.clone(), g, true));
    }
}

/// Un sens du lien : lire, retarder (sans jamais réordonner), limiter le débit, écrire.
async fn pompe(mut r: OwnedReadHalf, mut w: OwnedWriteHalf, lien: Arc<Lien>, g: u64, retour: bool) {
    let (tx, mut rx) = mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    let l2 = lien.clone();
    let ecrivain = tokio::spawn(async move {
        let mut libre = Instant::now();
        while let Some((quand, octets)) = rx.recv().await {
            let debit = l2.debit.load(Relaxed);
            let debut = quand.max(libre);
            let fin = if debit == 0 { debut } else { debut + Duration::from_secs_f64(octets.len() as f64 / debit as f64) };
            tokio::select! {
                _ = sleep_until(fin) => {}
                _ = l2.coupure.notified() => break,
            }
            if l2.generation.load(Relaxed) != g || w.write_all(&octets).await.is_err() {
                break;
            }
            libre = fin;
            if retour {
                l2.octets_retour.fetch_add(octets.len() as u64, Relaxed);
                l2.ecritures_retour.fetch_add(1, Relaxed);
            } else {
                l2.octets_aller.fetch_add(octets.len() as u64, Relaxed);
            }
        }
        let _ = w.shutdown().await;
    });
    let mut buf = vec![0u8; 16 * 1024];
    let mut dernier = Instant::now();
    let mut alea = 0x2545_F491_4F6C_DD1Du64 ^ g ^ u64::from(retour);
    loop {
        let n = tokio::select! {
            n = r.read(&mut buf) => n,
            _ = lien.coupure.notified() => break,
        };
        let Ok(n) = n else { break };
        if n == 0 || lien.generation.load(Relaxed) != g {
            break;
        }
        alea ^= alea << 13;
        alea ^= alea >> 7;
        alea ^= alea << 17;
        let gigue = lien.gigue_ms.load(Relaxed);
        let d = lien.delai_ms.load(Relaxed) + if gigue == 0 { 0 } else { (alea % u64::from(gigue + 1)) as u32 };
        // Un lien TCP ne réordonne pas : un morceau ne part jamais avant le précédent.
        let quand = (Instant::now() + Duration::from_millis(u64::from(d))).max(dernier);
        dernier = quand;
        if tx.send((quand, buf[..n].to_vec())).is_err() {
            break;
        }
    }
    drop(tx);
    let _ = ecrivain.await;
}
