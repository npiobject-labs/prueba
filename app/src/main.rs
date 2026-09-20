use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::cors::CorsLayer;

// Fly siempre usa 8080; PUERTO solo lo fija tools/arrancar.ps1 al probar en el PC.
fn puerto() -> u16 {
    std::env::var("PUERTO")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080)
}

// RUTA_DB si existe; si no, el volumen de Fly (/data) y, en el PC, el directorio actual.
fn ruta_db() -> String {
    std::env::var("RUTA_DB").unwrap_or_else(|_| {
        if std::path::Path::new("/data").is_dir() {
            "/data/notas.db".into()
        } else {
            "notas.db".into()
        }
    })
}

type Db = Arc<Mutex<Connection>>;

fn abrir_db() -> Connection {
    let ruta = ruta_db();
    let con = Connection::open(&ruta).unwrap_or_else(|e| panic!("no se pudo abrir {ruta}: {e}"));
    con.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS notas (
           id           TEXT PRIMARY KEY,
           titulo       TEXT NOT NULL,
           contenido    TEXT NOT NULL,
           creada_en    TEXT NOT NULL,
           pendiente_ia INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS notas_creada_en ON notas(creada_en DESC);
         CREATE TABLE IF NOT EXISTS etiquetas (
           nota_id  TEXT NOT NULL REFERENCES notas(id) ON DELETE CASCADE,
           etiqueta TEXT NOT NULL,
           PRIMARY KEY (nota_id, etiqueta)
         );
         CREATE INDEX IF NOT EXISTS etiquetas_etiqueta ON etiquetas(etiqueta);",
    )
    .expect("no se pudo crear el esquema");
    println!("base de datos en {ruta}");
    con
}

#[derive(Serialize)]
struct Nota {
    id: String,
    titulo: String,
    contenido: String,
    creada_en: String,
    pendiente_ia: bool,
    etiquetas: Vec<String>,
}

#[derive(Deserialize)]
struct NuevaNota {
    contenido: String,
}

#[derive(Deserialize, Default)]
struct Filtro {
    q: Option<String>,
    etiqueta: Option<String>,
    desde: Option<String>,
    hasta: Option<String>,
}

type Respuesta<T> = Result<T, (StatusCode, Json<serde_json::Value>)>;

fn error<T>(codigo: StatusCode, msg: impl ToString) -> Respuesta<T> {
    Err((codigo, Json(json!({ "error": msg.to_string() }))))
}

fn interno<T>(e: rusqlite::Error) -> Respuesta<T> {
    eprintln!("sqlite: {e}");
    error(StatusCode::INTERNAL_SERVER_ERROR, "error de base de datos")
}

// Título de respaldo (D6): primera frase, máximo 60 caracteres. En F2 lo pone el LLM.
fn titulo_respaldo(contenido: &str) -> String {
    let primera = contenido
        .split(|c| matches!(c, '.' | '!' | '?' | '\n'))
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("");
    let mut titulo: String = primera.chars().take(60).collect();
    if primera.chars().count() > 60 {
        if let Some(i) = titulo.rfind(' ') {
            titulo.truncate(i);
        }
        titulo.push('…');
    }
    let mut c = titulo.chars();
    match c.next() {
        Some(p) => p.to_uppercase().collect::<String>() + c.as_str(),
        None => "Nota sin título".into(),
    }
}

fn etiquetas_de(con: &Connection, id: &str) -> rusqlite::Result<Vec<String>> {
    con.prepare_cached("SELECT etiqueta FROM etiquetas WHERE nota_id = ?1 ORDER BY etiqueta")?
        .query_map([id], |f| f.get(0))?
        .collect()
}

fn leer_nota(con: &Connection, id: &str) -> rusqlite::Result<Option<Nota>> {
    let fila = con
        .prepare_cached("SELECT id, titulo, contenido, creada_en, pendiente_ia FROM notas WHERE id = ?1")?
        .query_row([id], |f| {
            Ok(Nota {
                id: f.get(0)?,
                titulo: f.get(1)?,
                contenido: f.get(2)?,
                creada_en: f.get(3)?,
                pendiente_ia: f.get::<_, i64>(4)? != 0,
                etiquetas: vec![],
            })
        })
        .optional()?;
    match fila {
        Some(mut n) => {
            n.etiquetas = etiquetas_de(con, id)?;
            Ok(Some(n))
        }
        None => Ok(None),
    }
}

async fn crear_nota(State(db): State<Db>, Json(nueva): Json<NuevaNota>) -> Respuesta<impl IntoResponse> {
    let contenido = nueva.contenido.trim().to_string();
    if contenido.is_empty() {
        return error(StatusCode::BAD_REQUEST, "contenido vacío");
    }
    let con = db.lock().unwrap();
    let titulo = titulo_respaldo(&contenido);
    // id aleatorio y fecha UTC los pone SQLite: sin dependencias extra.
    let id: String = match con.query_row(
        "INSERT INTO notas (id, titulo, contenido, creada_en, pendiente_ia)
         VALUES (lower(hex(randomblob(8))), ?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 1)
         RETURNING id",
        params![titulo, contenido],
        |f| f.get(0),
    ) {
        Ok(id) => id,
        Err(e) => return interno(e),
    };
    match leer_nota(&con, &id) {
        Ok(Some(n)) => Ok((StatusCode::CREATED, Json(n))),
        Ok(None) => error(StatusCode::INTERNAL_SERVER_ERROR, "la nota no se guardó"),
        Err(e) => interno(e),
    }
}

async fn listar_notas(State(db): State<Db>, Query(f): Query<Filtro>) -> Respuesta<Json<Vec<Nota>>> {
    let con = db.lock().unwrap();
    let mut sql = String::from("SELECT id FROM notas n WHERE 1=1");
    let mut args: Vec<String> = vec![];
    if let Some(q) = f.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        // Búsqueda por palabra en título y contenido. Acentos y prefijos: F3 (FTS5).
        args.push(format!("%{}%", q.replace('%', "\\%").replace('_', "\\_")));
        let i = args.len();
        sql.push_str(&format!(" AND (n.titulo LIKE ?{i} ESCAPE '\\' OR n.contenido LIKE ?{i} ESCAPE '\\')"));
    }
    if let Some(e) = f.etiqueta.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        args.push(e.to_string());
        sql.push_str(&format!(" AND EXISTS (SELECT 1 FROM etiquetas x WHERE x.nota_id = n.id AND x.etiqueta = ?{})", args.len()));
    }
    if let Some(d) = f.desde.as_deref().filter(|s| !s.is_empty()) {
        args.push(d.to_string());
        sql.push_str(&format!(" AND n.creada_en >= ?{}", args.len()));
    }
    if let Some(h) = f.hasta.as_deref().filter(|s| !s.is_empty()) {
        args.push(h.to_string());
        sql.push_str(&format!(" AND n.creada_en <= ?{}", args.len()));
    }
    sql.push_str(" ORDER BY n.creada_en DESC LIMIT 500");

    let ids: Vec<String> = match con
        .prepare(&sql)
        .and_then(|mut s| s.query_map(rusqlite::params_from_iter(args.iter()), |f| f.get(0))?.collect())
    {
        Ok(ids) => ids,
        Err(e) => return interno(e),
    };
    let mut notas = Vec::with_capacity(ids.len());
    for id in ids {
        match leer_nota(&con, &id) {
            Ok(Some(n)) => notas.push(n),
            Ok(None) => {}
            Err(e) => return interno(e),
        }
    }
    Ok(Json(notas))
}

async fn ver_nota(State(db): State<Db>, Path(id): Path<String>) -> Respuesta<Json<Nota>> {
    let con = db.lock().unwrap();
    match leer_nota(&con, &id) {
        Ok(Some(n)) => Ok(Json(n)),
        Ok(None) => error(StatusCode::NOT_FOUND, "nota no encontrada"),
        Err(e) => interno(e),
    }
}

async fn borrar_nota(State(db): State<Db>, Path(id): Path<String>) -> Respuesta<StatusCode> {
    let con = db.lock().unwrap();
    match con.execute("DELETE FROM notas WHERE id = ?1", [&id]) {
        Ok(0) => error(StatusCode::NOT_FOUND, "nota no encontrada"),
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(e) => interno(e),
    }
}

async fn listar_etiquetas(State(db): State<Db>) -> Respuesta<Json<serde_json::Value>> {
    let con = db.lock().unwrap();
    let res: rusqlite::Result<Vec<serde_json::Value>> = con
        .prepare_cached("SELECT etiqueta, COUNT(*) FROM etiquetas GROUP BY etiqueta ORDER BY 2 DESC, 1")
        .and_then(|mut s| {
            s.query_map([], |f| Ok(json!({ "etiqueta": f.get::<_, String>(0)?, "notas": f.get::<_, i64>(1)? })))?
                .collect()
        });
    match res {
        Ok(v) => Ok(Json(json!(v))),
        Err(e) => interno(e),
    }
}

async fn raiz() -> &'static str {
    "prueba backend"
}

// Prueba "hola mundo" consumida desde Pages (otro origen): CORS abierto.
async fn holamundo() -> impl IntoResponse {
    ([(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")], "holamundo")
}

// /salud tambien se lee desde Pages (docs/holamundo.html): misma cabecera.
async fn salud() -> impl IntoResponse {
    let build = std::env::var("BUILD_ID").unwrap_or_else(|_| "dev".to_string());
    ([(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")], Json(json!({ "ok": true, "build": build })))
}

#[tokio::main]
async fn main() {
    let db: Db = Arc::new(Mutex::new(abrir_db()));

    // Todo lo consume docs/ desde Pages (otro origen): CORS abierto, con preflight para POST/DELETE.
    let app = Router::new()
        .route("/", get(raiz))
        .route("/salud", get(salud))
        .route("/holamundo", get(holamundo))
        .route("/notas", get(listar_notas).post(crear_nota))
        .route("/notas/{id}", get(ver_nota).delete(borrar_nota))
        .route("/etiquetas", get(listar_etiquetas))
        .layer(CorsLayer::permissive())
        .with_state(db);

    let direccion = format!("0.0.0.0:{}", puerto());
    let listener = tokio::net::TcpListener::bind(&direccion)
        .await
        .unwrap_or_else(|e| panic!("no se pudo abrir {direccion}: {e}"));

    println!("prueba backend escuchando en {direccion}");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("fallo del servidor HTTP");
}
