//! Banc du stockage récent de Moli sur Turso (ADR D13). Voir README.md.

mod compat;
mod crash;
mod perf;
mod schema;
mod util;

use util::R;

const USAGE: &str = "storage-bench <mode> …
  compat <dossier>                      fonctions nécessaires, prouvées à la version figée
  crash <dossier> <tours>               kill -9 pendant des écritures, puis invariants
  fsync-probe <base> <OFF|NORMAL|FULL> <n>   n validations (à lancer sous strace -c)
  load <forme.json> <base>              charge ×2 (relevés, outbox, fenêtre d'énergie)
  reads <forme.json> <base>             lectures du dashboard (p50/p95/p99)
  burst <forme.json> <base> <op/s>      5 lecteurs + un rédacteur durable
  purge <forme.json> <base>             purge d'un jour, ancrages, outbox
  reopen <forme.json> <base>            réouverture, première courbe, integrity_check
  migrate <history.db> <energy.db> <cible>   copie des fichiers actuels vers la cible";

fn main() -> R<()> {
    let args: Vec<String> = std::env::args().collect();
    let a = |i: usize| args.get(i).map(String::as_str).ok_or(USAGE);
    // Comme Moli au démarrage : le dossier temporaire de Turso (base
    // temporaire, VACUUM) sur le volume de données, l'image `FROM scratch`
    // n'ayant pas de /tmp.
    if let Some(dir) = std::env::var_os("TURSO_TMPDIR") {
        std::fs::create_dir_all(dir)?;
    }
    match a(1)? {
        "compat" => compat::run(a(2)?),
        "hold" => compat::hold(a(2)?),
        "crash" => crash::run(a(2)?, a(3)?.parse()?),
        "crash-writer" => crash::writer(a(2)?, a(3)?.parse()?),
        "fsync-probe" => crash::fsync_probe(a(2)?, a(3)?, a(4)?.parse()?),
        "load" => perf::load(&perf::shape(a(2)?)?, a(3)?),
        "reads" => perf::reads(&perf::shape(a(2)?)?, a(3)?),
        "burst" => perf::burst(&perf::shape(a(2)?)?, a(3)?, a(4)?.parse()?),
        "purge" => perf::purge(&perf::shape(a(2)?)?, a(3)?),
        "reopen" => perf::reopen(&perf::shape(a(2)?)?, a(3)?),
        "migrate" => compat::migrate(a(2)?, a(3)?, a(4)?).map(|n| println!("{{\"migrated_ops\":{n}}}")),
        _ => Err(USAGE.into()),
    }
}

/// io_uring est-il permis ici ? Sous le profil seccomp par défaut de Docker,
/// `io_uring_setup` est refusé (EPERM) : Turso doit fonctionner sans.
pub fn probe_io_uring() -> String {
    #[cfg(target_os = "linux")]
    {
        // struct io_uring_params : 120 octets, à zéro.
        let mut params = [0u8; 120];
        // SAFETY: appel système documenté, tampon de la taille attendue.
        let fd = unsafe { libc::syscall(libc::SYS_io_uring_setup, 1u32, params.as_mut_ptr()) };
        if fd >= 0 {
            // SAFETY: descripteur que l'on vient d'obtenir.
            unsafe { libc::close(fd as i32) };
            "allowed".into()
        } else {
            format!("refused: {}", std::io::Error::last_os_error())
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        "not linux".into()
    }
}
