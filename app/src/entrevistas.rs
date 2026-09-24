//! Entrevistas (D27 a D42): una grabación con lo que sale de ella. El audio
//! vive en el volumen, junto a la base (D29); la base guarda metadatos, la
//! transcripción por trozos (D31) y los dos resúmenes (D33).
//!
//! Transcribir y resumir van en segundo plano, como los documentos (D11): la
//! petición contesta al momento y la app pregunta por el estado.

use std::{
    path::{Path as RutaFs, PathBuf},
    sync::OnceLock,
    time::Duration,
};

use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::{
    completar_con_ia, error, es_solo, ia, interno, leer_nota, llm, proyecto_de_fila,
    proyecto_valido, sql_proyecto, Db, ProyectoRef, Respuesta, CONTEXTO_DESCRIPCION,
    CONTEXTO_NOMBRE,
};

/// Una hora de audio a 32 kbps son unos 14 MB; el tope deja sitio de sobra
/// para un fichero subido desde la grabadora del sistema, que pesa más.
pub const MAXIMO_SUBIDA: usize = 200 * 1024 * 1024;

/// D31: cinco minutos por trozo caben en el límite de 2 MB del proxy.
const SEGUNDOS_POR_TROZO: u64 = 300;

/// Esperas entre reintentos de un trozo. Tres intentos iguales en un minuto
/// no llegan a lo que el proxy considera un bucle (cinco).
const REINTENTOS: [u64; 2] = [5, 15];

/// Formatos que se aceptan al subir; `ffmpeg` los lee todos.
const EXTENSIONES: [&str; 12] = [
    "webm", "ogg", "oga", "opus", "m4a", "mp4", "mp3", "wav", "aac", "3gp", "amr", "flac",
];

/// Carpeta del audio: al lado de la base, así en Fly cae en el volumen.
pub fn dir_audio() -> PathBuf {
    let db = crate::ruta_db();
    RutaFs::new(&db)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| RutaFs::new("."))
        .join("audio")
}

/// `ffmpeg` hace falta para transcribir, no para grabar: sin él la app guarda
/// y reproduce igual, y `/salud` lo delata.
pub fn hay_ffmpeg() -> bool {
    static HAY: OnceLock<bool> = OnceLock::new();
    *HAY.get_or_init(|| {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// Al arrancar: lo que se estaba haciendo cuando el servicio se paró no lo
/// termina nadie. La transcripción queda «fallida» con sus trozos hechos, para
/// que Reintentar siga donde estaba; las subidas a medias se borran.
pub fn recuperar(con: &Connection) {
    let _ = con.execute(
        "UPDATE entrevistas SET estado = 'fallida',
         error = 'la transcripción se interrumpió al reiniciarse el servicio; Reintentar sigue donde se quedó'
         WHERE estado = 'transcribiendo'",
        [],
    );
    let _ = con.execute(
        "UPDATE entrevistas SET estado_resumen = 'fallido',
         error_resumen = 'el resumen se interrumpió al reiniciarse el servicio'
         WHERE estado_resumen = 'pendiente'",
        [],
    );
    let dir = dir_audio();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("no se pudo crear {}: {e}", dir.display());
        return;
    }
    if let Ok(ficheros) = std::fs::read_dir(&dir) {
        for f in ficheros.flatten() {
            if f.path().extension().is_some_and(|e| e == "parcial") {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }
}

#[derive(Serialize)]
pub struct Entrevista {
    id: String,
    titulo: String,
    con_quien: String,
    creada_en: String,
    duracion_s: i64,
    audio: bool,
    audio_bytes: i64,
    audio_tipo: String,
    estado: String,
    trozos_total: i64,
    trozos_hechos: i64,
    error: String,
    estado_resumen: String,
    error_resumen: String,
    proyecto: Option<ProyectoRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transcripcion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resumen_ejecutivo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resumen: Option<String>,
}

const SQL_ENTREVISTA: &str = "SELECT e.id, e.titulo, e.con_quien, e.creada_en, e.duracion_s,
    e.audio IS NOT NULL, e.audio_bytes, e.audio_tipo, e.estado, e.trozos_total, e.trozos_hechos,
    e.error, e.estado_resumen, e.error_resumen, p.id, p.nombre, pp.id, pp.nombre,
    e.transcripcion, e.resumen_ejecutivo, e.resumen
    FROM entrevistas e LEFT JOIN proyectos p ON p.id = e.proyecto_id
    LEFT JOIN proyectos pp ON pp.id = p.padre_id";

fn fila(f: &rusqlite::Row, textos: bool) -> rusqlite::Result<Entrevista> {
    Ok(Entrevista {
        id: f.get(0)?,
        titulo: f.get(1)?,
        con_quien: f.get(2)?,
        creada_en: f.get(3)?,
        duracion_s: f.get(4)?,
        audio: f.get(5)?,
        audio_bytes: f.get(6)?,
        audio_tipo: f.get(7)?,
        estado: f.get(8)?,
        trozos_total: f.get(9)?,
        trozos_hechos: f.get(10)?,
        error: f.get(11)?,
        estado_resumen: f.get(12)?,
        error_resumen: f.get(13)?,
        proyecto: proyecto_de_fila(f, 14)?,
        transcripcion: if textos { Some(f.get(18)?) } else { None },
        resumen_ejecutivo: if textos { Some(f.get(19)?) } else { None },
        resumen: if textos { Some(f.get(20)?) } else { None },
    })
}

fn leer(con: &Connection, id: &str) -> rusqlite::Result<Option<Entrevista>> {
    con.prepare_cached(&format!("{SQL_ENTREVISTA} WHERE e.id = ?1"))?
        .query_row([id], |f| fila(f, true))
        .optional()
}

fn existe_o_404(con: &Connection, id: &str) -> Respuesta<Entrevista> {
    match leer(con, id) {
        Ok(Some(e)) => Ok(e),
        Ok(None) => error(StatusCode::NOT_FOUND, "entrevista no encontrada"),
        Err(e) => interno(e),
    }
}

#[derive(Deserialize, Default)]
pub struct FiltroEntrevistas {
    proyecto: Option<String>,
    /// `solo=1`: sin las de sus subcarpetas (D53).
    solo: Option<String>,
    /// `textos=1` trae transcripción y resúmenes: solo lo usa la exportación.
    textos: Option<String>,
}

pub async fn listar(
    State(db): State<Db>,
    Query(f): Query<FiltroEntrevistas>,
) -> Respuesta<Json<Vec<Entrevista>>> {
    let con = db.lock().unwrap();
    let mut sql = format!("{SQL_ENTREVISTA} WHERE 1=1");
    let mut args: Vec<String> = vec![];
    sql_proyecto(
        &mut sql,
        &mut args,
        "e.proyecto_id",
        f.proyecto.as_deref(),
        es_solo(f.solo.as_deref()),
    );
    sql.push_str(" ORDER BY e.creada_en DESC LIMIT 500");
    let textos = f.textos.as_deref() == Some("1");
    let lista = con.prepare(&sql).and_then(|mut st| {
        st.query_map(rusqlite::params_from_iter(args.iter()), |r| fila(r, textos))?
            .collect::<rusqlite::Result<Vec<_>>>()
    });
    match lista {
        Ok(l) => Ok(Json(l)),
        Err(e) => interno(e),
    }
}

pub async fn ver(State(db): State<Db>, Path(id): Path<String>) -> Respuesta<Json<Entrevista>> {
    let con = db.lock().unwrap();
    existe_o_404(&con, &id).map(Json)
}

/// Extensión del fichero subido: la del nombre si es conocida; si no, la que
/// corresponde a su tipo. Lo demás no es audio que podamos leer.
fn extension(nombre: Option<&str>, tipo: Option<&str>) -> Option<&'static str> {
    if let Some(ext) = nombre
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, e)| e.to_lowercase())
    {
        if let Some(e) = EXTENSIONES.iter().find(|x| **x == ext) {
            return Some(e);
        }
    }
    let tipo = tipo?.split(';').next()?.trim().to_lowercase();
    Some(match tipo.as_str() {
        "audio/webm" | "video/webm" => "webm",
        "audio/ogg" | "application/ogg" => "ogg",
        "audio/mp4" | "audio/x-m4a" | "audio/m4a" | "video/mp4" => "m4a",
        "audio/aac" => "aac",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => "wav",
        "audio/3gpp" | "video/3gpp" => "3gp",
        "audio/amr" => "amr",
        "audio/flac" | "audio/x-flac" => "flac",
        _ => return None,
    })
}

fn tipo_de(ext: &str) -> &'static str {
    match ext {
        "webm" => "audio/webm",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "m4a" | "mp4" => "audio/mp4",
        "aac" => "audio/aac",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "3gp" => "audio/3gpp",
        "amr" => "audio/amr",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
}

/// `POST /entrevistas` (multipart): `audio`, y opcionales `proyecto`,
/// `con_quien`, `duracion_s` y `titulo`. El audio se vuelca a disco según
/// llega, sin cargarlo en memoria (256 MB de máquina), a un `.parcial` que
/// solo se renombra cuando está entero.
pub async fn crear(State(db): State<Db>, mut partes: Multipart) -> Respuesta<impl IntoResponse> {
    let id: String = {
        let con = db.lock().unwrap();
        match con.query_row("SELECT lower(hex(randomblob(8)))", [], |f| f.get(0)) {
            Ok(id) => id,
            Err(e) => return interno(e),
        }
    };
    let dir = dir_audio();
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        eprintln!("audio: no se pudo crear {}: {e}", dir.display());
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "no se pudo guardar el audio",
        );
    }
    let parcial = dir.join(format!("{id}.parcial"));
    let resultado = recibir(&mut partes, &parcial).await;
    let (campos, ext, bytes) = match resultado {
        Ok(r) => r,
        Err(fallo) => {
            let _ = tokio::fs::remove_file(&parcial).await;
            return Err(fallo);
        }
    };
    let fichero = format!("{id}.{ext}");
    if let Err(e) = tokio::fs::rename(&parcial, dir.join(&fichero)).await {
        let _ = tokio::fs::remove_file(&parcial).await;
        eprintln!("audio: no se pudo renombrar {}: {e}", parcial.display());
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "no se pudo guardar el audio",
        );
    }

    let con = db.lock().unwrap();
    let proyecto = match proyecto_valido(&con, campos.proyecto) {
        Ok(p) => p,
        Err(fallo) => {
            let _ = std::fs::remove_file(dir.join(&fichero));
            return Err(fallo);
        }
    };
    let titulo = campos
        .titulo
        .map(|t| t.trim().chars().take(120).collect::<String>())
        .filter(|t| !t.is_empty());
    let insertado = con.execute(
        "INSERT INTO entrevistas (id, proyecto_id, titulo, con_quien, creada_en, duracion_s,
           audio, audio_tipo, audio_bytes)
         VALUES (?1, ?2, COALESCE(?3, 'Entrevista · ' || strftime('%d/%m %H:%M', 'now') || ' UTC'),
           ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?5, ?6, ?7, ?8)",
        params![
            id,
            proyecto,
            titulo,
            campos
                .con_quien
                .trim()
                .chars()
                .take(120)
                .collect::<String>(),
            campos.duracion_s.max(0),
            fichero,
            tipo_de(ext),
            bytes as i64
        ],
    );
    if let Err(e) = insertado {
        let _ = std::fs::remove_file(dir.join(&fichero));
        return interno(e);
    }
    println!("entrevista {id}: {bytes} bytes de audio ({ext})");
    match leer(&con, &id) {
        Ok(Some(e)) => Ok((StatusCode::CREATED, Json(e))),
        Ok(None) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "la entrevista no se guardó",
        ),
        Err(e) => interno(e),
    }
}

#[derive(Default)]
struct Campos {
    proyecto: Option<String>,
    con_quien: String,
    duracion_s: i64,
    titulo: Option<String>,
}

async fn recibir(
    partes: &mut Multipart,
    destino: &RutaFs,
) -> Respuesta<(Campos, &'static str, u64)> {
    let mut campos = Campos::default();
    let mut audio: Option<(&'static str, u64)> = None;
    loop {
        let parte = match partes.next_field().await {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => return error(StatusCode::BAD_REQUEST, format!("subida incompleta: {e}")),
        };
        let nombre = parte.name().unwrap_or_default().to_string();
        if nombre == "audio" {
            let Some(ext) = extension(parte.file_name(), parte.content_type()) else {
                return error(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "formato de audio no admitido (webm, ogg, m4a, mp3, wav, aac, 3gp, amr, flac)",
                );
            };
            let mut f = match tokio::fs::File::create(destino).await {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("audio: no se pudo crear {}: {e}", destino.display());
                    return error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "no se pudo guardar el audio",
                    );
                }
            };
            let mut parte = parte;
            let mut bytes = 0u64;
            loop {
                match parte.chunk().await {
                    Ok(Some(trozo)) => {
                        bytes += trozo.len() as u64;
                        if let Err(e) = f.write_all(&trozo).await {
                            eprintln!("audio: escritura fallida: {e}");
                            return error(
                                StatusCode::INSUFFICIENT_STORAGE,
                                "no queda sitio para el audio",
                            );
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        return error(StatusCode::BAD_REQUEST, format!("subida cortada: {e}"))
                    }
                }
            }
            if let Err(e) = f.flush().await {
                eprintln!("audio: {e}");
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "no se pudo guardar el audio",
                );
            }
            if bytes == 0 {
                return error(StatusCode::BAD_REQUEST, "el audio está vacío");
            }
            audio = Some((ext, bytes));
        } else {
            let valor = parte.text().await.unwrap_or_default();
            match nombre.as_str() {
                "proyecto" => campos.proyecto = Some(valor),
                "con_quien" => campos.con_quien = valor,
                "duracion_s" => {
                    campos.duracion_s = valor.trim().parse::<f64>().unwrap_or(0.0).round() as i64
                }
                "titulo" => campos.titulo = Some(valor),
                _ => {}
            }
        }
    }
    match audio {
        Some((ext, bytes)) => Ok((campos, ext, bytes)),
        None => error(StatusCode::BAD_REQUEST, "falta el campo audio"),
    }
}

#[derive(Deserialize)]
pub struct EdicionEntrevista {
    titulo: Option<String>,
    con_quien: Option<String>,
    transcripcion: Option<String>,
    resumen_ejecutivo: Option<String>,
    resumen: Option<String>,
}

/// `PUT /entrevistas/{id}`: cada campo que venga se sustituye; los que no, se
/// quedan. Un título puesto a mano ya no lo pisa el resumen (D34).
pub async fn editar(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(c): Json<EdicionEntrevista>,
) -> Respuesta<Json<Entrevista>> {
    let con = db.lock().unwrap();
    let actual = existe_o_404(&con, &id)?;
    if c.transcripcion.is_some() && actual.estado == "transcribiendo" {
        return error(
            StatusCode::CONFLICT,
            "se está transcribiendo; espera a que acabe",
        );
    }
    let escribir = || -> rusqlite::Result<()> {
        let tx = con.unchecked_transaction()?;
        if let Some(t) = c.titulo.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            let t: String = t.chars().take(120).collect();
            tx.execute(
                "UPDATE entrevistas SET titulo = ?1, titulo_editado = 1 WHERE id = ?2",
                params![t, id],
            )?;
        }
        if let Some(q) = &c.con_quien {
            let q: String = q.trim().chars().take(120).collect();
            tx.execute(
                "UPDATE entrevistas SET con_quien = ?1 WHERE id = ?2",
                params![q, id],
            )?;
        }
        if let Some(t) = &c.transcripcion {
            // Editada a mano, pasa a ser la buena: aunque no hubiera acabado de
            // transcribirse, lo que hay es lo que el usuario da por válido.
            tx.execute(
                "UPDATE entrevistas SET transcripcion = ?1,
                 estado = CASE WHEN ?1 = '' THEN estado ELSE 'transcrita' END, error = ''
                 WHERE id = ?2",
                params![t.trim(), id],
            )?;
        }
        if let Some(r) = &c.resumen_ejecutivo {
            tx.execute(
                "UPDATE entrevistas SET resumen_ejecutivo = ?1 WHERE id = ?2",
                params![r.trim(), id],
            )?;
        }
        if let Some(r) = &c.resumen {
            tx.execute(
                "UPDATE entrevistas SET resumen = ?1 WHERE id = ?2",
                params![r.trim(), id],
            )?;
        }
        tx.commit()
    };
    if let Err(e) = escribir() {
        return interno(e);
    }
    existe_o_404(&con, &id).map(Json)
}

#[derive(Deserialize)]
pub struct Destino {
    proyecto: Option<String>,
}

/// `POST /entrevistas/{id}/mover` `{proyecto: id|null}`: como mover una nota.
pub async fn mover(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(d): Json<Destino>,
) -> Respuesta<Json<Entrevista>> {
    let con = db.lock().unwrap();
    existe_o_404(&con, &id)?;
    let proyecto = proyecto_valido(&con, d.proyecto)?;
    if let Err(e) = con.execute(
        "UPDATE entrevistas SET proyecto_id = ?1 WHERE id = ?2",
        params![proyecto, id],
    ) {
        return interno(e);
    }
    existe_o_404(&con, &id).map(Json)
}

fn fichero_de(con: &Connection, id: &str) -> Respuesta<Option<String>> {
    match con
        .query_row("SELECT audio FROM entrevistas WHERE id = ?1", [id], |f| {
            f.get::<_, Option<String>>(0)
        })
        .optional()
    {
        Ok(Some(a)) => Ok(a),
        Ok(None) => error(StatusCode::NOT_FOUND, "entrevista no encontrada"),
        Err(e) => interno(e),
    }
}

/// `DELETE /entrevistas/{id}`: la fila (y sus trozos, en cascada) y después el
/// fichero. Si el fichero no se deja borrar se anota y ya: la entrevista ya no
/// existe para nadie.
pub async fn borrar(State(db): State<Db>, Path(id): Path<String>) -> Respuesta<StatusCode> {
    let fichero = {
        let con = db.lock().unwrap();
        let fichero = fichero_de(&con, &id)?;
        if let Err(e) = con.execute("DELETE FROM entrevistas WHERE id = ?1", [&id]) {
            return interno(e);
        }
        fichero
    };
    if let Some(f) = fichero {
        if let Err(e) = tokio::fs::remove_file(dir_audio().join(&f)).await {
            eprintln!("audio: no se pudo borrar {f}: {e}");
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /entrevistas/{id}/audio` (D29): solo el audio, con la
/// transcripción hecha; sin ella, borrar el audio sería perder la entrevista.
pub async fn borrar_audio(
    State(db): State<Db>,
    Path(id): Path<String>,
) -> Respuesta<Json<Entrevista>> {
    let fichero = {
        let con = db.lock().unwrap();
        let e = existe_o_404(&con, &id)?;
        if e.estado != "transcrita" {
            return error(
                StatusCode::CONFLICT,
                "sin transcripción, borrar el audio perdería la entrevista",
            );
        }
        let fichero = fichero_de(&con, &id)?;
        if let Err(e) = con.execute(
            "UPDATE entrevistas SET audio = NULL, audio_bytes = 0 WHERE id = ?1",
            [&id],
        ) {
            return interno(e);
        }
        fichero
    };
    if let Some(f) = fichero {
        if let Err(e) = tokio::fs::remove_file(dir_audio().join(&f)).await {
            eprintln!("audio: no se pudo borrar {f}: {e}");
        }
    }
    let con = db.lock().unwrap();
    existe_o_404(&con, &id).map(Json)
}

/// Rango pedido en `Range: bytes=a-b` (uno solo; los múltiples no los usa
/// ningún reproductor). `None` si no hay cabecera o no se entiende.
fn rango(cabeceras: &HeaderMap, total: u64) -> Option<Result<(u64, u64), ()>> {
    let valor = cabeceras.get(header::RANGE)?.to_str().ok()?;
    let spec = valor.strip_prefix("bytes=")?.trim();
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let r = match (a.trim(), b.trim()) {
        ("", sufijo) => {
            let n: u64 = sufijo.parse().ok()?;
            if n == 0 || total == 0 {
                return Some(Err(()));
            }
            (total.saturating_sub(n), total - 1)
        }
        (ini, "") => (ini.parse().ok()?, total.saturating_sub(1)),
        (ini, fin) => (
            ini.parse().ok()?,
            fin.parse::<u64>().ok()?.min(total.saturating_sub(1)),
        ),
    };
    if r.0 > r.1 || r.0 >= total {
        return Some(Err(()));
    }
    Some(Ok(r))
}

/// `GET /entrevistas/{id}/audio`, con `Range` para los reproductores que lo
/// piden. La app lo baja entero con su token y lo reproduce desde un blob.
pub async fn audio(
    State(db): State<Db>,
    Path(id): Path<String>,
    cabeceras: HeaderMap,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let (fichero, tipo) = {
        let con = db.lock().unwrap();
        let e = existe_o_404(&con, &id)?;
        match fichero_de(&con, &id)? {
            Some(f) => (f, e.audio_tipo),
            None => return error(StatusCode::GONE, "el audio de esta entrevista se borró"),
        }
    };
    let ruta = dir_audio().join(&fichero);
    let mut f = match tokio::fs::File::open(&ruta).await {
        Ok(f) => f,
        Err(e) => {
            eprintln!("audio: no se pudo abrir {}: {e}", ruta.display());
            return error(StatusCode::NOT_FOUND, "el fichero de audio no está");
        }
    };
    let total = f.metadata().await.map(|m| m.len()).unwrap_or(0);
    let (estado, ini, fin) = match rango(&cabeceras, total) {
        None => (StatusCode::OK, 0, total.saturating_sub(1)),
        Some(Ok((a, b))) => (StatusCode::PARTIAL_CONTENT, a, b),
        Some(Err(())) => {
            return Ok((
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{total}"))],
            )
                .into_response())
        }
    };
    let largo = if total == 0 { 0 } else { fin - ini + 1 };
    let mut datos = vec![0u8; largo as usize];
    let leido = async {
        f.seek(std::io::SeekFrom::Start(ini)).await?;
        f.read_exact(&mut datos).await
    }
    .await;
    if let Err(e) = leido {
        eprintln!("audio: lectura fallida de {}: {e}", ruta.display());
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "no se pudo leer el audio",
        );
    }
    let mut r = Response::builder()
        .status(estado)
        .header(header::CONTENT_TYPE, tipo)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, largo)
        .header(header::CACHE_CONTROL, "private, max-age=3600");
    if estado == StatusCode::PARTIAL_CONTENT {
        r = r.header(header::CONTENT_RANGE, format!("bytes {ini}-{fin}/{total}"));
    }
    Ok(r.body(Body::from(datos)).unwrap())
}

// --- Transcripción (D30-D32) ---

/// `POST /entrevistas/{id}/transcribir`: arranca o, si falló, sigue desde el
/// último trozo hecho. Transcribir otra vez algo ya transcrito empieza de cero.
pub async fn transcribir(
    State(db): State<Db>,
    Path(id): Path<String>,
) -> Respuesta<(StatusCode, Json<Entrevista>)> {
    if ia().is_none() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no hay IA configurada");
    }
    if !hay_ffmpeg() {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "falta ffmpeg en el servidor",
        );
    }
    {
        let con = db.lock().unwrap();
        let e = existe_o_404(&con, &id)?;
        if !e.audio {
            return error(StatusCode::CONFLICT, "el audio de esta entrevista se borró");
        }
        if e.estado == "transcribiendo" {
            return error(StatusCode::CONFLICT, "ya se está transcribiendo");
        }
        let desde_cero = e.estado != "fallida";
        let marcar = || -> rusqlite::Result<()> {
            let tx = con.unchecked_transaction()?;
            if desde_cero {
                tx.execute(
                    "DELETE FROM entrevista_trozos WHERE entrevista_id = ?1",
                    [&id],
                )?;
                tx.execute(
                    "UPDATE entrevistas SET trozos_hechos = 0, trozos_total = 0 WHERE id = ?1",
                    [&id],
                )?;
            }
            tx.execute(
                "UPDATE entrevistas SET estado = 'transcribiendo', error = '' WHERE id = ?1",
                [&id],
            )?;
            tx.commit()
        };
        if let Err(e) = marcar() {
            return interno(e);
        }
    }
    let db2 = db.clone();
    let id2 = id.clone();
    tokio::spawn(async move {
        if let Err(e) = transcribir_tarea(&db2, &id2).await {
            eprintln!("entrevista {id2}: {e}");
            let con = db2.lock().unwrap();
            let _ = con.execute(
                "UPDATE entrevistas SET estado = 'fallida', error = ?1 WHERE id = ?2",
                params![e, id2],
            );
        }
    });
    let con = db.lock().unwrap();
    existe_o_404(&con, &id).map(|e| (StatusCode::ACCEPTED, Json(e)))
}

struct DatosTranscripcion {
    fichero: String,
    con_quien: String,
    proyecto: Option<llm::Proyecto>,
    hechos: usize,
}

async fn transcribir_tarea(db: &Db, id: &str) -> Result<(), String> {
    let ia = ia().ok_or("no hay IA configurada")?;
    let datos = {
        let con = db.lock().unwrap();
        con.query_row(
            &format!(
                "SELECT e.audio, e.con_quien, {CONTEXTO_NOMBRE}, {CONTEXTO_DESCRIPCION}, e.trozos_hechos
                 FROM entrevistas e LEFT JOIN proyectos p ON p.id = e.proyecto_id
                 LEFT JOIN proyectos pp ON pp.id = p.padre_id WHERE e.id = ?1"
            ),
            [id],
            |f| {
                Ok(DatosTranscripcion {
                    fichero: f.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    con_quien: f.get(1)?,
                    proyecto: f.get::<_, Option<String>>(2)?.map(|nombre| llm::Proyecto {
                        nombre,
                        descripcion: f
                            .get::<_, Option<String>>(3)
                            .ok()
                            .flatten()
                            .unwrap_or_default(),
                    }),
                    hechos: f.get::<_, i64>(4)?.max(0) as usize,
                })
            },
        )
        .map_err(|_| "la entrevista ya no está".to_string())?
    };
    if datos.fichero.is_empty() {
        return Err("el audio de esta entrevista se borró".into());
    }
    let origen = dir_audio().join(&datos.fichero);
    let tmp = std::env::temp_dir().join(format!("entrevista-{id}"));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp)
        .await
        .map_err(|e| format!("no se pudo preparar el troceo: {e}"))?;
    let resultado = transcribir_trozos(db, id, ia, &datos, &origen, &tmp).await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    resultado
}

async fn transcribir_trozos(
    db: &Db,
    id: &str,
    ia: &llm::Llm,
    datos: &DatosTranscripcion,
    origen: &RutaFs,
    tmp: &RutaFs,
) -> Result<(), String> {
    // La grabación del navegador no trae la duración en la cabecera: la mide
    // ffprobe si puede, y si no se queda la que mandó la app.
    if let Some(d) = medir(origen).await {
        let con = db.lock().unwrap();
        let _ = con.execute(
            "UPDATE entrevistas SET duracion_s = ?1 WHERE id = ?2",
            params![d, id],
        );
    }
    let trozos = trocear(origen, tmp).await?;
    let total = trozos.len();
    if total == 0 {
        return Err("el audio no tiene nada que transcribir".into());
    }
    {
        let con = db.lock().unwrap();
        con.execute(
            "UPDATE entrevistas SET trozos_total = ?1 WHERE id = ?2",
            params![total as i64, id],
        )
        .map_err(|e| format!("no se pudo anotar el troceo: {e}"))?;
    }
    let desde = datos.hechos.min(total);
    for (n, trozo) in trozos.iter().enumerate().skip(desde) {
        let mp3 = tokio::fs::read(trozo)
            .await
            .map_err(|e| format!("no se pudo leer el trozo {}: {e}", n + 1))?;
        let cola = {
            let con = db.lock().unwrap();
            con.query_row(
                "SELECT texto FROM entrevista_trozos WHERE entrevista_id = ?1 AND n = ?2",
                params![id, n as i64 - 1],
                |f| f.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(|t| ultimas_lineas(&t, 20))
            .unwrap_or_default()
        };
        let ctx = llm::ContextoTrozo {
            n,
            total,
            con_quien: &datos.con_quien,
            proyecto: datos.proyecto.as_ref(),
            cola_anterior: &cola,
        };
        let mut intento = 0;
        let texto = loop {
            match ia.transcribir(&mp3, &ctx).await {
                Ok(t) => break t,
                Err(e) if intento < REINTENTOS.len() => {
                    eprintln!("entrevista {id}: trozo {} falló ({e}); reintento", n + 1);
                    tokio::time::sleep(Duration::from_secs(REINTENTOS[intento])).await;
                    intento += 1;
                }
                Err(e) => return Err(format!("trozo {} de {total}: {e}", n + 1)),
            }
        };
        let texto = desplazar_tiempos(&texto, n as u64 * SEGUNDOS_POR_TROZO);
        let con = db.lock().unwrap();
        let guardar = || -> rusqlite::Result<()> {
            let tx = con.unchecked_transaction()?;
            tx.execute(
                "INSERT OR REPLACE INTO entrevista_trozos (entrevista_id, n, texto) VALUES (?1, ?2, ?3)",
                params![id, n as i64, texto],
            )?;
            tx.execute(
                "UPDATE entrevistas SET trozos_hechos = ?1 WHERE id = ?2",
                params![n as i64 + 1, id],
            )?;
            tx.commit()
        };
        guardar().map_err(|e| format!("no se pudo guardar el trozo {}: {e}", n + 1))?;
    }
    let con = db.lock().unwrap();
    let completa: String = con
        .prepare("SELECT texto FROM entrevista_trozos WHERE entrevista_id = ?1 ORDER BY n")
        .and_then(|mut st| {
            st.query_map([id], |f| f.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| format!("no se pudo leer la transcripción: {e}"))?
        .join("\n");
    con.execute(
        "UPDATE entrevistas SET transcripcion = ?1, estado = 'transcrita', error = '' WHERE id = ?2",
        params![completa, id],
    )
    .map_err(|e| format!("no se pudo guardar la transcripción: {e}"))?;
    println!("entrevista {id}: transcrita en {total} trozo(s)");
    Ok(())
}

/// Duración en segundos según ffprobe; `None` si no la sabe (webm del
/// navegador) o si no está.
async fn medir(fichero: &RutaFs) -> Option<i64> {
    let salida = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(fichero)
        .output()
        .await
        .ok()?;
    let texto = String::from_utf8_lossy(&salida.stdout);
    let s: f64 = texto.trim().parse().ok()?;
    (s.is_finite() && s > 0.0).then(|| s.round() as i64)
}

/// D31: mp3 mono de 16 kHz y 24 kbps en trozos de cinco minutos. Devuelve los
/// ficheros en orden.
async fn trocear(origen: &RutaFs, tmp: &RutaFs) -> Result<Vec<PathBuf>, String> {
    let salida = tokio::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
        .arg(origen)
        .args([
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-b:a",
            "24k",
            "-f",
            "segment",
            "-segment_time",
            &SEGUNDOS_POR_TROZO.to_string(),
            "-reset_timestamps",
            "1",
        ])
        .arg(tmp.join("%03d.mp3"))
        .output()
        .await
        .map_err(|e| format!("no se pudo lanzar ffmpeg: {e}"))?;
    if !salida.status.success() {
        let err = String::from_utf8_lossy(&salida.stderr);
        let err: String = err.trim().chars().take(300).collect();
        return Err(format!("ffmpeg no pudo leer el audio: {err}"));
    }
    let mut trozos: Vec<PathBuf> = std::fs::read_dir(tmp)
        .map_err(|e| format!("no se pudieron listar los trozos: {e}"))?
        .flatten()
        .map(|f| f.path())
        .filter(|p| p.extension().is_some_and(|e| e == "mp3"))
        // Un trozo final de menos de un segundo no tiene nada que decir.
        .filter(|p| {
            std::fs::metadata(p)
                .map(|m| m.len() > 3_000)
                .unwrap_or(false)
        })
        .collect();
    trozos.sort();
    Ok(trozos)
}

fn ultimas_lineas(texto: &str, n: usize) -> String {
    let lineas: Vec<&str> = texto.lines().filter(|l| !l.trim().is_empty()).collect();
    lineas[lineas.len().saturating_sub(n)..].join("\n")
}

/// Formato de tiempo de la transcripción: `mm:ss`, o `h:mm:ss` pasada la hora.
fn reloj(s: u64) -> String {
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// Lee `[mm:ss]` o `[h:mm:ss]` al principio de una línea.
fn leer_marca(linea: &str) -> Option<(u64, &str)> {
    let resto = linea.trim_start().strip_prefix('[')?;
    let (marca, tras) = resto.split_once(']')?;
    let partes: Vec<u64> = marca
        .split(':')
        .map(|p| p.trim().parse::<u64>())
        .collect::<Result<_, _>>()
        .ok()?;
    let s = match partes.as_slice() {
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3600 + m * 60 + s,
        _ => return None,
    };
    Some((s, tras))
}

/// El modelo cuenta el tiempo desde el principio de su trozo; aquí se pasa al
/// tiempo de la entrevista. Las líneas sin marca se quedan como están.
fn desplazar_tiempos(texto: &str, desplazamiento: u64) -> String {
    texto
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| match leer_marca(l) {
            Some((s, tras)) => format!("[{}]{}", reloj(s + desplazamiento), tras),
            None => l.trim_end().to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// --- Resúmenes (D33) ---

/// `POST /entrevistas/{id}/resumir`: los dos resúmenes en segundo plano.
pub async fn resumir(
    State(db): State<Db>,
    Path(id): Path<String>,
) -> Respuesta<(StatusCode, Json<Entrevista>)> {
    if ia().is_none() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no hay IA configurada");
    }
    {
        let con = db.lock().unwrap();
        let e = existe_o_404(&con, &id)?;
        if e.estado != "transcrita" || e.transcripcion.as_deref().unwrap_or("").trim().is_empty() {
            return error(StatusCode::CONFLICT, "primero hay que transcribirla");
        }
        if e.estado_resumen == "pendiente" {
            return error(StatusCode::CONFLICT, "ya se está resumiendo");
        }
        if let Err(e) = con.execute(
            "UPDATE entrevistas SET estado_resumen = 'pendiente', error_resumen = '' WHERE id = ?1",
            [&id],
        ) {
            return interno(e);
        }
    }
    let db2 = db.clone();
    let id2 = id.clone();
    tokio::spawn(async move {
        if let Err(e) = resumir_tarea(&db2, &id2).await {
            eprintln!("entrevista {id2}: resumen: {e}");
            let con = db2.lock().unwrap();
            let _ = con.execute(
                "UPDATE entrevistas SET estado_resumen = 'fallido', error_resumen = ?1 WHERE id = ?2",
                params![e, id2],
            );
        }
    });
    let con = db.lock().unwrap();
    existe_o_404(&con, &id).map(|e| (StatusCode::ACCEPTED, Json(e)))
}

async fn resumir_tarea(db: &Db, id: &str) -> Result<(), String> {
    let ia = ia().ok_or("no hay IA configurada")?;
    let (transcripcion, con_quien, duracion, proyecto) = {
        let con = db.lock().unwrap();
        con.query_row(
            &format!(
                "SELECT e.transcripcion, e.con_quien, e.duracion_s, {CONTEXTO_NOMBRE}, {CONTEXTO_DESCRIPCION}
                 FROM entrevistas e LEFT JOIN proyectos p ON p.id = e.proyecto_id
                 LEFT JOIN proyectos pp ON pp.id = p.padre_id WHERE e.id = ?1"
            ),
            [id],
            |f| {
                Ok((
                    f.get::<_, String>(0)?,
                    f.get::<_, String>(1)?,
                    f.get::<_, i64>(2)?,
                    f.get::<_, Option<String>>(3)?.map(|nombre| llm::Proyecto {
                        nombre,
                        descripcion: f
                            .get::<_, Option<String>>(4)
                            .ok()
                            .flatten()
                            .unwrap_or_default(),
                    }),
                ))
            },
        )
        .map_err(|_| "la entrevista ya no está".to_string())?
    };
    let r = ia
        .resumir_entrevista(
            &transcripcion,
            &con_quien,
            &reloj(duracion.max(0) as u64),
            proyecto.as_ref(),
        )
        .await?;
    let amplio = amplio_en_markdown(&r.amplio);
    let con = db.lock().unwrap();
    con.execute(
        "UPDATE entrevistas SET resumen_ejecutivo = ?1, resumen = ?2, estado_resumen = 'listo',
         error_resumen = '',
         titulo = CASE WHEN titulo_editado = 0 AND ?3 <> '' THEN ?3 ELSE titulo END
         WHERE id = ?4",
        params![r.ejecutivo, amplio, r.titulo, id],
    )
    .map_err(|e| format!("no se pudo guardar el resumen: {e}"))?;
    println!("entrevista {id}: resumida");
    Ok(())
}

/// El resumen amplio con la estructura fija de D33. Una sección sin material
/// lo dice, en vez de desaparecer: así se ve que se buscó y no había.
fn amplio_en_markdown(a: &llm::Amplio) -> String {
    let mut md = String::new();
    let seccion = |md: &mut String, titulo: &str, lineas: Vec<String>| {
        md.push_str(&format!("## {titulo}\n\n"));
        let lineas: Vec<String> = lineas
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .collect();
        if lineas.is_empty() {
            md.push_str("_Nada en la conversación._\n\n");
        } else {
            for l in lineas {
                md.push_str(&l);
                md.push('\n');
            }
            md.push('\n');
        }
    };
    let puntos = |v: &[String]| {
        v.iter()
            .map(|x| format!("- {}", x.trim()))
            .collect::<Vec<_>>()
    };
    seccion(&mut md, "Participantes", puntos(&a.participantes));
    seccion(&mut md, "Temas tratados", puntos(&a.temas));
    seccion(&mut md, "Puntos clave", puntos(&a.puntos_clave));
    seccion(&mut md, "Acuerdos y decisiones", puntos(&a.acuerdos));
    seccion(
        &mut md,
        "Tareas pendientes",
        a.tareas
            .iter()
            .filter(|t| !t.que.trim().is_empty())
            .map(|t| match t.quien.trim() {
                "" => format!("- {}", t.que.trim()),
                q => format!("- **{q}**: {}", t.que.trim()),
            })
            .collect(),
    );
    seccion(
        &mut md,
        "Frases literales relevantes",
        a.frases
            .iter()
            .map(|f| {
                format!(
                    "> «{}»",
                    f.trim().trim_matches(|c| matches!(c, '"' | '«' | '»'))
                )
            })
            .collect(),
    );
    md.trim_end().to_string()
}

#[derive(Deserialize)]
pub struct QueGuardar {
    cual: Option<String>,
}

/// `POST /entrevistas/{id}/nota` `{cual: ejecutivo|amplio|ambos}` (D33): una
/// nota más en el proyecto de la entrevista, que la IA titula como cualquiera.
pub async fn guardar_como_nota(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(q): Json<QueGuardar>,
) -> Respuesta<impl IntoResponse> {
    let cual = q.cual.unwrap_or_else(|| "ejecutivo".into());
    let nota_id: String = {
        let con = db.lock().unwrap();
        let e = existe_o_404(&con, &id)?;
        let ejecutivo = e.resumen_ejecutivo.unwrap_or_default();
        let amplio = e.resumen.unwrap_or_default();
        let cuerpo = match cual.as_str() {
            "ejecutivo" => ejecutivo,
            "amplio" => amplio,
            "ambos" => format!("{ejecutivo}\n\n{amplio}"),
            _ => return error(StatusCode::BAD_REQUEST, "cual: ejecutivo, amplio o ambos"),
        };
        if cuerpo.trim().is_empty() {
            return error(
                StatusCode::CONFLICT,
                "la entrevista aún no tiene ese resumen",
            );
        }
        let fecha: String = e
            .creada_en
            .chars()
            .take(16)
            .collect::<String>()
            .replace('T', " ");
        let con_quien = if e.con_quien.trim().is_empty() {
            String::new()
        } else {
            format!(" · con {}", e.con_quien.trim())
        };
        let contenido = format!(
            "{}\n\n_Entrevista «{}» · {fecha} UTC · {}{con_quien}_",
            cuerpo.trim(),
            e.titulo,
            reloj(e.duracion_s.max(0) as u64)
        );
        let titulo = e.titulo.chars().take(60).collect::<String>();
        let proyecto = e.proyecto.map(|p| p.id);
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
    completar_con_ia(&db, &nota_id).await;
    let con = db.lock().unwrap();
    match leer_nota(&con, &nota_id) {
        Ok(Some(n)) => Ok((StatusCode::CREATED, Json(json!(n)))),
        Ok(None) => error(StatusCode::INTERNAL_SERVER_ERROR, "la nota no se guardó"),
        Err(e) => interno(e),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn desplaza_las_marcas_de_tiempo_al_trozo() {
        let t = "[00:05] Entrevistador: Hola.\n\n[01:10] Marta: Buenas.\nsin marca";
        assert_eq!(
            desplazar_tiempos(t, 600),
            "[10:05] Entrevistador: Hola.\n[11:10] Marta: Buenas.\nsin marca"
        );
    }

    #[test]
    fn pasada_la_hora_lleva_horas() {
        assert_eq!(desplazar_tiempos("[04:59] A: x", 3360), "[1:00:59] A: x");
        assert_eq!(desplazar_tiempos("[1:00:00] A: x", 0), "[1:00:00] A: x");
    }

    #[test]
    fn extension_por_nombre_o_por_tipo() {
        assert_eq!(extension(Some("grabacion.M4A"), None), Some("m4a"));
        assert_eq!(
            extension(Some("blob"), Some("audio/webm;codecs=opus")),
            Some("webm")
        );
        assert_eq!(extension(None, Some("audio/mpeg")), Some("mp3"));
        assert_eq!(extension(Some("x.pdf"), Some("application/pdf")), None);
    }

    #[test]
    fn rangos() {
        let mut h = HeaderMap::new();
        assert!(rango(&h, 100).is_none());
        h.insert(header::RANGE, "bytes=0-9".parse().unwrap());
        assert_eq!(rango(&h, 100), Some(Ok((0, 9))));
        h.insert(header::RANGE, "bytes=90-".parse().unwrap());
        assert_eq!(rango(&h, 100), Some(Ok((90, 99))));
        h.insert(header::RANGE, "bytes=-10".parse().unwrap());
        assert_eq!(rango(&h, 100), Some(Ok((90, 99))));
        h.insert(header::RANGE, "bytes=0-500".parse().unwrap());
        assert_eq!(rango(&h, 100), Some(Ok((0, 99))));
        h.insert(header::RANGE, "bytes=200-300".parse().unwrap());
        assert_eq!(rango(&h, 100), Some(Err(())));
    }

    #[test]
    fn el_amplio_dice_lo_que_falta() {
        let md = amplio_en_markdown(&llm::Amplio {
            participantes: vec!["Marta, compras".into()],
            temas: vec![],
            puntos_clave: vec![],
            acuerdos: vec!["Entrega el 10".into()],
            tareas: vec![llm::Tarea {
                quien: "Marta".into(),
                que: "Confirmar transporte".into(),
            }],
            frases: vec!["«Si llega el 10, es vuestro»".into()],
        });
        assert!(md.starts_with("## Participantes\n\n- Marta, compras"));
        assert!(md.contains("## Temas tratados\n\n_Nada en la conversación._"));
        assert!(md.contains("- **Marta**: Confirmar transporte"));
        assert!(md.contains("> «Si llega el 10, es vuestro»"));
    }
}
