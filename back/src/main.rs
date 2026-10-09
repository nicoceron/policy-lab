use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::{NaiveDate, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex},
};
use tower_http::cors::CorsLayer;

mod seed;

const BRANCHES: [&str; 3] = ["AUTOMOVILES", "HOGAR", "VIDA"];
const STATUSES: [&str; 2] = ["ACTIVA", "CANCELADA"];
const DB_PATH: &str = "data/polizas.db";
const SELECT_COLUMNS: &str = "id, client_name, branch, monthly_premium_cop, \
                              start_date, end_date, status, created_at";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub id: String,
    pub client_name: String,
    pub branch: String,
    pub monthly_premium_cop: i64,
    pub start_date: String,
    pub end_date: String,
    pub status: String,
    pub created_at: String,
}

struct AppState {
    db: Mutex<Connection>,
    tokens: Mutex<HashSet<String>>,
}

type Shared = Arc<AppState>;
type ApiError = (StatusCode, Json<Value>);

fn error(code: StatusCode, msg: &str) -> ApiError {
    (code, Json(json!({ "error": msg })))
}

#[tokio::main]
async fn main() {
    std::fs::create_dir_all("data").expect("no se pudo crear la carpeta data/");
    let conn = Connection::open(DB_PATH).expect("no se pudo abrir la base de datos");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS policies (
            id                  TEXT PRIMARY KEY,
            client_name         TEXT NOT NULL,
            branch              TEXT NOT NULL,
            monthly_premium_cop INTEGER NOT NULL,
            start_date          TEXT NOT NULL,
            end_date            TEXT NOT NULL,
            status              TEXT NOT NULL,
            created_at          TEXT NOT NULL
        );",
    )
    .expect("no se pudo crear el esquema");
    seed::seed_if_empty(&conn).expect("no se pudo sembrar la base de datos");

    let state: Shared = Arc::new(AppState {
        db: Mutex::new(conn),
        tokens: Mutex::new(HashSet::new()),
    });

    let app = Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/policies", get(list_policies).post(create_policy))
        .route("/api/policies/{id}", get(get_policy))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("no se pudo abrir el puerto 8080");
    println!("Laboratorio de Polizas API escuchando en http://0.0.0.0:8080");
    axum::serve(listener, app).await.unwrap();
}

// ------------------------------------------------------------------ auth

async fn login(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let user = std::env::var("API_USER").unwrap_or_else(|_| "admin".to_string());
    let password = std::env::var("API_PASSWORD").unwrap_or_else(|_| "mundial2026".to_string());

    let sent_user = body.get("username").and_then(Value::as_str).unwrap_or("");
    let sent_password = body.get("password").and_then(Value::as_str).unwrap_or("");
    if sent_user != user || sent_password != password {
        return Err(error(StatusCode::UNAUTHORIZED, "Credenciales invalidas"));
    }

    let token = uuid::Uuid::new_v4().to_string();
    state.tokens.lock().unwrap().insert(token.clone());
    Ok(Json(json!({ "token": token })))
}

fn check_auth(state: &Shared, headers: &HeaderMap) -> Result<(), ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");

    if !token.is_empty() && state.tokens.lock().unwrap().contains(token) {
        Ok(())
    } else {
        Err(error(StatusCode::UNAUTHORIZED, "Token invalido o ausente"))
    }
}

// -------------------------------------------------------------- policies

#[derive(Deserialize)]
struct ListQuery {
    status: Option<String>,
}

async fn list_policies(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    check_auth(&state, &headers)?;
    let conn = state.db.lock().unwrap();

    let items: Vec<Policy> = match q.status.as_deref() {
        Some(status) => {
            let sql = format!(
                "SELECT {SELECT_COLUMNS} FROM policies WHERE status = ?1 \
                 ORDER BY created_at DESC, id DESC"
            );
            let mut stmt = conn.prepare(&sql).map_err(db_error)?;
            let rows = stmt
                .query_map(rusqlite::params![status], row_to_policy)
                .map_err(db_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_error)?;
            rows
        }
        None => {
            let sql =
                format!("SELECT {SELECT_COLUMNS} FROM policies ORDER BY created_at DESC, id DESC");
            let mut stmt = conn.prepare(&sql).map_err(db_error)?;
            let rows = stmt
                .query_map([], row_to_policy)
                .map_err(db_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_error)?;
            rows
        }
    };

    Ok(Json(json!({ "total": items.len(), "items": items })))
}

async fn get_policy(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Policy>, ApiError> {
    check_auth(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let sql = format!("SELECT {SELECT_COLUMNS} FROM policies WHERE id = ?1");

    match conn.query_row(&sql, rusqlite::params![id], row_to_policy) {
        Ok(policy) => Ok(Json(policy)),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            Err(error(StatusCode::NOT_FOUND, "Poliza no encontrada"))
        }
        Err(e) => Err(db_error(e)),
    }
}

async fn create_policy(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Policy>), ApiError> {
    check_auth(&state, &headers)?;

    let mut fields: BTreeMap<String, String> = BTreeMap::new();

    let client_name = body
        .get("clientName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if client_name.is_empty() {
        fields.insert(
            "clientName".into(),
            "El nombre del cliente es obligatorio".into(),
        );
    }

    let branch = body
        .get("branch")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !BRANCHES.contains(&branch.as_str()) {
        fields.insert("branch".into(), "Debe ser AUTOMOVILES, HOGAR o VIDA".into());
    }

    let premium = body
        .get("monthlyPremiumCop")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if premium <= 0 {
        fields.insert(
            "monthlyPremiumCop".into(),
            "Debe ser un entero mayor a cero".into(),
        );
    }

    let start_date = body
        .get("startDate")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let end_date = body
        .get("endDate")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let start = parse_date(&start_date);
    let end = parse_date(&end_date);
    if start.is_none() {
        fields.insert("startDate".into(), "Fecha invalida, use YYYY-MM-DD".into());
    }
    if end.is_none() {
        fields.insert("endDate".into(), "Fecha invalida, use YYYY-MM-DD".into());
    }
    if let (Some(start), Some(end)) = (start, end) {
        if end <= start {
            fields.insert(
                "endDate".into(),
                "La fecha de fin debe ser posterior a la de inicio".into(),
            );
        }
    }

    let status = match body.get("status") {
        None | Some(Value::Null) => "ACTIVA".to_string(),
        Some(value) => {
            let status = value.as_str().unwrap_or("").to_string();
            if !STATUSES.contains(&status.as_str()) {
                fields.insert("status".into(), "Debe ser ACTIVA o CANCELADA".into());
            }
            status
        }
    };

    if !fields.is_empty() {
        return Err(validation_error(fields, "Datos de la poliza invalidos"));
    }

    let conn = state.db.lock().unwrap();
    let policy = Policy {
        id: next_id(&conn).map_err(db_error)?,
        client_name,
        branch,
        monthly_premium_cop: premium,
        start_date,
        end_date,
        status,
        created_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    };

    conn.execute(
        "INSERT INTO policies (id, client_name, branch, monthly_premium_cop,
                               start_date, end_date, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            policy.id,
            policy.client_name,
            policy.branch,
            policy.monthly_premium_cop,
            policy.start_date,
            policy.end_date,
            policy.status,
            policy.created_at,
        ],
    )
    .map_err(db_error)?;

    Ok((StatusCode::CREATED, Json(policy)))
}

// --------------------------------------------------------------- helpers

fn row_to_policy(row: &rusqlite::Row) -> rusqlite::Result<Policy> {
    Ok(Policy {
        id: row.get(0)?,
        client_name: row.get(1)?,
        branch: row.get(2)?,
        monthly_premium_cop: row.get(3)?,
        start_date: row.get(4)?,
        end_date: row.get(5)?,
        status: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn parse_date(raw: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()
}

fn next_id(conn: &Connection) -> rusqlite::Result<String> {
    let last: Option<String> = conn
        .query_row("SELECT id FROM policies ORDER BY id DESC LIMIT 1", [], |r| {
            r.get(0)
        })
        .ok();
    let n = last
        .and_then(|id| id.trim_start_matches("POL-").parse::<u32>().ok())
        .unwrap_or(0);
    Ok(format!("POL-{:04}", n + 1))
}

fn validation_error(fields: BTreeMap<String, String>, msg: &str) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": msg, "fields": fields })),
    )
}

fn db_error(e: rusqlite::Error) -> ApiError {
    eprintln!("error de base de datos: {e}");
    error(StatusCode::INTERNAL_SERVER_ERROR, "Error interno")
}
