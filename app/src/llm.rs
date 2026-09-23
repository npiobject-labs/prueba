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

/// D31: un trozo son cinco minutos de audio. El proxy espera 120 s al
/// proveedor; aquí algo menos, para que el error sea nuestro y legible.
const ESPERA_TRANSCRIPCION: Duration = Duration::from_secs(110);

/// Modelo con entrada de audio si no se fija `LLM_MODELO_AUDIO`. El de
/// titular por defecto es un «lite», que con audio separa peor las voces.
const MODELO_AUDIO: &str = "google/gemini-2.5-flash";

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
        let texto = self
            .completar(
                self.peticion_resumen(transcripcion, con_quien, duracion, proyecto),
                "entrevista-resumir",
                ESPERA_DOCUMENTO,
            )
            .await?;
        let mut r: Resumen = serde_json::from_str(&texto)
            .map_err(|e| format!("el modelo no devolvió el JSON esperado: {e}"))?;
        r.titulo = recortar_adornos(&r.titulo).chars().take(80).collect();
        r.ejecutivo = sin_cerco(&r.ejecutivo);
        if r.ejecutivo.is_empty() {
            return Err("el modelo devolvió un resumen vacío".into());
        }
        Ok(r)
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
    ) -> Result<Documento, String> {
        if notas.is_empty() {
            return Err("no hay notas que documentar".into());
        }
        let texto = self
            .completar(
                self.peticion_documento(notas, instruccion, proyecto),
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

    /// El envío, igual para todo lo que se le pide al servicio: mismo sobre de
    /// error y mismo sitio donde viene el texto de la respuesta.
    async fn completar(
        &self,
        peticion: Value,
        operacion: &str,
        espera: Duration,
    ) -> Result<String, String> {
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

        cuerpo
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(str::to_string)
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
    ) -> Value {
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
            // Se recorta por nota, no por el final: con veinte notas largas, un
            // corte global dejaría fuera las últimas enteras.
            let sitio = MAXIMO_DOCUMENTO
                .saturating_sub(material.chars().count() + cabecera.chars().count());
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
                "(Aviso: {recortadas} nota(s) se han recortado por longitud; dilo en «Dudas por resolver».)\n"
            ));
        }

        let encargo = match instruccion.trim() {
            "" => String::new(),
            i => format!(
                "Instrucción del usuario para este documento, por encima de todo lo demás: {i}\n\n"
            ),
        };

        let contexto = match proyecto {
            None => String::new(),
            Some(p) if p.descripcion.trim().is_empty() => {
                format!("Todas las notas son del proyecto «{}».\n\n", p.nombre)
            }
            Some(p) => format!(
                "Todas las notas son del proyecto «{}»: {}\n\n",
                p.nombre,
                p.descripcion.trim()
            ),
        };

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

        if let Some(modelo) = &self.modelo_documento {
            cuerpo["model"] = json!(modelo);
        }
        cuerpo
    }
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
fn recortar_adornos(texto: &str) -> String {
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
