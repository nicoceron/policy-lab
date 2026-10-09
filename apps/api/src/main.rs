use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

use axum::{
    body::Bytes,
    extract::{Path, Query, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{Duration, NaiveDate, TimeZone, Utc};
use rand::{distributions::Alphanumeric, seq::SliceRandom, Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

const BRANCHES: [&str; 3] = ["AUTOMOVILES", "HOGAR", "VIDA"];
const STATUSES: [&str; 2] = ["ACTIVA", "CANCELADA"];
const TS_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    tokens: Arc<Mutex<HashSet<String>>>,
    user: Arc<String>,
    password: Arc<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Policy {
    id: String,
    client_name: String,
    branch: String,
    monthly_premium_cop: i64,
    start_date: String,
    end_date: String,
    status: String,
    created_at: String,
}

enum ApiError {
    Unauthorized,
    BadRequest(String, BTreeMap<String, String>),
    NotFound(String),
    Internal,
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        eprintln!("db error: {e}");
        ApiError::Internal
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "No autorizado"})),
            ),
            ApiError::BadRequest(error, fields) => (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": error, "fields": fields})),
            ),
            ApiError::NotFound(error) => (StatusCode::NOT_FOUND, Json(json!({"error": error}))),
            ApiError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Error interno"})),
            ),
        }
        .into_response()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn policy_id(n: i64) -> String {
    format!("POL-{n:04}")
}

fn parse_policy_id(id: &str) -> Option<i64> {
    let n: i64 = id.strip_prefix("POL-")?.parse().ok()?;
    (policy_id(n) == id).then_some(n)
}

fn row_to_policy(r: &rusqlite::Row) -> rusqlite::Result<Policy> {
    Ok(Policy {
        id: policy_id(r.get(0)?),
        client_name: r.get(1)?,
        branch: r.get(2)?,
        monthly_premium_cop: r.get(3)?,
        start_date: r.get(4)?,
        end_date: r.get(5)?,
        status: r.get(6)?,
        created_at: r.get(7)?,
    })
}

const SELECT_POLICY: &str = "SELECT n, client_name, branch, monthly_premium_cop, start_date, \
                             end_date, status, created_at FROM policies";

// ---------- DB ----------

fn init_db(path: &PathBuf) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut conn = Connection::open(path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS policies (
            n INTEGER PRIMARY KEY,
            client_name TEXT NOT NULL,
            branch TEXT NOT NULL,
            monthly_premium_cop INTEGER NOT NULL,
            start_date TEXT NOT NULL,
            end_date TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL
        );",
    )?;
    seed_if_empty(&mut conn)?;
    Ok(conn)
}

fn seed_if_empty(conn: &mut Connection) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM policies", [], |r| r.get(0))?;
    if count > 0 {
        return Ok(());
    }

    const FIRST: [&str; 20] = [
        "Camila", "Santiago", "Valentina", "Mateo", "Sofía", "Julián", "Isabella", "Andrés",
        "Daniela", "Sebastián", "Mariana", "Felipe", "Laura", "Nicolás", "Paula", "Esteban",
        "Carolina", "Tomás", "Juliana", "Diego",
    ];
    const LAST: [&str; 20] = [
        "Gómez", "Rodríguez", "Martínez", "López", "García", "Hernández", "Ramírez", "Torres",
        "Vargas", "Castro", "Ortiz", "Rojas", "Moreno", "Jiménez", "Salazar", "Medina",
        "Cardona", "Restrepo", "Quintero", "Arango",
    ];

    let mut rng = ChaCha8Rng::seed_from_u64(42);
    // Exactly 14 ACTIVA / 6 CANCELADA and an even branch mix, in a seeded random order.
    let mut statuses: Vec<&str> = (0..20)
        .map(|i| if i < 14 { "ACTIVA" } else { "CANCELADA" })
        .collect();
    let mut branches: Vec<&str> = (0..20).map(|i| BRANCHES[i % 3]).collect();
    statuses.shuffle(&mut rng);
    branches.shuffle(&mut rng);

    let first_start = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let first_created = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();

    let tx = conn.transaction()?;
    for i in 0..20usize {
        let name = format!(
            "{} {}",
            FIRST[rng.gen_range(0..FIRST.len())],
            LAST[rng.gen_range(0..LAST.len())]
        );
        let premium = (rng.gen_range(40_000..=900_000i64) / 1_000) * 1_000;
        let start = first_start + Duration::days(rng.gen_range(0..540));
        let end = start + Duration::days(rng.gen_range(180..=730));
        let created = (first_created + Duration::hours(i as i64))
            .format(TS_FORMAT)
            .to_string();
        tx.execute(
            "INSERT INTO policies (n, client_name, branch, monthly_premium_cop, start_date, end_date, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                i as i64 + 1,
                name,
                branches[i],
                premium,
                start.to_string(),
                end.to_string(),
                statuses[i],
                created
            ],
        )?;
    }
    tx.commit()
}

// ---------- Auth ----------

async fn login(State(s): State<AppState>, body: Bytes) -> Result<Json<Value>, ApiError> {
    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let ok = v["username"].as_str() == Some(s.user.as_str())
        && v["password"].as_str() == Some(s.password.as_str());
    if !ok {
        return Err(ApiError::Unauthorized);
    }
    let token: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(40)
        .map(char::from)
        .collect();
    lock(&s.tokens).insert(token.clone());
    Ok(Json(json!({ "token": token })))
}

async fn require_auth(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));
    match token {
        Some(t) if lock(&s.tokens).contains(t) => next.run(req).await,
        _ => ApiError::Unauthorized.into_response(),
    }
}

// ---------- Policies ----------

async fn list_policies(
    State(s): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let status = q.get("status").map(String::as_str);
    if let Some(st) = status {
        if !STATUSES.contains(&st) {
            let fields = BTreeMap::from([(
                "status".to_string(),
                "Debe ser ACTIVA o CANCELADA".to_string(),
            )]);
            return Err(ApiError::BadRequest("Filtro inválido".into(), fields));
        }
    }

    let conn = lock(&s.db);
    let sql = format!(
        "{SELECT_POLICY}{} ORDER BY created_at DESC, n DESC",
        if status.is_some() { " WHERE status = ?1" } else { "" }
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = match status {
        Some(st) => stmt.query_map([st], row_to_policy)?,
        None => stmt.query_map([], row_to_policy)?,
    };
    let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Json(json!({ "total": items.len(), "items": items })))
}

async fn get_policy(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Policy>, ApiError> {
    let not_found = || ApiError::NotFound(format!("Póliza {id} no encontrada"));
    let n = parse_policy_id(&id).ok_or_else(not_found)?;
    let conn = lock(&s.db);
    let policy = conn
        .query_row(&format!("{SELECT_POLICY} WHERE n = ?1"), [n], row_to_policy)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => not_found(),
            e => e.into(),
        })?;
    Ok(Json(policy))
}

fn parse_date(v: &Value, field: &str, fields: &mut BTreeMap<String, String>) -> Option<NaiveDate> {
    let date = v[field]
        .as_str()
        .filter(|s| s.len() == 10)
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    if date.is_none() {
        fields.insert(
            field.into(),
            "Debe ser una fecha válida en formato YYYY-MM-DD".into(),
        );
    }
    date
}

async fn create_policy(
    State(s): State<AppState>,
    body: Bytes,
) -> Result<(StatusCode, Json<Policy>), ApiError> {
    let v = match serde_json::from_slice::<Value>(&body) {
        Ok(v) if v.is_object() => v,
        _ => {
            return Err(ApiError::BadRequest(
                "Cuerpo JSON inválido".into(),
                BTreeMap::new(),
            ))
        }
    };
    let mut fields = BTreeMap::new();

    let client_name = v["clientName"].as_str().map(str::trim).unwrap_or("");
    if client_name.is_empty() {
        fields.insert("clientName".into(), "Es obligatorio".into());
    }

    let branch = v["branch"].as_str().filter(|b| BRANCHES.contains(b));
    if branch.is_none() {
        fields.insert("branch".into(), "Debe ser AUTOMOVILES, HOGAR o VIDA".into());
    }

    let premium = v["monthlyPremiumCop"].as_i64().filter(|p| *p > 0);
    if premium.is_none() {
        fields.insert(
            "monthlyPremiumCop".into(),
            "Debe ser un entero mayor a 0".into(),
        );
    }

    let start = parse_date(&v, "startDate", &mut fields);
    let end = parse_date(&v, "endDate", &mut fields);
    if let (Some(start), Some(end)) = (start, end) {
        if end <= start {
            fields.insert("endDate".into(), "Debe ser posterior a startDate".into());
        }
    }

    let status = match v.get("status") {
        None | Some(Value::Null) => Some("ACTIVA"),
        Some(st) => st.as_str().filter(|st| STATUSES.contains(st)),
    };
    if status.is_none() {
        fields.insert("status".into(), "Debe ser ACTIVA o CANCELADA".into());
    }

    if !fields.is_empty() {
        return Err(ApiError::BadRequest("Datos inválidos".into(), fields));
    }
    // All validated above, so these unwraps cannot fail.
    let (branch, premium, start, end, status) = (
        branch.unwrap(),
        premium.unwrap(),
        start.unwrap(),
        end.unwrap(),
        status.unwrap(),
    );

    let created_at = Utc::now().format(TS_FORMAT).to_string();
    let conn = lock(&s.db);
    let n: i64 = conn.query_row("SELECT COALESCE(MAX(n), 0) + 1 FROM policies", [], |r| {
        r.get(0)
    })?;
    conn.execute(
        "INSERT INTO policies (n, client_name, branch, monthly_premium_cop, start_date, end_date, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            n,
            client_name,
            branch,
            premium,
            start.to_string(),
            end.to_string(),
            status,
            created_at
        ],
    )?;

    let policy = Policy {
        id: policy_id(n),
        client_name: client_name.to_string(),
        branch: branch.to_string(),
        monthly_premium_cop: premium,
        start_date: start.to_string(),
        end_date: end.to_string(),
        status: status.to_string(),
        created_at,
    };
    Ok((StatusCode::CREATED, Json(policy)))
}

#[tokio::main]
async fn main() {
    let db_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("polizas.db");
    let conn = init_db(&db_path).expect("no se pudo inicializar la base de datos");

    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
        tokens: Arc::new(Mutex::new(HashSet::new())),
        user: Arc::new(std::env::var("API_USER").unwrap_or_else(|_| "admin".into())),
        password: Arc::new(std::env::var("API_PASSWORD").unwrap_or_else(|_| "mundial2026".into())),
    };

    let policies = Router::new()
        .route("/api/policies", get(list_policies).post(create_policy))
        .route("/api/policies/:id", get(get_policy))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let app = Router::new()
        .route("/api/auth/login", post(login))
        .merge(policies)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("no se pudo abrir el puerto 8080");
    println!("API escuchando en http://0.0.0.0:8080");
    axum::serve(listener, app).await.unwrap();
}
