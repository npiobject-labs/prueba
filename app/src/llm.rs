//! Título y etiquetas por LLM (F2, D5).
//!
//! Todo el contrato con el proveedor vive aquí y se configura por entorno, así que
//! cambiar de API es cambiar secretos, no código:
//!
//! | Variable        | Por defecto             | Qué es |
//! |-----------------|-------------------------|--------|
//! | `LLM_URL`       | —                       | Endpoint completo. Vacío = IA apagada (D6: título de respaldo). |
//! | `LLM_MODELO`    | (no se envía)           | Identificador del modelo. Si la pasarela ya lo decide, se deja vacío. |
//! | `LLM_API_KEY`   | —                       | Clave. Nunca se imprime. |
//! | `LLM_CABECERA`  | `Authorization`         | Cabecera donde va la clave. |
//! | `LLM_PREFIJO`   | `Bearer ` si la cabecera es `Authorization`, si no vacío | Prefijo del valor. |
//! | `LLM_ESPERA`    | `8`                     | Segundos de espera máxima (D6). |
//! | `LLM_OPERACION` | `notas-titulo`          | Etiqueta de la llamada, en la cabecera `X-Operacion` que usa la pasarela. |
//!
//! Petición estilo OpenAI (`{model, messages}`), que es lo que expone OpenRouter y sus
//! pasarelas. La lectura de la respuesta es tolerante: vale `choices[].message.content`
//! y también un JSON plano, porque no todas las pasarelas devuelven lo mismo.

use std::time::Duration;

use serde_json::{json, Value};

pub struct Sugerencia {
    pub titulo: String,
    pub etiquetas: Vec<String>,
}

fn var(nombre: &str) -> Option<String> {
    std::env::var(nombre)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Hay IA configurada. Lo publica `/salud` para que el móvil sepa qué esperar.
pub fn configurada() -> bool {
    var("LLM_URL").is_some()
}

fn espera() -> Duration {
    Duration::from_secs(
        var("LLM_ESPERA")
            .and_then(|v| v.parse().ok())
            .filter(|s| (1..=60).contains(s))
            .unwrap_or(8),
    )
}

const SISTEMA: &str = "Eres un archivero de notas dictadas en español. \
Devuelves SOLO un objeto JSON, sin texto alrededor y sin vallas de código, con esta forma: \
{\"titulo\": \"...\", \"etiquetas\": [\"...\"]}. \
El título resume el asunto de la nota en español, máximo 60 caracteres, sin comillas ni punto final, \
y no empieza por \"Nota sobre\". \
Las etiquetas son de una o dos palabras, en minúsculas y sin almohadilla: entre una y tres. \
Reutiliza las etiquetas existentes que encajen antes de inventar otras nuevas.";

/// Pide título y etiquetas. `None` si no hay IA configurada, si falla o si tarda de más:
/// quien llama se queda con el título de respaldo y `pendiente_ia` (D6).
pub async fn sugerir(contenido: &str, etiquetas_existentes: &[String]) -> Option<Sugerencia> {
    let url = var("LLM_URL")?;
    let cabecera = var("LLM_CABECERA").unwrap_or_else(|| "Authorization".to_string());
    let prefijo = var("LLM_PREFIJO").unwrap_or_else(|| {
        if cabecera.eq_ignore_ascii_case("authorization") {
            "Bearer ".to_string()
        } else {
            String::new()
        }
    });

    // Notas largas: al LLM le sobra con el principio para titular, y así el coste no se dispara.
    let recorte: String = contenido.chars().take(4000).collect();
    let existentes = if etiquetas_existentes.is_empty() {
        "(ninguna todavía)".to_string()
    } else {
        etiquetas_existentes.join(", ")
    };
    let mut peticion = json!({
        "temperature": 0.2,
        "max_tokens": 200,
        "messages": [
            { "role": "system", "content": SISTEMA },
            { "role": "user", "content": format!("Etiquetas existentes: {existentes}\n\nNota:\n{recorte}") }
        ]
    });
    // Solo se manda "model" si hay uno fijado: hay pasarelas que eligen ellas el modelo.
    if let Some(modelo) = var("LLM_MODELO") {
        peticion["model"] = Value::String(modelo);
    }

    let mut cabeceras = reqwest::header::HeaderMap::new();
    if let Some(clave) = var("LLM_API_KEY") {
        let nombre = reqwest::header::HeaderName::from_bytes(cabecera.as_bytes()).ok()?;
        let mut valor = reqwest::header::HeaderValue::from_str(&format!("{prefijo}{clave}")).ok()?;
        valor.set_sensitive(true);
        cabeceras.insert(nombre, valor);
    }

    // La pasarela traza cada llamada por esta cabecera.
    if let Ok(valor) = reqwest::header::HeaderValue::from_str(
        &var("LLM_OPERACION").unwrap_or_else(|| "notas-titulo".to_string()),
    ) {
        cabeceras.insert("X-Operacion", valor);
    }

    let cliente = reqwest::Client::builder()
        .timeout(espera())
        .build()
        .map_err(|e| eprintln!("llm: no se pudo crear el cliente: {e}"))
        .ok()?;

    let respuesta = cliente
        .post(&url)
        .headers(cabeceras)
        .json(&peticion)
        .send()
        .await
        .map_err(|e| eprintln!("llm: fallo la peticion: {e}"))
        .ok()?;

    let estado = respuesta.status();
    let cuerpo = respuesta
        .text()
        .await
        .map_err(|e| eprintln!("llm: no se pudo leer la respuesta: {e}"))
        .ok()?;
    if !estado.is_success() {
        // El cuerpo puede traer el motivo (modelo desconocido, sin saldo); la clave nunca viaja aquí.
        eprintln!("llm: HTTP {estado}: {}", cuerpo.chars().take(300).collect::<String>());
        return None;
    }

    let texto = serde_json::from_str::<Value>(&cuerpo)
        .ok()
        .and_then(|v| texto_de(&v))
        .unwrap_or(cuerpo);
    let sugerencia = interpretar(&texto);
    if sugerencia.is_none() {
        eprintln!("llm: respuesta no interpretable: {}", texto.chars().take(300).collect::<String>());
    }
    sugerencia
}

/// Saca el texto generado mire donde mire el proveedor: estilo OpenAI o JSON plano.
fn texto_de(v: &Value) -> Option<String> {
    let candidatos = [
        v.pointer("/choices/0/message/content"),
        v.pointer("/choices/0/text"),
        v.pointer("/content/0/text"),
        v.get("respuesta"),
        v.get("resultado"),
        v.get("texto"),
        v.get("content"),
        v.get("output_text"),
    ];
    for c in candidatos.into_iter().flatten() {
        if let Some(s) = c.as_str() {
            if !s.trim().is_empty() {
                return Some(s.to_string());
            }
        }
        // Algunas pasarelas devuelven ya el objeto {titulo, etiquetas} en vez de texto.
        if c.is_object() {
            return Some(c.to_string());
        }
    }
    // Último recurso: el propio cuerpo ya es {titulo, etiquetas}.
    v.get("titulo").map(|_| v.to_string())
}

/// Texto del LLM -> sugerencia validada. Acepta vallas de código y texto alrededor.
fn interpretar(texto: &str) -> Option<Sugerencia> {
    let inicio = texto.find('{')?;
    let fin = texto.rfind('}')?;
    if fin < inicio {
        return None;
    }
    let objeto: Value = serde_json::from_str(&texto[inicio..=fin]).ok()?;

    let titulo = limpiar_titulo(objeto.get("titulo").and_then(Value::as_str).unwrap_or_default())?;
    let etiquetas = objeto
        .get("etiquetas")
        .and_then(Value::as_array)
        .map(|v| {
            let mut limpias: Vec<String> = vec![];
            for e in v.iter().filter_map(Value::as_str) {
                if let Some(e) = limpiar_etiqueta(e) {
                    if !limpias.contains(&e) {
                        limpias.push(e);
                    }
                }
            }
            limpias.truncate(4);
            limpias
        })
        .unwrap_or_default();

    Some(Sugerencia { titulo, etiquetas })
}

fn limpiar_titulo(bruto: &str) -> Option<String> {
    let t = bruto.trim().trim_matches(['"', '\'', '«', '»', '“', '”']).trim();
    let t = t.trim_end_matches('.').trim();
    if t.is_empty() {
        return None;
    }
    let mut titulo: String = t.chars().take(70).collect();
    if t.chars().count() > 70 {
        if let Some(i) = titulo.rfind(' ') {
            titulo.truncate(i);
        }
        titulo.push('…');
    }
    Some(titulo)
}

fn limpiar_etiqueta(bruto: &str) -> Option<String> {
    let e: String = bruto
        .trim()
        .trim_start_matches('#')
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let largo = e.chars().count();
    // Ni vacías ni frases: una etiqueta es una palabra o dos.
    if (2..=24).contains(&largo) && e.split_whitespace().count() <= 2 {
        Some(e)
    } else {
        None
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn interpreta_json_con_vallas() {
        let s = interpretar("```json\n{\"titulo\":\"Compra semanal.\",\"etiquetas\":[\"#Compra\",\"casa\",\"x\"]}\n```")
            .expect("deberia interpretar");
        assert_eq!(s.titulo, "Compra semanal");
        assert_eq!(s.etiquetas, vec!["compra".to_string(), "casa".to_string()]);
    }

    #[test]
    fn sin_titulo_no_hay_sugerencia() {
        assert!(interpretar("{\"etiquetas\":[\"casa\"]}").is_none());
        assert!(interpretar("lo siento, no puedo").is_none());
    }

    #[test]
    fn texto_estilo_openai_y_plano() {
        let openai = json!({ "choices": [{ "message": { "content": "{\"titulo\":\"Hola\"}" } }] });
        assert_eq!(texto_de(&openai).as_deref(), Some("{\"titulo\":\"Hola\"}"));
        let plano = json!({ "titulo": "Hola", "etiquetas": [] });
        assert!(texto_de(&plano).is_some());
    }
}
