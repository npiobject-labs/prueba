//! Lo que la aplicación le pide al servicio de modelos (D5): título y
//! etiquetas de una nota, el documento que resume un conjunto de notas
//! (D11), y la transcripción y los resúmenes de una entrevista (D30, D33).
//!
//! Todo lo que sabe la aplicación del proveedor vive aquí: si mañana cambia el
//! protocolo se reescribe este fichero y nada más, que es lo que el plan dejó
//! decidido como plan B de D5. Hacia fuera solo se ofrecen `titular`,
//! `documentar`, `transcribir` y `resumir_entrevista`.
//!
//! El servicio al que se llama es `openrouter`, el proxy propio: habla el
//! formato de la API de OpenAI, cobra contra la clave de esta aplicación y
//! anota cada llamada con su coste. La clave nunca sale del backend.

use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

/// D6: pasados ocho segundos la nota se guarda sin IA. Más vale una nota con
/// título de respaldo que un usuario esperando.
const ESPERA: Duration = Duration::from_secs(8);

/// Cuántas etiquetas ya existentes se le enseñan al modelo. Suficientes para
/// que reutilice en vez de inventar, sin inflar la petición.
const ETIQUETAS_DE_CONTEXTO: usize = 40;

/// Recorte del contenido que se envía. Una nota dictada muy larga no mejora el
/// título y sí encarece la llamada.
const MAXIMO_CONTENIDO: usize = 4000;

/// D11: un documento no lo espera nadie delante (se genera en segundo plano y
/// el móvil pregunta por él), así que aquí la espera larga no molesta a nadie.
const ESPERA_DOCUMENTO: Duration = Duration::from_secs(120);

/// Tope de material que entra en un documento. Veinte notas dictadas caben de
/// sobra; pasado esto se recorta y el documento lo dice.
const MAXIMO_DOCUMENTO: usize = 30_000;

/// Tope de salida de cada llamada de un informe (D67): un rol o la síntesis.
const MAXIMO_TOKENS_INFORME: u32 = 2500;

/// Consultas que no generan nada (catálogo, coste): si tardan, algo va mal.
const ESPERA_CONSULTA: Duration = Duration::from_secs(20);

/// D31: un trozo son cinco minutos de audio. El proxy espera 120 s al
/// proveedor; aquí algo menos, para que el error sea nuestro y legible.
const ESPERA_TRANSCRIPCION: Duration = Duration::from_secs(110);

/// Modelo con entrada de audio si no se fija `LLM_MODELO_AUDIO`. El de
/// titular por defecto es un «lite», que con audio separa peor las voces.
const MODELO_AUDIO: &str = "google/gemini-2.5-flash";

/// Intentos del resumen de una entrevista si la respuesta llega rota.
const INTENTOS_RESUMEN: usize = 3;

/// Una hora de conversación son unos 60 000 caracteres; el doble da margen sin
/// que una transcripción desmesurada dispare el coste.
const MAXIMO_TRANSCRIPCION: usize = 120_000;

/// Lo que el modelo devuelve para una nota.
#[derive(Debug, Deserialize)]
pub struct Sugerencia {
    pub titulo: String,
    #[serde(default)]
    pub etiquetas: Vec<String>,
}

/// Una nota tal y como entra en un documento (D11). El backend la arma; aquí
/// solo se formatea para el modelo.
pub struct NotaFuente {
    pub titulo: String,
    pub creada_en: String,
    pub contenido: String,
    pub etiquetas: Vec<String>,
}

/// El proyecto del que salen las notas de un documento (D21): contexto para el
/// modelo, que asi sabe de que va el encargo sin que el usuario lo repita.
pub struct Proyecto {
    pub nombre: String,
    pub descripcion: String,
}

/// Lo que el modelo devuelve para un conjunto de notas (D11). `markdown` trae
/// el documento entero menos la sección de notas de origen, que la escribe el
/// backend para que sea fiel y no dependa de lo que el modelo recuerde.
#[derive(Debug, Deserialize)]
pub struct Documento {
    pub titulo: String,
    pub markdown: String,
}

/// Lo que acompaña a un trozo de audio para que el modelo sepa dónde está
/// (D31): qué trozo es, con quién se habla y cómo acabó el anterior, para que
/// no cambie el nombre de los hablantes a mitad.
pub struct ContextoTrozo<'a> {
    pub n: usize,
    pub total: usize,
    pub con_quien: &'a str,
    pub proyecto: Option<&'a Proyecto>,
    pub cola_anterior: &'a str,
}

/// Una tarea pendiente que sale de la entrevista.
#[derive(Debug, Deserialize)]
pub struct Tarea {
    pub quien: String,
    pub que: String,
}

/// El resumen amplio por partes (D33): el markdown lo compone el backend, así
/// que el formato nunca depende de lo que el modelo decida.
#[derive(Debug, Deserialize)]
pub struct Amplio {
    pub participantes: Vec<String>,
    pub temas: Vec<String>,
    pub puntos_clave: Vec<String>,
    pub acuerdos: Vec<String>,
    pub tareas: Vec<Tarea>,
    pub frases: Vec<String>,
}

/// Los dos resúmenes de una entrevista y el título que propone el modelo.
#[derive(Debug, Deserialize)]
pub struct Resumen {
    pub titulo: String,
    pub ejecutivo: String,
    pub amplio: Amplio,
}

/// Un rol del análisis (D55/D59) tal y como se le pide al modelo: quién es y
/// qué mira. La estructura común de la respuesta la pone esta capa.
pub struct RolPedido<'a> {
    pub nombre: &'a str,
    pub icono: &'a str,
    pub enfoque: &'a str,
}

/// Lo que ya escribió un rol, para la síntesis (D57).
pub struct AnalisisHecho<'a> {
    pub nombre: &'a str,
    pub icono: &'a str,
    pub texto: &'a str,
}

/// Un modelo del catálogo del servicio (D60), con lo que hace falta para
/// elegirlo: precio en dólares por millón de tokens, contexto y si da salida
/// estructurada.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct ModeloCatalogo {
    pub id: String,
    #[serde(default)]
    pub nombre: String,
    #[serde(default)]
    pub contexto: u64,
    #[serde(default)]
    pub entrada: f64,
    #[serde(default)]
    pub salida: f64,
    #[serde(default)]
    pub json: bool,
    #[serde(default)]
    pub modalidades: Vec<String>,
}

/// Configuración del servicio, leída del entorno una sola vez al arrancar.
/// Sin clave no hay IA, y la aplicación funciona igual (D6).
pub struct Llm {
    http: Client,
    base: String,
    clave: String,
    modelo: Option<String>,
    modelo_documento: Option<String>,
    modelo_audio: String,
}

impl Llm {
    /// Devuelve `None` si no hay `LLM_API_KEY`: es el modo sin IA, legítimo y
    /// verificado, no un error.
    pub fn del_entorno() -> Option<Self> {
        let clave = std::env::var("LLM_API_KEY")
            .ok()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())?;
        let base = std::env::var("LLM_BASE")
            .ok()
            .map(|b| b.trim().trim_end_matches('/').to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "https://openrouter-npiobject-labs.fly.dev/v1".to_string());
        let modelo = std::env::var("LLM_MODELO")
            .ok()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        // Titular es tarea de modelo barato; redactar un documento no. Si no se
        // fija ninguno manda el mismo de titular, y en último término el que el
        // servicio ponga por defecto.
        let modelo_documento = std::env::var("LLM_MODELO_DOCUMENTO")
            .ok()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .or_else(|| modelo.clone());
        let modelo_audio = std::env::var("LLM_MODELO_AUDIO")
            .ok()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| MODELO_AUDIO.to_string());
        let http = Client::builder()
            .user_agent("prueba-backend")
            .build()
            .expect("no se pudo construir el cliente HTTP");
        println!("IA activa contra {base}");
        Some(Self {
            http,
            base,
            clave,
            modelo,
            modelo_documento,
            modelo_audio,
        })
    }

    /// Transcribe un trozo de audio mp3 (D30/D31) con los hablantes separados.
    /// Devuelve líneas `[mm:ss] Hablante: texto` con el tiempo relativo al
    /// principio del trozo; el backend las desplaza al tiempo de la entrevista.
    pub async fn transcribir(&self, mp3: &[u8], ctx: &ContextoTrozo<'_>) -> Result<String, String> {
        use base64::Engine;
        if mp3.is_empty() {
            return Err("trozo de audio vacío".into());
        }
        let datos = base64::engine::general_purpose::STANDARD.encode(mp3);
        let texto = self
            .completar(
                self.peticion_transcripcion(&datos, ctx),
                "entrevista-transcribir",
                ESPERA_TRANSCRIPCION,
            )
            .await?;
        let texto = sin_cerco(&texto);
        if texto.is_empty() {
            return Err("el modelo devolvió una transcripción vacía".into());
        }
        Ok(texto)
    }

    /// Los dos resúmenes de una entrevista ya transcrita (D33), en una sola
    /// llamada con salida estructurada.
    pub async fn resumir_entrevista(
        &self,
        transcripcion: &str,
        con_quien: &str,
        duracion: &str,
        proyecto: Option<&Proyecto>,
    ) -> Result<Resumen, String> {
        if transcripcion.trim().is_empty() {
            return Err("no hay transcripción que resumir".into());
        }
        // A veces el modelo corta el JSON a mitad de una cadena: se reintenta
        // antes de dar el resumen por fallido. Tres llamadas iguales quedan
        // lejos del corte por bucle del proxy (cinco en un minuto).
        let mut ultimo_error = String::new();
        for _ in 0..INTENTOS_RESUMEN {
            let texto = match self
                .completar(
                    self.peticion_resumen(transcripcion, con_quien, duracion, proyecto),
                    "entrevista-resumir",
                    ESPERA_DOCUMENTO,
                )
                .await
            {
                Ok(t) => t,
                Err(e) => {
                    ultimo_error = e;
                    continue;
                }
            };
            let mut r: Resumen = match serde_json::from_str(&sin_cerco(&texto)) {
                Ok(r) => r,
                Err(e) => {
                    ultimo_error = format!("el modelo no devolvió el JSON esperado: {e}");
                    eprintln!("resumen: {ultimo_error}; se reintenta");
                    continue;
                }
            };
            r.titulo = recortar_adornos(&r.titulo).chars().take(80).collect();
            r.ejecutivo = sin_cerco(&r.ejecutivo);
            if r.ejecutivo.is_empty() {
                ultimo_error = "el modelo devolvió un resumen vacío".into();
                continue;
            }
            return Ok(r);
        }
        Err(ultimo_error)
    }

    fn peticion_transcripcion(&self, datos_b64: &str, ctx: &ContextoTrozo<'_>) -> Value {
        let hablantes = match ctx.con_quien.trim() {
            "" => "Son dos personas: llámalas «Entrevistador» (quien hace las preguntas) y \
                   «Entrevistado». Si en la conversación se dice el nombre de alguna, usa el \
                   nombre a partir de ahí."
                .to_string(),
            q => format!(
                "Son dos personas: «Entrevistador» (quien hace las preguntas) y la persona \
                 entrevistada, que es: {q}. Llama a esta por su nombre de pila."
            ),
        };
        let proyecto = match ctx.proyecto {
            Some(p) => format!("La conversación es del proyecto «{}». ", p.nombre),
            None => String::new(),
        };
        let anterior = match ctx.cola_anterior.trim() {
            "" => String::new(),
            c => format!(
                "\n\nAsí terminaba el fragmento anterior; mantén exactamente los mismos nombres \
                 de hablante:\n{c}"
            ),
        };
        json!({
            "model": self.modelo_audio,
            "messages": [
                {
                    "role": "system",
                    "content": "Transcribes conversaciones grabadas. Escribes literalmente lo que \
                                se dice, en el idioma en que se dice, sin resumir, sin corregir \
                                el estilo y sin añadir nada tuyo. Cada intervención va en su \
                                propia línea con el formato exacto `[mm:ss] Hablante: texto`, \
                                donde mm:ss es el momento en que empieza, contado desde el \
                                principio de este audio. Si una intervención es larga, pártela \
                                en varias líneas con su tiempo cada una, cada 30 segundos más o \
                                menos. Lo que no se entienda va como [inaudible]. Si el audio \
                                no tiene voz, responde solo `[00:00] [sin voz]`. No escribas \
                                nada más: ni encabezados, ni comentarios, ni cercos de código."
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": format!(
                                "{proyecto}Fragmento {} de {} de la conversación. {hablantes}{anterior}",
                                ctx.n + 1, ctx.total
                            )
                        },
                        {
                            "type": "input_audio",
                            "input_audio": { "data": datos_b64, "format": "mp3" }
                        }
                    ]
                }
            ],
            // Cinco minutos de conversación rara vez pasan de 1500 tokens; el
            // margen es para que un trozo muy hablado no salga cortado.
            "max_tokens": 8000,
            "temperature": 0
        })
    }

    fn peticion_resumen(
        &self,
        transcripcion: &str,
        con_quien: &str,
        duracion: &str,
        proyecto: Option<&Proyecto>,
    ) -> Value {
        let recorte: String = transcripcion.chars().take(MAXIMO_TRANSCRIPCION).collect();
        let aviso = if recorte.chars().count() < transcripcion.chars().count() {
            "\n\n(La transcripción se ha recortado por longitud: dilo en el resumen amplio.)"
        } else {
            ""
        };
        let contexto = match proyecto {
            None => String::new(),
            Some(p) if p.descripcion.trim().is_empty() => {
                format!("Es una entrevista del proyecto «{}».\n", p.nombre)
            }
            Some(p) => format!(
                "Es una entrevista del proyecto «{}»: {}\n",
                p.nombre,
                p.descripcion.trim()
            ),
        };
        let con = match con_quien.trim() {
            "" => String::new(),
            q => format!("La persona entrevistada es: {q}.\n"),
        };
        let lista = json!({ "type": "array", "items": { "type": "string" } });
        let mut cuerpo = json!({
            "messages": [
                {
                    "role": "system",
                    "content": "Resumes entrevistas de trabajo entre dos personas a partir de su \
                                transcripción. Devuelves solo JSON. «titulo»: una frase corta y \
                                concreta que diga de qué fue la conversación, sin comillas ni \
                                punto final. «ejecutivo»: diez líneas como mucho, en prosa y sin \
                                encabezados, para alguien que no estuvo: de qué iba, qué se \
                                acordó y qué queda por hacer y de quién. «amplio»: el detalle \
                                por partes; cada elemento de cada lista es una idea completa en \
                                una frase. «participantes»: quién habla y en calidad de qué. \
                                «temas»: los asuntos tratados, en el orden en que salieron. \
                                «puntos_clave»: lo importante que se dijo, con cifras y fechas \
                                si las hay. «acuerdos»: solo lo que las dos partes dieron por \
                                decidido. «tareas»: lo que alguien tiene que hacer después, con \
                                quién lo hace. «frases»: hasta cinco citas literales que valga \
                                la pena conservar, sin comillas. No inventes nada que no esté en \
                                la transcripción: una lista sin material va vacía. Español, tono \
                                directo, sin preámbulos."
                },
                {
                    "role": "user",
                    "content": format!("{contexto}{con}Duración: {duracion}.\n\nTranscripción:\n\n{recorte}{aviso}")
                }
            ],
            "max_tokens": 4000,
            "temperature": 0.2,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "resumen_de_entrevista",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": {
                            "titulo": { "type": "string", "maxLength": 80 },
                            "ejecutivo": { "type": "string" },
                            "amplio": {
                                "type": "object",
                                "properties": {
                                    "participantes": lista,
                                    "temas": lista,
                                    "puntos_clave": lista,
                                    "acuerdos": lista,
                                    "tareas": {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "quien": { "type": "string" },
                                                "que": { "type": "string" }
                                            },
                                            "required": ["quien", "que"],
                                            "additionalProperties": false
                                        }
                                    },
                                    "frases": lista
                                },
                                "required": ["participantes", "temas", "puntos_clave", "acuerdos", "tareas", "frases"],
                                "additionalProperties": false
                            }
                        },
                        "required": ["titulo", "ejecutivo", "amplio"],
                        "additionalProperties": false
                    }
                }
            }
        });
        if let Some(modelo) = &self.modelo_documento {
            cuerpo["model"] = json!(modelo);
        }
        cuerpo
    }

    /// Pide título y etiquetas para una nota. `Err` describe el fallo para el
    /// log; quien llama decide qué hacer, y lo que hace es guardar igual (D6).
    pub async fn titular(
        &self,
        contenido: &str,
        etiquetas_existentes: &[String],
    ) -> Result<Sugerencia, String> {
        let texto = self
            .completar(
                self.peticion(contenido, etiquetas_existentes),
                "notas-titular",
                ESPERA,
            )
            .await?;
        let sugerencia: Sugerencia = serde_json::from_str(&texto)
            .map_err(|e| format!("el modelo no devolvió el JSON esperado: {e}"))?;
        Ok(limpiar(sugerencia))
    }

    /// Redacta el documento de un conjunto de notas (D11). El backend le añade
    /// después la sección de notas de origen.
    pub async fn documentar(
        &self,
        notas: &[NotaFuente],
        instruccion: &str,
        proyecto: Option<&Proyecto>,
        modelo: Option<&str>,
    ) -> Result<Documento, String> {
        if notas.is_empty() {
            return Err("no hay notas que documentar".into());
        }
        let texto = self
            .completar(
                self.peticion_documento(notas, instruccion, proyecto, modelo),
                "notas-documento",
                ESPERA_DOCUMENTO,
            )
            .await?;
        let documento: Documento = serde_json::from_str(&texto)
            .map_err(|e| format!("el modelo no devolvió el JSON esperado: {e}"))?;
        let documento = limpiar_documento(documento);
        if documento.markdown.trim().is_empty() {
            return Err("el modelo devolvió un documento vacío".into());
        }
        Ok(documento)
    }

    /// El análisis de un rol (D57): markdown con Veredicto, Hallazgos,
    /// Recomendaciones y Preguntas abiertas, sin títulos propios (los baja a
    /// cuarto nivel si se le escapan, porque el rol va bajo un `###`).
    pub async fn analizar_rol(
        &self,
        rol: &RolPedido<'_>,
        notas: &[NotaFuente],
        instruccion: &str,
        proyecto: Option<&Proyecto>,
        modelo: Option<&str>,
        operacion: &str,
    ) -> Result<String, String> {
        if notas.is_empty() {
            return Err("no hay notas que analizar".into());
        }
        let (texto, _) = self
            .completar_detalle(
                self.peticion_rol(rol, notas, instruccion, proyecto, modelo),
                operacion,
                ESPERA_DOCUMENTO,
            )
            .await?;
        let texto = bajar_titulos(&sin_cerco(&texto));
        if texto.trim().is_empty() {
            return Err("el modelo devolvió un análisis vacío".into());
        }
        Ok(texto)
    }

    /// La cabecera del informe (D57): título, resumen ejecutivo, conclusiones,
    /// tensiones entre roles, plan de acción y preguntas. Devuelve también el
    /// modelo que sirvió la llamada, que puede no ser el pedido.
    pub async fn sintetizar(
        &self,
        notas: &[NotaFuente],
        instruccion: &str,
        proyecto: Option<&Proyecto>,
        analisis: &[AnalisisHecho<'_>],
        modelo: Option<&str>,
        operacion: &str,
    ) -> Result<(String, Option<String>), String> {
        if analisis.is_empty() {
            return Err("no hay análisis que sintetizar".into());
        }
        let (texto, servido) = self
            .completar_detalle(
                self.peticion_sintesis(notas, instruccion, proyecto, analisis, modelo),
                operacion,
                ESPERA_DOCUMENTO,
            )
            .await?;
        let texto = sin_cerco(&texto);
        if texto.trim().is_empty() {
            return Err("el modelo devolvió una síntesis vacía".into());
        }
        Ok((texto, servido))
    }

    /// El modelo con que se redacta si nadie elige otro: el fijado en el
    /// entorno o, si no, el que el servicio pone por defecto.
    pub async fn modelo_defecto(&self) -> Option<String> {
        if let Some(m) = &self.modelo_documento {
            return Some(m.clone());
        }
        let cuerpo = self.consultar("/estado").await.ok()?;
        cuerpo
            .get("modelo_defecto")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    /// El catálogo del servicio (D60/D61). El servicio lo guarda una hora, así
    /// que preguntar a menudo no cuesta nada ni gasta crédito.
    pub async fn catalogo(&self) -> Result<Vec<ModeloCatalogo>, String> {
        let cuerpo = self.consultar("/models").await?;
        let datos = cuerpo
            .get("data")
            .cloned()
            .ok_or("el catálogo no trae «data»")?;
        serde_json::from_value(datos).map_err(|e| format!("catálogo ilegible: {e}"))
    }

    /// Lo que costó una operación según el servicio (D62): suma lo anotado en
    /// cada llamada con esa `X-Operacion`. `None` si no hay llamadas.
    pub async fn coste(&self, operacion: &str) -> Result<Option<f64>, String> {
        let cuerpo = self
            .consultar(&format!("/uso/resumen?operacion={}", codificar(operacion)))
            .await?;
        let llamadas = cuerpo
            .pointer("/totales/llamadas")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if llamadas == 0 {
            return Ok(None);
        }
        Ok(cuerpo.pointer("/totales/coste").and_then(Value::as_f64))
    }

    /// Un `GET` al servicio con la clave de la aplicación, que no sale de aquí.
    async fn consultar(&self, ruta: &str) -> Result<Value, String> {
        let respuesta = self
            .http
            .get(format!("{}{ruta}", self.base))
            .bearer_auth(&self.clave)
            .timeout(ESPERA_CONSULTA)
            .send()
            .await
            .map_err(|e| format!("no se pudo llamar al servicio: {}", causas(&e)))?;
        let estado = respuesta.status();
        let cuerpo: Value = respuesta
            .json()
            .await
            .map_err(|e| format!("respuesta ilegible: {e}"))?;
        if !estado.is_success() {
            let mensaje = cuerpo
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("sin detalle");
            return Err(format!("el servicio respondió {estado}: {mensaje}"));
        }
        Ok(cuerpo)
    }

    /// El envío, igual para todo lo que se le pide al servicio: mismo sobre de
    /// error y mismo sitio donde viene el texto de la respuesta.
    async fn completar(
        &self,
        peticion: Value,
        operacion: &str,
        espera: Duration,
    ) -> Result<String, String> {
        self.completar_detalle(peticion, operacion, espera)
            .await
            .map(|(texto, _)| texto)
    }

    /// Como `completar`, y además el modelo que sirvió la respuesta.
    async fn completar_detalle(
        &self,
        peticion: Value,
        operacion: &str,
        espera: Duration,
    ) -> Result<(String, Option<String>), String> {
        let respuesta = self
            .http
            .post(format!("{}/chat/completions", self.base))
            .bearer_auth(&self.clave)
            // Agrupa en el servicio todo lo que gasta esta aplicación.
            .header("X-Operacion", operacion)
            .timeout(espera)
            .json(&peticion)
            .send()
            .await
            .map_err(|e| format!("no se pudo llamar al servicio: {}", causas(&e)))?;

        let estado = respuesta.status();
        let cuerpo: Value = respuesta
            .json()
            .await
            .map_err(|e| format!("respuesta ilegible: {e}"))?;

        if !estado.is_success() {
            // El servicio manda siempre el mismo sobre de error; su `code` es
            // lo único que merece la pena en el log.
            let codigo = cuerpo
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("desconocido");
            let mensaje = cuerpo
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("sin detalle");
            return Err(format!(
                "el servicio respondió {estado} ({codigo}): {mensaje}"
            ));
        }

        // Una respuesta cortada por el tope de tokens llega con éxito pero a
        // medias: mejor decirlo que intentar leerla.
        if cuerpo
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            == Some("length")
        {
            return Err("el modelo cortó la respuesta por longitud".into());
        }
        let servido = cuerpo
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string);
        cuerpo
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(|t| (t.to_string(), servido))
            .ok_or_else(|| "la respuesta no trae contenido".to_string())
    }

    /// El cuerpo de la petición, en formato de la API de OpenAI. Sin `model` el
    /// servicio pone el suyo por defecto, que es el barato y suficiente para
    /// esto; `LLM_MODELO` lo sustituye sin tocar código.
    fn peticion(&self, contenido: &str, etiquetas_existentes: &[String]) -> Value {
        let recorte: String = contenido.chars().take(MAXIMO_CONTENIDO).collect();
        let conocidas: Vec<&str> = etiquetas_existentes
            .iter()
            .take(ETIQUETAS_DE_CONTEXTO)
            .map(String::as_str)
            .collect();

        let instruccion = if conocidas.is_empty() {
            "No hay etiquetas previas: crea las que la nota pida.".to_string()
        } else {
            format!(
                "Etiquetas que ya existen, reutilízalas antes de inventar una nueva: {}.",
                conocidas.join(", ")
            )
        };

        let mut cuerpo = json!({
            "messages": [
                {
                    "role": "system",
                    "content": "Titulas notas personales dictadas en español. \
                                Devuelves solo JSON con las claves «titulo» y «etiquetas». \
                                El título es una frase corta y concreta, en español, sin comillas \
                                ni punto final, de 60 caracteres como mucho. \
                                Las etiquetas son de una o dos palabras, en minúsculas, sin \
                                acentos ni almohadillas, y nombran el asunto de la nota: de qué \
                                trata, con quién o para cuándo. Prefiere una sola etiqueta buena \
                                a cuatro mediocres. Nunca etiquetes con palabras que valdrían \
                                para cualquier nota, como «nota», «personal», «general», \
                                «recordatorio», «tarea» o «varios»: no distinguen nada y \
                                ensucian el buscador."
                },
                {
                    "role": "user",
                    "content": format!("{instruccion}\n\nNota:\n{recorte}")
                }
            ],
            "max_tokens": 200,
            // Titular no es tarea creativa: cuanto más previsible, mejor.
            "temperature": 0.2,
            // Salida estructurada: evita tener que rescatar el JSON de entre la
            // prosa del modelo.
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "titulo_y_etiquetas",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": {
                            "titulo": { "type": "string", "maxLength": 60 },
                            "etiquetas": {
                                "type": "array",
                                "maxItems": 4,
                                "items": { "type": "string", "maxLength": 30 }
                            }
                        },
                        "required": ["titulo", "etiquetas"],
                        "additionalProperties": false
                    }
                }
            }
        });

        if let Some(modelo) = &self.modelo {
            cuerpo["model"] = json!(modelo);
        }
        cuerpo
    }

    /// El cuerpo de la petición del documento (D11). Misma API que titular,
    /// otro encargo: aquí el modelo no clasifica, redacta un enunciado de
    /// trabajo a partir de lo que dictó el usuario.
    fn peticion_documento(
        &self,
        notas: &[NotaFuente],
        instruccion: &str,
        proyecto: Option<&Proyecto>,
        modelo: Option<&str>,
    ) -> Value {
        let material = material_de(notas, "dilo en «Dudas por resolver»");
        let encargo = encargo_de(instruccion);
        let contexto = contexto_de(proyecto);
        let mut cuerpo = json!({
            "messages": [
                {
                    "role": "system",
                    "content": "Conviertes notas personales dictadas en un encargo de trabajo \
                                listo para dárselo a un agente de programación. No ejecutas nada: \
                                redactas el enunciado. Devuelves solo JSON con las claves \
                                «titulo» y «markdown». El título es una frase corta y concreta, \
                                sin comillas ni punto final. El markdown empieza por un \
                                encabezado de primer nivel con ese mismo título y lleva estas \
                                seis secciones de segundo nivel, en este orden y sin añadir ni \
                                quitar ninguna: «Contexto», «Objetivo», «Requisitos», \
                                «Decisiones y restricciones», «Criterios de aceptación», \
                                «Dudas por resolver». \
                                Reglas: no inventes ni un requisito que las notas no pidan; lo \
                                que quede ambiguo va en «Dudas por resolver» como pregunta \
                                concreta y cerrada, no como comentario. «Objetivo» es una sola \
                                frase. «Requisitos» y «Criterios de aceptación» van numerados, \
                                una idea por línea, y cada criterio tiene que ser comprobable. \
                                Funde lo que se repita en varias notas y lleva a «Dudas» lo que \
                                se contradiga, diciendo qué nota dice cada cosa. Si las notas no \
                                dan material para una sección, escribe en ella una línea que diga \
                                qué falta, en vez de rellenar. Español, tono directo, sin \
                                preámbulos ni cortesías, sin hablar de ti mismo ni del proceso. \
                                No escribas ninguna sección de notas de origen: esa la añade la \
                                aplicación."
                },
                {
                    "role": "user",
                    "content": format!("{encargo}{contexto}Notas dictadas, de la más antigua a la más reciente:\n\n{material}")
                }
            ],
            "max_tokens": 3000,
            // Algo más suelto que titular: hay que redactar, no clasificar.
            "temperature": 0.3,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "documento_de_notas",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": {
                            "titulo": { "type": "string", "maxLength": 80 },
                            "markdown": { "type": "string" }
                        },
                        "required": ["titulo", "markdown"],
                        "additionalProperties": false
                    }
                }
            }
        });

        if let Some(m) = modelo.or(self.modelo_documento.as_deref()) {
            cuerpo["model"] = json!(m);
        }
        cuerpo
    }

    /// El cuerpo de la petición de un rol (D57/D58): markdown libre, sin
    /// `json_schema`, para que valga cualquier modelo del catálogo.
    fn peticion_rol(
        &self,
        rol: &RolPedido<'_>,
        notas: &[NotaFuente],
        instruccion: &str,
        proyecto: Option<&Proyecto>,
        modelo: Option<&str>,
    ) -> Value {
        let material = material_de(notas, "dilo en «Hallazgos»");
        let encargo = encargo_de(instruccion);
        let contexto = contexto_de(proyecto);
        let sistema = format!(
            "Eres el rol «{nombre}» ({icono}) de un equipo que analiza notas personales dictadas \
             sobre un proyecto o una idea. Tu enfoque: {enfoque}\n\n\
             Lee todas las notas y analízalas desde ese enfoque y solo desde ese: otros roles \
             cubren lo demás. Devuelve solo markdown, sin ningún encabezado con almohadillas, con \
             exactamente estas cuatro partes y en este orden:\n\
             **Veredicto:** una o dos frases con tu conclusión.\n\
             **Hallazgos** y debajo una lista con guiones de 3 a 6 puntos; cada uno es una idea \
             concreta que se apoya en lo que dicen las notas.\n\
             **Recomendaciones** y debajo una lista numerada de 2 a 5 acciones concretas.\n\
             **Preguntas abiertas** y debajo una lista con guiones de 1 a 3 preguntas cerradas \
             que habría que contestar.\n\n\
             Reglas: no inventes datos que las notas no den; si te falta información para tu \
             enfoque, dilo en Hallazgos. Una cifra que no salga de las notas va marcada como \
             orden de magnitud. Español, tono directo, sin preámbulos ni cortesías y sin hablar \
             de ti mismo ni del proceso.",
            nombre = rol.nombre,
            icono = rol.icono,
            enfoque = rol.enfoque.trim(),
        );
        let mut cuerpo = json!({
            "messages": [
                { "role": "system", "content": sistema },
                {
                    "role": "user",
                    "content": format!("{encargo}{contexto}Notas dictadas, de la más antigua a la más reciente:\n\n{material}")
                }
            ],
            "max_tokens": MAXIMO_TOKENS_INFORME,
            "temperature": 0.4
        });
        if let Some(m) = modelo.or(self.modelo_documento.as_deref()) {
            cuerpo["model"] = json!(m);
        }
        cuerpo
    }

    /// El cuerpo de la síntesis (D57): notas y análisis de cada rol; escribe
    /// solo la cabecera del informe. Lo de cada rol lo añade el backend.
    fn peticion_sintesis(
        &self,
        notas: &[NotaFuente],
        instruccion: &str,
        proyecto: Option<&Proyecto>,
        analisis: &[AnalisisHecho<'_>],
        modelo: Option<&str>,
    ) -> Value {
        let material = material_de(notas, "dilo en «Preguntas abiertas»");
        let encargo = encargo_de(instruccion);
        let contexto = contexto_de(proyecto);
        let mut roles = String::new();
        for a in analisis {
            roles.push_str(&format!(
                "### {} {}\n\n{}\n\n",
                a.icono,
                a.nombre,
                a.texto.trim()
            ));
        }
        let tensiones = if analisis.len() == 1 {
            "Como solo hay un rol, en «Tensiones entre roles» escribe una sola línea que diga que \
             con un rol no hay tensiones que comparar."
        } else {
            "En «Tensiones entre roles», cada punto va como `**Rol A ↔ Rol B:** en qué chocan y \
             qué propones`; si no chocan en nada importante, una línea que lo diga."
        };
        let sistema = format!(
            "Coordinas un equipo de analistas que ha revisado las mismas notas desde enfoques \
             distintos. Recibes las notas y el análisis de cada rol, y escribes la cabecera de un \
             informe único. Devuelve solo markdown con esta estructura exacta:\n\
             `# ` y el título: una frase corta y concreta sobre de qué va el informe, sin comillas \
             ni punto final.\n\
             `## Resumen ejecutivo`: un párrafo de cinco a ocho líneas para alguien que no ha leído \
             nada: qué es, qué concluyen los roles y cuál es el mayor riesgo.\n\
             `## Conclusiones clave`: lista numerada de 3 a 6 conclusiones en las que coinciden \
             varios roles, cada una empezando por la idea en negrita.\n\
             `## Tensiones entre roles`: {tensiones}\n\
             `## Plan de acción`: lista numerada de 3 a 7 acciones ordenadas por prioridad; cada una \
             empieza por **Alta**, **Media** o **Baja** seguida de una raya.\n\
             `## Preguntas abiertas`: lista con guiones de las preguntas más importantes que \
             quedan, sin repetirlas.\n\n\
             Reglas: no copies el análisis de cada rol, que va entero detrás; sintetiza y cruza. \
             No inventes nada que no esté en las notas o en los análisis. No escribas ninguna otra \
             sección: el resto del informe lo añade la aplicación. Español, tono directo, sin \
             preámbulos ni cortesías."
        );
        let mut cuerpo = json!({
            "messages": [
                { "role": "system", "content": sistema },
                {
                    "role": "user",
                    "content": format!("{encargo}{contexto}Notas dictadas, de la más antigua a la más reciente:\n\n{material}\n\nAnálisis de cada rol:\n\n{roles}")
                }
            ],
            "max_tokens": MAXIMO_TOKENS_INFORME,
            "temperature": 0.3
        });
        if let Some(m) = modelo.or(self.modelo_documento.as_deref()) {
            cuerpo["model"] = json!(m);
        }
        cuerpo
    }
}

/// Las notas en el formato que lee el modelo, recortadas por nota si no caben
/// (con veinte notas largas, un corte global dejaría fuera las últimas).
/// `aviso` dice dónde tiene que contar el modelo que hubo recorte.
fn material_de(notas: &[NotaFuente], aviso: &str) -> String {
    let mut material = String::new();
    let mut recortadas = 0usize;
    for (i, n) in notas.iter().enumerate() {
        let cabecera = if n.etiquetas.is_empty() {
            format!("### Nota {} — {} ({})\n", i + 1, n.titulo, n.creada_en)
        } else {
            format!(
                "### Nota {} — {} ({}) · etiquetas: {}\n",
                i + 1,
                n.titulo,
                n.creada_en,
                n.etiquetas.join(", ")
            )
        };
        let sitio =
            MAXIMO_DOCUMENTO.saturating_sub(material.chars().count() + cabecera.chars().count());
        let por_nota = (MAXIMO_DOCUMENTO / notas.len().max(1)).min(sitio);
        let contenido: String = n.contenido.chars().take(por_nota).collect();
        if contenido.chars().count() < n.contenido.chars().count() {
            recortadas += 1;
        }
        material.push_str(&cabecera);
        material.push_str(&contenido);
        material.push_str("\n\n");
    }
    if recortadas > 0 {
        material.push_str(&format!(
            "(Aviso: {recortadas} nota(s) se han recortado por longitud; {aviso}.)\n"
        ));
    }
    material
}

/// La instrucción del usuario, si la hay, por delante de todo lo demás.
fn encargo_de(instruccion: &str) -> String {
    match instruccion.trim() {
        "" => String::new(),
        i => format!(
            "Instrucción del usuario para este documento, por encima de todo lo demás: {i}\n\n"
        ),
    }
}

/// El proyecto como contexto para el modelo (D21/D53).
fn contexto_de(proyecto: Option<&Proyecto>) -> String {
    match proyecto {
        None => String::new(),
        Some(p) if p.descripcion.trim().is_empty() => {
            format!("Todas las notas son del proyecto «{}».\n\n", p.nombre)
        }
        Some(p) => format!(
            "Todas las notas son del proyecto «{}»: {}\n\n",
            p.nombre,
            p.descripcion.trim()
        ),
    }
}

/// Un rol va bajo un `###` en el informe: cualquier título que se le escape
/// al modelo baja a cuarto nivel para no romper la estructura.
fn bajar_titulos(texto: &str) -> String {
    texto
        .lines()
        .map(|l| {
            let almohadillas = l.chars().take_while(|c| *c == '#').count();
            if (1..4).contains(&almohadillas) && l[almohadillas..].starts_with(' ') {
                format!("####{}", &l[almohadillas..])
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Lo justo para meter un valor en la query string sin traer otra dependencia.
fn codificar(v: &str) -> String {
    v.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `reqwest::Error` no enseña por si solo por que fallo la conexion: el motivo
/// real (DNS, TLS, rechazo) vive en la cadena de causas. Sin esto el log dice
/// solo «error sending request», que no sirve para arreglar nada.
fn causas(e: &reqwest::Error) -> String {
    let mut texto = e.to_string();
    let mut fuente = std::error::Error::source(e);
    while let Some(f) = fuente {
        texto.push_str(&format!(": {f}"));
        fuente = f.source();
    }
    texto
}

/// Etiquetas que valdrían para cualquier nota. El modelo tiene instrucciones
/// de no usarlas, pero se le escapan, y una sola que entre se perpetúa: a la
/// nota siguiente se le pasan las existentes para que reutilice, asi que el
/// ruido se propaga solo. Se filtran aqui, que es donde no hay margen de error.
const ETIQUETAS_INUTILES: [&str; 10] = [
    "nota",
    "notas",
    "personal",
    "general",
    "recordatorio",
    "tarea",
    "tareas",
    "varios",
    "otros",
    "apuntes",
];

/// Quita comillas y punto final, en cualquier orden y cuantas veces hagan
/// falta: el modelo devuelve «Título». y una sola pasada deja el cierre
/// colgando.
pub fn recortar_adornos(texto: &str) -> String {
    let mut t = texto.trim();
    loop {
        let antes = t;
        t = t
            .trim_matches(|c| matches!(c, '"' | '«' | '»' | '\u{201c}' | '\u{201d}'))
            .trim_end_matches('.')
            .trim();
        if t == antes {
            return t.to_string();
        }
    }
}

/// El esquema acota la forma, no el contenido: el título puede venir con
/// comillas y las etiquetas repetidas o vacías. Se normaliza aquí para que la
/// base de datos no herede la creatividad del modelo.
fn limpiar(mut s: Sugerencia) -> Sugerencia {
    s.titulo = recortar_adornos(&s.titulo).chars().take(60).collect();

    let mut vistas: Vec<String> = Vec::new();
    for etiqueta in s.etiquetas.drain(..) {
        let limpia: String = etiqueta
            .trim()
            .trim_start_matches('#')
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
            .take(30)
            .collect();
        let limpia = limpia.trim().to_string();
        if !limpia.is_empty()
            && !ETIQUETAS_INUTILES.contains(&limpia.as_str())
            && !vistas.contains(&limpia)
        {
            vistas.push(limpia);
        }
    }
    s.etiquetas = vistas.into_iter().take(4).collect();
    s
}

/// Quita el cerco de código con que el modelo envuelve a veces un texto.
fn sin_cerco(texto: &str) -> String {
    let t = texto.trim();
    if t.starts_with("```") {
        let sin_apertura = t.split_once('\n').map(|(_, resto)| resto).unwrap_or("");
        sin_apertura
            .trim_end()
            .trim_end_matches("```")
            .trim()
            .to_string()
    } else {
        t.to_string()
    }
}

/// El modelo a veces envuelve el markdown en un cerco de código, y el título
/// se le va en comillas igual que en las notas. Se quita aquí.
fn limpiar_documento(mut d: Documento) -> Documento {
    d.titulo = recortar_adornos(&d.titulo).chars().take(80).collect();

    d.markdown = sin_cerco(&d.markdown);
    d
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn quita_comillas_punto_y_recorta_el_titulo() {
        let s = limpiar(Sugerencia {
            titulo: format!("  \"{}\".  ", "a".repeat(80)),
            etiquetas: vec![],
        });
        assert_eq!(s.titulo.chars().count(), 60);
        assert!(!s.titulo.contains('"'));
    }

    #[test]
    fn quita_comillas_aunque_vayan_antes_del_punto() {
        let s = limpiar(Sugerencia {
            titulo: "  «Llamar al taller».  ".into(),
            etiquetas: vec![],
        });
        assert_eq!(s.titulo, "Llamar al taller");
    }

    #[test]
    fn normaliza_etiquetas_y_quita_repetidas() {
        let s = limpiar(Sugerencia {
            titulo: "Da igual".into(),
            etiquetas: vec![
                "#Trabajo".into(),
                "trabajo".into(),
                "  Ideas  ".into(),
                "".into(),
                "compra".into(),
                "viaje".into(),
                "sobrante".into(),
            ],
        });
        assert_eq!(s.etiquetas, vec!["trabajo", "ideas", "compra", "viaje"]);
    }

    #[test]
    fn descarta_las_etiquetas_que_valen_para_cualquier_nota() {
        let s = limpiar(Sugerencia {
            titulo: "Pedir presupuesto para los neumáticos".into(),
            etiquetas: vec![
                "coche".into(),
                "nota".into(),
                "Personal".into(),
                "presupuesto".into(),
            ],
        });
        assert_eq!(s.etiquetas, vec!["coche", "presupuesto"]);
    }

    #[test]
    fn quita_el_cerco_de_codigo_del_documento() {
        let d = limpiar_documento(Documento {
            titulo: "  «Migrar el buscador».  ".into(),
            markdown: "```markdown\n# Migrar el buscador\n\n## Contexto\nTexto.\n```".into(),
        });
        assert_eq!(d.titulo, "Migrar el buscador");
        assert!(d.markdown.starts_with("# Migrar"));
        assert!(!d.markdown.contains("```"));
    }

    #[test]
    fn el_documento_sin_cerco_se_queda_igual() {
        let d = limpiar_documento(Documento {
            titulo: "Migrar el buscador".into(),
            markdown: "# Migrar el buscador\n\n## Contexto\nTexto.".into(),
        });
        assert!(d.markdown.ends_with("Texto."));
    }

    #[test]
    fn sin_clave_no_hay_ia() {
        // La ausencia de LLM_API_KEY es el modo sin IA, no un fallo.
        temporal_sin_clave(|| assert!(Llm::del_entorno().is_none()));
    }

    fn temporal_sin_clave(f: impl FnOnce()) {
        let previa = std::env::var("LLM_API_KEY").ok();
        std::env::remove_var("LLM_API_KEY");
        f();
        if let Some(v) = previa {
            std::env::set_var("LLM_API_KEY", v);
        }
    }
}
