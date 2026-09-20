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
         END;
         CREATE TABLE IF NOT EXISTS documentos (
           id             TEXT PRIMARY KEY,
           titulo         TEXT NOT NULL,
           instruccion    TEXT NOT NULL DEFAULT '',
           texto          TEXT NOT NULL DEFAULT '',
           estado         TEXT NOT NULL,
           error          TEXT,
           editado        INTEGER NOT NULL DEFAULT 0,
           creado_en      TEXT NOT NULL,
           actualizado_en TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS documentos_creado_en ON documentos(creado_en DESC);
         -- nota_id sin clave ajena a proposito (D12): borrar una nota no borra
         -- el documento que salio de ella, y por eso se copia aqui su titulo.
         CREATE TABLE IF NOT EXISTS documento_notas (
           documento_id TEXT NOT NULL REFERENCES documentos(id) ON DELETE CASCADE,
           nota_id      TEXT NOT NULL,
           orden        INTEGER NOT NULL,
           titulo       TEXT NOT NULL,
           creada_en    TEXT NOT NULL,
           PRIMARY KEY (documento_id, nota_id)
         );
         CREATE INDEX IF NOT EXISTS documento_notas_nota ON documento_notas(nota_id);",
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
    // La maquina de Fly se suspende y se reinicia sola: un documento que se
    // estaba generando en ese momento no lo termina nadie. Se marca fallido
    // para que la app ofrezca el boton de regenerar en vez de girar para
    // siempre (D11).
    match con.execute(
        "UPDATE documentos SET estado = 'fallido', error = ?1,
         actualizado_en = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE estado = 'pendiente'",
        ["la generacion se interrumpio al reiniciarse el servicio"],
    ) {
        Ok(0) => {}
        Ok(n) => println!("{n} documento(s) a medias marcados para regenerar"),
        Err(e) => eprintln!("no se pudieron revisar los documentos pendientes: {e}"),
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
    /// Rastro nota -> documento (D12). Solo se rellena en el detalle: en la
    /// lista costaria una consulta por nota y alli no se enseña.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    documentos: Vec<DocumentoDeNota>,
}

#[derive(Deserialize)]
struct NuevaNota {
    contenido: String,
}

/// Tope de notas por documento (D11). Veinte notas dictadas dan de sobra para
/// un encargo; mas es senal de que hace falta filtrar antes, no generar mas.
const MAXIMO_NOTAS_DOCUMENTO: usize = 20;

#[derive(Serialize)]
struct Documento {
    id: String,
    titulo: String,
    instruccion: String,
    texto: String,
    /// `pendiente` mientras el modelo trabaja, luego `listo` o `fallido`.
    estado: String,
    error: Option<String>,
    editado: bool,
    creado_en: String,
    actualizado_en: String,
    notas: Vec<NotaDeDocumento>,
}

/// Una nota vista desde su documento. `existe` es false si se borro despues:
/// el documento sobrevive con el titulo que tenia (D12).
#[derive(Serialize)]
struct NotaDeDocumento {
    id: String,
    titulo: String,
    creada_en: String,
    existe: bool,
}

/// Un documento visto desde una de sus notas.
#[derive(Serialize)]
struct DocumentoDeNota {
    id: String,
    titulo: String,
    estado: String,
}

#[derive(Deserialize)]
struct NuevoDocumento {
    notas: Vec<String>,
    #[serde(default)]
    instruccion: String,
}

#[derive(Deserialize)]
struct TextoEditado {
    texto: String,
}

#[derive(Deserialize, Default)]
struct Regeneracion {
    instruccion: Option<String>,
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
        .split(['.', '!', '?', '\n'])
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
        .prepare_cached(
            "SELECT id, titulo, contenido, creada_en, pendiente_ia FROM notas WHERE id = ?1",
        )?
        .query_row([id], |f| {
            Ok(Nota {
                id: f.get(0)?,
                titulo: f.get(1)?,
                contenido: f.get(2)?,
                creada_en: f.get(3)?,
                pendiente_ia: f.get::<_, i64>(4)? != 0,
                etiquetas: vec![],
                documentos: vec![],
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

async fn crear_nota(
    State(db): State<Db>,
    Json(nueva): Json<NuevaNota>,
) -> Respuesta<impl IntoResponse> {
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
        return error(
            StatusCode::BAD_GATEWAY,
            "la IA no respondió; la nota sigue pendiente",
        );
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
        sql.push_str(&format!(
            " AND n.id IN (SELECT id FROM notas_fts WHERE notas_fts MATCH ?{})",
            args.len()
        ));
    }
    if let Some(e) = f
        .etiqueta
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push(e.to_string());
        sql.push_str(&format!(
            " AND EXISTS (SELECT 1 FROM etiquetas x WHERE x.nota_id = n.id AND x.etiqueta = ?{})",
            args.len()
        ));
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

    let ids: Vec<String> = match con.prepare(&sql).and_then(|mut s| {
        s.query_map(rusqlite::params_from_iter(args.iter()), |f| f.get(0))?
            .collect()
    }) {
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
        Ok(Some(mut n)) => {
            // Rastro nota -> documento (D12): desde la nota se ve en que
            // documentos entro, igual que desde el documento se ven sus notas.
            n.documentos = match con
                .prepare_cached(
                    "SELECT d.id, d.titulo, d.estado FROM documento_notas dn
                     JOIN documentos d ON d.id = dn.documento_id
                     WHERE dn.nota_id = ?1 ORDER BY d.creado_en DESC",
                )
                .and_then(|mut st| {
                    st.query_map([&id], |f| {
                        Ok(DocumentoDeNota {
                            id: f.get(0)?,
                            titulo: f.get(1)?,
                            estado: f.get(2)?,
                        })
                    })?
                    .collect()
                }) {
                Ok(v) => v,
                Err(e) => return interno(e),
            };
            Ok(Json(n))
        }
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

// ---------------------------------------------------------------------------
// Documentos (D11): varias notas -> un encargo de trabajo redactado por la IA.
// ---------------------------------------------------------------------------

fn leer_documento(con: &Connection, id: &str) -> rusqlite::Result<Option<Documento>> {
    let doc = con
        .prepare_cached(
            "SELECT id, titulo, instruccion, texto, estado, error, editado, creado_en, actualizado_en
             FROM documentos WHERE id = ?1",
        )?
        .query_row([id], |f| {
            Ok(Documento {
                id: f.get(0)?,
                titulo: f.get(1)?,
                instruccion: f.get(2)?,
                texto: f.get(3)?,
                estado: f.get(4)?,
                error: f.get(5)?,
                editado: f.get::<_, i64>(6)? != 0,
                creado_en: f.get(7)?,
                actualizado_en: f.get(8)?,
                notas: vec![],
            })
        })
        .optional()?;
    match doc {
        Some(mut d) => {
            // `existe` sale de mirar la nota: si se borro, el documento la
            // sigue enseñando con el titulo copiado, pero sin enlace (D12).
            d.notas = con
                .prepare_cached(
                    "SELECT dn.nota_id, dn.titulo, dn.creada_en, n.id IS NOT NULL
                     FROM documento_notas dn LEFT JOIN notas n ON n.id = dn.nota_id
                     WHERE dn.documento_id = ?1 ORDER BY dn.orden",
                )?
                .query_map([id], |f| {
                    Ok(NotaDeDocumento {
                        id: f.get(0)?,
                        titulo: f.get(1)?,
                        creada_en: f.get(2)?,
                        existe: f.get::<_, i64>(3)? != 0,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Some(d))
        }
        None => Ok(None),
    }
}

async fn crear_documento(
    State(db): State<Db>,
    Json(nuevo): Json<NuevoDocumento>,
) -> Respuesta<impl IntoResponse> {
    // Sin IA no hay documento que valga: aqui no cabe el respaldo que si tiene
    // el titulo de una nota (D6), asi que se dice y no se crea nada.
    if ia().is_none() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no hay IA configurada");
    }
    // Repetir una nota en la seleccion no es un error del usuario, es un dedo
    // en el movil: se quita y punto.
    let mut ids: Vec<String> = Vec::new();
    for id in nuevo.notas {
        let id = id.trim().to_string();
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return error(StatusCode::BAD_REQUEST, "no hay notas seleccionadas");
    }
    if ids.len() > MAXIMO_NOTAS_DOCUMENTO {
        return error(
            StatusCode::BAD_REQUEST,
            format!(
                "demasiadas notas ({}), el maximo es {MAXIMO_NOTAS_DOCUMENTO}",
                ids.len()
            ),
        );
    }
    let instruccion = nuevo
        .instruccion
        .trim()
        .chars()
        .take(500)
        .collect::<String>();

    let id = {
        let con = db.lock().unwrap();
        // Las notas entran en el documento de la mas antigua a la mas reciente:
        // asi el modelo lee la historia en el orden en que se dicto.
        let mut fuentes: Vec<(String, String, String)> = Vec::with_capacity(ids.len());
        for nota_id in &ids {
            let fila = con
                .prepare_cached("SELECT id, titulo, creada_en FROM notas WHERE id = ?1")
                .and_then(|mut st| {
                    st.query_row([nota_id], |f| {
                        Ok((
                            f.get::<_, String>(0)?,
                            f.get::<_, String>(1)?,
                            f.get::<_, String>(2)?,
                        ))
                    })
                    .optional()
                });
            match fila {
                Ok(Some(f)) => fuentes.push(f),
                Ok(None) => {
                    return error(
                        StatusCode::BAD_REQUEST,
                        format!("la nota {nota_id} no existe"),
                    )
                }
                Err(e) => return interno(e),
            }
        }
        fuentes.sort_by(|a, b| a.2.cmp(&b.2));

        let crear = || -> rusqlite::Result<String> {
            let tx = con.unchecked_transaction()?;
            let id: String = tx.query_row(
                "INSERT INTO documentos (id, titulo, instruccion, texto, estado, editado, creado_en, actualizado_en)
                 VALUES (lower(hex(randomblob(8))), ?1, ?2, '', 'pendiente', 0,
                         strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                 RETURNING id",
                params![titulo_provisional(&fuentes), instruccion],
                |f| f.get(0),
            )?;
            {
                let mut ins = tx.prepare(
                    "INSERT INTO documento_notas (documento_id, nota_id, orden, titulo, creada_en)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )?;
                for (orden, (nota_id, titulo, creada_en)) in fuentes.iter().enumerate() {
                    ins.execute(params![id, nota_id, orden as i64, titulo, creada_en])?;
                }
            }
            tx.commit()?;
            Ok(id)
        };
        match crear() {
            Ok(id) => id,
            Err(e) => return interno(e),
        }
    };

    // Redactar puede tardar un minuto largo: se contesta ya y el movil pregunta
    // luego por el documento. Si se cierra la pestaña, sigue generandose (D11).
    lanzar_generacion(db.clone(), id.clone());

    let con = db.lock().unwrap();
    match leer_documento(&con, &id) {
        Ok(Some(d)) => Ok((StatusCode::CREATED, Json(d))),
        Ok(None) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "el documento no se guardo",
        ),
        Err(e) => interno(e),
    }
}

/// Titulo mientras el modelo no devuelve el suyo: el de la primera nota y
/// cuantas mas hay. Nunca se enseña un documento sin nombre.
fn titulo_provisional(fuentes: &[(String, String, String)]) -> String {
    match fuentes.len() {
        0 => "Documento".to_string(),
        1 => fuentes[0].1.chars().take(80).collect(),
        n => {
            let primera: String = fuentes[0].1.chars().take(60).collect();
            let resto = n - 1;
            if resto == 1 {
                format!("{primera} y una nota mas")
            } else {
                format!("{primera} y {resto} notas mas")
            }
        }
    }
}

async fn listar_documentos(State(db): State<Db>) -> Respuesta<Json<Vec<Documento>>> {
    let con = db.lock().unwrap();
    let ids: rusqlite::Result<Vec<String>> = con
        .prepare_cached("SELECT id FROM documentos ORDER BY creado_en DESC LIMIT 200")
        .and_then(|mut st| st.query_map([], |f| f.get(0))?.collect());
    let ids = match ids {
        Ok(v) => v,
        Err(e) => return interno(e),
    };
    let mut docs = Vec::with_capacity(ids.len());
    for id in ids {
        match leer_documento(&con, &id) {
            Ok(Some(d)) => docs.push(d),
            Ok(None) => {}
            Err(e) => return interno(e),
        }
    }
    Ok(Json(docs))
}

async fn ver_documento(State(db): State<Db>, Path(id): Path<String>) -> Respuesta<Json<Documento>> {
    let con = db.lock().unwrap();
    match leer_documento(&con, &id) {
        Ok(Some(d)) => Ok(Json(d)),
        Ok(None) => error(StatusCode::NOT_FOUND, "documento no encontrado"),
        Err(e) => interno(e),
    }
}

/// Edicion a mano del documento generado (D11): lo que se guarda aqui manda
/// sobre lo que dijo el modelo, y `editado` lo deja claro en la app.
async fn editar_documento(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(cambio): Json<TextoEditado>,
) -> Respuesta<Json<Documento>> {
    let texto = cambio.texto.trim().to_string();
    if texto.is_empty() {
        return error(StatusCode::BAD_REQUEST, "documento vacio");
    }
    let con = db.lock().unwrap();
    match con.execute(
        "UPDATE documentos SET texto = ?1, editado = 1, estado = 'listo', error = NULL,
         actualizado_en = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?2",
        params![texto, id],
    ) {
        Ok(0) => return error(StatusCode::NOT_FOUND, "documento no encontrado"),
        Ok(_) => {}
        Err(e) => return interno(e),
    }
    match leer_documento(&con, &id) {
        Ok(Some(d)) => Ok(Json(d)),
        Ok(None) => error(StatusCode::NOT_FOUND, "documento no encontrado"),
        Err(e) => interno(e),
    }
}

/// Vuelve a pedirselo al modelo con las mismas notas. Sirve tanto para un
/// documento fallido como para uno que no gusto, cambiando la instruccion.
async fn regenerar_documento(
    State(db): State<Db>,
    Path(id): Path<String>,
    cuerpo: Option<Json<Regeneracion>>,
) -> Respuesta<Json<Documento>> {
    if ia().is_none() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no hay IA configurada");
    }
    let instruccion = cuerpo
        .and_then(|Json(r)| r.instruccion)
        .map(|i| i.trim().chars().take(500).collect::<String>());
    {
        let con = db.lock().unwrap();
        let existe = match leer_documento(&con, &id) {
            Ok(Some(d)) => d,
            Ok(None) => return error(StatusCode::NOT_FOUND, "documento no encontrado"),
            Err(e) => return interno(e),
        };
        if existe.estado == "pendiente" {
            return error(StatusCode::CONFLICT, "ese documento ya se esta generando");
        }
        let sql = match &instruccion {
            Some(_) => {
                "UPDATE documentos SET estado = 'pendiente', error = NULL, instruccion = ?2,
                        actualizado_en = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1"
            }
            None => {
                "UPDATE documentos SET estado = 'pendiente', error = NULL,
                     actualizado_en = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1"
            }
        };
        let res = match &instruccion {
            Some(i) => con.execute(sql, params![id, i]),
            None => con.execute(sql, params![id]),
        };
        if let Err(e) = res {
            return interno(e);
        }
    }
    lanzar_generacion(db.clone(), id.clone());
    let con = db.lock().unwrap();
    match leer_documento(&con, &id) {
        Ok(Some(d)) => Ok(Json(d)),
        Ok(None) => error(StatusCode::NOT_FOUND, "documento no encontrado"),
        Err(e) => interno(e),
    }
}

async fn borrar_documento(State(db): State<Db>, Path(id): Path<String>) -> Respuesta<StatusCode> {
    let con = db.lock().unwrap();
    match con.execute("DELETE FROM documentos WHERE id = ?1", [&id]) {
        Ok(0) => error(StatusCode::NOT_FOUND, "documento no encontrado"),
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(e) => interno(e),
    }
}

/// La generacion vive fuera de la peticion: el usuario no espera y cerrar la
/// app no la cancela. Todo lo que pueda salir mal acaba en `estado = fallido`
/// con su motivo, que es lo que la app enseña con el boton de regenerar.
fn lanzar_generacion(db: Db, id: String) {
    tokio::spawn(async move {
        if let Err(e) = generar_documento(&db, &id).await {
            eprintln!("documento {id}: {e}");
            let con = db.lock().unwrap();
            let _ = con.execute(
                "UPDATE documentos SET estado = 'fallido', error = ?1,
                 actualizado_en = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?2",
                params![e, id],
            );
        }
    });
}

async fn generar_documento(db: &Db, id: &str) -> Result<(), String> {
    let ia = ia().ok_or("no hay IA configurada")?;

    // Se lee todo y se suelta el candado antes de salir a la red: el modelo
    // puede tardar un minuto y la app tiene que seguir respondiendo.
    let (instruccion, fuentes) = {
        let con = db.lock().unwrap();
        let instruccion: String = con
            .query_row(
                "SELECT instruccion FROM documentos WHERE id = ?1",
                [id],
                |f| f.get(0),
            )
            .map_err(|_| "el documento ya no esta".to_string())?;
        let notas: Vec<llm::NotaFuente> = con
            .prepare_cached(
                "SELECT n.id, n.titulo, n.creada_en, n.contenido FROM documento_notas dn
                 JOIN notas n ON n.id = dn.nota_id WHERE dn.documento_id = ?1 ORDER BY dn.orden",
            )
            .and_then(|mut st| {
                st.query_map([id], |f| {
                    Ok((
                        f.get::<_, String>(0)?,
                        llm::NotaFuente {
                            titulo: f.get(1)?,
                            creada_en: f.get(2)?,
                            contenido: f.get(3)?,
                            etiquetas: vec![],
                        },
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|e| format!("no se pudieron leer las notas: {e}"))?
            .into_iter()
            .map(|(nota_id, mut n)| {
                n.etiquetas = etiquetas_de(&con, &nota_id).unwrap_or_default();
                n
            })
            .collect();
        (instruccion, notas)
    };

    if fuentes.is_empty() {
        // Todas las notas del documento se borraron antes de generarlo.
        return Err("ya no queda ninguna de las notas de este documento".into());
    }

    let documento = ia.documentar(&fuentes, &instruccion).await?;
    // La seccion de notas de origen la escribe la aplicacion, no el modelo:
    // asi es fiel a lo que habia y no depende de lo que el modelo recuerde.
    let texto = format!(
        "{}\n\n{}",
        documento.markdown.trim(),
        notas_de_origen(&fuentes)
    );

    let con = db.lock().unwrap();
    con.execute(
        "UPDATE documentos SET titulo = ?1, texto = ?2, estado = 'listo', error = NULL,
         actualizado_en = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?3",
        params![documento.titulo, texto, id],
    )
    .map_err(|e| format!("no se pudo guardar el documento: {e}"))?;
    println!("documento {id} listo ({} notas)", fuentes.len());
    Ok(())
}

/// Ultima seccion del documento: de donde sale cada cosa. Fecha en UTC, que es
/// lo que guarda el servidor (D7), recortada al minuto.
fn notas_de_origen(fuentes: &[llm::NotaFuente]) -> String {
    let mut texto = String::from("## Notas de origen\n\n");
    for (i, n) in fuentes.iter().enumerate() {
        let fecha: String = n.creada_en.chars().take(16).collect();
        let fecha = fecha.replace('T', " ");
        if n.etiquetas.is_empty() {
            texto.push_str(&format!("{}. **{}** · {} UTC\n", i + 1, n.titulo, fecha));
        } else {
            texto.push_str(&format!(
                "{}. **{}** · {} UTC · etiquetas: {}\n",
                i + 1,
                n.titulo,
                fecha,
                n.etiquetas.join(", ")
            ));
        }
    }
    texto
}

async fn listar_etiquetas(State(db): State<Db>) -> Respuesta<Json<serde_json::Value>> {
    let con = db.lock().unwrap();
    let res: rusqlite::Result<Vec<serde_json::Value>> = con
        .prepare_cached(
            "SELECT etiqueta, COUNT(*) FROM etiquetas GROUP BY etiqueta ORDER BY 2 DESC, 1",
        )
        .and_then(|mut s| {
            s.query_map([], |f| {
                Ok(json!({ "etiqueta": f.get::<_, String>(0)?, "notas": f.get::<_, i64>(1)? }))
            })?
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
        .get_or_init(|| {
            std::env::var("TOKEN_API")
                .ok()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        })
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
            .query_row("SELECT contenido FROM notas WHERE id = ?1", [id], |f| {
                f.get(0)
            })
            .optional()
            .unwrap_or(None);
        // Las mas usadas primero: son las que conviene que reutilice.
        let existentes: Vec<String> = con
            .prepare_cached(
                "SELECT etiqueta FROM etiquetas GROUP BY etiqueta ORDER BY COUNT(*) DESC, etiqueta",
            )
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
            let mut ins =
                tx.prepare("INSERT INTO etiquetas (nota_id, etiqueta) VALUES (?1, ?2)")?;
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
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
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
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "token de acceso requerido" })),
            )
                .into_response();
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
        Json(
            json!({ "ok": true, "build": build, "token": token_api().is_some(), "ia": ia().is_some() }),
        ),
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
        .route("/documentos", get(listar_documentos).post(crear_documento))
        .route(
            "/documentos/{id}",
            get(ver_documento)
                .put(editar_documento)
                .delete(borrar_documento),
        )
        .route("/documentos/{id}/regenerar", post(regenerar_documento))
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
        println!(
            "AVISO: sin LLM_API_KEY, las notas se guardan con título de respaldo y sin etiquetas"
        );
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
