# Plan — sección Entrevista

Fecha: 2026-09-23 · Estado: **v3, F18a a F21 hechas** (build `PR-B1-20260923-021`, sección 13); F22 opcional sin hacer · Continúa `plan-proyectos.md` (D14 a D26, F13 a F17) · Fuente de verdad: este fichero (la copia en Drive es solo copia).

Cambios de v1 a v2: entra la **pantalla de captura rápida** (botones grandes, una mano), el **control sin mirar la pantalla** (auriculares, notificación, vibración) y el **arranque directo** desde el icono (D38 a D42); F18 se parte en dos (F18a captura, F18b guardar); se añade la guía de pruebas (sección 9) y qué se tomó de la propuesta externa (sección 11). D27 a D37 siguen como estaban salvo D27, que ahora remite a D38.

## 1. Qué se pide (en palabras del usuario)

Entrada completa en `entrevistas/recogida.md`. De la nota «Nueva sección de entrevista en la aplicación» (2026-09-23 12:31):

1. Una sección nueva, **Entrevista**. No es una nota que se escribe o se dicta, sino un **diálogo con otra persona**.
2. Primer paso: **grabar el diálogo** como fichero de voz.
3. Segundo paso: cuando se pida **con un botón**, **transcribir** el diálogo a texto.
4. Tercer paso: «decides tú cuál es el más conveniente», y si ya hay herramientas o librerías que lo hagan, usarlas.
5. Al grabar, **una pantalla en el móvil en la que solo aparezca eso**: Grabar, Pausar y Cerrar, **bien distinguidos y bien grandes**, para manejarlo con comodidad «sin tener que sacar el móvil y abrir y no sé qué».
6. «Si puede ser con alguna tecla del móvil, mejor: no haría falta ni abrir la aplicación».

Además, una propuesta de otro LLM, **solo informativa** (sección 11).

## 2. Qué es una entrevista aquí

Una entrevista es **una grabación con lo que sale de ella**: el audio, la transcripción con quién dice qué y, en el tercer paso, un resumen útil. Vive **dentro de un proyecto**, igual que las notas (D14/D15): se graba en el proyecto activo y aparece en su lista.

No se mezcla con las notas. Una nota es una idea corta que titula la IA al momento. Una entrevista dura minutos u horas, pesa megas y se transcribe cuando se pide. Tienen tabla propia y pantalla propia. Lo que sale de una entrevista **sí puede acabar como nota** (D33): así entra en todo lo que ya existe (etiquetas, búsqueda, documentos, copiar).

La grabación tiene un requisito propio que las notas no tienen: **se usa delante de otra persona**, con el móvil en la mano o sobre la mesa, sin atención para la pantalla. De ahí la pantalla de captura (D38): tres botones que se acierten sin mirar, respuesta por vibración y control desde los auriculares.

**Tercer paso elegido: dos resúmenes** generados desde la transcripción: uno **ejecutivo**, de diez líneas, para saber en un vistazo de qué iba y qué se acordó, y otro **amplio** con Participantes, Temas tratados, Puntos clave, Acuerdos y decisiones, Tareas pendientes (quién y qué) y Frases literales relevantes. Un botón «Guardar como nota» lo pasa al proyecto. Es lo que da valor a una conversación grabada: nadie relee una transcripción de una hora, pero todo el mundo necesita saber qué se acordó y qué queda por hacer.

## 3. Decisiones y supuestos

Numeración continua (última: D26). D38 a D42 son nuevas en v2.

| # | Decisión | Estado |
|---|---|---|
| D27 | **Grabar en el navegador con `MediaRecorder`** (`audio/webm;codecs=opus`, mono, unos 32 kbps: ~14 MB por hora). Cronómetro y medidor de nivel. **Wake Lock** mientras graba, para que la pantalla no se apague. La interfaz es la de D38. | [SUPUESTO] Chrome en Android sigue grabando con la pantalla encendida y la pestaña delante. Plan B si corta en segundo plano: aviso fijo «No salgas de la app mientras grabas» y, si no basta, la grabadora del sistema más «Subir audio» (D28). |
| D28 | **No perder nunca una grabación.** El audio se va guardando en IndexedDB a trozos de 10 s mientras se graba. Si la subida falla o se cierra la pestaña, al volver aparece «Hay una grabación sin subir · Subir / Descartar». Además, **«Subir audio»** acepta un fichero ya grabado (m4a, mp3, ogg, webm, wav). | Decidido. Es la misma filosofía que D6/D12: lo del usuario no se pierde. |
| D29 | **El audio se guarda en el volumen de Fly** (`/data/audio/<id>.<ext>`). La base guarda metadatos y texto, no binarios. `GET /entrevistas/{id}/audio` lo sirve con `Range` para el reproductor; la app lo pide con `fetch` y el token (un `<audio src>` no manda cabeceras) y lo reproduce desde un blob. **Se guarda siempre**, y en el detalle hay «Borrar audio» (pide confirmación, deja `audio NULL` y conserva transcripción y resúmenes): `DELETE /entrevistas/{id}/audio`. | Decidido por el usuario (pregunta 4). Con entrevistas de hasta 1 h (pregunta 1), 1 GB son ~70 entrevistas; si se acerca, `flyctl volumes extend`. |
| D30 | **Transcribir con el mismo proveedor de siempre** (D5): un modelo con entrada de audio por el proxy `openrouter`, mensajes `input_audio` y un modelo con audio (por defecto un Gemini Flash). **Solo `llm.rs` lo sabe**, con una función nueva `transcribir(trozo, contexto)`. Se pide la transcripción **con hablantes** («Entrevistador:», «Entrevistado:» o nombres si se dicen), en el idioma original y sin resumir. | [SUPUESTO, lo primero que se prueba en F19] El proxy deja pasar `input_audio` y el modelo separa bien dos voces. Plan B: un servicio de transcripción con separación de hablantes de verdad (Deepgram o AssemblyAI) con su propia clave, `STT_API_KEY`, detrás de la misma función de `llm.rs`. |
| D31 | **Trocear antes de transcribir.** El backend convierte el audio con `ffmpeg` a mp3 mono de 24 kbps en **trozos de 5 min** (~0,9 MB, ~1,2 MB en base64). Así cada petición cabe en el límite de 2 MB del proxy (axum por defecto), un fallo solo repite un trozo, y la salida del modelo nunca pasa de su tope. Cada trozo lleva el final del anterior como contexto, para no cambiar de nombre a los hablantes a mitad. | Decidido. `ffmpeg` entra en la imagen de Docker (unos 80 MB más). |
| D32 | **Transcribir es asíncrono**, como los documentos (D11): `POST /entrevistas/{id}/transcribir` contesta al momento, la entrevista pasa por `pendiente → transcribiendo (3/12) → lista | fallida`, y la app pregunta cada 3 s y enseña el avance. Si falla, «Reintentar» sigue desde el trozo que falló. La transcripción se puede **editar** (como las notas, `PUT`). | Decidido. |
| D33 | **Tercer paso: dos resúmenes** (sección 2). `POST /entrevistas/{id}/resumir` genera los dos en una sola tarea asíncrona con `LLM_MODELO_DOCUMENTO` y el proyecto como contexto (D21): primero el **ejecutivo** (10 líneas como mucho: de qué iba, qué se acordó, qué queda por hacer), y después el **amplio** con la estructura fija (Participantes, Temas tratados, Puntos clave, Acuerdos y decisiones, Tareas pendientes con quién y qué, Frases literales relevantes). En el detalle, el ejecutivo va arriba a la vista y el amplio plegado debajo. «Guardar como nota» ofrece **Ejecutivo / Amplio / Los dos**; la IA titula y etiqueta la nota como a cualquier otra. «📋 Copiar» copia los dos resúmenes y la transcripción. Columnas `resumen_ejecutivo` y `resumen` (sección 4). | Decidido por el usuario (pregunta 3). |
| D34 | **Título** propuesto por la IA con el resumen, editable siempre. Hasta entonces, «Entrevista · 23 sept 12:40». Campo opcional «Con quién» al grabar, que también se pasa al modelo para nombrar a los hablantes. Con la captura rápida (D38) el campo no estorba: se pide **después** de parar, no antes de grabar. | Decidido. |
| D35 | **Navegación**: tercera pestaña **🎙 Entrevistas** junto a Proyectos y Notas (dentro del proyecto activo). En la lista, cada entrevista enseña duración, estado y fecha. | Comprobado en el mock 18: caben a 360 px bajando la letra a 0,8 rem por debajo de 420 px. |
| D36 | **Consentimiento**: antes de la primera grabación, aviso de que grabar a otra persona requiere su permiso. Se enseña una vez y se recuerda en `localStorage`. | Decidido. Solo un aviso; la app no guarda ninguna prueba del consentimiento. |
| D37 | **Fuera de esta versión**: transcripción en directo mientras se habla, más de un idioma en la misma entrevista, preguntar a la entrevista («¿qué dijo de los plazos?») y documentos (F5) a partir de entrevistas. La pregunta a la entrevista es la siguiente candidata (F22). | Decidido. |
| D38 | **Pantalla de captura rápida** (sección 6.2): una vista a pantalla completa que tapa cabecera, pestañas y buscador, con **solo tres botones**: **Grabar** (rojo, el más grande, en la zona del pulgar), **Pausa / Seguir** (ámbar) y **Cerrar** (gris, arriba y aparte, para no pulsarlo sin querer). Cronómetro grande y nivel de audio. Nada más: ni título, ni «con quién», ni ajustes. Cada cambio de estado **vibra** con un patrón distinto (`navigator.vibrate`: 1 pulso al grabar, 2 al pausar, 1 largo al parar), para saber qué ha pasado sin mirar. Cerrar mientras graba pregunta «Guardar / Seguir grabando / Descartar». | Decidido. Es la lectura literal del punto 5 de la sección 1. |
| D39 | **Control sin tocar la pantalla, vía 1 (opcional, solo si hay auriculares): auriculares y pantalla de bloqueo con Media Session.** Las teclas de volumen **no llegan a una página web** en Android: no hay forma de capturarlas desde una PWA. Lo que sí llega es el botón de **auriculares o manos libres Bluetooth** (play/pause) a través de `navigator.mediaSession`, siempre que la página sea la sesión de medios activa. Para serlo, mientras se graba se reproduce en bucle un **audio silencioso**; con eso aparece además la **tarjeta de medios del sistema** (barra de notificaciones y pantalla de bloqueo) con Play/Pausa, que mapeamos a **Seguir/Pausa** de la grabación. El título de la tarjeta enseña «Grabando entrevista · 12:34». | [SUPUESTO, **lo primero que se prueba en F18a**, en el móvil del usuario] Chrome en Android mantiene la sesión de medios con el audio silencioso y entrega `play`/`pause` desde el manos libres y la pantalla de bloqueo. Plan B (D40). |
| D40 | **Control sin tocar la pantalla, vía 2: notificación con botones.** El service worker enseña una notificación persistente «Grabando · Pausa · Parar»; al pulsar, `notificationclick` avisa a la página por `postMessage` y esta actúa. Vale en la barra de notificaciones y en la pantalla de bloqueo. Exige permiso de notificaciones (se pide con la primera grabación, junto al del micrófono). | Plan B de D39, y se hace **también** si D39 funciona, porque cubre lo que D39 no cubre (parar) y no depende de tener auriculares. Si la página muere, la notificación ya no puede hacer nada: la grabación es de la página (D27). |
| D41 | **Arranque directo desde el icono**: un **acceso directo del manifest** (`shortcuts`, pulsación larga sobre el icono de la app instalada → «🎙 Grabar entrevista») abre `index.html?grabar=1`, que entra **directamente** en la pantalla de captura del proyecto activo con el micrófono ya pedido: **un solo toque más** para empezar. Con la PWA instalada, Chrome recuerda el permiso del micrófono, así que no vuelve a preguntar. | Decidido. Es lo más cerca de «sin abrir la aplicación» que llega una web: se abre, pero sin pasar por ninguna pantalla. **No es posible** desde una PWA: grabar con la pantalla bloqueada sin abrirla, un widget, un mosaico de ajustes rápidos o el botón de volumen. Eso solo lo da una app nativa (F22, opcional). |
| D42 | **Máquina de estados de la grabadora**, explícita en el código: `inactiva → pidiendo_permiso → grabando ⇄ pausada → parando → subiendo → guardada`, con `sin_permiso` (con instrucciones para dar el permiso en Ajustes del sitio) y `sin_subir` (D28) como estados de error. Todo lo que cambia de estado pasa por una sola función, y los tres botones, Media Session, la notificación y `?grabar=1` solo mandan **eventos** (`grabar`, `pausar`, `seguir`, `parar`, `cerrar`). | Decidido. Con cuatro fuentes de eventos, sin esto se descoordinan en una semana. |

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
  resumen_ejecutivo TEXT NOT NULL DEFAULT '',
  resumen       TEXT NOT NULL DEFAULT '',            -- el amplio
  estado_resumen TEXT NOT NULL DEFAULT 'ninguno', -- ninguno|pendiente|listo|fallido
  error         TEXT NOT NULL DEFAULT ''
);
CREATE INDEX entrevistas_proyecto ON entrevistas(proyecto_id, creada_en DESC);
```

Los trozos transcritos se guardan por separado (`entrevista_trozos(entrevista_id, n, texto)`), para que un reintento siga donde se quedó y la transcripción final sea su concatenación.

En el navegador, IndexedDB `notas-grabaciones` con un registro por grabación en curso (`{id, proyecto, empezada_en, trozos: Blob[], duracion_s, estado}`), que se borra al confirmar la subida (D28).

## 5. API (CORS abierto y bajo el token de D8)

| Ruta | Qué hace |
|---|---|
| `GET /entrevistas?proyecto=` | Lista, sin el texto largo |
| `POST /entrevistas` | Sube el audio (`multipart`: `audio`, `proyecto`, `con_quien`, `duracion_s`). Límite de cuerpo de 200 MB solo en esta ruta |
| `GET /entrevistas/{id}` | Todo, con la transcripción y el resumen |
| `PUT /entrevistas/{id}` | Editar título, con quién, transcripción y los dos resúmenes |
| `DELETE /entrevistas/{id}` | Borra la entrevista y su audio |
| `DELETE /entrevistas/{id}/audio` | Borra solo el audio (D29); 409 si la transcripción no está hecha |
| `GET /entrevistas/{id}/audio` | El audio, con `Range` |
| `POST /entrevistas/{id}/transcribir` | Arranca o reanuda la transcripción (503 sin `LLM_API_KEY`) |
| `POST /entrevistas/{id}/resumir` | Arranca los dos resúmenes (409 sin transcripción) |
| `POST /entrevistas/{id}/nota` `{cual: ejecutivo|amplio|ambos}` | Crea la nota con el resumen elegido en el mismo proyecto |

## 6. Pantallas (mock 18: `docs/mocks/018-entrevista.html`, build `PR-B1-20260923-020`)

El mock es estático y navegable: lista, detalle resumido, detalle transcribiendo, captura (con la máquina de estados simulada: cronómetro, pausa, hojas de cerrar y guardar) y el arranque directo con cuenta atrás. Probado con Chromium a 360 y 390 px sin errores de consola. Hallazgo: con tres pestañas, a 390 px «Entrevistas» se recortaba con la letra de hoy; a ≤ 420 px la letra de las pestañas baja a 0,8 rem y caben las tres (D35 confirmada, sin plan B).

### 6.1 Pestaña 🎙 Entrevistas

La lista del proyecto activo y el botón «🎙 Nueva entrevista», que abre la captura (6.2). Si hay una grabación sin subir (D28), una banda arriba: «Hay una grabación de 12:34 sin subir · Subir / Descartar». «Subir audio» como enlace pequeño bajo la lista.

### 6.2 Captura rápida (D38)

```
┌──────────────────────────────┐
│  ✕ Cerrar                    │   fila superior, aparte, 56 px de alto
│                              │
│         ● GRABANDO           │   estado en texto, colores de estado
│          12:34               │   cronómetro, ~64 px
│      ▂▄▆█▆▄▂▁                │   nivel de audio
│                              │
│ ┌──────────────────────────┐ │
│ │       ⏸  PAUSA           │ │   ámbar, 96 px de alto
│ └──────────────────────────┘ │
│ ┌──────────────────────────┐ │
│ │       ■  PARAR           │ │   rojo (antes de grabar: ● GRABAR), 128 px
│ └──────────────────────────┘ │   zona del pulgar
└──────────────────────────────┘
```

- Antes de grabar solo hay Grabar y Cerrar; Pausa aparece al empezar. Parar y Grabar comparten sitio: el botón grande siempre hace «lo siguiente».
- Botones a todo el ancho, textos de 24 px, sin iconos sueltos, `touch-action: manipulation` (sin retardo de doble toque) y `user-select: none`. Contraste de estado también por **color de fondo de toda la pantalla**: gris en reposo, rojo oscuro grabando, ámbar en pausa, para verlo de reojo sobre la mesa.
- Vibración en cada cambio (D38). Wake Lock activo (D27). La tarjeta de medios (D39) y la notificación (D40) salen al empezar a grabar y se retiran al parar.
- Al parar: hoja con «Con quién» (opcional, D34), duración y **Guardar** (sube, con barra de avance) o **Descartar** (con confirmación). Si se cierra la hoja sin guardar, la grabación queda en IndexedDB y sale la banda de 6.1.
- Permiso denegado: pantalla con el motivo y cómo activarlo (Chrome → ⓘ → Permisos → Micrófono), y «Subir audio» como salida.
- Con `?grabar=1` (D41) la app abre aquí directamente y, con el permiso ya dado, **arranca sola tras una cuenta atrás de 3 s** (3 · 2 · 1 en grande, una vibración corta por segundo) que **un toque en cualquier sitio cancela** y deja la pantalla en reposo. Así el camino normal es «pulsación larga, toque en el acceso, guardar el móvil» y un toque accidental en el acceso directo no deja una grabación fantasma. El usuario no tenía preferencia (pregunta 6); esto cubre las dos. Si el permiso no está dado, se pide y se espera el toque en Grabar.

### 6.3 Entrevista (detalle)

Reproductor (o «Audio borrado» si no está), estado, **Transcribir** (con avance «3 de 12»), transcripción con hablantes (plegada cuando hay resumen), **Resumir**, resumen ejecutivo a la vista y amplio plegado, «Guardar como nota» (Ejecutivo / Amplio / Los dos), «📋 Copiar», Editar, Mover, «Borrar audio» y Borrar.

## 7. Fases

| Fase | Qué | Se prueba |
|---|---|---|
| F18a | **Captura**: pantalla 6.2 con la máquina de estados (D42), `MediaRecorder` + IndexedDB (D28), Wake Lock, vibración, Media Session con audio silencioso (D39), notificación con botones (D40), `shortcuts` en el manifest y `?grabar=1` (D41), consentimiento (D36). **Sin backend**: al parar, de momento «Descargar .webm». | **Primero D39 en el móvil del usuario**: si el manos libres y la pantalla de bloqueo no pausan, se documenta y se sigue solo con D40. Después la guía de la sección 9. |
| F18b | **Guardar**: migración 1 → 2, tabla, `POST`/`GET`/`DELETE`/`audio`, fichero en el volumen, subida con avance y reintento, pestaña y lista (6.1), «Subir audio», reproductor. | Grabar 2 min en el móvil, cerrar la pestaña a mitad y recuperar lo grabado; `deploy.yml` sube un audio pequeño y lo baja igual. |
| F19 | **Transcribir**: `ffmpeg` en la imagen, trocear (D31), `llm.rs::transcribir`, estado y avance, reintento por trozo, editar el texto. **Primero la prueba de D30** (un trozo real por el proxy). | Una conversación de dos personas de ~10 min; `deploy.yml` transcribe 5 s de audio sintético si hay clave. |
| F20 | **Los dos resúmenes** y «Guardar como nota», copiar. | El ejecutivo cabe en una pantalla y el amplio trae acuerdos y tareas reconocibles. |
| F21 | Exportar (F11) con entrevistas, «Borrar audio» (D29), detalles de la lista. | — |
| F22 (opcional) | **App nativa mínima de Android** (Kotlin): solo graba y sube a `POST /entrevistas` con el token; da lo que la PWA no puede (D41): mosaico de ajustes rápidos, widget, grabar con la pantalla bloqueada, teclas de volumen con la app delante, servicio en primer plano que no muere. Todo lo demás sigue en la PWA. | Solo si tras usar F18 el usuario echa en falta justo eso. Necesita instalación por APK (sin Play). |

F18a se puede publicar sola en Pages (no toca `app/`) y es donde se resuelven los supuestos del móvil; F18b es el primer PR de backend. Cada fase termina con push, verificación por workflow, entrada en bitácora y resumen de sesión, como marca `CLAUDE.md`.

## 8. Preguntas al usuario (contestadas el 2026-09-23)

| # | Pregunta | Respuesta | Efecto |
|---|---|---|---|
| 1 | Duración típica | **Máximo una hora** | ~14 MB y 12 trozos por entrevista; el volumen de 1 GB va sobrado (D29) |
| 2 | ¿Dos personas? | **Sí, dos** | La transcripción pide dos hablantes por defecto (D30); si aparece un tercero, el modelo lo nombra igual |
| 3 | ¿Resumen como tercer paso? | **Dos resúmenes: primero uno ejecutivo, después otro más amplio** | D33 reescrita: dos textos, una tarea; «Guardar como nota» elige cuál |
| 4 | ¿Borrar el audio al transcribir? | **Guardar, con opción de borrar** | D29: se guarda siempre y hay «Borrar audio» por entrevista (F21) |
| 5 | ¿Auriculares Bluetooth? | **Sí, de uso opcional** | D39 se hace, pero como extra; D40 (notificación) es el control principal sin pantalla |
| 6 | ¿El acceso directo arranca solo o espera? | **No sabe cuál es más práctico** | Lo decido yo: arranca solo tras 3 s con vibración y un toque cancela (6.2). Se revisa tras la prueba 1 de la sección 9 |

## 9. Guía de pruebas en el móvil (F18a y F18b)

Se hacen con la PWA instalada en el Android del usuario y quedan anotadas en la sección 10 con el resultado.

| # | Prueba | Se espera |
|---|---|---|
| 1 | Pulsación larga en el icono → «Grabar entrevista» | Abre en la captura, sin pasar por la lista; el micrófono no vuelve a preguntar |
| 2 | Grabar 30 s, pausar, seguir, parar | Vibra en cada paso; el cronómetro se para en pausa; el audio final no tiene el hueco de la pausa |
| 3 | Grabando, apagar la pantalla con el botón, esperar 1 min, encender | Sigue grabando y el cronómetro cuadra (D27). Si no: anotar y aplicar el plan B de D27 |
| 4 | Grabando, pulsar el botón del manos libres | Pausa; otra vez, sigue (D39) |
| 5 | Grabando, pantalla bloqueada, tarjeta de medios | Pausa/Play funcionan sin desbloquear |
| 6 | Grabando, barra de notificaciones → «Parar» | Para y abre la hoja de guardar (D40) |
| 7 | Grabando, cambiar a otra app 2 min y volver | Sigue grabando o, si Chrome la cortó, lo grabado hasta ahí está en IndexedDB |
| 8 | Grabando, matar Chrome desde recientes, volver a abrir | Banda «Hay una grabación sin subir» con la duración correcta; Subir funciona |
| 9 | Guardar en modo avión | Queda «sin subir»; al volver la red, Subir funciona |
| 10 | Denegar el micrófono | Pantalla de permiso con instrucciones y «Subir audio» disponible |
| 11 | Llamada entrante durante la grabación | Anotar qué pasa; no se puede controlar desde la web |
| 12 | Grabar 45 min seguidos | Sin cortes, tamaño ~10 MB, sube entera y se reproduce |

## 10. Riesgos

- **Separación de hablantes con un modelo de chat** (D30): puede confundir voces parecidas. Por eso el plan B está en la misma función de `llm.rs` y el texto es editable.
- **Grabación en segundo plano en Android** (D27, pruebas 3 y 7): si Chrome la corta, el plan B es subir el audio de la grabadora del sistema, que ya queda hecho en F18b.
- **Media Session con audio silencioso** (D39): es un truco conocido pero no una API pensada para esto; una versión de Chrome puede dejar de mantener la sesión. D40 está para eso.
- **Expectativa del punto 6 de la sección 1** («sin abrir la aplicación»): la PWA no llega ahí (D41). Hay que decirlo claro antes de F18a para que la decisión de F22 sea del usuario, no una sorpresa.
- **Coste**: [SUPUESTO] una hora de audio con un Gemini Flash anda por céntimos. El proxy lo apunta en `/uso`; se revisa tras la primera entrevista real.
- **Privacidad**: el audio sale del backend hacia el proveedor del modelo. Va bajo la misma clave y el mismo proxy que las notas, pero es voz de un tercero; por eso D36.
- **Memoria de la máquina de Fly** (256 MB): `ffmpeg` trabaja por trozos y en disco, y la subida va en streaming al fichero, sin cargarla en memoria.

## 11. Qué se tomó de la propuesta externa

Resumen en `entrevistas/recogida.md`, bloque 2.

| Propuesta | Aquí |
|---|---|
| Pantalla «quick capture» a una mano, tres botones | Sí, D38 y 6.2 |
| Botones físicos (volumen) y grabación en segundo plano | Las teclas de volumen no existen para una web; se sustituye por Media Session, notificación y acceso directo (D39 a D41), y lo nativo queda en F22 |
| Reconocedor nativo del sistema (SpeechRecognizer / SFSpeechRecognizer) | No: la app es web y no hay Web Speech sobre ficheros. Se transcribe en el backend por el proxy (D30) |
| Resumen ejecutivo, puntos clave, turnos de palabra | Ya era D33; los turnos salen en la transcripción (D30) |
| Esquema `{audio_path, duracion_segundos, transcripcion, resumen_ia}` | El de la sección 4 lo cubre y añade estado, avance y error, que la asincronía necesita |
| Máquina de estados IDLE/RECORDING/PAUSED/PROCESSING/COMPLETED | Sí, D42, con más estados (permiso, subida, sin subir) porque el navegador los impone |
| Permisos en tiempo de ejecución | Sí, estado `sin_permiso` y pantalla de ayuda (6.2) |
| Guía de pruebas | Sí, sección 9 |
| Stack nativo (React Native, Flutter, Kotlin, Swift) | No aplica: PWA + Rust. Solo F22 sería Kotlin |

## 12. Detalle técnico (para no decidirlo a mitad de F18)

### 12.1 Grabadora en el navegador

- `getUserMedia({audio: {echoCancellation: false, noiseSuppression: true, autoGainControl: true, channelCount: 1}})`. Sin cancelación de eco: no hay altavoz sonando y recorta voz.
- `MediaRecorder(stream, {mimeType: 'audio/webm;codecs=opus', audioBitsPerSecond: 32000})`; si `isTypeSupported` falla (Safari), `audio/mp4`. `start(10000)` → un `dataavailable` cada 10 s que va a IndexedDB (D28).
- **Pausa** = `MediaRecorder.pause()`, no parar y arrancar: así el fichero sigue siendo uno y la pausa no se graba. El cronómetro se para con él.
- **Nivel**: `AudioContext` + `AnalyserNode` (`fftSize 256`, RMS del dominio temporal), 12 barras, refresco con `requestAnimationFrame` solo mientras la pantalla está visible.
- **Wake Lock**: `navigator.wakeLock.request('screen')` al grabar; se pierde al ocultar la pestaña y se vuelve a pedir en `visibilitychange`.
- **Vibración** (D38): `[80]` grabar, `[60,60,60]` pausar, `[250]` parar, `[40]` por segundo de cuenta atrás. Si `navigator.vibrate` no existe (iOS), nada.
- **IndexedDB** `notas-grabaciones`, almacén `grabaciones` con clave `id` (ulid del cliente): `{id, proyecto: {id, nombre}, empezada_en, duracion_s, estado: 'grabando'|'parada'|'subiendo', trozos: Blob[]}`. Se actualiza en cada `dataavailable` (`put` del registro entero: con ~360 trozos por hora son ~14 MB, aceptable). Al montar la app, si hay registros con `estado != 'subiendo'` → banda de 6.1. Se borra tras el 201 del `POST`.
- **Subida**: `new Blob(trozos, {type})` + `FormData` en `fetch`; `XMLHttpRequest` si se quiere avance de subida (`upload.onprogress`), que `fetch` aún no da en todos los Chrome. Reintento manual, no automático: la banda ya lo ofrece.
- **Duración**: el cronómetro del cliente (segundos grabados sin pausas) es lo que se manda en `duracion_s`; `ffmpeg` la recalcula en el servidor al trocear y gana la suya.

### 12.2 Media Session (D39) y notificación (D40)

- Audio silencioso: un `<audio loop>` con un WAV de 1 s de ceros embebido en `data:`; `play()` dentro del gesto de «Grabar» (autoplay). `navigator.mediaSession.metadata = new MediaMetadata({title: 'Grabando entrevista', artist: proyecto, artwork: [icono-192]})` y `setActionHandler` para `play` → `seguir`, `pause` → `pausar`, `stop` → `parar`. `setPositionState` cada segundo para que la tarjeta enseñe el tiempo. Al parar, `audio.pause()` y `metadata = null`.
- Notificación: `registration.showNotification('Grabando entrevista', {tag: 'grabacion', body: '12:34', silent: true, requireInteraction: true, actions: [{action: 'pausar', title: 'Pausa'}, {action: 'parar', title: 'Parar'}]})`, refrescada cada 10 s con el mismo `tag` (no vuelve a sonar). En `sw.js`, `notificationclick` → `clients.matchAll({type: 'window'})` → `client.postMessage({accion})` y `client.focus()` si es `parar`. La página escucha `navigator.serviceWorker.onmessage` y manda el evento a la máquina de estados (D42). Se cierra con `getNotifications({tag})` al parar.
- Permiso de notificaciones: se pide **después** del micrófono y solo si el usuario no lo ha denegado ya; si lo deniega, la grabación sigue igual y sin notificación.
- `sw.js` gana el `notificationclick` y el mensaje; `CACHE` cambia con el build como siempre.

### 12.3 Acceso directo (D41)

```json
"shortcuts": [{
  "name": "Grabar entrevista", "short_name": "Grabar",
  "description": "Abre la grabadora en el proyecto activo",
  "url": "./index.html?grabar=1",
  "icons": [{ "src": "icono-grabar-96.png", "sizes": "96x96" }]
}]
```

- `?grabar=1` se lee en el arranque antes de pintar la lista; si no hay proyecto activo, se graba en «Sin proyecto» (D16) sin preguntar. El parámetro se quita con `history.replaceState` para que un «recargar» no vuelva a la cuenta atrás.
- Icono nuevo `icono-grabar-96.png` (micrófono sobre el color de acento). Chrome exige que el acceso directo esté en el manifest **antes** de instalar la PWA o que se actualice el manifest (lo hace solo al abrir, en un día como mucho); si no aparece, desinstalar e instalar.
- `sw.js` sirve `index.html` para `?grabar=1` sin red (`ignoreSearch: true`, ya está así).

### 12.4 Backend (F18b)

- `axum` con `multipart` (`features = ["multipart"]`) y `DefaultBodyLimit::max(200 MB)` solo en `POST /entrevistas`. El campo `audio` se vuelca en streaming a `/data/audio/<id>.<ext>.parcial` con `tokio::fs::File` y `write_all` por trozo (`field.chunk()`), y se renombra al terminar: nunca queda un fichero a medias con nombre definitivo. Extensión por `content-type` del campo (`webm`, `mp4`/`m4a`, `mp3`, `ogg`, `wav`); otro → 415.
- `GET /entrevistas/{id}/audio`: `tower-http` `ServeFile` no vale con el token por cabecera y la ruta dinámica; se implementa `Range` a mano (`bytes=a-b`, 206, `Accept-Ranges`, `Content-Length`), que son 40 líneas. Sin `Range`, 200 entero.
- `DELETE /entrevistas/{id}`: borra fila (y `entrevista_trozos` por `ON DELETE CASCADE`) y después el fichero; si el fichero falla, se registra y no se devuelve error (la fila ya no está).
- Migración 1 → 2 en `migrar` con la tabla de la sección 4 y `entrevista_trozos(entrevista_id TEXT REFERENCES entrevistas(id) ON DELETE CASCADE, n INTEGER, texto TEXT, PRIMARY KEY(entrevista_id, n))`.
- `ffmpeg` en la imagen: `apt-get install -y --no-install-recommends ffmpeg` en la etapa de ejecución (~80 MB). Se comprueba al arrancar (`ffmpeg -version`) y `/salud` gana `"ffmpeg": bool`.
- Al arrancar, las entrevistas en `transcribiendo` vuelven a `grabada` con `trozos_hechos` intacto (la tarea murió con el proceso), para que «Reintentar» siga donde estaba.

### 12.5 Transcripción (F19)

- Trocear: `ffmpeg -i in.webm -ac 1 -ar 16000 -b:a 24k -f segment -segment_time 300 -reset_timestamps 1 /tmp/<id>/%03d.mp3`. Se cuentan los ficheros → `trozos_total`. El directorio temporal se borra al terminar, bien o mal.
- Por cada trozo `n`: base64 → `llm.rs::transcribir(TrozoAudio {mp3_b64, n, total, inicio_s}, Contexto {con_quien, proyecto, cola_anterior})`, donde `cola_anterior` son las últimas 20 líneas del trozo `n-1`. Se guarda en `entrevista_trozos` y se actualiza `trozos_hechos`. Tres intentos por trozo con espera 5/15/45 s; al tercer fallo, `estado = 'fallida'` y `error` con el motivo.
- Petición (formato OpenAI por el proxy): `messages: [{role: 'system', content: <instrucciones>}, {role: 'user', content: [{type: 'text', text: <contexto>}, {type: 'input_audio', input_audio: {data: <b64>, format: 'mp3'}}]}]`. Modelo: variable `LLM_MODELO_AUDIO` (por defecto `google/gemini-2.5-flash`). Sin salida estructurada: texto plano con el formato de abajo, que se valida por regex y, si no cuadra, se repite el trozo una vez con «Responde SOLO en el formato indicado».
- Instrucciones (resumidas): «Transcribe literalmente este fragmento (minutos X a Y de una conversación de N minutos) en su idioma original. Son dos personas: Entrevistador y Entrevistado [o el nombre de `con_quien`]. Cada intervención en una línea: `[mm:ss] Hablante: texto`, con el tiempo relativo al inicio de la conversación (empieza en X). Mantén los nombres de hablante del fragmento anterior. No resumas, no corrijas, no añadas nada. Marca lo inaudible como [inaudible].»
- La transcripción final es la concatenación por `n`, y es lo que se edita con `PUT`; los trozos no se vuelven a tocar.

### 12.6 Resúmenes (F20)

- Una tarea, dos llamadas seguidas con `LLM_MODELO_DOCUMENTO` y salida estructurada (`json_schema`) como en D9: `{titulo, ejecutivo, amplio: {participantes, temas, puntos_clave, acuerdos, tareas: [{quien, que}], frases}}`. Si la transcripción pasa de ~60 000 caracteres (raro con 1 h), se resume por mitades y se funde; con 1 h de conversación son ~9 000 palabras, cabe de sobra.
- Ejecutivo: «Diez líneas como mucho, en prosa, para alguien que no estuvo: de qué iba, qué se acordó, qué queda por hacer y de quién. Sin encabezados.»
- Amplio: el markdown de D33 lo compone el backend desde el JSON (como «Notas de origen» en D11), así el formato nunca depende del modelo. `titulo` sustituye al provisional solo si el usuario no lo ha editado (`titulo_editado` en la fila; si no, se compara con el provisional).
- «Guardar como nota»: `POST /entrevistas/{id}/nota {cual}` crea la nota con `contenido = <resumen elegido> + '\n\n_Entrevista: <titulo>, <fecha>, <duración>_'`, `proyecto_id` el de la entrevista, y pasa por `titular` como cualquier nota (D6). Devuelve la nota; la app abre su detalle.

### 12.7 Verificación en `deploy.yml`

- **F18b**: paso «Verificar /entrevistas»: genera 3 s de WAV con `ffmpeg -f lavfi -i sine=frequency=440:duration=3` en el runner, `POST` multipart, `GET` lista y detalle, `GET .../audio` con `Range: bytes=0-99` → 206 y 100 bytes, `DELETE` → 204 y 404 después. Con `TOKEN_API`, 401 sin cabecera.
- **F19**: si hay `LLM_API_KEY`, `POST .../transcribir` sobre un audio de voz sintética (`espeak-ng` en el runner, «hola, esto es una prueba») y esperar hasta 90 s a `transcrita`; el texto tiene que contener «prueba». Sin clave, 503.
- **F20**: `POST .../resumir` sobre esa transcripción y comprobar que `resumen_ejecutivo` no está vacío; `POST .../nota` crea una nota en el proyecto y se borra al final.
- `pages.yml` no cambia; el mock se archiva como siempre y `sw.js` cambia de `CACHE`.

### 12.8 Coste y tamaño (con las respuestas de la sección 8)

- Audio: 1 h ≈ 14 MB en webm/opus a 32 kbps; troceado a mp3 24 kbps ≈ 10,8 MB en total (se borra tras transcribir; solo queda el original).
- Transcripción: [SUPUESTO] un Gemini Flash cobra la entrada de audio a ~32 tokens/s → 1 h ≈ 115 000 tokens de entrada + ~12 000 de salida: **del orden de 0,05 a 0,10 € por hora**. Los dos resúmenes, ~15 000 tokens de entrada: céntimos. Se confirma en `/uso` del proxy tras la primera entrevista real.
- Volumen: 70 entrevistas de 1 h con audio; con «Borrar audio» (D29), ilimitado en la práctica (el texto de 1 h son ~60 KB).

### 12.9 Errores que la app tiene que enseñar (no solo registrar)

| Situación | Qué ve el usuario |
|---|---|
| Micrófono denegado | Pantalla de permiso con pasos y «Subir audio» |
| `MediaRecorder` no soportado | «Este navegador no puede grabar; usa la grabadora del móvil y Subir audio» |
| Sin red al guardar | Queda en IndexedDB; banda «sin subir» y botón Subir |
| Subida cortada | Igual que sin red; el `.parcial` del servidor se borra en el siguiente arranque |
| 413 (más de 200 MB) | «Grabación demasiado larga para subirla de una vez» (no debería pasar con 1 h: 14 MB) |
| Transcripción fallida | Estado «Fallida · Reintentar» con el motivo en el detalle |
| 503 sin clave de IA | «Transcribir necesita la clave de IA configurada» y el botón deshabilitado |
| Audio borrado | Reproductor sustituido por «Audio borrado el <fecha>»; Transcribir deshabilitado si no había transcripción |

## 13. Cómo quedó (2026-09-23, tarde)

El usuario aprobó el mock 18 y pidió desarrollarlo entero. F18a, F18b, F19, F20 y F21 salieron en una sola entrega; F22 (app nativa) sigue sin hacer.

**Cambios respecto al plan:**

- **Resúmenes en una sola llamada** (12.6 decía dos): el esquema JSON trae `titulo`, `ejecutivo` y `amplio`; sale más barato y los dos resúmenes no se contradicen.
- **Reintentos por trozo**: tres intentos con esperas de 5 y 15 s (el plan decía 5/15/45). Un cuarto intento idéntico dentro del minuto se acercaría al corte por bucle del proxy (5 repeticiones en 60 s).
- **Mover una entrevista** va por `POST /entrevistas/{id}/mover`: el `PUT` no distingue entre «sin proyecto» y «no tocar».
- **Título provisional** lo manda la app en hora local («Entrevista · 23 sept, 14:05»); el servidor solo sabe UTC.
- **La tarjeta «Sin proyecto»** sale también si solo hay entrevistas sueltas: sin ella, una entrevista movida a «Sin proyecto» era inalcanzable.
- **El aviso de grabaciones sin subir** cambia el icono de la pestaña (🎙 → ⚠️) en vez de alargar el texto, que a 390 px se recortaba.
- **Vibración de la cuenta atrás**: Chrome no deja vibrar antes del primer toque en la página, y al abrir por el acceso directo no lo hay. La cuenta atrás se ve y el arranque sí vibra si ya se ha tocado. Es una limitación del navegador, no del código.
- **Duración**: la mide `ffprobe` al transcribir si el fichero la trae; la grabación del navegador (webm) no la trae, y entonces vale la que manda la app.

**Probado en local** (backend real, `ffmpeg` real, modelo simulado que comprueba lo que recibe):

- API: 11 min de audio → 3 trozos de 1,2 MB por petición (por debajo de 2 MB), marcas de tiempo desplazadas por trozo (`[05:04]`, `[10:04]`…), `Range` 206/416, 409 al borrar el audio sin transcribir o al resumir sin transcripción, 415 a un PDF, 400 a un audio vacío o a un proyecto que no existe (sin ficheros huérfanos), título editado que el resumen no pisa, borrado en cascada de los trozos.
- Reanudación: con el trozo 2 fallando siempre, la entrevista queda «fallida» con 1 de 3 hechos; «Reintentar» pide solo el 2 y el 3.
- App en Chromium con micrófono simulado, a 390 y 360 px, 38 comprobaciones sin errores de consola: consentimiento, grabar, pausa desde la notificación (el cronómetro se para), seguir, cerrar grabando, parar, guardar, transcribir, resumir, título propuesto, guardar como nota, recarga a mitad de grabación → ⚠️ y banda «sin subir» → subir, acceso directo con cuenta atrás que arranca sola y que un toque cancela, descartar, subir un fichero, escuchar, editar, mover, borrar audio, exportación con entrevistas y borrar. El editor de notas se comprobó aparte, porque los ids del detalle chocaban con los suyos (corregido: el detalle usa `ent*`).
- `deploy.yml` gana «Verificar /entrevistas», probado primero en local con el modelo simulado y después en Fly contra el modelo real (run `35858580052` sobre la rama y `35859052561` sobre `main`): la voz de `espeak-ng` salió transcrita palabra por palabra en unos 3 s, con su resumen y su nota; sin clave exige el 503.
- **D30 confirmado**: el proxy deja pasar `input_audio` y `google/gemini-2.5-flash` transcribe en español con el formato pedido. Queda por ver, con una conversación real, qué tal separa dos voces.

**Queda para el móvil del usuario** (sección 9): pantalla apagada y app en segundo plano (D27), botón de los auriculares y tarjeta de la pantalla de bloqueo (D39), botones de la notificación de verdad (D40) y el acceso directo del icono (D41; si no aparece, desinstalar e instalar la app para que Chrome relea el manifest).
