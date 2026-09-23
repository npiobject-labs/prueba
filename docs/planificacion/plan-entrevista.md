# Plan — sección Entrevista

Fecha: 2026-09-23 · Estado: v1, **solo planificación**; F18 a F21 sin hacer · Continúa `plan-proyectos.md` (D14 a D26, F13 a F17) · Fuente de verdad: este fichero (la copia en Drive es solo copia).

## 1. Qué se pide (en palabras del usuario)

De la nota «Nueva sección de entrevista en la aplicación», dictada el 2026-09-23:

1. Una sección nueva, **Entrevista**. No es una nota que se escribe o se dicta, sino un **diálogo con otra persona**.
2. Primer paso: **grabar el diálogo** como fichero de voz.
3. Segundo paso: cuando se pida **con un botón**, **transcribir** el diálogo a texto.
4. Tercer paso: «decides tú cuál es el más conveniente», y si ya hay herramientas o librerías que lo hagan, usarlas.
5. (En la sesión) «Vamos a comenzar con una nueva sección dentro de la aplicación, una sección que se va a llamar entrevista».

## 2. Qué es una entrevista aquí

Una entrevista es **una grabación con lo que sale de ella**: el audio, la transcripción con quién dice qué y, en el tercer paso, un resumen útil. Vive **dentro de un proyecto**, igual que las notas (D14/D15): se graba en el proyecto activo y aparece en su lista.

No se mezcla con las notas. Una nota es una idea corta que titula la IA al momento. Una entrevista dura minutos u horas, pesa megas y se transcribe cuando se pide. Tienen tabla propia y pantalla propia. Lo que sale de una entrevista **sí puede acabar como nota** (D33): así entra en todo lo que ya existe (etiquetas, búsqueda, documentos, copiar).

**Tercer paso elegido: resumen estructurado de la entrevista**, generado desde la transcripción, con Participantes, Temas tratados, Puntos clave, Acuerdos y decisiones, Tareas pendientes (quién y qué) y Frases literales relevantes. Un botón «Guardar como nota» lo pasa al proyecto. Es lo que da valor a una conversación grabada: nadie relee una transcripción de una hora, pero todo el mundo necesita saber qué se acordó y qué queda por hacer.

## 3. Decisiones y supuestos

Numeración continua (última: D26).

| # | Decisión | Estado |
|---|---|---|
| D27 | **Grabar en el navegador con `MediaRecorder`** (`audio/webm;codecs=opus`, mono, unos 32 kbps: ~14 MB por hora). Grabar, Pausa/Seguir y Parar, con cronómetro y medidor de nivel. **Wake Lock** mientras graba, para que la pantalla no se apague. | [SUPUESTO] Chrome en Android sigue grabando con la pantalla encendida y la pestaña delante. Plan B si corta en segundo plano: aviso fijo «No salgas de la app mientras grabas» y, si no basta, la grabadora del sistema más «Subir audio» (D28). |
| D28 | **No perder nunca una grabación.** El audio se va guardando en IndexedDB a trozos de 10 s mientras se graba. Si la subida falla o se cierra la pestaña, al volver aparece «Hay una grabación sin subir · Subir / Descartar». Además, **«Subir audio»** acepta un fichero ya grabado (m4a, mp3, ogg, webm, wav). | Decidido. Es la misma filosofía que D6/D12: lo del usuario no se pierde. |
| D29 | **El audio se guarda en el volumen de Fly** (`/data/audio/<id>.<ext>`). La base guarda metadatos y texto, no binarios. `GET /entrevistas/{id}/audio` lo sirve con `Range` para el reproductor; la app lo pide con `fetch` y el token (un `<audio src>` no manda cabeceras) y lo reproduce desde un blob. | [SUPUESTO] El volumen de 1 GB llega (~70 h de audio). Plan B: `flyctl volumes extend` o borrar el audio tras transcribir, dejando el texto (opción por entrevista). |
| D30 | **Transcribir con el mismo proveedor de siempre** (D5): un modelo con entrada de audio por el proxy `openrouter`, mensajes `input_audio` y un modelo con audio (por defecto un Gemini Flash). **Solo `llm.rs` lo sabe**, con una función nueva `transcribir(trozo, contexto)`. Se pide la transcripción **con hablantes** («Entrevistador:», «Entrevistado:» o nombres si se dicen), en el idioma original y sin resumir. | [SUPUESTO, lo primero que se prueba en F19] El proxy deja pasar `input_audio` y el modelo separa bien dos voces. Plan B: un servicio de transcripción con separación de hablantes de verdad (Deepgram o AssemblyAI) con su propia clave, `STT_API_KEY`, detrás de la misma función de `llm.rs`. |
| D31 | **Trocear antes de transcribir.** El backend convierte el audio con `ffmpeg` a mp3 mono de 24 kbps en **trozos de 5 min** (~0,9 MB, ~1,2 MB en base64). Así cada petición cabe en el límite de 2 MB del proxy (axum por defecto), un fallo solo repite un trozo, y la salida del modelo nunca pasa de su tope. Cada trozo lleva el final del anterior como contexto, para no cambiar de nombre a los hablantes a mitad. | Decidido. `ffmpeg` entra en la imagen de Docker (unos 80 MB más). |
| D32 | **Transcribir es asíncrono**, como los documentos (D11): `POST /entrevistas/{id}/transcribir` contesta al momento, la entrevista pasa por `pendiente → transcribiendo (3/12) → lista | fallida`, y la app pregunta cada 3 s y enseña el avance. Si falla, «Reintentar» sigue desde el trozo que falló. La transcripción se puede **editar** (como las notas, `PUT`). | Decidido. |
| D33 | **Tercer paso: resumen** (sección 2). `POST /entrevistas/{id}/resumir`, asíncrono, con `LLM_MODELO_DOCUMENTO`, estructura fija en markdown y el proyecto como contexto (D21). «Guardar como nota» crea una nota en el proyecto con el resumen y un enlace a la entrevista; la IA la titula y etiqueta como a cualquier otra. «📋 Copiar» copia resumen y transcripción. | Decidido. |
| D34 | **Título** propuesto por la IA con el resumen, editable siempre. Hasta entonces, «Entrevista · 23 sept 12:40». Campo opcional «Con quién» al grabar, que también se pasa al modelo para nombrar a los hablantes. | Decidido. |
| D35 | **Navegación**: tercera pestaña **🎙 Entrevistas** junto a Proyectos y Notas (dentro del proyecto activo). En la lista, cada entrevista enseña duración, estado y fecha. | [SUPUESTO] Tres pestañas caben a 360 px. Plan B: la entrevista entra como un tipo de elemento más en la lista de notas, con un icono. |
| D36 | **Consentimiento**: antes de la primera grabación, aviso de que grabar a otra persona requiere su permiso. Se enseña una vez y se recuerda en `localStorage`. | Decidido. Solo un aviso; la app no guarda ninguna prueba del consentimiento. |
| D37 | **Fuera de esta versión**: transcripción en directo mientras se habla, más de un idioma en la misma entrevista, preguntar a la entrevista («¿qué dijo de los plazos?») y documentos (F5) a partir de entrevistas. La pregunta a la entrevista es la siguiente candidata (F22). | Decidido. |

## 4. Modelo de datos (migración `user_version` 1 → 2)

```sql
CREATE TABLE entrevistas (
  id            TEXT PRIMARY KEY,
  proyecto_id   TEXT REFERENCES proyectos(id) ON DELETE SET NULL,
  titulo        TEXT NOT NULL,
  con_quien     TEXT NOT NULL DEFAULT '',
  creada_en     TEXT NOT NULL,
  duracion_s    INTEGER NOT NULL DEFAULT 0,
  audio         TEXT,              -- nombre del fichero en /data/audio, NULL si se borró
  audio_bytes   INTEGER NOT NULL DEFAULT 0,
  estado        TEXT NOT NULL DEFAULT 'grabada',  -- grabada|transcribiendo|transcrita|fallida
  trozos_total  INTEGER NOT NULL DEFAULT 0,
  trozos_hechos INTEGER NOT NULL DEFAULT 0,
  transcripcion TEXT NOT NULL DEFAULT '',
  resumen       TEXT NOT NULL DEFAULT '',
  estado_resumen TEXT NOT NULL DEFAULT 'ninguno', -- ninguno|pendiente|listo|fallido
  error         TEXT NOT NULL DEFAULT ''
);
CREATE INDEX entrevistas_proyecto ON entrevistas(proyecto_id, creada_en DESC);
```

Los trozos transcritos se guardan por separado (`entrevista_trozos(entrevista_id, n, texto)`), para que un reintento siga donde se quedó y la transcripción final sea su concatenación.

## 5. API (CORS abierto y bajo el token de D8)

| Ruta | Qué hace |
|---|---|
| `GET /entrevistas?proyecto=` | Lista, sin el texto largo |
| `POST /entrevistas` | Sube el audio (`multipart`: `audio`, `proyecto`, `con_quien`, `duracion_s`). Límite de cuerpo de 200 MB solo en esta ruta |
| `GET /entrevistas/{id}` | Todo, con la transcripción y el resumen |
| `PUT /entrevistas/{id}` | Editar título, con quién, transcripción y resumen |
| `DELETE /entrevistas/{id}` | Borra la entrevista y su audio |
| `GET /entrevistas/{id}/audio` | El audio, con `Range` |
| `POST /entrevistas/{id}/transcribir` | Arranca o reanuda la transcripción (503 sin `LLM_API_KEY`) |
| `POST /entrevistas/{id}/resumir` | Arranca el resumen (409 sin transcripción) |
| `POST /entrevistas/{id}/nota` | Crea la nota con el resumen en el mismo proyecto |

## 6. Pantallas (mock 18 en adelante)

1. **Pestaña 🎙 Entrevistas**: la lista del proyecto y el botón «🎙 Nueva entrevista».
2. **Grabar**: campo «Con quién», botón grande Grabar / Pausa / Parar, cronómetro, nivel de audio y «Subir audio» como alternativa. Al parar: «Guardar» (sube) o «Descartar» (con confirmación).
3. **Entrevista**: reproductor, estado, **Transcribir** (con avance «3 de 12»), transcripción con hablantes, **Resumir**, resumen, «Guardar como nota», «📋 Copiar», Editar, Mover y Borrar.

## 7. Fases

| Fase | Qué | Se prueba |
|---|---|---|
| F18 | **Grabar y guardar**: tabla, `POST`/`GET`/`DELETE`, audio en el volumen, pestaña, pantalla de grabar con IndexedDB, subir fichero y reproductor. | Grabar 2 min en el móvil, cerrar la pestaña a mitad y recuperar lo grabado; `deploy.yml` sube un audio pequeño y lo baja igual. |
| F19 | **Transcribir**: `ffmpeg` en la imagen, trocear, `llm.rs::transcribir`, estado y avance, reintento por trozo, editar el texto. **Primero la prueba de D30** (un trozo real por el proxy). | Una conversación de dos personas de ~10 min; `deploy.yml` transcribe 5 s de audio sintético si hay clave. |
| F20 | **Resumen** y «Guardar como nota», copiar. | El resumen de F19 trae acuerdos y tareas reconocibles. |
| F21 | Aviso de consentimiento, exportar y detalles de la lista. | — |

## 8. Preguntas al usuario (antes de F18)

1. ¿Cuánto dura una entrevista típica? (¿15 min, 1 h, más?) Cambia el tamaño del volumen y el coste.
2. ¿Casi siempre son **dos personas**, o también reuniones de varias?
3. ¿Te vale el resumen como tercer paso, o preferías otro (por ejemplo, sacar solo tareas, o un acta formal)?
4. ¿Borrar el audio al transcribir para ahorrar espacio, o guardarlo siempre?

## 9. Riesgos

- **Separación de hablantes con un modelo de chat** (D30): puede confundir voces parecidas. Por eso el plan B está en la misma función de `llm.rs` y el texto es editable.
- **Grabación en segundo plano en Android** (D27): si Chrome la corta, el plan B es subir el audio de la grabadora del sistema, que ya queda hecho en F18.
- **Coste**: [SUPUESTO] una hora de audio con un Gemini Flash anda por céntimos. El proxy lo apunta en `/uso`; se revisa tras la primera entrevista real.
- **Privacidad**: el audio sale del backend hacia el proveedor del modelo. Va bajo la misma clave y el mismo proxy que las notas, pero es voz de un tercero; por eso D36.
- **Memoria de la máquina de Fly** (256 MB): `ffmpeg` trabaja por trozos y en disco, y la subida va en streaming al fichero, sin cargarla en memoria.
