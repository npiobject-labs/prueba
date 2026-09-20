//! Título y etiquetas de una nota, pedidos al servicio de modelos (D5).
//!
//! Todo lo que sabe la aplicación del proveedor vive aquí: si mañana cambia el
//! protocolo se reescribe este fichero y nada más, que es lo que el plan dejó
//! decidido como plan B de D5. Hacia fuera solo se ofrece `titular`.
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

/// Lo que el modelo devuelve para una nota.
#[derive(Debug, Deserialize)]
pub struct Sugerencia {
    pub titulo: String,
    #[serde(default)]
    pub etiquetas: Vec<String>,
}

/// Configuración del servicio, leída del entorno una sola vez al arrancar.
/// Sin clave no hay IA, y la aplicación funciona igual (D6).
pub struct Llm {
    http: Client,
    base: String,
    clave: String,
    modelo: Option<String>,
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
        })
    }

    /// Pide título y etiquetas para una nota. `Err` describe el fallo para el
    /// log; quien llama decide qué hacer, y lo que hace es guardar igual (D6).
    pub async fn titular(
        &self,
        contenido: &str,
        etiquetas_existentes: &[String],
    ) -> Result<Sugerencia, String> {
        let respuesta = self
            .http
            .post(format!("{}/chat/completions", self.base))
            .bearer_auth(&self.clave)
            // Agrupa en el servicio todo lo que gasta esta aplicación.
            .header("X-Operacion", "notas-titular")
            .timeout(ESPERA)
            .json(&self.peticion(contenido, etiquetas_existentes))
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
            return Err(format!("el servicio respondió {estado} ({codigo}): {mensaje}"));
        }

        let texto = cuerpo
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| "la respuesta no trae contenido".to_string())?;

        let sugerencia: Sugerencia = serde_json::from_str(texto)
            .map_err(|e| format!("el modelo no devolvió el JSON esperado: {e}"))?;

        Ok(limpiar(sugerencia))
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
    "nota", "notas", "personal", "general", "recordatorio", "tarea", "tareas", "varios", "otros",
    "apuntes",
];

/// El esquema acota la forma, no el contenido: el título puede venir con
/// comillas y las etiquetas repetidas o vacías. Se normaliza aquí para que la
/// base de datos no herede la creatividad del modelo.
fn limpiar(mut s: Sugerencia) -> Sugerencia {
    s.titulo = s
        .titulo
        .trim()
        .trim_matches(|c| c == '"' || c == '«' || c == '»')
        .trim_end_matches('.')
        .trim()
        .chars()
        .take(60)
        .collect();

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
