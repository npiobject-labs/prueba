mod llm;

use std::sync::{Arc, Mutex, OnceLock};

use axum::{
    extract::{Path, Query, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
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
         CREATE INDEX IF NOT EXISTS etiquetas_etiqueta ON etiquetas(etiqueta);
         CREATE VIRTUAL TABLE IF NOT EXISTS notas_fts USING fts5(
           id UNINDEXED, titulo, contenido,
           tokenize = 'unicode61 remove_diacritics 2'
         );
         CREATE TRIGGER IF NOT EXISTS notas_fts_ins AFTER INSERT ON notas BEGIN
           INSERT INTO notas_fts (id, titulo, contenido) VALUES (new.id, new.titulo, new.contenido);
         END;
         CREATE TRIGGER IF NOT EXISTS notas_fts_del AFTER DELETE ON notas BEGIN
           DELETE FROM notas_fts WHERE id = old.id;
         END;
         CREATE TRIGGER IF NOT EXISTS notas_fts_upd AFTER UPDATE OF titulo, contenido ON notas BEGIN
           DELETE FROM notas_fts WHERE id = old.id;
           INSERT INTO notas_fts (id, titulo, contenido) VALUES (new.id, new.titulo, new.contenido);
         END;",
    )
    .expect("no se pudo crear el esquema");
    // Notas anteriores al indice (o indice desincronizado): se reconstruye entero, es barato.
    let (n_notas, n_fts): (i64, i64) = con
        .query_row(
            "SELECT (SELECT COUNT(*) FROM notas), (SELECT COUNT(*) FROM notas_fts)",
            [],
            |f| Ok((f.get(0)?, f.get(1)?)),
        )
        .expect("no se pudo contar el indice");
    if n_notas != n_fts {
        con.execute_batch(
            "DELETE FROM notas_fts;
             INSERT INTO notas_fts (id, titulo, contenido) SELECT id, titulo, contenido FROM notas;",
        )
        .expect("no se pudo reconstruir el indice");
        println!("indice de busqueda reconstruido: {n_notas} notas");
    }
    println!("base de datos en {ruta}");
    con
}

// Texto libre -> consulta FTS5: cada palabra entre comillas (sin sintaxis especial) y con
// prefijo, unidas por AND implicito. Acentos y mayusculas los iguala el tokenizador.
fn consulta_fts(q: &str) -> Option<String> {
    let terminos: Vec<String> = q
        .split_whitespace()
        .map(|p| p.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|p| !p.is_empty())
        .map(|p| format!("\"{}\"*", p.replace('"', "\"\"")))
        .collect();
    if terminos.is_empty() {
        None
    } else {
        Some(terminos.join(" "))
    }
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
    // La nota se guarda antes de saber nada de la IA, con su titulo de respaldo
    // y marcada pendiente (D6). El candado se suelta aqui: no puede seguir
    // tomado mientras se espera al modelo.
    let id: String = {
        let con = db.lock().unwrap();
        let titulo = titulo_respaldo(&contenido);
        // id aleatorio y fecha UTC los pone SQLite: sin dependencias extra.
        match con.query_row(
            "INSERT INTO notas (id, titulo, contenido, creada_en, pendiente_ia)
             VALUES (lower(hex(randomblob(8))), ?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 1)
             RETURNING id",
            params![titulo, contenido],
            |f| f.get(0),
        ) {
            Ok(id) => id,
            Err(e) => return interno(e),
        }
    };

    completar_con_ia(&db, &id).await;

    let con = db.lock().unwrap();
    match leer_nota(&con, &id) {
        Ok(Some(n)) => Ok((StatusCode::CREATED, Json(n))),
        Ok(None) => error(StatusCode::INTERNAL_SERVER_ERROR, "la nota no se guardó"),
        Err(e) => interno(e),
    }
}

/// Reintento manual (D6): la nota ya existe y se vuelve a pedir su titulo.
async fn reintentar_ia(State(db): State<Db>, Path(id): Path<String>) -> Respuesta<Json<Nota>> {
    {
        let con = db.lock().unwrap();
        match leer_nota(&con, &id) {
            Ok(Some(_)) => {}
            Ok(None) => return error(StatusCode::NOT_FOUND, "nota no encontrada"),
            Err(e) => return interno(e),
        }
    }
    if ia().is_none() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no hay IA configurada");
    }
    if !completar_con_ia(&db, &id).await {
        return error(StatusCode::BAD_GATEWAY, "la IA no respondió; la nota sigue pendiente");
    }
    let con = db.lock().unwrap();
    match leer_nota(&con, &id) {
        Ok(Some(n)) => Ok(Json(n)),
        Ok(None) => error(StatusCode::NOT_FOUND, "nota no encontrada"),
        Err(e) => interno(e),
    }
}

async fn listar_notas(State(db): State<Db>, Query(f): Query<Filtro>) -> Respuesta<Json<Vec<Nota>>> {
    let con = db.lock().unwrap();
    let mut sql = String::from("SELECT id FROM notas n WHERE 1=1");
    let mut args: Vec<String> = vec![];
    if let Some(consulta) = f.q.as_deref().and_then(consulta_fts) {
        // Búsqueda por palabra en título y contenido: FTS5, sin acentos y por prefijo.
        args.push(consulta);
        sql.push_str(&format!(" AND n.id IN (SELECT id FROM notas_fts WHERE notas_fts MATCH ?{})", args.len()));
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

// Token de acceso (D8): secreto TOKEN_API en Fly. Sin el, la API queda abierta (aviso en /salud).
fn token_api() -> Option<&'static str> {
    static TOKEN: OnceLock<Option<String>> = OnceLock::new();
    TOKEN
        .get_or_init(|| std::env::var("TOKEN_API").ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()))
        .as_deref()
}

/// El servicio de modelos (D5), leido una sola vez. `None` = sin IA, que es un
/// modo previsto: la aplicacion guarda notas igual.
fn ia() -> Option<&'static llm::Llm> {
    static IA: OnceLock<Option<llm::Llm>> = OnceLock::new();
    IA.get_or_init(llm::Llm::del_entorno).as_ref()
}

/// Pide titulo y etiquetas para una nota ya guardada y los escribe. Devuelve si
/// lo consiguio. Cualquier fallo se queda en el log y la nota conserva su
/// titulo de respaldo con `pendiente_ia = 1` (D6): la IA nunca hace perder una
/// nota ni rompe la respuesta al usuario.
async fn completar_con_ia(db: &Db, id: &str) -> bool {
    let Some(ia) = ia() else { return false };

    // Se lee lo que hace falta y se suelta el candado antes de salir a la red.
    let datos = {
        let con = db.lock().unwrap();
        let contenido: Option<String> = con
            .query_row("SELECT contenido FROM notas WHERE id = ?1", [id], |f| f.get(0))
            .optional()
            .unwrap_or(None);
        // Las mas usadas primero: son las que conviene que reutilice.
        let existentes: Vec<String> = con
            .prepare_cached("SELECT etiqueta FROM etiquetas GROUP BY etiqueta ORDER BY COUNT(*) DESC, etiqueta")
            .and_then(|mut s| s.query_map([], |f| f.get(0))?.collect())
            .unwrap_or_default();
        contenido.map(|c| (c, existentes))
    };
    let Some((contenido, existentes)) = datos else {
        eprintln!("ia: la nota {id} ya no está");
        return false;
    };

    let sugerencia = match ia.titular(&contenido, &existentes).await {
        Ok(s) if !s.titulo.trim().is_empty() => s,
        Ok(_) => {
            eprintln!("ia: título vacío para la nota {id}");
            return false;
        }
        Err(e) => {
            eprintln!("ia: {e}");
            return false;
        }
    };

    let con = db.lock().unwrap();
    // Titulo y etiquetas entran juntos o no entra ninguno: media sugerencia
    // guardada es peor que ninguna.
    let escribir = || -> rusqlite::Result<()> {
        let tx = con.unchecked_transaction()?;
        tx.execute(
            "UPDATE notas SET titulo = ?1, pendiente_ia = 0 WHERE id = ?2",
            params![sugerencia.titulo, id],
        )?;
        tx.execute("DELETE FROM etiquetas WHERE nota_id = ?1", [id])?;
        {
            let mut ins = tx.prepare("INSERT INTO etiquetas (nota_id, etiqueta) VALUES (?1, ?2)")?;
            for etiqueta in &sugerencia.etiquetas {
                ins.execute(params![id, etiqueta])?;
            }
        }
        tx.commit()
    };
    match escribir() {
        Ok(()) => true,
        Err(e) => {
            eprintln!("ia: no se pudo guardar la sugerencia de {id}: {e}");
            false
        }
    }
}

fn iguales_en_tiempo_constante(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn exigir_token(req: Request, next: Next) -> Response {
    if let Some(esperado) = token_api() {
        let valido = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|v| iguales_en_tiempo_constante(v.trim(), esperado))
            .unwrap_or(false);
        if !valido {
            return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "token de acceso requerido" }))).into_response();
        }
    }
    next.run(req).await
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
    (
        [(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")],
        Json(json!({ "ok": true, "build": build, "token": token_api().is_some(), "ia": ia().is_some() })),
    )
}

#[tokio::main]
async fn main() {
    let db: Db = Arc::new(Mutex::new(abrir_db()));

    // Todo lo consume docs/ desde Pages (otro origen): CORS abierto, con preflight para POST/DELETE.
    // Las rutas de datos exigen el token si TOKEN_API esta definido; /salud y /holamundo no.
    let protegidas = Router::new()
        .route("/notas", get(listar_notas).post(crear_nota))
        .route("/notas/{id}", get(ver_nota).delete(borrar_nota))
        .route("/notas/{id}/reintentar-ia", post(reintentar_ia))
        .route("/etiquetas", get(listar_etiquetas))
        .route_layer(middleware::from_fn(exigir_token));
    let app = Router::new()
        .route("/", get(raiz))
        .route("/salud", get(salud))
        .route("/holamundo", get(holamundo))
        .merge(protegidas)
        .layer(CorsLayer::permissive())
        .with_state(db);
    match token_api() {
        Some(_) => println!("token de acceso activo"),
        None => println!("AVISO: sin TOKEN_API, la API de notas queda abierta"),
    }
    if ia().is_none() {
        println!("AVISO: sin LLM_API_KEY, las notas se guardan con título de respaldo y sin etiquetas");
    }

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
