//! Schémas : la cible proposée par l'ADR D13 (Turso) et, recopiés à
//! l'identique, ceux de `crates/moli-history` et `crates/moli-energy` pour
//! les essais de compatibilité de fichier.

/// Stockage récent de Moli sur Turso : uniquement des tables rowid (Turso
/// 0.8.3 ne modifie pas les tables `WITHOUT ROWID`), un seul fichier.
///
/// `samples` est rangée par point puis par instant, comme la clé primaire
/// `(point_id, ts)` actuelle : son rowid vaut `point_id · 2^44 + ts` (voir
/// [`key`]), si bien qu'une courbe ou une purge est une plage de rowid, sans
/// index secondaire ni aller-retour index → table. `energy_hour` est rangée
/// par heure puis par compteur (rowid = `heure · 1024 + compteur`, voir
/// [`hour_key`]) : un graphique horaire est une plage, l'UPSERT d'une heure
/// une recherche de rowid. `energy_total` tient les totaux de jour et de mois
/// civils locaux dans la même transaction que l'heure.
pub const TARGET: &str = "
CREATE TABLE IF NOT EXISTS meta (
    id                INTEGER PRIMARY KEY CHECK (id = 1),
    schema_version    INTEGER NOT NULL,
    installation      TEXT    NOT NULL,
    next_seq          INTEGER NOT NULL,
    first_energy_hour INTEGER
) STRICT;
CREATE TABLE IF NOT EXISTS catalog (
    id       INTEGER PRIMARY KEY,
    point    TEXT    NOT NULL UNIQUE,
    family   TEXT    NOT NULL,
    unit     TEXT,
    hot_days INTEGER
) STRICT;
CREATE TABLE IF NOT EXISTS samples (
    id       INTEGER PRIMARY KEY CHECK (id = (point_id << 44) + ts),
    point_id INTEGER NOT NULL CHECK (point_id BETWEEN 0 AND 524287),
    ts       INTEGER NOT NULL CHECK (ts BETWEEN 0 AND 17592186044415),
    seq      INTEGER NOT NULL,
    kind     TEXT    NOT NULL,
    num      REAL,
    txt      TEXT
) STRICT;
CREATE TABLE IF NOT EXISTS meter_state (
    id             INTEGER PRIMARY KEY CHECK (id BETWEEN 0 AND 1023),
    meter          TEXT    NOT NULL UNIQUE,
    last_kwh       REAL,
    last_ts        INTEGER,
    first_ts       INTEGER,
    imported_until INTEGER
) STRICT;
CREATE TABLE IF NOT EXISTS energy_hour (
    id       INTEGER PRIMARY KEY CHECK (id = (hour / 3600000) * 1024 + meter_id),
    meter_id INTEGER NOT NULL,
    hour     INTEGER NOT NULL CHECK (hour % 3600000 = 0),
    kwh      REAL    NOT NULL,
    cost     REAL,
    seq      INTEGER NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS energy_total (
    id       INTEGER PRIMARY KEY,
    span     TEXT    NOT NULL CHECK (span IN ('day', 'month')),
    start    INTEGER NOT NULL,
    meter_id INTEGER NOT NULL,
    kwh      REAL    NOT NULL,
    cost     REAL
) STRICT;
CREATE UNIQUE INDEX IF NOT EXISTS energy_total_key ON energy_total (span, start, meter_id);
CREATE TABLE IF NOT EXISTS outbox (
    seq  INTEGER PRIMARY KEY,
    kind INTEGER NOT NULL,
    ref  INTEGER NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS archive_ack (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    durable_upto   INTEGER NOT NULL,
    published_upto INTEGER NOT NULL,
    generation     INTEGER NOT NULL,
    at             INTEGER NOT NULL
) STRICT;
INSERT OR IGNORE INTO meta (id, schema_version, installation, next_seq)
    VALUES (1, 1, '00000000000000000000000000000000', 1);
INSERT OR IGNORE INTO archive_ack (id, durable_upto, published_upto, generation, at)
    VALUES (1, 0, 0, 0, 0);
";

/// Rowid d'un relevé : 19 bits de point (524 288 points), 44 bits de
/// millisecondes depuis 1970 (jusqu'en 2527). Deux relevés d'un point à la
/// même milliseconde ont la même clé : le second est ignoré
/// (`ON CONFLICT (id) DO NOTHING`), comme `ON CONFLICT (point_id, ts) DO
/// NOTHING` aujourd'hui.
pub fn key(point: i64, ts: i64) -> i64 {
    (point << 44) + ts
}

/// Ajout d'un relevé numérique, la clé calculée en SQL.
pub const SAMPLE_INSERT: &str = "INSERT INTO samples (id, point_id, ts, seq, kind, num)
    VALUES ((?1 << 44) + ?2, ?1, ?2, ?3, 'f', ?4) ON CONFLICT (id) DO NOTHING";

/// Rowid d'une heure d'énergie : les heures se suivent, les 1 024 compteurs
/// possibles d'une même heure sont voisins.
pub fn hour_key(meter: i64, hour: i64) -> i64 {
    (hour / 3_600_000) * 1024 + meter
}

/// L'UPSERT de `moli-energy` sur la cible (le coût inconnu ne remplace pas un
/// coût connu), qui rend le rowid pour l'outbox.
pub const ENERGY_UPSERT: &str = "INSERT INTO energy_hour (id, meter_id, hour, kwh, cost, seq)
    VALUES ((?2 / 3600000) * 1024 + ?1, ?1, ?2, ?3, ?4, ?5)
    ON CONFLICT (id) DO UPDATE SET kwh = kwh + excluded.kwh,
        cost = CASE WHEN excluded.cost IS NULL THEN cost ELSE coalesce(cost, 0) + excluded.cost END,
        seq = excluded.seq
    RETURNING id";

/// Le même apport ajouté au total d'un jour ou d'un mois civil local.
pub const TOTAL_UPSERT: &str = "INSERT INTO energy_total (span, start, meter_id, kwh, cost) VALUES (?1, ?2, ?3, ?4, ?5)
    ON CONFLICT (span, start, meter_id) DO UPDATE SET kwh = kwh + excluded.kwh,
        cost = CASE WHEN excluded.cost IS NULL THEN cost ELSE coalesce(cost, 0) + excluded.cost END";

const DAY: i64 = 86_400_000;

/// Début du jour et du mois civils locaux d'une heure (ms UTC). Le banc prend
/// un décalage fixe (valeurs fictives) ; Moli prend son fuseau, comme
/// `Energy::summary` aujourd'hui.
pub fn local_day_month(hour: i64, offset: i64) -> (i64, i64) {
    let days = (hour + offset).div_euclid(DAY);
    // Jour du mois depuis le nombre de jours (algorithme civil de H. Hinnant).
    let z = days + 719_468;
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let mday = doy - (153 * mp + 2) / 5 + 1;
    (days * DAY - offset, (days - mday + 1) * DAY - offset)
}

/// Opération d'export : un échantillon ajouté.
pub const OP_SAMPLE: i64 = 1;
/// Opération d'export : l'état d'une heure d'énergie (la dernière version gagne).
pub const OP_ENERGY_HOUR: i64 = 2;

/// `crates/moli-history/src/store.rs`, constante `SCHEMA`, à l'identique.
pub const HISTORY_CURRENT: &str = "
CREATE TABLE IF NOT EXISTS points (
    id    INTEGER PRIMARY KEY,
    point TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS samples (
    point_id INTEGER NOT NULL REFERENCES points(id),
    ts       INTEGER NOT NULL,           -- ms since the Unix epoch
    kind     TEXT    NOT NULL,           -- b(ool) i(nt) f(loat) t(ext) z(null)
    num      REAL,                       -- bool as 0/1, int, float
    txt      TEXT,
    PRIMARY KEY (point_id, ts)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS samples_ts ON samples(ts);
-- What agents query: one row per recorded change.
CREATE VIEW IF NOT EXISTS history AS
    SELECT p.point AS point, s.ts AS ts, s.kind AS kind, s.num AS num, s.txt AS txt
    FROM samples s JOIN points p ON p.id = s.point_id;
";

/// `crates/moli-energy/src/store.rs`, constante `SCHEMA`, à l'identique.
pub const ENERGY_CURRENT: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    CREATE TABLE IF NOT EXISTS meter_last (
        meter TEXT PRIMARY KEY,
        kwh   REAL NOT NULL,
        ts    INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE IF NOT EXISTS energy_hour (
        meter TEXT NOT NULL,
        hour  INTEGER NOT NULL,
        kwh   REAL NOT NULL,
        cost  REAL,
        PRIMARY KEY (meter, hour)
    ) STRICT, WITHOUT ROWID;
    CREATE INDEX IF NOT EXISTS energy_hour_by_hour ON energy_hour (hour);
    -- When live recording of each meter began (its first reference
    -- reading): imported history stops there, so nothing counts twice.
    CREATE TABLE IF NOT EXISTS meter_first (
        meter TEXT PRIMARY KEY,
        ts    INTEGER NOT NULL
    ) STRICT;
    -- Databases from before meter_first: the older of the last reading
    -- and the first recorded hour.
    INSERT OR IGNORE INTO meter_first (meter, ts)
        SELECT l.meter, min(l.ts, coalesce(
            (SELECT min(h.hour) FROM energy_hour h WHERE h.meter = l.meter), l.ts))
        FROM meter_last l;
";

/// Requêtes d'écriture du code actuel, à l'identique (énergie, historique).
pub const ENERGY_RECORD_UPSERT: &str = "INSERT INTO energy_hour (meter, hour, kwh, cost) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (meter, hour) DO UPDATE SET
                    kwh = kwh + excluded.kwh,
                    cost = CASE WHEN excluded.cost IS NULL THEN cost
                                ELSE coalesce(cost, 0) + excluded.cost END";
pub const HISTORY_POINT_UPSERT: &str = "INSERT INTO points(point) VALUES (?1)
                 ON CONFLICT(point) DO UPDATE SET point = excluded.point RETURNING id";
pub const HISTORY_SAMPLE_INSERT: &str = "INSERT INTO samples(point_id, ts, kind, num, txt) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(point_id, ts) DO NOTHING";
pub const HISTORY_PRUNE: &str = "DELETE FROM samples WHERE ts < ?1
             AND ts < (SELECT MAX(s.ts) FROM samples s WHERE s.point_id = samples.point_id)";

/// Retour arrière : reconstruit `history.db` et `energy.db` (schémas actuels)
/// depuis le fichier Turso, avec SQLite seul (`sqlite3` ou rusqlite). La clé
/// `(point_id, ts)` est la même des deux côtés : rien à renuméroter.
pub const ROLLBACK_HISTORY: &str = "
INSERT OR IGNORE INTO points (id, point) SELECT id, point FROM n.catalog;
INSERT OR IGNORE INTO samples (point_id, ts, kind, num, txt)
    SELECT point_id, ts, kind, num, txt FROM n.samples;
";
pub const ROLLBACK_ENERGY: &str = "
INSERT OR REPLACE INTO energy_hour (meter, hour, kwh, cost)
    SELECT m.meter, e.hour, e.kwh, e.cost FROM n.energy_hour e JOIN n.meter_state m ON m.id = e.meter_id;
INSERT OR REPLACE INTO meter_last (meter, kwh, ts)
    SELECT meter, last_kwh, last_ts FROM n.meter_state WHERE last_ts IS NOT NULL;
INSERT OR REPLACE INTO meter_first (meter, ts)
    SELECT meter, first_ts FROM n.meter_state WHERE first_ts IS NOT NULL;
";
