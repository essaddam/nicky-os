//! Les façons de calculer les conditions des workflows, confrontées sur la même maison :
//! aujourd'hui (relire l'état du hub par nom), à la demande par numéro, en réseau incrémental,
//! en cycle d'automate ; puis les durées et les « attendre que », aujourd'hui et proposés.

use std::collections::HashMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::outils::{Alea, ns, poids, resultat};
use crate::valeurs::{Value, hub_state, states_aujourdhui};
use crate::workflows::{Rule, Step, genre_regle, op, preparer, tenir};

/// Le réseau incrémental : chaque règle d'état est une feuille qui garde sa vérité ; chaque « si »
/// compte ses feuilles vraies. Un changement ne touche que les feuilles de son point.
pub struct Reseau {
    pub point: Vec<u32>,
    pub op: Vec<u8>,
    pub valeur: Vec<i64>,
    pub parent: Vec<u32>,
    pub vrai: Vec<bool>,
    pub nb: Vec<u16>,
    pub vraies: Vec<u16>,
    pub parent_vrai: Vec<bool>,
    pub debut: Vec<u32>,
    pub index: Vec<u32>,
    /// Par condition : depuis quand elle est vraie (ms). Pour un « et », c'est le plus grand
    /// des instants « changé » de ses feuilles : il se recalcule, rien à ranger.
    pub depuis: Vec<u64>,
}

fn comparer(v: i64, op: u8, r: i64) -> bool {
    match op {
        0 => v == r,
        1 => v != r,
        2 => v > r,
        3 => v >= r,
        4 => v < r,
        _ => v <= r,
    }
}

impl Reseau {
    /// Les plages horaires et le soleil deviennent des points virtuels (« nuit », « 22 h–6 h en
    /// semaine »), tenus par l'échéancier : alors toute règle est une règle d'état.
    pub fn depuis(p: &crate::workflows::Programme, etat: &crate::valeurs::Etat) -> Self {
        let nb_points = etat.annuaire.len() as u32;
        let mut r = Reseau {
            point: vec![], op: vec![], valeur: vec![], parent: vec![], vrai: vec![],
            nb: vec![], vraies: vec![], parent_vrai: vec![], debut: vec![], index: vec![], depuis: vec![],
        };
        let mut virtuels: HashMap<i64, u32> = HashMap::new();
        for nd in p.noeuds.iter().filter(|n| n.op == op::SI) {
            let parent = r.nb.len() as u32;
            r.nb.push(nd.b as u16);
            r.vraies.push(0);
            r.parent_vrai.push(false);
            for regle in &p.regles[nd.a as usize..(nd.a + nd.b) as usize] {
                let (pt, o, v) = match regle.genre {
                    genre_regle::ETAT => (regle.point, regle.op, regle.valeur),
                    _ => {
                        let cle = i64::from(regle.genre) << 56 | regle.valeur << 8 | i64::from(regle.jours);
                        let n = virtuels.len() as u32;
                        (nb_points + *virtuels.entry(cle).or_insert(n), 0, 1)
                    }
                };
                r.point.push(pt);
                r.op.push(o);
                r.valeur.push(v);
                r.parent.push(parent);
                r.vrai.push(false);
            }
        }
        let total = nb_points as usize + virtuels.len();
        let mut debut = vec![0u32; total + 1];
        for &pt in &r.point {
            debut[pt as usize + 1] += 1;
        }
        for i in 0..total {
            debut[i + 1] += debut[i];
        }
        let mut cur = debut.clone();
        let mut index = vec![0u32; r.point.len()];
        for (f, &pt) in r.point.iter().enumerate() {
            index[cur[pt as usize] as usize] = f as u32;
            cur[pt as usize] += 1;
        }
        r.debut = debut;
        r.index = index;
        r.depuis = vec![0; r.nb.len()];
        r
    }

    /// L'état de vérité, recalculé d'un coup depuis la mémoire des valeurs : au démarrage, après
    /// une recompilation, après une perte de messages. Les points virtuels (plages horaires,
    /// soleil) sont lus vrais dans le banc.
    pub fn reconstruire(&mut self, etat: &crate::valeurs::Etat) {
        self.vraies.fill(0);
        self.depuis.fill(0);
        let nb_points = etat.vifs.len() as u32;
        for i in 0..self.point.len() {
            let pt = self.point[i];
            let (valeur, change) = if pt < nb_points {
                (etat.lire(pt).0, etat.vifs[pt as usize].change_ms)
            } else {
                (1, 0)
            };
            let t = comparer(valeur, self.op[i], self.valeur[i]);
            self.vrai[i] = t;
            let p = self.parent[i] as usize;
            if t {
                self.vraies[p] += 1;
                self.depuis[p] = self.depuis[p].max(change);
            }
        }
        for p in 0..self.nb.len() {
            self.parent_vrai[p] = self.vraies[p] == self.nb[p];
        }
    }

    #[inline]
    pub fn changement(&mut self, n: u32, valeur: i64, bascules: &mut Vec<u32>) {
        let (d, f) = (self.debut[n as usize] as usize, self.debut[n as usize + 1] as usize);
        for &feuille in &self.index[d..f] {
            let i = feuille as usize;
            let t = comparer(valeur, self.op[i], self.valeur[i]);
            if t != self.vrai[i] {
                self.vrai[i] = t;
                let p = self.parent[i] as usize;
                if t { self.vraies[p] += 1 } else { self.vraies[p] -= 1 }
                let pv = self.vraies[p] == self.nb[p];
                if pv != self.parent_vrai[p] {
                    self.parent_vrai[p] = pv;
                    bascules.push(p as u32);
                }
            }
        }
    }
}

pub fn banc(echelle: u32) {
    let sec = "conditions";
    let b = preparer(echelle);
    let p = &b.programme;
    let sis: Vec<u32> = (0..p.noeuds.len() as u32).filter(|&k| p.noeuds[k as usize].op == op::SI).collect();
    resultat(sec, echelle, "conditions_si", sis.len() as f64, "");

    // 1. Aujourd'hui : `holds` (run.rs:594) relit l'état du hub par nom, pour chaque règle d'état.
    let states = RwLock::new(states_aujourdhui(&b.maison));
    let regles_etat: Vec<(&str, f64)> = b
        .defs
        .iter()
        .flat_map(|a| a.graph.nodes.iter())
        .filter_map(|n| match &n.step {
            Step::If { rules, .. } => Some(rules),
            _ => None,
        })
        .flatten()
        .filter_map(|r| match r {
            Rule::State { point, value, .. } => Some((point.as_str(), value.as_f64().unwrap_or(0.0))),
            _ => None,
        })
        .collect();
    let t = ns(regles_etat.len(), 31, || {
        let mut x = 0u64;
        for (point, seuil) in &regles_etat {
            let ok = hub_state(&states, point).is_some_and(|s| match s.value {
                Value::Float(f) => f > *seuil,
                Value::Int(i) => (i as f64) > *seuil,
                _ => false,
            });
            x += u64::from(ok);
        }
        x
    });
    resultat(sec, echelle, "regle_etat_aujourdhui", t, "ns");

    // 2. Proposé, à la demande : la règle compilée lit la fiche par numéro.
    let t = ns(sis.len(), 201, || {
        let mut x = 0u64;
        for &k in &sis {
            let nd = &p.noeuds[k as usize];
            let ok = p.regles[nd.a as usize..(nd.a + nd.b) as usize].iter().all(|r| tenir(r, &b.etat, 23 * 60, 2, true));
            x += u64::from(ok);
        }
        x
    });
    resultat(sec, echelle, "si_trois_regles_a_la_demande", t, "ns");

    // 3. Réseau incrémental : mémoire, puis coût d'un changement.
    let (mut reseau, o_reseau) = poids(|| Reseau::depuis(p, &b.etat));
    resultat(sec, echelle, "memoire_reseau", o_reseau as f64, "o");
    let mut a = Alea::new(5);
    const N: usize = 2_048;
    let quelconques: Vec<u32> = (0..N).map(|_| a.sous(b.maison.points.len()) as u32).collect();
    let lus: Vec<u32> = (0..N).map(|_| b.maison.lus_mesures[a.sous(b.maison.lus_mesures.len())]).collect();
    let mut bascules = Vec::new();
    for (nom, liste) in [("quelconque", &quelconques), ("lu_par_une_regle", &lus)] {
        let t = ns(N, 101, || {
            bascules.clear();
            for (k, &n) in liste.iter().enumerate() {
                reseau.changement(n, 1_900 + (k as i64 % 300), &mut bascules);
            }
            bascules.len() as u64
        });
        resultat(sec, echelle, &format!("reseau_changement_{nom}"), t, "ns");
    }
    // La bascule « nuit » : un point virtuel lu par tous les automatismes.
    let nuit = reseau.point.iter().copied().max().unwrap();
    let t = ns(1, 101, || {
        bascules.clear();
        reseau.changement(nuit, 1, &mut bascules);
        reseau.changement(nuit, 0, &mut bascules);
        bascules.len() as u64
    });
    resultat(sec, echelle, "reseau_bascule_nuit", t / 2.0, "ns");

    // 4. Cycle d'automate : toutes les conditions, à chaque tour, comparées au tour d'avant.
    let mut avant = vec![false; sis.len()];
    let t = ns(1, 201, || {
        let mut changees = 0u64;
        for (i, &k) in sis.iter().enumerate() {
            let nd = &p.noeuds[k as usize];
            let ok = p.regles[nd.a as usize..(nd.a + nd.b) as usize].iter().all(|r| tenir(r, &b.etat, 23 * 60, 2, true));
            changees += u64::from(ok != avant[i]);
            avant[i] = ok;
        }
        changees
    });
    resultat(sec, echelle, "cycle_automate_un_tour", t / 1_000.0, "us");

    // 5. Les durées aujourd'hui : une tâche Tokio qui dort par déclencheur armé (engine.rs:hold_until).
    let h = 1_000 * echelle as usize;
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    let o_taches = rt.block_on(async {
        let avant = crate::outils::vivants();
        let mut holds: HashMap<(String, String), tokio::task::AbortHandle> = HashMap::new();
        for i in 0..h {
            let (a_id, trigger, point, why) = (
                format!("couloir-{i}"),
                "declencheur".to_string(),
                b.maison.points[i % b.maison.points.len()].nom.clone(),
                format!("Couloir {i} → oui"),
            );
            let cle = (a_id.clone(), trigger.clone());
            let tache = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(600)).await;
                black_box((a_id, trigger, point, why));
            });
            holds.insert(cle, tache.abort_handle());
        }
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        let o = crate::outils::vivants() - avant;
        for (_, t) in holds.drain() {
            t.abort();
        }
        o
    });
    drop(rt);
    resultat(sec, echelle, "duree_tache_tokio_par_attente", o_taches as f64 / h as f64, "o");
    // L'attente de la v2 : échéance 4, depuis 8, parti 8, point 4 (workflows::Attentes).
    resultat(sec, echelle, "duree_case_proposee_par_attente", (4 + 8 + 8 + 4) as f64, "o");

    // 6. « Attendre que » aujourd'hui : chaque exécution qui attend s'abonne à tout le bus du hub
    // (run.rs:429) et réévalue sa règle à chaque événement, quel que soit le point.
    let partage = Arc::new(states);
    for attentes in [10usize, 100, 400] {
      let mut essais = Vec::new();
      for _ in 0..3 {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let (cout, evenements) = rt.block_on(async {
            let (tx, _) = tokio::sync::broadcast::channel::<(u32, i64)>(1_024);
            let vus = Arc::new(AtomicU64::new(0));
            let mut taches = Vec::new();
            for w in 0..attentes {
                let mut rx = tx.subscribe();
                let s = partage.clone();
                let vus = vus.clone();
                let nom = b.maison.points[b.maison.lus_mesures[w % b.maison.lus_mesures.len()] as usize].nom.clone();
                taches.push(tokio::spawn(async move {
                    while let Ok((n, _)) = rx.recv().await {
                        if n == u32::MAX {
                            break;
                        }
                        black_box(hub_state(&s, &nom).is_some());
                        vus.fetch_add(1, Relaxed);
                    }
                }));
            }
            let e = 4_096u64;
            let debut = Instant::now();
            for k in 0..e {
                tx.send((k as u32, 1)).unwrap();
                if k % 256 == 255 {
                    while vus.load(Relaxed) < (k + 1) * attentes as u64 {
                        tokio::task::yield_now().await;
                    }
                }
            }
            while vus.load(Relaxed) < e * attentes as u64 {
                tokio::task::yield_now().await;
            }
            let d = debut.elapsed();
            tx.send((u32::MAX, 0)).unwrap();
            for t in taches {
                let _ = t.await;
            }
            (d, e)
        });
        essais.push(cout.as_nanos() as f64 / evenements as f64);
      }
      essais.sort_by(f64::total_cmp);
      resultat(sec, echelle, &format!("attendre_que_aujourdhui_{attentes}_attentes_par_evenement"), essais[1] / 1_000.0, "us");
    }
}
