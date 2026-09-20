# Plan — app de notas dictadas

Fecha: 2026-09-20 · Estado: propuesta v1 · Fuente de verdad: este fichero (la copia en Drive es solo copia).

## 1. Qué se pide (notas del usuario, en sus términos)

1. Botón «Nueva nota» (o icono `+`) → dictar o escribir → botón «Finalizar/Guardar» → la nota se almacena con **título**, **día y hora** de guardado.
2. Las notas se **clasifican con etiquetas**. Se busca por etiqueta, por fecha o por una palabra: la búsqueda por palabra mira **título y contenido** y devuelve las notas que la contienen.
3. **Título y etiquetas los genera la aplicación**, no el usuario. Si hace falta un agente/LLM, el usuario dispone de una API de agentes LLM.
4. Entregables de esta sesión: esta planificación (repo + copia en Drive) y un mock en el repo para verlo desde el móvil.

## 2. Decisiones y supuestos

| # | Decisión | Estado |
|---|---|---|
| D1 | Frontend: PWA estática en `docs/` (Pages), mobile-first, sin framework. Es lo que ya publica `pages.yml` y lo que el usuario abre desde el móvil. | Decidido |
| D2 | Backend: el `app/` existente (Rust, axum) crece con una API REST `/notas`. Un solo binario en Fly. | Decidido |
| D3 | Persistencia: SQLite en un volumen de Fly (`/data/notas.db`), con FTS5 para la búsqueda por palabra. | [SUPUESTO] Un solo usuario y volumen bastan. Plan B: Postgres gestionado (Neon/Supabase) si se necesitan varias máquinas o backup automático. |
| D4 | Dictado: en el navegador con Web Speech API (`SpeechRecognition`, `lang=es-ES`), resultado en el mismo `textarea`. | [SUPUESTO] Chrome/Android es el uso principal. Plan B: en iOS Safari no hay `SpeechRecognition`; se usa el dictado del teclado del sistema (el micro del teclado escribe en el `textarea`), sin cambios en la app. |
| D5 | Título y etiquetas: los genera el backend al guardar, llamando al LLM del usuario con un prompt cerrado que devuelve JSON `{titulo, etiquetas[]}`. Vocabulario de etiquetas controlado: se le pasan las etiquetas ya existentes para que reutilice antes de inventar. | [SUPUESTO] La «API de agentes LLM» es compatible con un `POST` HTTPS + clave en cabecera (OpenAI-like o Anthropic-like). Plan B: si es otro protocolo, se aísla en un módulo `llm.rs` con la firma `fn titular(texto, etiquetas_existentes) -> {titulo, etiquetas}` y se cambia solo ese módulo. **Pendiente**: URL, formato y nombre del secreto. |
| D6 | Si el LLM falla o tarda >8 s, la nota se guarda igual con un título de respaldo (primera frase, ≤60 caracteres) y sin etiquetas, marcada `pendiente_ia=true`; un reintento posterior la completa. La nota nunca se pierde por culpa de la IA. | Decidido |
| D7 | Fecha/hora: el servidor guarda UTC (`creada_en`); el cliente muestra en hora local. Búsqueda por fecha = rango de día en hora local convertido a UTC en el cliente. | Decidido |
| D8 | Acceso: un solo usuario. Token estático en cabecera `Authorization: Bearer` guardado en el móvil una vez (pantalla «Ajustes»). | [SUPUESTO] No hace falta multiusuario. Plan B: Passkeys/WebAuthn si se abre a más gente. |
| D9 | Secreto del LLM: secreto de Fly (`fly secrets set LLM_API_KEY=...`), nunca en el repo ni en `docs/`. | Decidido |
| D10 | Mock de esta sesión: funcional sin backend, guarda en `localStorage` y simula título/etiquetas con una heurística local, para validar la UX desde el móvil antes de tocar `app/`. | Decidido |

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

## 5. Fases

| Fase | Entregable | Verificación |
|---|---|---|
| **F0** (esta sesión) | Este plan + mock navegable en `docs/index.html` con `localStorage`. | Pages verde; probar desde el móvil: crear, dictar, buscar, filtrar. |
| **F1** Backend CRUD | `app/`: SQLite + volumen de Fly, `POST/GET/DELETE /notas`, `GET /etiquetas`. Título de respaldo (D6) sin LLM aún. | `deploy.yml` verde; `docs/index.html` pasa de `localStorage` a la API (con el `?api=` local que ya describe `CLAUDE.md`). |
| **F2** IA | `llm.rs` + secreto en Fly; título y etiquetas reales; reintento. | Guardar tres notas distintas desde el móvil y ver títulos/etiquetas coherentes; apagar el secreto y comprobar que la nota se guarda igual (D6). |
| **F3** Búsqueda completa | FTS5 (`q`), filtro por etiqueta y por rango de fecha en la API y en la UI. Prefijos y acentos (`unicode61 remove_diacritics 2`). | Buscar «reunión» encuentra «reunion» y «Reuniones». |
| **F4** Pulido móvil | Manifest PWA + icono, instalable, aviso de «sin conexión», ajustes con token (D8). | Instalar en pantalla de inicio y usar sin abrir el navegador. |

Cada fase termina con push, verificación por workflow, entrada en bitácora y resumen de sesión, como marca `CLAUDE.md`.

## 6. Pendiente del usuario

- Datos de la API de agentes LLM: URL, formato de petición y cómo se llama el secreto (D5).
- Confirmar D3 (SQLite en Fly) y D8 (un solo usuario con token).
- Validar el mock desde el móvil y decir qué cambia antes de F1.
