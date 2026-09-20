# Plan — app de notas dictadas

Fecha: 2026-09-20 · Estado: v6, F0 a F6 hechas: notas dictadas, tituladas por IA, buscables, con token, y documentos generados a partir de una selección de notas · Fuente de verdad: este fichero (la copia en Drive es solo copia).

## 1. Qué se pide (notas del usuario, en sus términos)

1. Botón «Nueva nota» (o icono `+`) → dictar o escribir → botón «Finalizar/Guardar» → la nota se almacena con **título**, **día y hora** de guardado.
2. Las notas se **clasifican con etiquetas**. Se busca por etiqueta, por fecha o por una palabra: la búsqueda por palabra mira **título y contenido** y devuelve las notas que la contienen.
3. **Título y etiquetas los genera la aplicación**, no el usuario. Si hace falta un agente/LLM, el usuario dispone de una API de agentes LLM.
4. **Seleccionar una o varias notas y generar con ellas un documento**: no un resumen, sino un prompt bien montado, con buena estructura, para pasárselo a Claude Code. Lo redacta un agente de resúmenes y generación de prompts.
5. Entregables de esta sesión: esta planificación (repo + copia en Drive) y un mock en el repo para verlo desde el móvil.

## 2. Decisiones y supuestos

| # | Decisión | Estado |
|---|---|---|
| D1 | Frontend: PWA estática en `docs/` (Pages), mobile-first, sin framework. Es lo que ya publica `pages.yml` y lo que el usuario abre desde el móvil. | Decidido |
| D2 | Backend: el `app/` existente (Rust, axum) crece con una API REST `/notas`. Un solo binario en Fly. | Decidido |
| D3 | Persistencia: SQLite en un volumen de Fly (`/data/notas.db`, volumen `datos` que crea `deploy.yml`), con FTS5 para la búsqueda por palabra (F3). | **Confirmado** por el usuario. Plan B: Postgres gestionado (Neon/Supabase) si se necesitan varias máquinas o backup automático. |
| D4 | Dictado: en el navegador con Web Speech API (`SpeechRecognition`, `lang=es-ES`), resultado en el mismo `textarea`. | [SUPUESTO] Chrome/Android es el uso principal. Plan B: en iOS Safari no hay `SpeechRecognition`; se usa el dictado del teclado del sistema (el micro del teclado escribe en el `textarea`), sin cambios en la app. |
| D5 | Título y etiquetas: los genera el backend al guardar, llamando al LLM del usuario con un prompt cerrado que devuelve JSON `{titulo, etiquetas[]}`. Vocabulario de etiquetas controlado: se le pasan las etiquetas ya existentes para que reutilice antes de inventar. | **Resuelto** en F2. La «API de agentes LLM» es el servicio `npiobject-labs/openrouter`: `POST {base}/chat/completions` con `Authorization: Bearer <clave de aplicación>`, formato de la API de OpenAI, salida estructurada por esquema JSON. El supuesto se cumplió, así que el plan B (otro protocolo) no hizo falta; el aislamiento en `llm.rs` se mantiene igual por si cambia. Base por defecto `https://openrouter-npiobject-labs.fly.dev/v1`, sustituible con la variable `LLM_BASE`. |
| D6 | Si el LLM falla o tarda >8 s, la nota se guarda igual con un título de respaldo (primera frase, ≤60 caracteres) y sin etiquetas, marcada `pendiente_ia=true`; un reintento posterior la completa. La nota nunca se pierde por culpa de la IA. | Decidido |
| D7 | Fecha/hora: el servidor guarda UTC (`creada_en`); el cliente muestra en hora local. Búsqueda por fecha = rango de día en hora local convertido a UTC en el cliente. | Decidido |
| D8 | Acceso: un solo usuario. Token estático en cabecera `Authorization: Bearer` guardado en el móvil una vez (pantalla «Ajustes»). El backend lo exige solo si existe el secreto `TOKEN_API` en Fly, que `deploy.yml` toma del secreto de repositorio del mismo nombre; sin él, la API queda abierta y `/salud` lo indica (`"token": false`). | Hecho en F4 y **activado el 2026-09-20**: el secreto existe, la API exige token y la app lo lleva guardado. [SUPUESTO] No hace falta multiusuario. Plan B: Passkeys/WebAuthn si se abre a más gente. |
| D9 | Secreto del LLM: secreto de Fly (`LLM_API_KEY`), nunca en el repo ni en `docs/`. | **Hecho** en F2. No se fija a mano: es un secreto de repositorio que `deploy.yml` vuelca a Fly en cada despliegue, y que borra de Fly si se quita del repositorio. Su valor es la clave de la aplicación `prueba` en el servicio, que solo se ve una vez al crearla. |
| D10 | Mock de esta sesión: funcional sin backend, guarda en `localStorage` y simula título/etiquetas con una heurística local, para validar la UX desde el móvil antes de tocar `app/`. | Decidido |
| D11 | Documentos (F5/F6): se seleccionan de 1 a 20 notas y el backend le pide a la IA un **encargo de trabajo** con estructura fija: Contexto, Objetivo, Requisitos, Decisiones y restricciones, Criterios de aceptación, Dudas por resolver. Estructura fija y no elegida por el modelo, para que sea predecible y verificable en `deploy.yml`. La última sección, «Notas de origen», la escribe el backend con los títulos y las fechas reales, no el modelo. Generación **asíncrona**: `POST /documentos` devuelve `estado:"pendiente"` al momento y la app pregunta cada 3 s, así cerrar la pestaña no pierde el trabajo. Instrucción libre opcional al generar y al regenerar. El documento se guarda y es editable a mano (`editado`). | **Hecho** en F5/F6. [SUPUESTO] Las notas pueden ser de proyectos distintos, así que el documento sale limpio, sin preámbulo del tipo «eres Claude Code trabajando en el repo X»: ese contexto lo pone el `CLAUDE.md` del repo destino. Plan B si molesta: campo «proyecto» fijo en Ajustes que se antepone al documento. |
| D12 | Rastro en los dos sentidos: desde el documento se llega a sus notas y desde la nota (`GET /notas/{id}`) a los documentos en los que entró. `documento_notas` **no** tiene clave ajena contra `notas` y copia el título: borrar una nota no borra ni mutila el documento que salió de ella, que aparece con `existe:false`. | Decidido. El detalle de la nota trae `documentos`; la lista no, que costaría una consulta por nota para algo que allí no se enseña. |
| D13 | Modelo de los documentos: variable de repositorio `LLM_MODELO_DOCUMENTO`, separada de `LLM_MODELO`. Espera de 120 s (frente a los 8 s de titular) y 3000 tokens de salida. | Decidido. [SUPUESTO] **Sin fijar**: hoy redacta el mismo modelo que titula, que es el barato. Plan B, y probablemente lo que toca: el usuario define la variable con un modelo mejor y relanza `deploy.yml`. |

## 3. Modelo de datos

```sql
CREATE TABLE notas (
  id          TEXT PRIMARY KEY,          -- ulid
  titulo      TEXT NOT NULL,
  contenido   TEXT NOT NULL,
  creada_en   TEXT NOT NULL,             -- ISO-8601 UTC
  pendiente_ia INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE etiquetas (nota_id TEXT REFERENCES notas(id) ON DELETE CASCADE, etiqueta TEXT NOT NULL, PRIMARY KEY (nota_id, etiqueta));
CREATE VIRTUAL TABLE notas_fts USING fts5(titulo, contenido, content='notas', content_rowid='rowid');

-- F5/F6
CREATE TABLE documentos (
  id TEXT PRIMARY KEY, titulo TEXT NOT NULL, instruccion TEXT NOT NULL DEFAULT '',
  texto TEXT NOT NULL DEFAULT '', estado TEXT NOT NULL,   -- pendiente | listo | fallido
  error TEXT, editado INTEGER NOT NULL DEFAULT 0,
  creado_en TEXT NOT NULL, actualizado_en TEXT NOT NULL
);
-- nota_id sin clave ajena a propósito (D12): el documento sobrevive a la nota.
CREATE TABLE documento_notas (
  documento_id TEXT NOT NULL REFERENCES documentos(id) ON DELETE CASCADE,
  nota_id TEXT NOT NULL, orden INTEGER NOT NULL, titulo TEXT NOT NULL, creada_en TEXT NOT NULL,
  PRIMARY KEY (documento_id, nota_id)
);
```

## 4. API (backend, todas con `Access-Control-Allow-Origin: *` como marca `CLAUDE.md`)

| Método y ruta | Qué hace |
|---|---|
| `POST /notas` `{contenido}` | Guarda, pide título+etiquetas al LLM (D5/D6), devuelve la nota completa. |
| `GET /notas?q=&etiqueta=&desde=&hasta=` | Lista filtrada. `q` usa FTS5 sobre título y contenido; `desde/hasta` ISO UTC; filtros combinables. Orden: más reciente primero. |
| `GET /notas/:id` | Una nota. |
| `DELETE /notas/:id` | Borra. |
| `POST /notas/:id/reintentar-ia` | Vuelve a pedir título/etiquetas (D6). |
| `GET /etiquetas` | Etiquetas con recuento, para los chips del buscador. |
| `POST /documentos` `{notas[], instruccion?}` | Crea el documento y lo redacta en segundo plano (D11). 201 con `estado:"pendiente"`. 400 si no hay notas, si pasan de 20 o si alguna no existe; 503 sin IA. |
| `GET /documentos` | Los 200 más recientes, cada uno con sus notas de origen. |
| `GET /documentos/:id` | Uno. Es lo que sondea la app mientras `estado` es `pendiente`. |
| `PUT /documentos/:id` `{texto}` | Guarda la edición a mano; marca `editado`. |
| `POST /documentos/:id/regenerar` `{instruccion?}` | Vuelve a redactarlo con las mismas notas. 409 si ya se está generando. |
| `DELETE /documentos/:id` | Borra el documento; las notas no se tocan. |

## 5. Fases

| Fase | Entregable | Verificación |
|---|---|---|
| **F0** ✅ | Plan + mock 1 con `localStorage` (archivado en `docs/mocks/002-notas-local.html`). | Pages verde (`a55de42`). |
| **F1** ✅ | `app/`: SQLite + volumen de Fly, `POST/GET/DELETE /notas`, `GET /notas?q=&etiqueta=&desde=&hasta=`, `GET /etiquetas`. Título de respaldo (D6); `etiquetas` vacías hasta F2. `docs/index.html` (mock 2) habla con la API. | `deploy.yml` verifica `/notas` (crear, buscar, borrar) además de `/salud`. **Riesgo cerrado** el 2026-09-20: la API exige token desde que se creó el secreto `TOKEN_API`. Estuvo abierta desde F1, y con F2 eso habría pasado de notas basura a gasto real en el modelo. |
| **F2** ✅ | `llm.rs` (aislado, D5) + secreto `LLM_API_KEY` volcado por `deploy.yml`; título y etiquetas reales al guardar; `POST /notas/{id}/reintentar-ia` y botón «Titular con IA» en el detalle; distintivo «sin titular» en la lista; `/salud` declara `"ia"`. | `deploy.yml` verifica las dos caras: con clave, una nota real vuelve con `pendiente_ia:false` y etiquetas; sin clave, `/salud` dice `"ia":false`. Probado además en local contra un servicio simulado: título y etiquetas correctos, normalización de comillas y etiquetas repetidas, servicio caído → nota guardada y pendiente, reintento → completada. |
| **F3** ✅ | FTS5 en `notas_fts` (`unicode61 remove_diacritics 2`) mantenida por triggers y reconstruida al arrancar si se desincroniza; `q` = palabras por prefijo con AND implícito, sin sintaxis especial. | `deploy.yml` crea «Reunión…» y la encuentra con `q=reunion`. |
| **F4** ✅ | `manifest.webmanifest`, iconos 192/512, `sw.js` (carcasa en caché, red primero; la API nunca), banda «sin conexión» con Guardar bloqueado, pantalla Ajustes con token (D8) y botón «Instalar» cuando el navegador lo ofrece. Backend: `TOKEN_API` opcional, 401 sin él. | Instalar en pantalla de inicio; recarga sin red muestra la carcasa; 401 abre Ajustes. |
| **F5** ✅ | Backend de documentos (D11/D12): tablas `documentos` y `documento_notas`, las seis rutas de la tabla de arriba, `llm.rs` crece con `documentar()` y su modelo propio (D13), generación en segundo plano con `tokio::spawn` y rescate al arrancar de lo que quedara a medias por un reinicio de la máquina. | `deploy.yml` crea dos notas, genera un documento real, espera a que esté listo y comprueba que trae sus secciones y sus notas de origen; sin clave de IA, comprueba que la ruta contesta 503. Probado antes en local contra un servicio simulado: deduplicación de la selección, orden cronológico, edición, regeneración, y que borrar una nota deja el documento en pie. |
| **F6** ✅ | Mock 7 (`PR-B1-20260920-009`): modo selección (botón ☑ o pulsación larga) que **sobrevive a los filtros**, contador de notas y caracteres, hoja de instrucción opcional, pantalla de documentos, detalle con el texto editable y Copiar / Compartir / Descargar `.md` / Regenerar / Borrar, y chips que llevan de la nota al documento y del documento a la nota. | Probado con Chromium a 390×844: ciclo entero sin un solo error de consola, incluida la selección mantenida al filtrar y el sondeo hasta que el documento queda listo. Falta la prueba en el móvil de verdad. |

Cada fase termina con push, verificación por workflow, entrada en bitácora y resumen de sesión, como marca `CLAUDE.md`.

## 6. Pendiente del usuario

- ~~Activar el token (D8)~~ **hecho el 2026-09-20**: secreto `TOKEN_API` creado, `deploy.yml` lanzado a mano (crear un secreto no dispara ningún despliegue por sí solo) y token guardado en Ajustes. La app avisó de que hacía falta antes de tenerlo, que es justo lo que D8 pedía. Para rotarlo: nuevo valor en el secreto, relanzar `deploy.yml` y volver a pegarlo en Ajustes, en ese orden.
- ~~Activar la IA (D5/D9)~~ **hecho**: el secreto `LLM_API_KEY` existe y cada despliegue lo verifica contra el servicio de verdad. El run de F5 tituló una nota real y redactó un documento real.
- Instalar la app en el móvil y decir qué cambia.
- **Elegir el modelo de los documentos (D13)**: hoy los redacta el mismo modelo barato que titula las notas. Definir la variable de repositorio `LLM_MODELO_DOCUMENTO` con un modelo mejor y relanzar `deploy.yml`. Es lo que más va a notarse en la calidad del prompt que sale.
