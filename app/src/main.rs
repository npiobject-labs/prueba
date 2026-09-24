mod analisis;
mod entrevistas;
mod llm;

use std::sync::{Arc, Mutex, OnceLock};

use axum::{
    extract::{DefaultBodyLimit, Path, Query, Request, State},
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
    migrar(&con);
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
    entrevistas::recuperar(&con);
    analisis::recuperar(&con);
    println!("base de datos en {ruta}");
    con
}

/// Cambios de esquema que `CREATE ... IF NOT EXISTS` no cubre (D23): cada
/// version de `user_version` se aplica una sola vez y en su transaccion.
fn migrar(con: &Connection) {
    let version: i64 = con
        .query_row("PRAGMA user_version", [], |f| f.get(0))
        .expect("no se pudo leer user_version");
    if version < 1 {
        // Proyectos (D14-D19): columnas nullable, asi todo lo que ya habia
        // queda en «Sin proyecto» y nada cambia para la app anterior.
        con.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS proyectos (
               id             TEXT PRIMARY KEY,
               nombre         TEXT NOT NULL,
               nombre_clave   TEXT NOT NULL UNIQUE,
               descripcion    TEXT NOT NULL DEFAULT '',
               archivado      INTEGER NOT NULL DEFAULT 0,
               creado_en      TEXT NOT NULL,
               actualizado_en TEXT NOT NULL
             );
             ALTER TABLE notas ADD COLUMN proyecto_id TEXT REFERENCES proyectos(id) ON DELETE SET NULL;
             ALTER TABLE documentos ADD COLUMN proyecto_id TEXT REFERENCES proyectos(id) ON DELETE SET NULL;
             CREATE INDEX IF NOT EXISTS notas_proyecto ON notas(proyecto_id, creada_en DESC);
             CREATE INDEX IF NOT EXISTS documentos_proyecto ON documentos(proyecto_id, creado_en DESC);
             PRAGMA user_version = 1;
             COMMIT;",
        )
        .expect("no se pudo migrar el esquema a la version 1 (proyectos)");
        println!("esquema migrado a la version 1: proyectos");
    }
    if version < 2 {
        // Entrevistas (D27-D42): el audio va en el volumen, aqui solo su
        // nombre; los trozos transcritos por separado, para reanudar (D32).
        con.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS entrevistas (
               id                TEXT PRIMARY KEY,
               proyecto_id       TEXT REFERENCES proyectos(id) ON DELETE SET NULL,
               titulo            TEXT NOT NULL,
               titulo_editado    INTEGER NOT NULL DEFAULT 0,
               con_quien         TEXT NOT NULL DEFAULT '',
               creada_en         TEXT NOT NULL,
               duracion_s        INTEGER NOT NULL DEFAULT 0,
               audio             TEXT,
               audio_tipo        TEXT NOT NULL DEFAULT '',
               audio_bytes       INTEGER NOT NULL DEFAULT 0,
               estado            TEXT NOT NULL DEFAULT 'grabada',
               trozos_total      INTEGER NOT NULL DEFAULT 0,
               trozos_hechos     INTEGER NOT NULL DEFAULT 0,
               transcripcion     TEXT NOT NULL DEFAULT '',
               error             TEXT NOT NULL DEFAULT '',
               resumen_ejecutivo TEXT NOT NULL DEFAULT '',
               resumen           TEXT NOT NULL DEFAULT '',
               estado_resumen    TEXT NOT NULL DEFAULT 'ninguno',
               error_resumen     TEXT NOT NULL DEFAULT ''
             );
             CREATE INDEX IF NOT EXISTS entrevistas_proyecto ON entrevistas(proyecto_id, creada_en DESC);
             CREATE TABLE IF NOT EXISTS entrevista_trozos (
               entrevista_id TEXT NOT NULL REFERENCES entrevistas(id) ON DELETE CASCADE,
               n             INTEGER NOT NULL,
               texto         TEXT NOT NULL,
               PRIMARY KEY (entrevista_id, n)
             );
             PRAGMA user_version = 2;
             COMMIT;",
        )
        .expect("no se pudo migrar el esquema a la version 2 (entrevistas)");
        println!("esquema migrado a la version 2: entrevistas");
    }
    if version < 3 {
        // Subcarpetas (D43-D53): un proyecto puede colgar de otro, un solo
        // nivel. Sin ON DELETE: borrar lo resuelve `borrar_proyecto` (D51).
        con.execute_batch(
            "BEGIN;
             ALTER TABLE proyectos ADD COLUMN padre_id TEXT REFERENCES proyectos(id);
             CREATE INDEX IF NOT EXISTS proyectos_padre ON proyectos(padre_id);
             PRAGMA user_version = 3;
             COMMIT;",
        )
        .expect("no se pudo migrar el esquema a la version 3 (subcarpetas)");
        println!("esquema migrado a la version 3: subcarpetas");
    }
    if version < 4 {
        // Informes con roles (D54-D67): un documento puede ser un encargo (lo
        // de siempre) o un informe; cada rol guarda su copia y su resultado,
        // para reintentar solo lo que falle. `generacion` separa el coste de
        // cada vez que se regenera.
        con.execute_batch(
            "BEGIN;
             ALTER TABLE documentos ADD COLUMN tipo TEXT NOT NULL DEFAULT 'encargo';
             ALTER TABLE documentos ADD COLUMN modelo TEXT;
             ALTER TABLE documentos ADD COLUMN coste REAL;
             ALTER TABLE documentos ADD COLUMN sintesis TEXT NOT NULL DEFAULT '';
             ALTER TABLE documentos ADD COLUMN generacion INTEGER NOT NULL DEFAULT 0;
             CREATE TABLE IF NOT EXISTS documento_roles (
               documento_id TEXT NOT NULL REFERENCES documentos(id) ON DELETE CASCADE,
               rol_id       TEXT NOT NULL,
               orden        INTEGER NOT NULL,
               nombre       TEXT NOT NULL,
               icono        TEXT NOT NULL,
               enfoque      TEXT NOT NULL,
               estado       TEXT NOT NULL DEFAULT 'espera',
               texto        TEXT,
               error        TEXT,
               PRIMARY KEY (documento_id, rol_id)
             );
             CREATE TABLE IF NOT EXISTS roles (
               id           TEXT PRIMARY KEY,
               nombre       TEXT NOT NULL,
               nombre_clave TEXT NOT NULL UNIQUE,
               icono        TEXT NOT NULL,
               enfoque      TEXT NOT NULL,
               creado_en    TEXT NOT NULL
             );
             PRAGMA user_version = 4;
             COMMIT;",
        )
        .expect("no se pudo migrar el esquema a la version 4 (informes)");
        println!("esquema migrado a la version 4: informes con roles");
    }
}

/// Nombre de proyecto comparable (D17): minusculas, sin acentos y con los
/// espacios colapsados. `COLLATE NOCASE` no sirve: solo iguala ASCII.
fn clave(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            c => c,
        })
        .collect()
}

/// `nombre_clave` guardado (D48): la clave a secas en un proyecto principal y
/// con el id del padre delante en una subcarpeta. Asi el `UNIQUE` de la
/// columna, que SQLite no deja quitar sin copiar la tabla, pasa a significar
/// «unico dentro de su padre» y renombrar el padre no obliga a tocar nada.
fn clave_en(nombre: &str, padre: Option<&str>) -> String {
    match padre {
        Some(p) => format!("{p}/{}", clave(nombre)),
        None => clave(nombre),
    }
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
    /// Proyecto de la nota (D14), `null` si esta en «Sin proyecto».
    proyecto: Option<ProyectoRef>,
    /// Rastro nota -> documento (D12). Solo se rellena en el detalle: en la
    /// lista costaria una consulta por nota y alli no se enseña.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    documentos: Vec<DocumentoDeNota>,
}

#[derive(Deserialize)]
struct NuevaNota {
    contenido: String,
    /// Proyecto donde cae la nota (D15). Sin el, «Sin proyecto» (D16).
    #[serde(default)]
    proyecto: Option<String>,
}

/// `PATCH /notas/{id}`: hoy solo mueve la nota de proyecto (D20). `null` o
/// ausente la deja en «Sin proyecto».
#[derive(Deserialize)]
struct CambioNota {
    #[serde(default)]
    proyecto: Option<String>,
}

/// `PUT /notas/{id}`: corregir el texto de una nota. Sin `titulo` (o vacío)
/// se queda el que tenía; el proyecto y las etiquetas no se tocan.
#[derive(Deserialize)]
struct EdicionNota {
    contenido: String,
    #[serde(default)]
    titulo: Option<String>,
}

#[derive(Deserialize)]
struct MoverNotas {
    notas: Vec<String>,
    #[serde(default)]
    proyecto: Option<String>,
}

/// Referencia corta a un proyecto, la que viaja dentro de notas y documentos.
/// Si es una subcarpeta, `padre` dice de que proyecto cuelga (D46).
#[derive(Serialize)]
struct ProyectoRef {
    id: String,
    nombre: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    padre: Option<PadreRef>,
}

#[derive(Serialize)]
struct PadreRef {
    id: String,
    nombre: String,
}

/// Un proyecto con sus cifras. `id` es `null` solo en la fila virtual
/// «Sin proyecto» que encabeza la lista (D16).
#[derive(Serialize)]
struct Proyecto {
    id: Option<String>,
    nombre: String,
    descripcion: String,
    archivado: bool,
    creado_en: Option<String>,
    actualizado_en: Option<String>,
    notas: i64,
    /// Entrevistas del proyecto (D35): la tarjeta las cuenta junto a las notas.
    entrevistas: i64,
    /// Lo más reciente del proyecto, nota o entrevista (orden por actividad).
    ultima_nota: Option<String>,
    /// En un proyecto principal, `notas`, `entrevistas` y `ultima_nota` suman
    /// las de sus subcarpetas (D45); `notas_propias` son las que cuelgan de el.
    notas_propias: i64,
    subcarpetas: i64,
    /// Proyecto del que cuelga, si es una subcarpeta.
    padre: Option<PadreRef>,
}

#[derive(Deserialize)]
struct NuevoProyecto {
    nombre: String,
    #[serde(default)]
    descripcion: String,
    /// Proyecto del que cuelga (D47); sin el, proyecto principal.
    #[serde(default)]
    padre: Option<String>,
}

#[derive(Deserialize)]
struct CambioProyecto {
    nombre: Option<String>,
    descripcion: Option<String>,
    archivado: Option<bool>,
    /// Ausente: no se toca. `null`: pasa a proyecto principal. Un id: pasa a
    /// colgar de ese proyecto (D47).
    #[serde(default, deserialize_with = "presente")]
    padre: Option<Option<String>>,
}

/// Distingue un campo ausente (`None`) de uno que llega a `null` (`Some(None)`).
fn presente<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Ok(Some(Option::deserialize(d)?))
}

#[derive(Deserialize, Default)]
struct FiltroProyectos {
    q: Option<String>,
    orden: Option<String>,
    archivados: Option<String>,
}

#[derive(Deserialize, Default)]
struct SoloProyecto {
    proyecto: Option<String>,
    /// `solo=1`: sin las subcarpetas del proyecto (D45, «Sin subcarpeta»).
    solo: Option<String>,
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
    /// Proyecto comun de sus notas, si lo tenian (D21).
    proyecto: Option<ProyectoRef>,
    notas: Vec<NotaDeDocumento>,
    /// `encargo` (D11) o `informe` (D54).
    tipo: String,
    /// El modelo pedido; `null` es el del servidor (D60).
    modelo: Option<String>,
    /// Dolares que costo la ultima generacion de un informe (D62).
    coste: Option<f64>,
    /// Los roles de un informe con su estado (D63); vacio en un encargo.
    roles: Vec<RolDeDocumento>,
}

/// Un rol dentro de un informe: `espera`, `trabajando`, `hecho` o `fallido`.
#[derive(Serialize)]
struct RolDeDocumento {
    id: String,
    nombre: String,
    icono: String,
    estado: String,
    error: Option<String>,
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
    /// `encargo` (por defecto) o `informe` (D54).
    tipo: Option<String>,
    /// Ids de los roles de un informe, de 1 a 8 (D55/D56).
    #[serde(default)]
    roles: Vec<String>,
    /// Modelo del catalogo; vacio o ausente, el del servidor (D60).
    modelo: Option<String>,
}

/// `encargo` o `informe`; cualquier otra cosa es un 400.
fn tipo_valido(t: Option<&str>) -> Respuesta<Option<&'static str>> {
    match t.map(str::trim) {
        None | Some("") => Ok(None),
        Some("encargo") => Ok(Some("encargo")),
        Some("informe") => Ok(Some("informe")),
        Some(otro) => error(StatusCode::BAD_REQUEST, format!("tipo desconocido: {otro}")),
    }
}

#[derive(Deserialize)]
struct TextoEditado {
    texto: String,
}

#[derive(Deserialize, Default)]
struct Regeneracion {
    instruccion: Option<String>,
    /// Ausente: el mismo tipo. Se puede pasar de encargo a informe y al reves.
    tipo: Option<String>,
    /// Ausente: los mismos roles (su copia). Una lista: esos roles.
    roles: Option<Vec<String>>,
    /// Ausente: el mismo modelo. `null` o vacio: el del servidor.
    #[serde(default, deserialize_with = "presente")]
    modelo: Option<Option<String>>,
}

#[derive(Deserialize, Default)]
struct Filtro {
    q: Option<String>,
    etiqueta: Option<String>,
    desde: Option<String>,
    hasta: Option<String>,
    /// Id de proyecto o `ninguno` (D15/D16). Sin el, todas las notas (D23).
    /// Un proyecto trae tambien lo de sus subcarpetas salvo con `solo=1` (D45).
    proyecto: Option<String>,
    solo: Option<String>,
}

type Respuesta<T> = Result<T, (StatusCode, Json<serde_json::Value>)>;

/// Filtro de proyecto en las listas: `None` sin filtro, `Some(None)` para
/// «Sin proyecto» (`ninguno`) y `Some(Some(id))` para uno concreto.
fn filtro_proyecto(p: Option<&str>) -> Option<Option<String>> {
    match p.map(str::trim) {
        None | Some("") => None,
        Some("ninguno") => Some(None),
        Some(id) => Some(Some(id.to_string())),
    }
}

/// `solo=1` (o `true`) en la consulta.
fn es_solo(s: Option<&str>) -> bool {
    matches!(s, Some("1") | Some("true"))
}

/// Anade al SQL la condicion de proyecto sobre la columna dada. Un proyecto
/// incluye sus subcarpetas (D45) salvo con `solo`.
fn sql_proyecto(
    sql: &mut String,
    args: &mut Vec<String>,
    columna: &str,
    p: Option<&str>,
    solo: bool,
) {
    match filtro_proyecto(p) {
        None => {}
        Some(None) => sql.push_str(&format!(" AND {columna} IS NULL")),
        Some(Some(id)) => {
            args.push(id);
            let n = args.len();
            if solo {
                sql.push_str(&format!(" AND {columna} = ?{n}"));
            } else {
                sql.push_str(&format!(
                    " AND {columna} IN (SELECT id FROM proyectos WHERE id = ?{n} OR padre_id = ?{n})"
                ));
            }
        }
    }
}

/// Lee `p.id, p.nombre, pp.id, pp.nombre` a partir de la columna `i`: el
/// proyecto y, si es una subcarpeta, su padre (`LEFT JOIN proyectos pp`).
fn proyecto_de_fila(f: &rusqlite::Row, i: usize) -> rusqlite::Result<Option<ProyectoRef>> {
    let padre = match (
        f.get::<_, Option<String>>(i + 2)?,
        f.get::<_, Option<String>>(i + 3)?,
    ) {
        (Some(id), Some(nombre)) => Some(PadreRef { id, nombre }),
        _ => None,
    };
    Ok(
        match (
            f.get::<_, Option<String>>(i)?,
            f.get::<_, Option<String>>(i + 1)?,
        ) {
            (Some(id), Some(nombre)) => Some(ProyectoRef { id, nombre, padre }),
            _ => None,
        },
    )
}

/// Nombre y descripcion del proyecto para el modelo (D53): en una subcarpeta,
/// «Padre › Subcarpeta» y las dos descripciones. Van con `p` y `pp`.
const CONTEXTO_NOMBRE: &str =
    "CASE WHEN pp.id IS NULL THEN p.nombre ELSE pp.nombre || ' › ' || p.nombre END";
const CONTEXTO_DESCRIPCION: &str = "TRIM(COALESCE(pp.descripcion, '') || ' ' || p.descripcion)";

/// Comprueba el proyecto que manda el cliente: vacio o `null` es «Sin
/// proyecto»; un id que no existe es un 400, no una nota perdida.
fn proyecto_valido(con: &Connection, p: Option<String>) -> Respuesta<Option<String>> {
    let Some(id) = p
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "ninguno")
    else {
        return Ok(None);
    };
    match con
        .query_row("SELECT 1 FROM proyectos WHERE id = ?1", [&id], |_| Ok(()))
        .optional()
    {
        Ok(Some(())) => Ok(Some(id)),
        Ok(None) => error(StatusCode::BAD_REQUEST, "el proyecto no existe"),
        Err(e) => interno(e),
    }
}

fn es_duplicado(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation)
}

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
            "SELECT n.id, n.titulo, n.contenido, n.creada_en, n.pendiente_ia, p.id, p.nombre, pp.id, pp.nombre
             FROM notas n LEFT JOIN proyectos p ON p.id = n.proyecto_id
             LEFT JOIN proyectos pp ON pp.id = p.padre_id WHERE n.id = ?1",
        )?
        .query_row([id], |f| {
            Ok(Nota {
                id: f.get(0)?,
                titulo: f.get(1)?,
                contenido: f.get(2)?,
                creada_en: f.get(3)?,
                pendiente_ia: f.get::<_, i64>(4)? != 0,
                etiquetas: vec![],
                proyecto: proyecto_de_fila(f, 5)?,
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
        let proyecto = proyecto_valido(&con, nueva.proyecto)?;
        let titulo = titulo_respaldo(&contenido);
        // id aleatorio y fecha UTC los pone SQLite: sin dependencias extra.
        match con.query_row(
            "INSERT INTO notas (id, titulo, contenido, creada_en, pendiente_ia, proyecto_id)
             VALUES (lower(hex(randomblob(8))), ?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 1, ?3)
             RETURNING id",
            params![titulo, contenido, proyecto],
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
    sql_proyecto(
        &mut sql,
        &mut args,
        "n.proyecto_id",
        f.proyecto.as_deref(),
        es_solo(f.solo.as_deref()),
    );
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

/// Mueve una nota de proyecto (D20). No toca titulo, etiquetas ni fecha.
async fn cambiar_nota(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(cambio): Json<CambioNota>,
) -> Respuesta<Json<Nota>> {
    let con = db.lock().unwrap();
    let proyecto = proyecto_valido(&con, cambio.proyecto)?;
    match con.execute(
        "UPDATE notas SET proyecto_id = ?1 WHERE id = ?2",
        params![proyecto, id],
    ) {
        Ok(0) => return error(StatusCode::NOT_FOUND, "nota no encontrada"),
        Ok(_) => {}
        Err(e) => return interno(e),
    }
    match leer_nota(&con, &id) {
        Ok(Some(n)) => Ok(Json(n)),
        Ok(None) => error(StatusCode::NOT_FOUND, "nota no encontrada"),
        Err(e) => interno(e),
    }
}

async fn editar_nota(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(e): Json<EdicionNota>,
) -> Respuesta<Json<Nota>> {
    let contenido = e.contenido.trim().to_string();
    if contenido.is_empty() {
        return error(StatusCode::BAD_REQUEST, "contenido vacío");
    }
    let titulo: Option<String> = e
        .titulo
        .map(|t| t.trim().chars().take(120).collect::<String>())
        .filter(|t| !t.is_empty());
    let con = db.lock().unwrap();
    match con.execute(
        "UPDATE notas SET contenido = ?1, titulo = COALESCE(?2, titulo) WHERE id = ?3",
        params![contenido, titulo, id],
    ) {
        Ok(0) => return error(StatusCode::NOT_FOUND, "nota no encontrada"),
        Ok(_) => {}
        Err(e) => return interno(e),
    }
    match leer_nota(&con, &id) {
        Ok(Some(n)) => Ok(Json(n)),
        Ok(None) => error(StatusCode::NOT_FOUND, "nota no encontrada"),
        Err(e) => interno(e),
    }
}

/// Mueve varias notas de una vez, desde el modo seleccion (D20). Todas o
/// ninguna: si una no existe, no se mueve nada.
async fn mover_notas(
    State(db): State<Db>,
    Json(m): Json<MoverNotas>,
) -> Respuesta<Json<serde_json::Value>> {
    let con = db.lock().unwrap();
    let proyecto = proyecto_valido(&con, m.proyecto)?;
    let mut ids: Vec<String> = Vec::new();
    for id in m.notas {
        let id = id.trim().to_string();
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return error(StatusCode::BAD_REQUEST, "no hay notas que mover");
    }
    let tx = match con.unchecked_transaction() {
        Ok(tx) => tx,
        Err(e) => return interno(e),
    };
    for id in &ids {
        match tx.execute(
            "UPDATE notas SET proyecto_id = ?1 WHERE id = ?2",
            params![proyecto, id],
        ) {
            Ok(0) => return error(StatusCode::BAD_REQUEST, format!("la nota {id} no existe")),
            Ok(_) => {}
            Err(e) => return interno(e),
        }
    }
    if let Err(e) = tx.commit() {
        return interno(e);
    }
    Ok(Json(json!({ "movidas": ids.len(), "proyecto": proyecto })))
}

// ---------------------------------------------------------------------------
// Proyectos (D14-D19): contenedores de notas que nombra el usuario.
// ---------------------------------------------------------------------------

/// Ids de un proyecto y de sus subcarpetas, para las cifras que suman (D45).
/// En una subcarpeta es solo ella: no hay mas niveles (D43).
macro_rules! familia {
    () => {
        "(SELECT h.id FROM proyectos h WHERE h.id = p.id OR h.padre_id = p.id)"
    };
}

const SQL_PROYECTO: &str = concat!(
    "SELECT p.id, p.nombre, p.descripcion, p.archivado, p.creado_en, p.actualizado_en,
       (SELECT COUNT(*) FROM notas n WHERE n.proyecto_id IN ",
    familia!(),
    "),
       (SELECT MAX(c) FROM (SELECT MAX(n.creada_en) AS c FROM notas n WHERE n.proyecto_id IN ",
    familia!(),
    "
          UNION ALL SELECT MAX(e.creada_en) FROM entrevistas e WHERE e.proyecto_id IN ",
    familia!(),
    ")),
       (SELECT COUNT(*) FROM entrevistas e WHERE e.proyecto_id IN ",
    familia!(),
    "),
       (SELECT COUNT(*) FROM notas n WHERE n.proyecto_id = p.id),
       (SELECT COUNT(*) FROM proyectos h WHERE h.padre_id = p.id),
       pp.id, pp.nombre
     FROM proyectos p LEFT JOIN proyectos pp ON pp.id = p.padre_id"
);

fn fila_proyecto(f: &rusqlite::Row) -> rusqlite::Result<Proyecto> {
    Ok(Proyecto {
        id: f.get(0)?,
        nombre: f.get(1)?,
        descripcion: f.get(2)?,
        archivado: f.get::<_, i64>(3)? != 0,
        creado_en: f.get(4)?,
        actualizado_en: f.get(5)?,
        notas: f.get(6)?,
        ultima_nota: f.get(7)?,
        entrevistas: f.get(8)?,
        notas_propias: f.get(9)?,
        subcarpetas: f.get(10)?,
        padre: match (
            f.get::<_, Option<String>>(11)?,
            f.get::<_, Option<String>>(12)?,
        ) {
            (Some(id), Some(nombre)) => Some(PadreRef { id, nombre }),
            _ => None,
        },
    })
}

fn leer_proyecto(con: &Connection, id: &str) -> rusqlite::Result<Option<Proyecto>> {
    con.prepare_cached(&format!("{SQL_PROYECTO} WHERE p.id = ?1"))?
        .query_row([id], fila_proyecto)
        .optional()
}

/// Nombre obligatorio, espacios colapsados, hasta 60 caracteres (D17).
fn nombre_valido(nombre: &str) -> Result<String, &'static str> {
    let nombre = nombre.split_whitespace().collect::<Vec<_>>().join(" ");
    match nombre.chars().count() {
        0 => Err("el proyecto necesita un nombre"),
        n if n > 60 => Err("el nombre pasa de 60 caracteres"),
        _ => Ok(nombre),
    }
}

fn descripcion_valida(d: &str) -> String {
    d.trim().chars().take(300).collect()
}

/// El padre que pide el cliente: vacio es «proyecto principal»; si no, tiene
/// que existir y ser principal, porque solo hay un nivel (D43). Devuelve su id
/// y si esta archivado, que la subcarpeta hereda (D52).
fn padre_valido(con: &Connection, padre: Option<String>) -> Respuesta<Option<(String, bool)>> {
    let Some(id) = padre
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    match con
        .query_row(
            "SELECT padre_id IS NOT NULL, archivado FROM proyectos WHERE id = ?1",
            [&id],
            |f| Ok((f.get::<_, bool>(0)?, f.get::<_, i64>(1)? != 0)),
        )
        .optional()
    {
        Ok(Some((true, _))) => error(
            StatusCode::BAD_REQUEST,
            "una subcarpeta no puede tener subcarpetas",
        ),
        Ok(Some((false, archivado))) => Ok(Some((id, archivado))),
        Ok(None) => error(StatusCode::BAD_REQUEST, "el proyecto padre no existe"),
        Err(e) => interno(e),
    }
}

fn nombre_repetido<T>(padre: Option<&str>) -> Respuesta<T> {
    match padre {
        Some(_) => error(
            StatusCode::CONFLICT,
            "ya hay una subcarpeta con ese nombre en ese proyecto",
        ),
        None => error(StatusCode::CONFLICT, "ya hay un proyecto con ese nombre"),
    }
}

/// Lista de proyectos (D18). Son decenas: se leen todos con sus cifras y se
/// filtran y ordenan aqui, sin indice de texto. Las subcarpetas van como filas
/// propias con su `padre`; agruparlas es cosa de la app (D49).
async fn listar_proyectos(
    State(db): State<Db>,
    Query(f): Query<FiltroProyectos>,
) -> Respuesta<Json<Vec<Proyecto>>> {
    let con = db.lock().unwrap();
    let mut v: Vec<Proyecto> = match con
        .prepare_cached(SQL_PROYECTO)
        .and_then(|mut st| st.query_map([], fila_proyecto)?.collect())
    {
        Ok(v) => v,
        Err(e) => return interno(e),
    };
    let con_archivados = matches!(f.archivados.as_deref(), Some("1") | Some("true"));
    v.retain(|p| con_archivados || !p.archivado);
    let terminos: Vec<String> =
        f.q.as_deref()
            .map(|q| clave(q).split_whitespace().map(String::from).collect())
            .unwrap_or_default();
    if !terminos.is_empty() {
        v.retain(|p| {
            let texto = clave(&format!("{} {}", p.nombre, p.descripcion));
            terminos.iter().all(|t| texto.contains(t.as_str()))
        });
    }
    // Actividad = la nota mas reciente; sin notas, la creacion del proyecto.
    let actividad = |p: &Proyecto| {
        p.ultima_nota
            .clone()
            .or_else(|| p.creado_en.clone())
            .unwrap_or_default()
    };
    match f.orden.as_deref().unwrap_or("actividad") {
        "nombre" => v.sort_by_key(|p| clave(&p.nombre)),
        "creacion" => v.sort_by(|a, b| b.creado_en.cmp(&a.creado_en)),
        "notas" => v.sort_by(|a, b| {
            b.notas
                .cmp(&a.notas)
                .then_with(|| actividad(b).cmp(&actividad(a)))
        }),
        _ => v.sort_by_key(|p| std::cmp::Reverse(actividad(p))),
    }
    // «Sin proyecto» encabeza la lista si tiene algo y no se esta buscando (D16).
    // Cuenta tambien una entrevista suelta: sin la tarjeta no habria forma de
    // llegar a ella.
    if terminos.is_empty() {
        let huerfanas = con.query_row(
            "SELECT COUNT(*),
                    (SELECT MAX(c) FROM (SELECT MAX(creada_en) AS c FROM notas WHERE proyecto_id IS NULL
                       UNION ALL SELECT MAX(creada_en) FROM entrevistas WHERE proyecto_id IS NULL)),
                    (SELECT COUNT(*) FROM entrevistas WHERE proyecto_id IS NULL)
             FROM notas WHERE proyecto_id IS NULL",
            [],
            |f| {
                Ok((
                    f.get::<_, i64>(0)?,
                    f.get::<_, Option<String>>(1)?,
                    f.get::<_, i64>(2)?,
                ))
            },
        );
        match huerfanas {
            Ok((n, ultima, entrevistas)) if n > 0 || entrevistas > 0 => v.insert(
                0,
                Proyecto {
                    id: None,
                    nombre: "Sin proyecto".into(),
                    descripcion: String::new(),
                    archivado: false,
                    creado_en: None,
                    actualizado_en: None,
                    notas: n,
                    entrevistas,
                    ultima_nota: ultima,
                    notas_propias: n,
                    subcarpetas: 0,
                    padre: None,
                },
            ),
            Ok(_) => {}
            Err(e) => return interno(e),
        }
    }
    Ok(Json(v))
}

async fn crear_proyecto(
    State(db): State<Db>,
    Json(nuevo): Json<NuevoProyecto>,
) -> Respuesta<impl IntoResponse> {
    let nombre = match nombre_valido(&nuevo.nombre) {
        Ok(n) => n,
        Err(m) => return error(StatusCode::BAD_REQUEST, m),
    };
    let con = db.lock().unwrap();
    let padre = padre_valido(&con, nuevo.padre)?;
    let padre_id = padre.as_ref().map(|(id, _)| id.as_str());
    let archivado = padre.as_ref().is_some_and(|(_, a)| *a);
    let id: String = match con.query_row(
        "INSERT INTO proyectos (id, nombre, nombre_clave, descripcion, archivado, creado_en, actualizado_en, padre_id)
         VALUES (lower(hex(randomblob(8))), ?1, ?2, ?3, ?4,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?5)
         RETURNING id",
        params![
            nombre,
            clave_en(&nombre, padre_id),
            descripcion_valida(&nuevo.descripcion),
            archivado as i64,
            padre_id
        ],
        |f| f.get(0),
    ) {
        Ok(id) => id,
        Err(e) if es_duplicado(&e) => return nombre_repetido(padre_id),
        Err(e) => return interno(e),
    };
    match leer_proyecto(&con, &id) {
        Ok(Some(p)) => Ok((StatusCode::CREATED, Json(p))),
        Ok(None) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "el proyecto no se guardo",
        ),
        Err(e) => interno(e),
    }
}

async fn ver_proyecto(State(db): State<Db>, Path(id): Path<String>) -> Respuesta<Json<Proyecto>> {
    let con = db.lock().unwrap();
    match leer_proyecto(&con, &id) {
        Ok(Some(p)) => Ok(Json(p)),
        Ok(None) => error(StatusCode::NOT_FOUND, "proyecto no encontrado"),
        Err(e) => interno(e),
    }
}

/// Renombrar, describir, archivar, desarchivar (D17/D19) y cambiar de padre
/// (D47). Archivar es de proyectos principales y arrastra a sus subcarpetas
/// (D52).
async fn editar_proyecto(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(c): Json<CambioProyecto>,
) -> Respuesta<Json<Proyecto>> {
    let nombre = match c.nombre.as_deref().map(nombre_valido).transpose() {
        Ok(n) => n,
        Err(m) => return error(StatusCode::BAD_REQUEST, m),
    };
    let con = db.lock().unwrap();
    let actual = match leer_proyecto(&con, &id) {
        Ok(Some(p)) => p,
        Ok(None) => return error(StatusCode::NOT_FOUND, "proyecto no encontrado"),
        Err(e) => return interno(e),
    };
    // Padre final: el que llega o el que tenia. Con subcarpetas no puede ir
    // dentro de otro (D43), y nunca dentro de si mismo.
    let cambia_clave = nombre.is_some() || c.padre.is_some();
    let (padre_id, archivado_padre): (Option<String>, Option<bool>) = match c.padre {
        None => (actual.padre.as_ref().map(|p| p.id.clone()), None),
        Some(nuevo) => match padre_valido(&con, nuevo)? {
            None => (None, None),
            Some((p, _)) if p == id => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "un proyecto no puede ir dentro de si mismo",
                )
            }
            Some(_) if actual.subcarpetas > 0 => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "tiene subcarpetas: no puede ir dentro de otro proyecto",
                )
            }
            Some((p, a)) => (Some(p), Some(a)),
        },
    };
    if c.archivado.is_some() && padre_id.is_some() {
        return error(
            StatusCode::BAD_REQUEST,
            "una subcarpeta se archiva con su proyecto",
        );
    }
    let nombre_final = nombre.clone().unwrap_or_else(|| actual.nombre.clone());
    let aplicar = || -> rusqlite::Result<()> {
        let tx = con.unchecked_transaction()?;
        if cambia_clave {
            tx.execute(
                "UPDATE proyectos SET nombre = ?1, nombre_clave = ?2, padre_id = ?3 WHERE id = ?4",
                params![
                    nombre_final,
                    clave_en(&nombre_final, padre_id.as_deref()),
                    padre_id,
                    id
                ],
            )?;
        }
        // Al entrar en un proyecto, la subcarpeta toma su estado (D52).
        if let Some(a) = archivado_padre {
            tx.execute(
                "UPDATE proyectos SET archivado = ?1 WHERE id = ?2",
                params![a as i64, id],
            )?;
        }
        if let Some(d) = &c.descripcion {
            tx.execute(
                "UPDATE proyectos SET descripcion = ?1 WHERE id = ?2",
                params![descripcion_valida(d), id],
            )?;
        }
        if let Some(a) = c.archivado {
            tx.execute(
                "UPDATE proyectos SET archivado = ?1 WHERE id = ?2 OR padre_id = ?2",
                params![a as i64, id],
            )?;
        }
        tx.execute(
            "UPDATE proyectos SET actualizado_en = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
            [&id],
        )?;
        tx.commit()
    };
    match aplicar() {
        Ok(()) => {}
        Err(e) if es_duplicado(&e) => return nombre_repetido(padre_id.as_deref()),
        Err(e) => return interno(e),
    }
    match leer_proyecto(&con, &id) {
        Ok(Some(p)) => Ok(Json(p)),
        Ok(None) => error(StatusCode::NOT_FOUND, "proyecto no encontrado"),
        Err(e) => interno(e),
    }
}

/// Borrar un proyecto nunca borra notas, documentos ni entrevistas (D19,
/// D51). De una subcarpeta, todo sube a su padre. De un proyecto principal,
/// lo suyo va a «Sin proyecto» y sus subcarpetas pasan a principales con lo
/// que tengan; si una choca de nombre con otro principal, se le añade el del
/// padre entre parentesis para que el borrado no falle.
async fn borrar_proyecto(State(db): State<Db>, Path(id): Path<String>) -> Respuesta<StatusCode> {
    let con = db.lock().unwrap();
    let borrar = || -> rusqlite::Result<usize> {
        let tx = con.unchecked_transaction()?;
        let Some((nombre, padre)): Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT nombre, padre_id FROM proyectos WHERE id = ?1",
                [&id],
                |f| Ok((f.get(0)?, f.get(1)?)),
            )
            .optional()?
        else {
            return Ok(0);
        };
        for tabla in ["notas", "documentos", "entrevistas"] {
            tx.execute(
                &format!("UPDATE {tabla} SET proyecto_id = ?1 WHERE proyecto_id = ?2"),
                params![padre, id],
            )?;
        }
        let hijas: Vec<(String, String)> = tx
            .prepare("SELECT id, nombre FROM proyectos WHERE padre_id = ?1")?
            .query_map([&id], |f| Ok((f.get(0)?, f.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (hija, nombre_hija) in hijas {
            let mut candidato = nombre_hija.clone();
            let mut n = 1;
            while tx
                .query_row(
                    "SELECT 1 FROM proyectos WHERE nombre_clave = ?1 AND id <> ?2",
                    params![clave(&candidato), hija],
                    |_| Ok(()),
                )
                .optional()?
                .is_some()
            {
                n += 1;
                candidato = if n == 2 {
                    format!("{nombre_hija} ({nombre})")
                } else {
                    format!("{nombre_hija} ({nombre}) {n}")
                };
            }
            tx.execute(
                "UPDATE proyectos SET padre_id = NULL, nombre = ?1, nombre_clave = ?2 WHERE id = ?3",
                params![candidato, clave(&candidato), hija],
            )?;
        }
        let n = tx.execute("DELETE FROM proyectos WHERE id = ?1", [&id])?;
        tx.commit()?;
        Ok(n)
    };
    match borrar() {
        Ok(0) => error(StatusCode::NOT_FOUND, "proyecto no encontrado"),
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
            "SELECT d.id, d.titulo, d.instruccion, d.texto, d.estado, d.error, d.editado,
                    d.creado_en, d.actualizado_en, p.id, p.nombre, pp.id, pp.nombre,
                    d.tipo, d.modelo, d.coste
             FROM documentos d LEFT JOIN proyectos p ON p.id = d.proyecto_id
             LEFT JOIN proyectos pp ON pp.id = p.padre_id WHERE d.id = ?1",
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
                proyecto: proyecto_de_fila(f, 9)?,
                notas: vec![],
                tipo: f.get(13)?,
                modelo: f.get(14)?,
                coste: f.get(15)?,
                roles: vec![],
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
            d.roles = con
                .prepare_cached(
                    "SELECT rol_id, nombre, icono, estado, error FROM documento_roles
                     WHERE documento_id = ?1 ORDER BY orden",
                )?
                .query_map([id], |f| {
                    Ok(RolDeDocumento {
                        id: f.get(0)?,
                        nombre: f.get(1)?,
                        icono: f.get(2)?,
                        estado: f.get(3)?,
                        error: f.get(4)?,
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
    let tipo = tipo_valido(nuevo.tipo.as_deref())?.unwrap_or("encargo");
    // El catalogo se consulta antes de tomar el candado: es una llamada a red.
    let modelo = analisis::modelo_valido(nuevo.modelo, tipo == "encargo").await?;

    let id = {
        let con = db.lock().unwrap();
        let roles = if tipo == "informe" {
            analisis::roles_pedidos(&con, &nuevo.roles)?
        } else {
            vec![]
        };
        // Las notas entran en el documento de la mas antigua a la mas reciente:
        // asi el modelo lee la historia en el orden en que se dicto.
        let mut fuentes: Vec<(String, String, String)> = Vec::with_capacity(ids.len());
        let mut proyectos: Vec<Option<String>> = Vec::with_capacity(ids.len());
        for nota_id in &ids {
            let fila = con
                .prepare_cached(
                    "SELECT id, titulo, creada_en, proyecto_id FROM notas WHERE id = ?1",
                )
                .and_then(|mut st| {
                    st.query_row([nota_id], |f| {
                        Ok((
                            (
                                f.get::<_, String>(0)?,
                                f.get::<_, String>(1)?,
                                f.get::<_, String>(2)?,
                            ),
                            f.get::<_, Option<String>>(3)?,
                        ))
                    })
                    .optional()
                });
            match fila {
                Ok(Some((f, p))) => {
                    fuentes.push(f);
                    proyectos.push(p);
                }
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
        // El documento es del proyecto solo si todas sus notas lo son (D21);
        // si son de subcarpetas distintas del mismo proyecto, del padre (D53).
        let proyecto: Option<String> = match proyectos.first() {
            Some(Some(p)) if proyectos.iter().all(|x| x.as_deref() == Some(p.as_str())) => {
                Some(p.clone())
            }
            Some(Some(_)) if proyectos.iter().all(Option::is_some) => {
                let mut raices: Vec<Option<String>> = Vec::with_capacity(proyectos.len());
                for p in proyectos.iter().flatten() {
                    match con
                        .query_row(
                            "SELECT COALESCE(padre_id, id) FROM proyectos WHERE id = ?1",
                            [p],
                            |f| f.get::<_, String>(0),
                        )
                        .optional()
                    {
                        Ok(r) => raices.push(r),
                        Err(e) => return interno(e),
                    }
                }
                match raices.first() {
                    Some(Some(r)) if raices.iter().all(|x| x.as_deref() == Some(r.as_str())) => {
                        Some(r.clone())
                    }
                    _ => None,
                }
            }
            _ => None,
        };

        let crear = || -> rusqlite::Result<String> {
            let tx = con.unchecked_transaction()?;
            let id: String = tx.query_row(
                "INSERT INTO documentos (id, titulo, instruccion, texto, estado, editado, creado_en, actualizado_en, proyecto_id, tipo, modelo)
                 VALUES (lower(hex(randomblob(8))), ?1, ?2, '', 'pendiente', 0,
                         strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?3, ?4, ?5)
                 RETURNING id",
                params![titulo_provisional(&fuentes), instruccion, proyecto, tipo, modelo],
                |f| f.get(0),
            )?;
            insertar_roles(&tx, &id, &roles)?;
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
    if tipo == "informe" {
        analisis::lanzar_informe(db.clone(), id.clone());
    } else {
        lanzar_generacion(db.clone(), id.clone());
    }

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

/// Los roles de un informe, cada uno con su copia y en espera (D59/D63).
fn insertar_roles(
    tx: &rusqlite::Transaction,
    id: &str,
    roles: &[analisis::RolCopia],
) -> rusqlite::Result<()> {
    let mut ins = tx.prepare(
        "INSERT INTO documento_roles (documento_id, rol_id, orden, nombre, icono, enfoque, estado)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'espera')",
    )?;
    for (orden, r) in roles.iter().enumerate() {
        ins.execute(params![
            id,
            r.id,
            orden as i64,
            r.nombre,
            r.icono,
            r.enfoque
        ])?;
    }
    Ok(())
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

async fn listar_documentos(
    State(db): State<Db>,
    Query(f): Query<SoloProyecto>,
) -> Respuesta<Json<Vec<Documento>>> {
    let con = db.lock().unwrap();
    let mut sql = String::from("SELECT id FROM documentos WHERE 1=1");
    let mut args: Vec<String> = vec![];
    sql_proyecto(
        &mut sql,
        &mut args,
        "proyecto_id",
        f.proyecto.as_deref(),
        es_solo(f.solo.as_deref()),
    );
    sql.push_str(" ORDER BY creado_en DESC LIMIT 200");
    let ids: rusqlite::Result<Vec<String>> = con.prepare(&sql).and_then(|mut st| {
        st.query_map(rusqlite::params_from_iter(args.iter()), |f| f.get(0))?
            .collect()
    });
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
/// documento fallido como para uno que no gusto: se puede cambiar la
/// instruccion, el tipo, los roles y el modelo (D66).
async fn regenerar_documento(
    State(db): State<Db>,
    Path(id): Path<String>,
    cuerpo: Option<Json<Regeneracion>>,
) -> Respuesta<Json<Documento>> {
    if ia().is_none() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no hay IA configurada");
    }
    let r = cuerpo.map(|Json(r)| r).unwrap_or_default();
    let instruccion = r
        .instruccion
        .map(|i| i.trim().chars().take(500).collect::<String>());
    let (tipo, modelo) = {
        let con = db.lock().unwrap();
        let existe = match leer_documento(&con, &id) {
            Ok(Some(d)) => d,
            Ok(None) => return error(StatusCode::NOT_FOUND, "documento no encontrado"),
            Err(e) => return interno(e),
        };
        if existe.estado == "pendiente" {
            return error(StatusCode::CONFLICT, "ese documento ya se esta generando");
        }
        let tipo = match tipo_valido(r.tipo.as_deref())? {
            Some(t) => t,
            None if existe.tipo == "informe" => "informe",
            None => "encargo",
        };
        let modelo = match r.modelo {
            None => existe.modelo,
            Some(m) => m,
        };
        (tipo, modelo)
    };
    let modelo = analisis::modelo_valido(modelo, tipo == "encargo").await?;
    {
        let con = db.lock().unwrap();
        let roles_nuevos = match (&r.roles, tipo) {
            (Some(ids), "informe") => Some(analisis::roles_pedidos(&con, ids)?),
            _ => None,
        };
        if tipo == "informe" && roles_nuevos.is_none() {
            let hay: i64 = match con.query_row(
                "SELECT COUNT(*) FROM documento_roles WHERE documento_id = ?1",
                [&id],
                |f| f.get(0),
            ) {
                Ok(n) => n,
                Err(e) => return interno(e),
            };
            if hay == 0 {
                return error(StatusCode::BAD_REQUEST, "elige al menos un rol");
            }
        }
        let preparar = || -> rusqlite::Result<()> {
            let tx = con.unchecked_transaction()?;
            tx.execute(
                "UPDATE documentos SET estado = 'pendiente', error = NULL, tipo = ?2, modelo = ?3,
                        instruccion = COALESCE(?4, instruccion), coste = NULL, sintesis = '',
                        generacion = generacion + 1,
                        actualizado_en = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                params![id, tipo, modelo, instruccion],
            )?;
            match (&roles_nuevos, tipo) {
                (Some(roles), _) => {
                    tx.execute("DELETE FROM documento_roles WHERE documento_id = ?1", [&id])?;
                    insertar_roles(&tx, &id, roles)?;
                }
                (None, "informe") => {
                    tx.execute(
                        "UPDATE documento_roles SET estado = 'espera', texto = NULL, error = NULL
                         WHERE documento_id = ?1",
                        [&id],
                    )?;
                }
                (None, _) => {
                    tx.execute("DELETE FROM documento_roles WHERE documento_id = ?1", [&id])?;
                }
            }
            tx.commit()
        };
        if let Err(e) = preparar() {
            return interno(e);
        }
    }
    if tipo == "informe" {
        analisis::lanzar_informe(db.clone(), id.clone());
    } else {
        lanzar_generacion(db.clone(), id.clone());
    }
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

/// Lo que el modelo necesita de un documento: instruccion, notas (de la mas
/// antigua a la mas reciente, con sus etiquetas), proyecto y modelo pedido.
struct FuentesDocumento {
    instruccion: String,
    notas: Vec<llm::NotaFuente>,
    proyecto: Option<llm::Proyecto>,
    modelo: Option<String>,
}

/// Las notas borradas despues de crear el documento ya no entran.
fn fuentes_de_documento(con: &Connection, id: &str) -> Result<FuentesDocumento, String> {
    let proyecto: Option<llm::Proyecto> = con
        .query_row(
            &format!(
                "SELECT {CONTEXTO_NOMBRE}, {CONTEXTO_DESCRIPCION} FROM documentos d
                 JOIN proyectos p ON p.id = d.proyecto_id
                 LEFT JOIN proyectos pp ON pp.id = p.padre_id WHERE d.id = ?1"
            ),
            [id],
            |f| {
                Ok(llm::Proyecto {
                    nombre: f.get(0)?,
                    descripcion: f.get(1)?,
                })
            },
        )
        .optional()
        .unwrap_or(None);
    let (instruccion, modelo): (String, Option<String>) = con
        .query_row(
            "SELECT instruccion, modelo FROM documentos WHERE id = ?1",
            [id],
            |f| Ok((f.get(0)?, f.get(1)?)),
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
            n.etiquetas = etiquetas_de(con, &nota_id).unwrap_or_default();
            n
        })
        .collect();
    Ok(FuentesDocumento {
        instruccion,
        notas,
        proyecto,
        modelo,
    })
}

async fn generar_documento(db: &Db, id: &str) -> Result<(), String> {
    let ia = ia().ok_or("no hay IA configurada")?;

    // Se lee todo y se suelta el candado antes de salir a la red: el modelo
    // puede tardar un minuto y la app tiene que seguir respondiendo.
    let FuentesDocumento {
        instruccion,
        notas: fuentes,
        proyecto,
        modelo,
    } = {
        let con = db.lock().unwrap();
        fuentes_de_documento(&con, id)?
    };

    if fuentes.is_empty() {
        // Todas las notas del documento se borraron antes de generarlo.
        return Err("ya no queda ninguna de las notas de este documento".into());
    }

    let documento = ia
        .documentar(&fuentes, &instruccion, proyecto.as_ref(), modelo.as_deref())
        .await?;
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

async fn listar_etiquetas(
    State(db): State<Db>,
    Query(f): Query<SoloProyecto>,
) -> Respuesta<Json<serde_json::Value>> {
    let con = db.lock().unwrap();
    // Con proyecto, los chips son solo los de ese proyecto (D15).
    let mut sql = String::from(
        "SELECT e.etiqueta, COUNT(*) FROM etiquetas e JOIN notas n ON n.id = e.nota_id WHERE 1=1",
    );
    let mut args: Vec<String> = vec![];
    sql_proyecto(
        &mut sql,
        &mut args,
        "n.proyecto_id",
        f.proyecto.as_deref(),
        es_solo(f.solo.as_deref()),
    );
    sql.push_str(" GROUP BY e.etiqueta ORDER BY 2 DESC, 1");
    let res: rusqlite::Result<Vec<serde_json::Value>> = con.prepare(&sql).and_then(|mut s| {
        s.query_map(rusqlite::params_from_iter(args.iter()), |f| {
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
            json!({ "ok": true, "build": build, "token": token_api().is_some(), "ia": ia().is_some(), "ffmpeg": entrevistas::hay_ffmpeg() }),
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
        .route("/notas/mover", post(mover_notas))
        .route(
            "/notas/{id}",
            get(ver_nota)
                .put(editar_nota)
                .delete(borrar_nota)
                .patch(cambiar_nota),
        )
        .route("/notas/{id}/reintentar-ia", post(reintentar_ia))
        .route("/etiquetas", get(listar_etiquetas))
        .route("/proyectos", get(listar_proyectos).post(crear_proyecto))
        .route(
            "/proyectos/{id}",
            get(ver_proyecto)
                .patch(editar_proyecto)
                .delete(borrar_proyecto),
        )
        .route("/documentos", get(listar_documentos).post(crear_documento))
        .route(
            "/documentos/{id}",
            get(ver_documento)
                .put(editar_documento)
                .delete(borrar_documento),
        )
        .route("/documentos/{id}/regenerar", post(regenerar_documento))
        .route("/documentos/{id}/reintentar", post(analisis::reintentar))
        .route(
            "/roles",
            get(analisis::listar_roles).post(analisis::crear_rol),
        )
        .route(
            "/roles/{id}",
            axum::routing::put(analisis::editar_rol).delete(analisis::borrar_rol),
        )
        .route("/modelos", get(analisis::listar_modelos))
        .route(
            "/entrevistas",
            get(entrevistas::listar)
                .post(entrevistas::crear)
                .layer(DefaultBodyLimit::max(entrevistas::MAXIMO_SUBIDA)),
        )
        .route(
            "/entrevistas/{id}",
            get(entrevistas::ver)
                .put(entrevistas::editar)
                .delete(entrevistas::borrar),
        )
        .route(
            "/entrevistas/{id}/audio",
            get(entrevistas::audio).delete(entrevistas::borrar_audio),
        )
        .route(
            "/entrevistas/{id}/transcribir",
            post(entrevistas::transcribir),
        )
        .route("/entrevistas/{id}/resumir", post(entrevistas::resumir))
        .route(
            "/entrevistas/{id}/nota",
            post(entrevistas::guardar_como_nota),
        )
        .route("/entrevistas/{id}/mover", post(entrevistas::mover))
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
    if !entrevistas::hay_ffmpeg() {
        println!("AVISO: sin ffmpeg, las entrevistas se graban pero no se pueden transcribir");
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
