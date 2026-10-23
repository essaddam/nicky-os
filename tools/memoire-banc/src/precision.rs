//! La précision du temps (mémoire v2, § 10.9) : ce que coûte lire l'heure, de combien un réveil est
//! en retard (au repos et sous charge), ce que coûterait un balayage à la seconde, au dixième ou à
//! la milliseconde, et ce que coûte le moteur du temps proposé (réveil au plus proche).

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crate::calendrier::{Calendrier, Evenement, Fiche, genre, soleil};
use crate::outils::{ns, poids, resultat};

fn centiles(mut v: Vec<u64>) -> (u64, u64, u64) {
    v.sort_unstable();
    let n = v.len();
    (v[n / 2], v[(n * 99 / 100).min(n - 1)], v[n - 1])
}

/// Le retard d'un réveil : on demande `d`, on mesure ce qu'on obtient de plus (µs).
fn retards(d: Duration, n: usize, tokio_rt: Option<&tokio::runtime::Runtime>) -> (u64, u64, u64) {
    let v: Vec<u64> = (0..n)
        .map(|_| {
            let t0 = Instant::now();
            match tokio_rt {
                Some(rt) => rt.block_on(async { tokio::time::sleep(d).await }),
                None => std::thread::sleep(d),
            }
            t0.elapsed().saturating_sub(d).as_micros() as u64
        })
        .collect();
    centiles(v)
}

/// Le balayage d'aujourd'hui (engine.rs, `on_clock`) : à chaque tour, pour chaque déclencheur
/// horaire, relire l'heure en texte, calculer le soleil, comparer à la minute courante.
fn balayage_aujourdhui(n: usize) -> impl FnMut() -> u64 {
    let tz = jiff::tz::TimeZone::get("Europe/Paris").unwrap();
    let heures: Vec<String> = (0..n).map(|i| format!("{:02}:{:02}", (i * 7) % 24, (i * 13) % 60)).collect();
    move || {
        let now = jiff::Timestamp::now().to_zoned(tz.clone());
        let minute = now.hour() as u32 * 60 + now.minute() as u32;
        let mut x = 0u64;
        for (i, at) in heures.iter().enumerate() {
            let due = match i % 5 {
                // 3 sur 5 à heure fixe : l'heure relue depuis le texte.
                0..3 => crate::moteur::secondes_de(at).map(|s| s / 60) == Some(minute),
                // 1 sur 5 au soleil : le lever et le coucher recalculés.
                3 => soleil(now.date(), 48.85, 2.35).is_some_and(|(_, c)| {
                    let z = jiff::Timestamp::from_millisecond(c as i64).unwrap().to_zoned(tz.clone());
                    z.hour() as u32 * 60 + z.minute() as u32 == minute
                }),
                // 1 sur 5 périodique.
                _ => minute % 15 == 0,
            };
            x += u64::from(due);
        }
        black_box(x)
    }
}

pub fn banc(echelle: u32) {
    let sec = "precision";
    // 1. Lire l'heure.
    let t = ns(4_096, 31, || {
        let mut x = 0u64;
        for _ in 0..4_096 {
            x = x.wrapping_add(Instant::now().elapsed().as_nanos() as u64);
        }
        x
    });
    resultat(sec, echelle, "lire_horloge_monotone", t, "ns");
    let t = ns(4_096, 31, || {
        let mut x = 0u64;
        for _ in 0..4_096 {
            x = x.wrapping_add(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos() as u64);
        }
        x
    });
    resultat(sec, echelle, "lire_horloge_murale", t, "ns");

    // 2. Le retard d'un réveil, au repos puis avec deux fils qui occupent les deux CPU.
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    for (charge, nom) in [(false, "repos"), (true, "charge")] {
        let stop = Arc::new(AtomicBool::new(false));
        let fils: Vec<_> = if charge {
            (0..2)
                .map(|_| {
                    let stop = stop.clone();
                    std::thread::spawn(move || {
                        let mut x = 0u64;
                        while !stop.load(Ordering::Relaxed) {
                            x = black_box(x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1));
                        }
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        for (d_ms, n) in [(1u64, 200usize), (10, 100), (100, 30), (1_000, 5)] {
            let (p50, p99, max) = retards(Duration::from_millis(d_ms), n, None);
            resultat(sec, echelle, &format!("retard_thread_{d_ms}ms_{nom}_p50"), p50 as f64, "us");
            resultat(sec, echelle, &format!("retard_thread_{d_ms}ms_{nom}_p99"), p99 as f64, "us");
            resultat(sec, echelle, &format!("retard_thread_{d_ms}ms_{nom}_max"), max as f64, "us");
            let (p50, p99, max) = retards(Duration::from_millis(d_ms), n, Some(&rt));
            resultat(sec, echelle, &format!("retard_tokio_{d_ms}ms_{nom}_p50"), p50 as f64, "us");
            resultat(sec, echelle, &format!("retard_tokio_{d_ms}ms_{nom}_p99"), p99 as f64, "us");
            resultat(sec, echelle, &format!("retard_tokio_{d_ms}ms_{nom}_max"), max as f64, "us");
        }
        stop.store(true, Ordering::Relaxed);
        for f in fils {
            f.join().unwrap();
        }
    }

    // 3. Le balayage d'aujourd'hui : un tour, pour 400 × échelle déclencheurs horaires.
    let n = 400 * echelle as usize;
    let mut tour = balayage_aujourdhui(n);
    let t = ns(1, 51, &mut tour);
    resultat(sec, echelle, "balayage_aujourdhui_un_tour", t / 1_000.0, "us");

    // 4. Le moteur du temps proposé : la mémoire des fiches, le coût d'un calcul d'occurrence.
    let maintenant = 1_792_778_400_000u64;
    let (mut cal, o_cal) = poids(|| {
        let mut c = Calendrier::new("Europe/Paris", 48.85, 2.35);
        for i in 0..n {
            let f = match i % 5 {
                0..3 => Fiche { genre: genre::HEURE, a: ((i * 7) % 24 * 3_600 + (i * 13) % 60 * 60) as u32, ..Fiche::default() },
                3 => Fiche { genre: genre::SOLEIL, coucher: 1, decalage_s: -1_800, ..Fiche::default() },
                _ => Fiche { genre: genre::PERIODE, a: 900, ..Fiche::default() },
            };
            c.ajouter(f);
        }
        c
    });
    resultat(sec, echelle, "memoire_fiches_horaires", o_cal as f64, "o");
    let t = ns(n, 11, || {
        cal.armer_tout(maintenant, &|_| None);
        cal.prochain()
    });
    resultat(sec, echelle, "calculer_une_occurrence", t, "ns");
    // Une journée réelle : tout ce qui part en 24 h, réveil au plus proche (sans dormir).
    cal.armer_tout(maintenant, &|_| None);
    let debut = Instant::now();
    let (mut reveils, mut parties) = (0u64, 0u64);
    loop {
        let t = cal.prochain();
        if t > maintenant + 86_400_000 {
            break;
        }
        reveils += 1;
        parties += cal.passage(t, &|_| None).len() as u64;
    }
    let jour_us = debut.elapsed().as_secs_f64() * 1e6;
    resultat(sec, echelle, "journee_reveils", reveils as f64, "");
    resultat(sec, echelle, "journee_occurrences", parties as f64, "");
    resultat(sec, echelle, "journee_cpu_moteur_du_temps", jour_us / 1_000.0, "ms");

    // 5. Un agenda : la mémoire d'un événement, et relire un agenda de 7 jours (réarmer).
    let evts = |k: u32| -> Vec<Evenement> {
        (0..k)
            .map(|j| Evenement {
                debut_ms: maintenant + u64::from(j) * 3_600_000,
                fin_ms: maintenant + u64::from(j) * 3_600_000 + 1_800_000,
                agenda: 1,
                numero: j,
            })
            .collect()
    };
    for (k, nom) in [(20u32, "jour"), (150, "semaine"), (3_000, "annee")] {
        let mut c = Calendrier::new("Europe/Paris", 48.85, 2.35);
        for _ in 0..10 {
            c.ajouter(Fiche { genre: genre::AGENDA, a: 1, decalage_s: -600, ..Fiche::default() });
        }
        let (_, o) = poids(|| c.agenda(1, evts(k), maintenant));
        let t = ns(1, 21, || c.agenda(1, evts(k), maintenant).len() as u64);
        resultat(sec, echelle, &format!("agenda_{nom}_evenements"), f64::from(k), "");
        resultat(sec, echelle, &format!("agenda_{nom}_memoire"), (c.evenements.capacity() * std::mem::size_of::<Evenement>()) as f64, "o");
        resultat(sec, echelle, &format!("agenda_{nom}_relire_et_rearmer_10_rappels"), t / 1_000.0, "us");
        black_box(o);
    }
}
