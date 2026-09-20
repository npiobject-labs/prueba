# Plan — app de notas dictadas

Fecha: 2026-09-20 · Estado: v4, F0 a F4 hechas · Fuente de verdad: este fichero (la copia en Drive es solo copia).

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
| D3 | Persistencia: SQLite en un volumen de Fly (`/data/notas.db`, volumen `datos` que crea `deploy.yml`), con FTS5 para la búsqueda por palabra (F3). | **Confirmado** por el usuario. Plan B: Postgres gestionado (Neon/Supabase) si se necesitan varias máquinas o backup automático. |
| D4 | Dictado: en el navegador con Web Speech API (`SpeechRecognition`, `lang=es-ES`), resultado en el mismo `textarea`. | [SUPUESTO] Chrome/Android es el uso principal. Plan B: en iOS Safari no hay `SpeechRecognition`; se usa el dictado del teclado del sistema (el micro del teclado escribe en el `textarea`), sin cambios en la app. |
| D5 | Título y etiquetas: los genera el backend al guardar, llamando al LLM con un prompt cerrado y **salida estructurada** que devuelve JSON `{titulo, etiquetas[]}`. Vocabulario controlado: se le pasan las 40 etiquetas ya existentes para que reutilice antes de inventar. | **Resuelto**: la «API de agentes LLM» es el servicio propio `npiobject-labs/openrouter` (contrato de OpenAI). Notas es una aplicación más de ese servicio: `POST {LLM_URL}/v1/chat/completions` con clave de aplicación. Aislado en `app/src/llm.rs`. Detalle en `titulado-ia.md`. |
| D6 | Si el LLM falla o tarda >8 s, la nota se guarda igual con un título de respaldo (primera frase, ≤60 caracteres) y sin etiquetas, marcada `pendiente_ia=true`; un reintento posterior la completa. La nota nunca se pierde por culpa de la IA. | Decidido |
| D7 | Fecha/hora: el servidor guarda UTC (`creada_en`); el cliente muestra en hora local. Búsqueda por fecha = rango de día en hora local convertido a UTC en el cliente. | Decidido |
| D8 | Acceso: un solo usuario. Token estático en cabecera `Authorization: Bearer` guardado en el móvil una vez (pantalla «Ajustes»). El backend lo exige solo si existe el secreto `TOKEN_API` en Fly, que `deploy.yml` toma del secreto de repositorio del mismo nombre; sin él, la API queda abierta y `/salud` lo indica (`"token": false`). | Hecho en F4. [SUPUESTO] No hace falta multiusuario. Plan B: Passkeys/WebAuthn si se abre a más gente. |
| D9 | Secreto del LLM: `LLM_CLAVE` (clave **de aplicación** del servicio `openrouter`, no la de OpenRouter), secreto de repositorio que `deploy.yml` vuelca a Fly. Nunca en el repo ni en `docs/`. `LLM_URL` y `LLM_MODELO` son variables de repositorio. | Decidido |
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
| `POST /notas/:id/reintentar-ia` | Vuelve a pedir título/etiquetas (D6). `503` sin LLM configurado, `502` si el servicio falla. |
| `GET /etiquetas` | Etiquetas con recuento, para los chips del buscador. |

## 5. Fases

| Fase | Entregable | Verificación |
|---|---|---|
| **F0** ✅ | Plan + mock 1 con `localStorage` (archivado en `docs/mocks/002-notas-local.html`). | Pages verde (`a55de42`). |
| **F1** ✅ | `app/`: SQLite + volumen de Fly, `POST/GET/DELETE /notas`, `GET /notas?q=&etiqueta=&desde=&hasta=`, `GET /etiquetas`. Título de respaldo (D6); `etiquetas` vacías hasta F2. `docs/index.html` (mock 2) habla con la API. | `deploy.yml` verifica `/notas` (crear, buscar, borrar) además de `/salud`. **Riesgo abierto**: la API es pública sin token hasta F4 (D8); cualquiera con la URL puede escribir. Si molesta antes, se adelanta D8. |
| **F2** ✅ | `app/src/llm.rs` contra el servicio `openrouter` (VPS o Fly), salida estructurada, etiquetas en uso como vocabulario, 8 s de espera y respaldo (D6); `POST /notas/{id}/reintentar-ia`; `/salud` dice `"ia"`; mock 6 marca «título provisional» y ofrece reintentar; ⚙ Ajustes dice si hay IA. **Falta la clave**: ver `titulado-ia.md`, «Puesta en marcha». | `deploy.yml` comprueba que `/salud` y el secreto concuerdan y, con clave, que la nota de prueba sale ya titulada (avisa sin fallar si el servicio externo no contesta: la nota se guarda igual). Probado en local contra un doble del servicio: titulado, etiquetas normalizadas, respaldo al caerse el servicio, reintento 502/404. |
| **F3** ✅ | FTS5 en `notas_fts` (`unicode61 remove_diacritics 2`) mantenida por triggers y reconstruida al arrancar si se desincroniza; `q` = palabras por prefijo con AND implícito, sin sintaxis especial. | `deploy.yml` crea «Reunión…» y la encuentra con `q=reunion`. |
| **F4** ✅ | `manifest.webmanifest`, iconos 192/512, `sw.js` (carcasa en caché, red primero; la API nunca), banda «sin conexión» con Guardar bloqueado, pantalla Ajustes con token (D8) y botón «Instalar» cuando el navegador lo ofrece. Backend: `TOKEN_API` opcional, 401 sin él. | Instalar en pantalla de inicio; recarga sin red muestra la carcasa; 401 abre Ajustes. |

Cada fase termina con push, verificación por workflow, entrada en bitácora y resumen de sesión, como marca `CLAUDE.md`.

## 6. Pendiente del usuario

- **Activar el token (D8)**: crear el secreto de repositorio `TOKEN_API` (GitHub → Settings → Secrets and variables → Actions) con un valor largo y aleatorio; el siguiente run de `deploy.yml` lo pasa a Fly. Después, en la app, ⚙ Ajustes → pegar el token → Guardar. Hasta entonces la API sigue abierta.
- **Activar la IA (F2)**: dar de alta la aplicación `notas` en el servicio `openrouter` (`tools\apps.ps1 -Crear notas` desde aquel repo, o la consola), ponerle presupuesto, y guardar la clave como secreto de repositorio `LLM_CLAVE` aquí. Pasos en `titulado-ia.md`. Hasta entonces las notas llevan título provisional.
- Instalar la app en el móvil y decir qué cambia.
