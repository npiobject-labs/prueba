//! Título y etiquetas de una nota, pedidos al servicio `openrouter` del usuario
//! (npiobject-labs/openrouter), que es un proxy con el contrato de OpenAI y su
//! propia clave, presupuesto y registro de uso.
//!
//! Aquí solo se conoce la URL del servicio y la clave de aplicación: la clave de
//! OpenRouter nunca sale de aquel backend. Si el servicio no está configurado,
//! falla o tarda más de la cuenta, el llamante se queda con el título de
//! respaldo y la nota marcada `pendiente_ia` (D6): la nota nunca se pierde.

use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

/// Producción del servicio (VPS). Con `LLM_URL` se apunta al de Fly
/// (`https://openrouter-npiobject-labs.fly.dev`) o a uno local.
const URL_DEFECTO: &str = "https://apisor.oracle402.com";
/// D6: pasados 8 s se abandona y la nota se guarda con el título de respaldo.
const ESPERA: Duration = Duration::from_secs(8);
/// Una nota dictada larga no mejora el título y sí encarece la llamada.
const MAX_CONTENIDO: usize = 4000;
const MAX_TITULO: usize = 60;
const MAX_ETIQUETA: usize = 24;
const MAX_ETIQUETAS: usize = 3;

const SISTEMA: &str = "Eres el archivero de una aplicación de notas dictadas en español. \
Recibes el texto de una nota, muchas veces dictado y sin puntuación, y devuelves cómo archivarla. \
El título es una frase nominal breve que diga de qué va la nota, en español, con mayúscula inicial, \
sin punto final, sin comillas y de 60 caracteres como mucho; nunca escribas «Nota sobre…». \
Las etiquetas son de una a tres, en minúsculas, una o dos palabras, en singular, sin almohadilla, \
y reutilizas las de la lista de etiquetas ya en uso siempre que encajen: solo inventas una nueva \
cuando ninguna sirva. Responde solo con el JSON pedido.";

pub struct Config {
    base: String,
    clave: String,
    /// Opcional: si falta, manda el modelo por defecto del servicio.
    modelo: Option<String>,
}

pub struct Sugerencia {
    pub titulo: String,
    pub etiquetas: Vec<String>,
}

fn variable(nombre: &str) -> Option<String> {
    std::env::var(nombre)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Sin `LLM_CLAVE` no hay IA: el servicio exige `Authorization: Bearer` en todo
/// `/v1`, así que llamar sin clave solo gastaría tiempo para acabar en 401.
pub fn config() -> Option<&'static Config> {
    static CONFIG: OnceLock<Option<Config>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            Some(Config {
                clave: variable("LLM_CLAVE")?,
                base: variable("LLM_URL")
                    .unwrap_or_else(|| URL_DEFECTO.to_string())
                    .trim_end_matches('/')
                    .to_string(),
                modelo: variable("LLM_MODELO"),
            })
        })
        .as_ref()
}

pub fn configurada() -> bool {
    config().is_some()
}

/// URL del servicio, para el log de arranque y `/salud`. Nunca la clave.
pub fn url() -> Option<&'static str> {
    config().map(|c| c.base.as_str())
}

/// Salida estructurada: así la respuesta es un JSON con forma fija en vez de
/// texto libre que habría que adivinar.
fn esquema() -> Value {
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": "titulo_de_nota",
            "strict": true,
            "schema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["titulo", "etiquetas"],
                "properties": {
                    "titulo": { "type": "string", "maxLength": MAX_TITULO },
                    "etiquetas": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": MAX_ETIQUETAS,
                        "items": { "type": "string", "maxLength": MAX_ETIQUETA }
                    }
                }
            }
        }
    })
}

fn http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(ESPERA)
            .user_agent("prueba-notas (npiobject-labs)")
            .build()
            .expect("no se pudo construir el cliente HTTP")
    })
}

#[derive(Deserialize)]
struct Cruda {
    titulo: String,
    #[serde(default)]
    etiquetas: Vec<String>,
}

pub async fn titular(contenido: &str, existentes: &[String]) -> Result<Sugerencia, String> {
    let config = config().ok_or_else(|| "sin LLM_CLAVE: IA desactivada".to_string())?;
    let texto: String = contenido.chars().take(MAX_CONTENIDO).collect();
    let lista = if existentes.is_empty() {
        "(ninguna todavía)".to_string()
    } else {
        existentes.join(", ")
    };

    let mut cuerpo = json!({
        "messages": [
            { "role": "system", "content": SISTEMA },
            { "role": "user", "content": format!("Etiquetas ya en uso: {lista}\n\nNota:\n{texto}") }
        ],
        "max_tokens": 200,
        "temperature": 0.2,
        "response_format": esquema(),
    });
    if let Some(modelo) = &config.modelo {
        cuerpo["model"] = json!(modelo);
    }

    let respuesta = http()
        .post(format!("{}/v1/chat/completions", config.base))
        .bearer_auth(&config.clave)
        // El servicio agrupa el gasto por operación; así se ve qué cuestan las notas.
        .header("X-Operacion", "notas-titulo")
        .json(&cuerpo)
        .send()
        .await
        .map_err(|e| format!("no se pudo llamar al servicio LLM: {e}"))?;

    let estado = respuesta.status();
    let datos: Value = respuesta
        .json()
        .await
        .map_err(|e| format!("respuesta ilegible del servicio LLM: {e}"))?;

    // El servicio siempre responde el mismo sobre de error: {"ok":false,"error":{...}}.
    if !estado.is_success() {
        let codigo = datos
            .pointer("/error/code")
            .and_then(Value::as_str)
            .unwrap_or("desconocido");
        let mensaje = datos
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("sin detalle");
        return Err(format!(
            "el servicio LLM respondió {estado} ({codigo}): {mensaje}"
        ));
    }

    let mensaje = datos
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| "la respuesta del LLM no traía contenido".to_string())?;
    let cruda: Cruda = serde_json::from_str(recortar_json(mensaje))
        .map_err(|e| format!("el LLM no devolvió el JSON pedido ({e}): {mensaje}"))?;

    limpiar(cruda)
}

/// Un modelo sin salida estructurada puede envolver el JSON en texto o en un
/// bloque ```json: nos quedamos con lo que hay entre la primera llave y la última.
fn recortar_json(texto: &str) -> &str {
    match (texto.find('{'), texto.rfind('}')) {
        (Some(i), Some(f)) if f > i => &texto[i..=f],
        _ => texto.trim(),
    }
}

fn limpiar(cruda: Cruda) -> Result<Sugerencia, String> {
    let titulo: String = cruda
        .titulo
        .trim()
        .trim_matches(|c| matches!(c, '"' | '«' | '»' | '.'))
        .trim()
        .chars()
        .take(MAX_TITULO)
        .collect();
    if titulo.is_empty() {
        return Err("el LLM devolvió un título vacío".to_string());
    }

    let mut etiquetas: Vec<String> = vec![];
    for etiqueta in cruda.etiquetas {
        let limpia: String = etiqueta
            .trim()
            .trim_start_matches('#')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
            .chars()
            .take(MAX_ETIQUETA)
            .collect();
        if !limpia.is_empty() && !etiquetas.contains(&limpia) {
            etiquetas.push(limpia);
        }
        if etiquetas.len() == MAX_ETIQUETAS {
            break;
        }
    }

    Ok(Sugerencia { titulo, etiquetas })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_json_se_recorta_aunque_venga_envuelto() {
        assert_eq!(
            recortar_json("```json\n{\"titulo\":\"A\"}\n```"),
            "{\"titulo\":\"A\"}"
        );
        assert_eq!(recortar_json("  {\"a\":1}  "), "{\"a\":1}");
    }

    #[test]
    fn las_etiquetas_se_normalizan_y_deduplican() {
        let s = limpiar(Cruda {
            titulo: "  «Compra de material».  ".into(),
            etiquetas: vec![
                "#Compras".into(),
                "compras".into(),
                "  Material   eléctrico ".into(),
                "extra".into(),
                "sobra".into(),
            ],
        })
        .unwrap();
        assert_eq!(s.titulo, "Compra de material");
        assert_eq!(s.etiquetas, vec!["compras", "material eléctrico", "extra"]);
    }

    #[test]
    fn un_titulo_vacio_es_un_fallo() {
        assert!(limpiar(Cruda {
            titulo: " \" ".into(),
            etiquetas: vec![]
        })
        .is_err());
    }
}
