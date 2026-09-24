//! Análisis con roles (D54-D67): varios roles leen las mismas notas, cada uno
//! desde su enfoque, y una síntesis lo junta en un informe único. Aquí viven
//! los roles de serie, las rutas de roles y modelos y la generación del
//! informe en segundo plano; lo que se le dice al modelo está en `llm.rs`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use crate::{
    clave, error, es_duplicado, ia, interno, leer_documento, llm, Db, Documento, Respuesta,
};

/// D56: más roles encarecen y alargan el informe sin añadir mucho.
pub const MAXIMO_ROLES: usize = 8;

/// D57: roles a la vez. El proxy tiene cuota por minuto por aplicación; cuatro
/// llamadas simultáneas quedan lejos de ella y un informe de ocho sale en dos
/// tandas.
const SIMULTANEOS: usize = 4;

/// Un rol que viene con la aplicación (D55). Viven en el código, no en la
/// base, para que mejoren con cada versión sin migrar nada.
pub struct DeSerie {
    pub id: &'static str,
    pub icono: &'static str,
    pub nombre: &'static str,
    pub enfoque: &'static str,
}

pub const DE_SERIE: [DeSerie; 9] = [
    DeSerie {
        id: "critico",
        icono: "🧐",
        nombre: "Crítico",
        enfoque: "Abogado del diablo: busca los puntos débiles, los supuestos que se dan por buenos sin prueba, lo que falta y qué puede salir mal. No suavices: si algo no se sostiene, dilo.",
    },
    DeSerie {
        id: "analitico",
        icono: "📊",
        nombre: "Analítico",
        enfoque: "Ordena el material: agrupa las ideas en bloques, detecta huecos, contradicciones entre notas y datos que faltan para decidir, y di qué nota dice cada cosa.",
    },
    DeSerie {
        id: "economico",
        icono: "💶",
        nombre: "Económico",
        enfoque: "Costes, ingresos, modelo de negocio y viabilidad económica. Da órdenes de magnitud cuando puedas y señala qué cifras faltan para decidir.",
    },
    DeSerie {
        id: "marketing",
        icono: "📣",
        nombre: "Marketing",
        enfoque: "Público objetivo, propuesta de valor, posicionamiento, canales para llegar a él y competencia. Cómo se explicaría en una frase y a quién.",
    },
    DeSerie {
        id: "usabilidad",
        icono: "🧭",
        nombre: "Usabilidad",
        enfoque: "Recorrido del usuario, fricciones, accesibilidad y uso real en el móvil: dónde se atasca alguien que lo usa por primera vez.",
    },
    DeSerie {
        id: "tecnico",
        icono: "🛠",
        nombre: "Técnico",
        enfoque: "Arquitectura, complejidad, riesgos técnicos, dependencias y qué conviene no construir porque ya existe.",
    },
    DeSerie {
        id: "legal",
        icono: "⚖️",
        nombre: "Legal y privacidad",
        enfoque: "Protección de datos (RGPD), consentimiento, pagos, propiedad intelectual y condiciones de uso: qué obligaciones aparecen y cómo cumplirlas sin frenar el proyecto.",
    },
    DeSerie {
        id: "producto",
        icono: "🎯",
        nombre: "Producto",
        enfoque: "Qué entra en la primera versión, en qué orden y qué se deja fuera; cómo se mide si funciona.",
    },
    DeSerie {
        id: "usuario",
        icono: "👤",
        nombre: "Usuario final",
        enfoque: "Ponte en la piel de quien lo va a usar: qué le convence, qué le molesta, qué no entiende y qué le haría dejarlo. Habla en primera persona cuando ayude.",
    },
];

/// Modelos que la app enseña arriba del selector (D60), solo si están en el
/// catálogo. `LLM_RECOMENDADOS` (ids separados por comas) los sustituye sin
/// tocar código, porque los nombres de los modelos cambian cada pocos meses.
const RECOMENDADOS: [(&str, &str); 5] = [
    (
        "google/gemini-2.5-pro",
        "Buen equilibrio para informes largos",
    ),
    (
        "anthropic/claude-sonnet-4.5",
        "El que mejor redacta; más caro",
    ),
    ("openai/gpt-5", "Muy bueno analizando; algo más lento"),
    (
        "google/gemini-2.5-flash",
        "Barato y decente para probar roles",
    ),
    ("deepseek/deepseek-chat-v3.1", "Muy barato"),
];

// ---------------------------------------------------------------------------
// Roles (D59): los de serie y los propios, que se guardan en `roles`.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct Rol {
    id: String,
    nombre: String,
    icono: String,
    enfoque: String,
    propio: bool,
}

#[derive(Deserialize)]
pub struct NuevoRol {
    nombre: String,
    #[serde(default)]
    icono: String,
    enfoque: String,
}

/// Lo que se copia de un rol en el informe: si luego se edita o se borra el
/// rol, el informe no cambia.
pub struct RolCopia {
    pub id: String,
    pub nombre: String,
    pub icono: String,
    pub enfoque: String,
}

fn de_serie(id: &str) -> Option<&'static DeSerie> {
    DE_SERIE.iter().find(|r| r.id == id)
}

/// Nombre, icono y enfoque ya normalizados, o el motivo del 400.
fn rol_valido(n: &NuevoRol) -> Result<(String, String, String), &'static str> {
    let nombre = n.nombre.split_whitespace().collect::<Vec<_>>().join(" ");
    if nombre.is_empty() || nombre.chars().count() > 30 {
        return Err("el nombre tiene que tener entre 1 y 30 caracteres");
    }
    let enfoque = n.enfoque.trim().to_string();
    if enfoque.is_empty() || enfoque.chars().count() > 600 {
        return Err("«qué mira este rol» tiene que tener entre 1 y 600 caracteres");
    }
    // Un emoji puede ser varios caracteres (⚖️ son dos); ocho dan margen.
    let icono: String = n.icono.trim().chars().take(8).collect();
    let icono = if icono.is_empty() {
        "🧪".to_string()
    } else {
        icono
    };
    Ok((nombre, icono, enfoque))
}

fn choca_con_serie(nombre: &str) -> bool {
    let c = clave(nombre);
    DE_SERIE.iter().any(|r| clave(r.nombre) == c)
}

fn leer_rol(con: &Connection, id: &str) -> rusqlite::Result<Option<Rol>> {
    con.query_row(
        "SELECT id, nombre, icono, enfoque FROM roles WHERE id = ?1",
        [id],
        |f| {
            Ok(Rol {
                id: f.get(0)?,
                nombre: f.get(1)?,
                icono: f.get(2)?,
                enfoque: f.get(3)?,
                propio: true,
            })
        },
    )
    .optional()
}

/// `GET /roles`: los de serie primero y luego los propios, por antigüedad.
pub async fn listar_roles(State(db): State<Db>) -> Respuesta<Json<Vec<Rol>>> {
    let mut roles: Vec<Rol> = DE_SERIE
        .iter()
        .map(|r| Rol {
            id: r.id.into(),
            nombre: r.nombre.into(),
            icono: r.icono.into(),
            enfoque: r.enfoque.into(),
            propio: false,
        })
        .collect();
    let con = db.lock().unwrap();
    let propios: rusqlite::Result<Vec<Rol>> = con
        .prepare("SELECT id, nombre, icono, enfoque FROM roles ORDER BY creado_en, nombre")
        .and_then(|mut st| {
            st.query_map([], |f| {
                Ok(Rol {
                    id: f.get(0)?,
                    nombre: f.get(1)?,
                    icono: f.get(2)?,
                    enfoque: f.get(3)?,
                    propio: true,
                })
            })?
            .collect()
        });
    match propios {
        Ok(p) => roles.extend(p),
        Err(e) => return interno(e),
    }
    Ok(Json(roles))
}

pub async fn crear_rol(
    State(db): State<Db>,
    Json(nuevo): Json<NuevoRol>,
) -> Respuesta<(StatusCode, Json<Rol>)> {
    let (nombre, icono, enfoque) = match rol_valido(&nuevo) {
        Ok(v) => v,
        Err(m) => return error(StatusCode::BAD_REQUEST, m),
    };
    if choca_con_serie(&nombre) {
        return error(
            StatusCode::CONFLICT,
            "ya hay un rol de serie con ese nombre",
        );
    }
    let con = db.lock().unwrap();
    let id: String = match con.query_row(
        "INSERT INTO roles (id, nombre, nombre_clave, icono, enfoque, creado_en)
         VALUES (lower(hex(randomblob(8))), ?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         RETURNING id",
        params![nombre, clave(&nombre), icono, enfoque],
        |f| f.get(0),
    ) {
        Ok(id) => id,
        Err(e) if es_duplicado(&e) => {
            return error(StatusCode::CONFLICT, "ya hay un rol con ese nombre")
        }
        Err(e) => return interno(e),
    };
    match leer_rol(&con, &id) {
        Ok(Some(r)) => Ok((StatusCode::CREATED, Json(r))),
        Ok(None) => error(StatusCode::INTERNAL_SERVER_ERROR, "el rol no se guardó"),
        Err(e) => interno(e),
    }
}

pub async fn editar_rol(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(cambio): Json<NuevoRol>,
) -> Respuesta<Json<Rol>> {
    if de_serie(&id).is_some() {
        return error(
            StatusCode::BAD_REQUEST,
            "los roles de serie no se editan: duplícalo como propio",
        );
    }
    let (nombre, icono, enfoque) = match rol_valido(&cambio) {
        Ok(v) => v,
        Err(m) => return error(StatusCode::BAD_REQUEST, m),
    };
    if choca_con_serie(&nombre) {
        return error(
            StatusCode::CONFLICT,
            "ya hay un rol de serie con ese nombre",
        );
    }
    let con = db.lock().unwrap();
    match con.execute(
        "UPDATE roles SET nombre = ?1, nombre_clave = ?2, icono = ?3, enfoque = ?4 WHERE id = ?5",
        params![nombre, clave(&nombre), icono, enfoque, id],
    ) {
        Ok(0) => return error(StatusCode::NOT_FOUND, "rol no encontrado"),
        Ok(_) => {}
        Err(e) if es_duplicado(&e) => {
            return error(StatusCode::CONFLICT, "ya hay un rol con ese nombre")
        }
        Err(e) => return interno(e),
    }
    match leer_rol(&con, &id) {
        Ok(Some(r)) => Ok(Json(r)),
        Ok(None) => error(StatusCode::NOT_FOUND, "rol no encontrado"),
        Err(e) => interno(e),
    }
}

/// Borrar un rol no toca los informes hechos: guardan su copia (D59).
pub async fn borrar_rol(State(db): State<Db>, Path(id): Path<String>) -> Respuesta<StatusCode> {
    if de_serie(&id).is_some() {
        return error(StatusCode::BAD_REQUEST, "los roles de serie no se borran");
    }
    let con = db.lock().unwrap();
    match con.execute("DELETE FROM roles WHERE id = ?1", [&id]) {
        Ok(0) => error(StatusCode::NOT_FOUND, "rol no encontrado"),
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(e) => interno(e),
    }
}

/// Los roles que pide un informe, resueltos y copiados: sin repetidos, de 1 a
/// `MAXIMO_ROLES`, y cada uno tiene que existir.
pub fn roles_pedidos(con: &Connection, ids: &[String]) -> Respuesta<Vec<RolCopia>> {
    let mut vistos: Vec<String> = Vec::new();
    for id in ids {
        let id = id.trim().to_string();
        if !id.is_empty() && !vistos.contains(&id) {
            vistos.push(id);
        }
    }
    if vistos.is_empty() {
        return error(StatusCode::BAD_REQUEST, "elige al menos un rol");
    }
    if vistos.len() > MAXIMO_ROLES {
        return error(
            StatusCode::BAD_REQUEST,
            format!(
                "demasiados roles ({}), el máximo es {MAXIMO_ROLES}",
                vistos.len()
            ),
        );
    }
    let mut copias = Vec::with_capacity(vistos.len());
    for id in vistos {
        if let Some(r) = de_serie(&id) {
            copias.push(RolCopia {
                id,
                nombre: r.nombre.into(),
                icono: r.icono.into(),
                enfoque: r.enfoque.into(),
            });
            continue;
        }
        match leer_rol(con, &id) {
            Ok(Some(r)) => copias.push(RolCopia {
                id,
                nombre: r.nombre,
                icono: r.icono,
                enfoque: r.enfoque,
            }),
            Ok(None) => return error(StatusCode::BAD_REQUEST, format!("el rol {id} no existe")),
            Err(e) => return interno(e),
        }
    }
    Ok(copias)
}

// ---------------------------------------------------------------------------
// Modelos (D60/D61): el catálogo del servicio, a través del backend.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ModeloApp {
    id: String,
    nombre: String,
    contexto: u64,
    entrada: f64,
    salida: f64,
    json: bool,
    /// Por qué sale arriba, si es de los recomendados.
    recomendado: Option<String>,
}

#[derive(Serialize)]
pub struct Modelos {
    /// El que se usa si no se elige ninguno.
    defecto: Option<String>,
    modelos: Vec<ModeloApp>,
}

/// Los recomendados en vigor: los de `LLM_RECOMENDADOS` o los del código.
fn recomendados() -> Vec<(String, String)> {
    match std::env::var("LLM_RECOMENDADOS") {
        Ok(v) if !v.trim().is_empty() => v
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| (s.to_string(), "Recomendado".to_string()))
            .collect(),
        _ => RECOMENDADOS
            .iter()
            .map(|(id, motivo)| (id.to_string(), motivo.to_string()))
            .collect(),
    }
}

/// `GET /modelos`: el catálogo de texto, por nombre, con los recomendados
/// marcados. La clave de la aplicación no sale del servidor (D61).
pub async fn listar_modelos() -> Respuesta<Json<Modelos>> {
    let Some(ia) = ia() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no hay IA configurada");
    };
    let catalogo = match ia.catalogo().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("modelos: {e}");
            return error(
                StatusCode::BAD_GATEWAY,
                "no se pudo consultar el catálogo de modelos",
            );
        }
    };
    let rec = recomendados();
    let mut modelos: Vec<ModeloApp> = catalogo
        .into_iter()
        .filter(|m| m.modalidades.is_empty() || m.modalidades.iter().any(|x| x == "text"))
        .map(|m| ModeloApp {
            recomendado: rec
                .iter()
                .find(|(id, _)| *id == m.id)
                .map(|(_, r)| r.clone()),
            nombre: if m.nombre.is_empty() {
                m.id.clone()
            } else {
                m.nombre
            },
            id: m.id,
            contexto: m.contexto,
            entrada: m.entrada,
            salida: m.salida,
            json: m.json,
        })
        .collect();
    modelos.sort_by_key(|m| m.nombre.to_lowercase());
    Ok(Json(Modelos {
        defecto: ia.modelo_defecto().await,
        modelos,
    }))
}

/// Comprueba el modelo que pide la app (D61): vacío o ausente es «el del
/// servidor»; uno que no está en el catálogo es un 400. Los encargos piden
/// salida estructurada, así que ahí el modelo tiene que darla.
pub async fn modelo_valido(
    modelo: Option<String>,
    para_encargo: bool,
) -> Respuesta<Option<String>> {
    let Some(id) = modelo
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
    else {
        return Ok(None);
    };
    let Some(ia) = ia() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no hay IA configurada");
    };
    let catalogo = match ia.catalogo().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("modelos: {e}");
            return error(
                StatusCode::BAD_GATEWAY,
                "no se pudo comprobar el modelo en el catálogo",
            );
        }
    };
    match catalogo.iter().find(|m| m.id == id) {
        None => error(
            StatusCode::BAD_REQUEST,
            format!("el modelo {id} no está en el catálogo"),
        ),
        Some(m) if para_encargo && !m.json => error(
            StatusCode::BAD_REQUEST,
            format!("{id} no da salida estructurada y el encargo la necesita: elige otro"),
        ),
        Some(_) => Ok(Some(id)),
    }
}

// ---------------------------------------------------------------------------
// Generación del informe (D57, D63, D64).
// ---------------------------------------------------------------------------

/// Al arrancar: un rol que se estaba haciendo cuando se reinició el servicio
/// no lo termina nadie. Queda fallido y «Reintentar» lo rehace (D63).
pub fn recuperar(con: &Connection) {
    match con.execute(
        "UPDATE documento_roles SET estado = 'fallido',
         error = 'se interrumpió al reiniciarse el servicio'
         WHERE estado IN ('espera', 'trabajando')",
        [],
    ) {
        Ok(0) => {}
        Ok(n) => println!("{n} rol(es) de informe a medias marcados para reintentar"),
        Err(e) => eprintln!("no se pudieron revisar los roles pendientes: {e}"),
    }
}

/// Lo que se lee de un informe para generarlo, con el candado suelto.
struct Trabajo {
    instruccion: String,
    modelo: Option<String>,
    generacion: i64,
    fuentes: Vec<llm::NotaFuente>,
    proyecto: Option<llm::Proyecto>,
    /// Todos los roles, en orden: id, nombre, icono, enfoque, estado, texto, error.
    roles: Vec<RolTrabajo>,
    /// Cuándo se pidió esta generación, para la ficha.
    pedido_en: String,
}

#[derive(Clone)]
struct RolTrabajo {
    id: String,
    nombre: String,
    icono: String,
    enfoque: String,
    estado: String,
    texto: String,
    error: String,
}

pub fn lanzar_informe(db: Db, id: String) {
    tokio::spawn(async move {
        if let Err(e) = generar_informe(&db, &id).await {
            eprintln!("informe {id}: {e}");
            let con = db.lock().unwrap();
            let _ = con.execute(
                "UPDATE documentos SET estado = 'fallido', error = ?1,
                 actualizado_en = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?2",
                params![e, id],
            );
        }
    });
}

fn marcar_rol(
    db: &Db,
    id: &str,
    rol: &str,
    estado: &str,
    texto: Option<&str>,
    error: Option<&str>,
) {
    let con = db.lock().unwrap();
    let _ = con.execute(
        "UPDATE documento_roles SET estado = ?1, texto = COALESCE(?2, texto), error = ?3
         WHERE documento_id = ?4 AND rol_id = ?5",
        params![estado, texto, error, id, rol],
    );
    let _ = con.execute(
        "UPDATE documentos SET actualizado_en = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
        [id],
    );
}

async fn generar_informe(db: &Db, id: &str) -> Result<(), String> {
    let ia = ia().ok_or("no hay IA configurada")?;
    let t = {
        let con = db.lock().unwrap();
        leer_trabajo(&con, id)?
    };
    if t.fuentes.is_empty() {
        return Err("ya no queda ninguna de las notas de este informe".into());
    }
    // Una operación por generación: el coste de la ficha es el de esta
    // generación con sus reintentos, no el de las anteriores (D62).
    let operacion = format!("informe-{id}-{}", t.generacion);
    let fuentes = std::sync::Arc::new(t.fuentes);
    let proyecto = std::sync::Arc::new(t.proyecto);

    // Los roles que faltan, de cuatro en cuatro (D57). Los ya hechos se quedan.
    let mut cola = t.roles.iter().filter(|r| r.estado != "hecho").cloned();
    let mut tareas: JoinSet<(String, Result<String, String>)> = JoinSet::new();
    loop {
        while tareas.len() < SIMULTANEOS {
            let Some(r) = cola.next() else { break };
            marcar_rol(db, id, &r.id, "trabajando", None, None);
            let (fuentes, proyecto) = (fuentes.clone(), proyecto.clone());
            let (instruccion, modelo, operacion) =
                (t.instruccion.clone(), t.modelo.clone(), operacion.clone());
            tareas.spawn(async move {
                let pedido = llm::RolPedido {
                    nombre: &r.nombre,
                    icono: &r.icono,
                    enfoque: &r.enfoque,
                };
                let res = ia
                    .analizar_rol(
                        &pedido,
                        &fuentes,
                        &instruccion,
                        proyecto.as_ref().as_ref(),
                        modelo.as_deref(),
                        &operacion,
                    )
                    .await;
                (r.id, res)
            });
        }
        match tareas.join_next().await {
            None => break,
            Some(Ok((rol, Ok(texto)))) => marcar_rol(db, id, &rol, "hecho", Some(&texto), None),
            Some(Ok((rol, Err(e)))) => {
                eprintln!("informe {id}, rol {rol}: {e}");
                marcar_rol(db, id, &rol, "fallido", None, Some(&e));
            }
            // Un pánico dentro de la tarea: el rol se queda «trabajando» y se
            // da por fallido abajo.
            Some(Err(e)) => eprintln!("informe {id}: una tarea de rol cayó: {e}"),
        }
    }

    let roles = {
        let con = db.lock().unwrap();
        let _ = con.execute(
            "UPDATE documento_roles SET estado = 'fallido', error = 'la tarea se interrumpió'
             WHERE documento_id = ?1 AND estado IN ('espera', 'trabajando')",
            [id],
        );
        leer_trabajo(&con, id)?.roles
    };
    let hechos: Vec<&RolTrabajo> = roles.iter().filter(|r| r.estado == "hecho").collect();
    if hechos.is_empty() {
        let motivo = roles
            .iter()
            .find(|r| !r.error.is_empty())
            .map(|r| r.error.clone())
            .unwrap_or_else(|| "sin detalle".into());
        return Err(format!("no respondió ningún rol ({motivo})"));
    }

    let analisis: Vec<llm::AnalisisHecho> = hechos
        .iter()
        .map(|r| llm::AnalisisHecho {
            nombre: &r.nombre,
            icono: &r.icono,
            texto: &r.texto,
        })
        .collect();
    let (sintesis, servido) = ia
        .sintetizar(
            &fuentes,
            &t.instruccion,
            proyecto.as_ref().as_ref(),
            &analisis,
            t.modelo.as_deref(),
            &operacion,
        )
        .await
        .map_err(|e| format!("la síntesis falló: {e}"))?;

    let coste = match ia.coste(&operacion).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("informe {id}: no se pudo leer el coste: {e}");
            None
        }
    };
    let (titulo, sintesis) = titulo_de(&sintesis);
    let texto = componer(
        &sintesis,
        &titulo,
        &roles,
        &fuentes,
        &Ficha {
            modelo: t.modelo.as_deref(),
            servido: servido.as_deref(),
            llamadas: roles.iter().filter(|r| r.estado == "hecho").count() + 1,
            coste,
            pedido_en: &t.pedido_en,
            instruccion: &t.instruccion,
        },
    );

    let con = db.lock().unwrap();
    con.execute(
        "UPDATE documentos SET titulo = ?1, sintesis = ?2, texto = ?3, coste = ?4,
         estado = 'listo', error = NULL, editado = 0,
         actualizado_en = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?5",
        params![titulo, sintesis, texto, coste, id],
    )
    .map_err(|e| format!("no se pudo guardar el informe: {e}"))?;
    println!(
        "informe {id} listo ({} de {} roles, coste {:?})",
        hechos.len(),
        roles.len(),
        coste
    );
    Ok(())
}

fn leer_trabajo(con: &Connection, id: &str) -> Result<Trabajo, String> {
    let crate::FuentesDocumento {
        instruccion,
        notas: fuentes,
        proyecto,
        modelo,
    } = crate::fuentes_de_documento(con, id)?;
    let (generacion, pedido_en): (i64, String) = con
        .query_row(
            "SELECT generacion, actualizado_en FROM documentos WHERE id = ?1",
            [id],
            |f| Ok((f.get(0)?, f.get(1)?)),
        )
        .map_err(|_| "el informe ya no está".to_string())?;
    let roles = con
        .prepare_cached(
            "SELECT rol_id, nombre, icono, enfoque, estado, COALESCE(texto, ''), COALESCE(error, '')
             FROM documento_roles WHERE documento_id = ?1 ORDER BY orden",
        )
        .and_then(|mut st| {
            st.query_map([id], |f| {
                Ok(RolTrabajo {
                    id: f.get(0)?,
                    nombre: f.get(1)?,
                    icono: f.get(2)?,
                    enfoque: f.get(3)?,
                    estado: f.get(4)?,
                    texto: f.get(5)?,
                    error: f.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| format!("no se pudieron leer los roles: {e}"))?;
    Ok(Trabajo {
        instruccion,
        modelo,
        generacion,
        fuentes,
        proyecto,
        roles,
        pedido_en,
    })
}

/// Separa el título (primer `# `) del resto de la síntesis. Si el modelo no
/// puso título, se usa uno genérico y la síntesis queda entera.
fn titulo_de(sintesis: &str) -> (String, String) {
    let mut lineas = sintesis.lines();
    let primera = sintesis
        .lines()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(0);
    let cuerpo_desde = |n: usize| sintesis.lines().skip(n).collect::<Vec<_>>().join("\n");
    if let Some(l) = lineas.nth(primera) {
        if let Some(t) = l.trim().strip_prefix("# ") {
            let titulo = limpiar_titulo(t);
            if !titulo.is_empty() {
                return (titulo, cuerpo_desde(primera + 1).trim().to_string());
            }
        }
    }
    ("Informe".to_string(), sintesis.trim().to_string())
}

fn limpiar_titulo(t: &str) -> String {
    llm::recortar_adornos(t.trim().trim_matches('*'))
        .chars()
        .take(80)
        .collect()
}

struct Ficha<'a> {
    modelo: Option<&'a str>,
    servido: Option<&'a str>,
    llamadas: usize,
    coste: Option<f64>,
    pedido_en: &'a str,
    instruccion: &'a str,
}

/// El informe entero (D57): la síntesis, lo de cada rol tal cual, las notas
/// de origen y la ficha. Lo escribe el backend para que la estructura no
/// dependa del modelo.
fn componer(
    sintesis: &str,
    titulo: &str,
    roles: &[RolTrabajo],
    fuentes: &[llm::NotaFuente],
    ficha: &Ficha,
) -> String {
    let mut t = format!(
        "# {titulo}\n\n{}\n\n## Análisis por rol\n\n",
        sintesis.trim()
    );
    for r in roles {
        t.push_str(&format!("### {} {}\n\n", r.icono, r.nombre));
        if r.estado == "hecho" {
            t.push_str(r.texto.trim());
        } else {
            t.push_str(&format!(
                "_Este rol no respondió ({}); se puede reintentar desde la app._",
                if r.error.is_empty() {
                    "sin detalle"
                } else {
                    &r.error
                }
            ));
        }
        t.push_str("\n\n");
    }
    t.push_str(crate::notas_de_origen(fuentes).trim_end());
    t.push_str("\n\n## Ficha\n\n");
    let modelo = match (ficha.modelo, ficha.servido) {
        (Some(m), Some(s)) if m != s => format!("`{m}` (servido por `{s}`)"),
        (Some(m), _) => format!("`{m}`"),
        (None, Some(s)) => format!("el del servidor (`{s}`)"),
        (None, None) => "el del servidor".to_string(),
    };
    t.push_str(&format!("- **Modelo:** {modelo}\n"));
    let nombres: Vec<&str> = roles.iter().map(|r| r.nombre.as_str()).collect();
    t.push_str(&format!("- **Roles:** {}\n", nombres.join(", ")));
    let coste = match ficha.coste {
        Some(c) => format!("{} $", formato_dolares(c)),
        None => "no disponible".to_string(),
    };
    t.push_str(&format!(
        "- **Llamadas:** {} · **Coste:** {coste}\n",
        ficha.llamadas
    ));
    let fecha: String = ficha.pedido_en.chars().take(16).collect();
    t.push_str(&format!(
        "- **Generado:** {} UTC\n",
        fecha.replace('T', " ")
    ));
    if !ficha.instruccion.trim().is_empty() {
        t.push_str(&format!(
            "- **Instrucción:** {}\n",
            ficha.instruccion.trim()
        ));
    }
    t
}

/// Dólares con coma decimal; por debajo del céntimo, tres decimales.
fn formato_dolares(c: f64) -> String {
    let s = if c < 0.01 {
        format!("{c:.3}")
    } else {
        format!("{c:.2}")
    };
    s.replace('.', ",")
}

/// `POST /documentos/{id}/reintentar` (D64): rehace los roles que fallaron y
/// la síntesis. Lo ya hecho no se repite ni se vuelve a pagar.
pub async fn reintentar(
    State(db): State<Db>,
    Path(id): Path<String>,
) -> Respuesta<Json<Documento>> {
    if ia().is_none() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no hay IA configurada");
    }
    {
        let con = db.lock().unwrap();
        let d = match leer_documento(&con, &id) {
            Ok(Some(d)) => d,
            Ok(None) => return error(StatusCode::NOT_FOUND, "documento no encontrado"),
            Err(e) => return interno(e),
        };
        if d.tipo != "informe" {
            return error(
                StatusCode::BAD_REQUEST,
                "solo se reintenta un informe; un encargo se regenera",
            );
        }
        if d.estado == "pendiente" {
            return error(StatusCode::CONFLICT, "ese informe ya se está generando");
        }
        let fallidos = d.roles.iter().filter(|r| r.estado != "hecho").count();
        if fallidos == 0 && d.estado == "listo" {
            return error(StatusCode::BAD_REQUEST, "no hay nada que reintentar");
        }
        let hecho = || -> rusqlite::Result<()> {
            let tx = con.unchecked_transaction()?;
            tx.execute(
                "UPDATE documento_roles SET estado = 'espera', error = NULL
                 WHERE documento_id = ?1 AND estado != 'hecho'",
                [&id],
            )?;
            tx.execute(
                "UPDATE documentos SET estado = 'pendiente', error = NULL,
                 actualizado_en = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                [&id],
            )?;
            tx.commit()
        };
        if let Err(e) = hecho() {
            return interno(e);
        }
    }
    lanzar_informe(db.clone(), id.clone());
    let con = db.lock().unwrap();
    match leer_documento(&con, &id) {
        Ok(Some(d)) => Ok(Json(d)),
        Ok(None) => error(StatusCode::NOT_FOUND, "documento no encontrado"),
        Err(e) => interno(e),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn rol(id: &str, estado: &str, texto: &str) -> RolTrabajo {
        RolTrabajo {
            id: id.into(),
            nombre: id.to_uppercase(),
            icono: "🧪".into(),
            enfoque: String::new(),
            estado: estado.into(),
            texto: texto.into(),
            error: if estado == "fallido" {
                "tiempo agotado".into()
            } else {
                String::new()
            },
        }
    }

    #[test]
    fn nueve_de_serie_con_ids_y_nombres_unicos() {
        assert_eq!(DE_SERIE.len(), 9);
        for (i, a) in DE_SERIE.iter().enumerate() {
            for b in DE_SERIE.iter().skip(i + 1) {
                assert_ne!(a.id, b.id);
                assert_ne!(clave(a.nombre), clave(b.nombre));
            }
        }
    }

    #[test]
    fn el_titulo_sale_del_primer_encabezado() {
        let (t, s) = titulo_de("\n# «Reservas del club».\n\n## Resumen ejecutivo\nTexto.");
        assert_eq!(t, "Reservas del club");
        assert!(s.starts_with("## Resumen ejecutivo"));
        let (t, s) = titulo_de("## Resumen ejecutivo\nTexto.");
        assert_eq!(t, "Informe");
        assert!(s.starts_with("## Resumen"));
    }

    #[test]
    fn el_informe_lleva_roles_notas_y_ficha() {
        let roles = vec![
            rol("critico", "hecho", "**Veredicto:** flojo."),
            rol("economico", "fallido", ""),
        ];
        let fuentes = vec![llm::NotaFuente {
            titulo: "Nota uno".into(),
            creada_en: "2026-09-24T10:00:00.000Z".into(),
            contenido: "x".into(),
            etiquetas: vec![],
        }];
        let t = componer(
            "## Resumen ejecutivo\nBien.",
            "Título",
            &roles,
            &fuentes,
            &Ficha {
                modelo: Some("a/b"),
                servido: Some("a/b-2"),
                llamadas: 2,
                coste: Some(0.0412),
                pedido_en: "2026-09-24T10:05:00.000Z",
                instruccion: "",
            },
        );
        assert!(t.starts_with("# Título\n\n## Resumen ejecutivo"));
        assert!(t.contains("## Análisis por rol\n\n### 🧪 CRITICO\n\n**Veredicto:** flojo."));
        assert!(t.contains("### 🧪 ECONOMICO\n\n_Este rol no respondió (tiempo agotado)"));
        assert!(t.contains("## Notas de origen"));
        assert!(t.contains("- **Modelo:** `a/b` (servido por `a/b-2`)"));
        assert!(t.contains("**Coste:** 0,04 $"));
        assert!(t.contains("- **Generado:** 2026-09-24 10:05 UTC"));
        assert!(!t.contains("Instrucción"));
    }

    #[test]
    fn un_nombre_de_serie_no_se_puede_usar_para_uno_propio() {
        assert!(choca_con_serie("  crítico "));
        assert!(choca_con_serie("LEGAL Y PRIVACIDAD"));
        assert!(!choca_con_serie("Accesibilidad"));
    }

    #[test]
    fn el_rol_propio_se_normaliza() {
        let (n, i, e) = rol_valido(&NuevoRol {
            nombre: "  Accesibilidad   web ".into(),
            icono: "".into(),
            enfoque: " contraste ".into(),
        })
        .unwrap();
        assert_eq!(
            (n.as_str(), i.as_str(), e.as_str()),
            ("Accesibilidad web", "🧪", "contraste")
        );
        assert!(rol_valido(&NuevoRol {
            nombre: "x".repeat(31),
            icono: "".into(),
            enfoque: "y".into()
        })
        .is_err());
        assert!(rol_valido(&NuevoRol {
            nombre: "x".into(),
            icono: "".into(),
            enfoque: "  ".into()
        })
        .is_err());
    }
}
