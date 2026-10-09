use chrono::{Duration, NaiveDate, TimeZone, Utc};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rusqlite::Connection;

use crate::{BRANCHES, STATUSES};

/// Nombres ficticios: no hay cedulas, telefonos ni correos en los datos sembrados.
const FIRST_NAMES: [&str; 12] = [
    "Valentina",
    "Mateo",
    "Camila",
    "Santiago",
    "Luciana",
    "Emiliano",
    "Isabela",
    "Tomas",
    "Mariana",
    "Nicolas",
    "Antonia",
    "Samuel",
];

const SURNAMES: [&str; 12] = [
    "Arango",
    "Betancur",
    "Cardenas",
    "Duarte",
    "Escobar",
    "Fajardo",
    "Gaviria",
    "Herrera",
    "Idarraga",
    "Jaramillo",
    "Lozano",
    "Montoya",
];

/// Inserta exactamente 20 polizas sinteticas y deterministas (semilla 42).
/// Si ya hay datos no hace nada, por lo que nunca se duplica al reiniciar.
pub fn seed_if_empty(conn: &Connection) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM policies", [], |r| r.get(0))?;
    if count > 0 {
        return Ok(());
    }

    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let base_date = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();

    for i in 0..20u32 {
        let client_name = format!(
            "{} {}",
            FIRST_NAMES[rng.gen_range(0..FIRST_NAMES.len())],
            SURNAMES[rng.gen_range(0..SURNAMES.len())]
        );
        let branch = BRANCHES[rng.gen_range(0..BRANCHES.len())];
        let monthly_premium_cop = rng.gen_range(40..=900) * 1_000i64;
        let start_date = base_date + Duration::days(rng.gen_range(0..420));
        let end_date = start_date + Duration::days(365);
        let status = if rng.gen_bool(0.7) {
            STATUSES[0]
        } else {
            STATUSES[1]
        };
        let created_at = Utc.with_ymd_and_hms(2026, 1, 1, 8, 0, 0).unwrap()
            + Duration::hours(i as i64 * 7);

        conn.execute(
            "INSERT INTO policies (id, client_name, branch, monthly_premium_cop,
                                   start_date, end_date, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                format!("POL-{:04}", i + 1),
                client_name,
                branch,
                monthly_premium_cop,
                start_date.format("%Y-%m-%d").to_string(),
                end_date.format("%Y-%m-%d").to_string(),
                status,
                created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            ],
        )?;
    }

    println!("Base de datos sembrada con 20 polizas sinteticas (semilla 42)");
    Ok(())
}
