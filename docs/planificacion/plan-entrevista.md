# Plan — sección Entrevista

Fecha: 2026-09-23 · Estado: **v2, solo planificación**; F18 a F21 sin hacer, F22 opcional · Continúa `plan-proyectos.md` (D14 a D26, F13 a F17) · Fuente de verdad: este fichero (la copia en Drive es solo copia).

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

**Tercer paso elegido: resumen estructurado de la entrevista**, generado desde la transcripción, con Participantes, Temas tratados, Puntos clave, Acuerdos y decisiones, Tareas pendientes (quién y qué) y Frases literales relevantes. Un botón «Guardar como nota» lo pasa al proyecto. Es lo que da valor a una conversación grabada: nadie relee una transcripción de una hora, pero todo el mundo necesita saber qué se acordó y qué queda por hacer.

## 3. Decisiones y supuestos

Numeración continua (última: D26). D38 a D42 son nuevas en v2.

| # | Decisión | Estado |
|---|---|---|
| D27 | **Grabar en el navegador con `MediaRecorder`** (`audio/webm;codecs=opus`, mono, unos 32 kbps: ~14 MB por hora). Cronómetro y medidor de nivel. **Wake Lock** mientras graba, para que la pantalla no se apague. La interfaz es la de D38. | [SUPUESTO] Chrome en Android sigue grabando con la pantalla encendida y la pestaña delante. Plan B si corta en segundo plano: aviso fijo «No salgas de la app mientras grabas» y, si no basta, la grabadora del sistema más «Subir audio» (D28). |
| D28 | **No perder nunca una grabación.** El audio se va guardando en IndexedDB a trozos de 10 s mientras se graba. Si la subida falla o se cierra la pestaña, al volver aparece «Hay una grabación sin subir · Subir / Descartar». Además, **«Subir audio»** acepta un fichero ya grabado (m4a, mp3, ogg, webm, wav). | Decidido. Es la misma filosofía que D6/D12: lo del usuario no se pierde. |
| D29 | **El audio se guarda en el volumen de Fly** (`/data/audio/<id>.<ext>`). La base guarda metadatos y texto, no binarios. `GET /entrevistas/{id}/audio` lo sirve con `Range` para el reproductor; la app lo pide con `fetch` y el token (un `<audio src>` no manda cabeceras) y lo reproduce desde un blob. | [SUPUESTO] El volumen de 1 GB llega (~70 h de audio). Plan B: `flyctl volumes extend` o borrar el audio tras transcribir, dejando el texto (opción por entrevista). |
| D30 | **Transcribir con el mismo proveedor de siempre** (D5): un modelo con entrada de audio por el proxy `openrouter`, mensajes `input_audio` y un modelo con audio (por defecto un Gemini Flash). **Solo `llm.rs` lo sabe**, con una función nueva `transcribir(trozo, contexto)`. Se pide la transcripción **con hablantes** («Entrevistador:», «Entrevistado:» o nombres si se dicen), en el idioma original y sin resumir. | [SUPUESTO, lo primero que se prueba en F19] El proxy deja pasar `input_audio` y el modelo separa bien dos voces. Plan B: un servicio de transcripción con separación de hablantes de verdad (Deepgram o AssemblyAI) con su propia clave, `STT_API_KEY`, detrás de la misma función de `llm.rs`. |
| D31 | **Trocear antes de transcribir.** El backend convierte el audio con `ffmpeg` a mp3 mono de 24 kbps en **trozos de 5 min** (~0,9 MB, ~1,2 MB en base64). Así cada petición cabe en el límite de 2 MB del proxy (axum por defecto), un fallo solo repite un trozo, y la salida del modelo nunca pasa de su tope. Cada trozo lleva el final del anterior como contexto, para no cambiar de nombre a los hablantes a mitad. | Decidido. `ffmpeg` entra en la imagen de Docker (unos 80 MB más). |
| D32 | **Transcribir es asíncrono**, como los documentos (D11): `POST /entrevistas/{id}/transcribir` contesta al momento, la entrevista pasa por `pendiente → transcribiendo (3/12) → lista | fallida`, y la app pregunta cada 3 s y enseña el avance. Si falla, «Reintentar» sigue desde el trozo que falló. La transcripción se puede **editar** (como las notas, `PUT`). | Decidido. |
| D33 | **Tercer paso: resumen** (sección 2). `POST /entrevistas/{id}/resumir`, asíncrono, con `LLM_MODELO_DOCUMENTO`, estructura fija en markdown y el proyecto como contexto (D21). «Guardar como nota» crea una nota en el proyecto con el resumen y un enlace a la entrevista; la IA la titula y etiqueta como a cualquier otra. «📋 Copiar» copia resumen y transcripción. | Decidido. |
| D34 | **Título** propuesto por la IA con el resumen, editable siempre. Hasta entonces, «Entrevista · 23 sept 12:40». Campo opcional «Con quién» al grabar, que también se pasa al modelo para nombrar a los hablantes. Con la captura rápida (D38) el campo no estorba: se pide **después** de parar, no antes de grabar. | Decidido. |
| D35 | **Navegación**: tercera pestaña **🎙 Entrevistas** junto a Proyectos y Notas (dentro del proyecto activo). En la lista, cada entrevista enseña duración, estado y fecha. | [SUPUESTO] Tres pestañas caben a 360 px. Plan B: la entrevista entra como un tipo de elemento más en la lista de notas, con un icono. |
| D36 | **Consentimiento**: antes de la primera grabación, aviso de que grabar a otra persona requiere su permiso. Se enseña una vez y se recuerda en `localStorage`. | Decidido. Solo un aviso; la app no guarda ninguna prueba del consentimiento. |
| D37 | **Fuera de esta versión**: transcripción en directo mientras se habla, más de un idioma en la misma entrevista, preguntar a la entrevista («¿qué dijo de los plazos?») y documentos (F5) a partir de entrevistas. La pregunta a la entrevista es la siguiente candidata (F22). | Decidido. |
| D38 | **Pantalla de captura rápida** (sección 6.2): una vista a pantalla completa que tapa cabecera, pestañas y buscador, con **solo tres botones**: **Grabar** (rojo, el más grande, en la zona del pulgar), **Pausa / Seguir** (ámbar) y **Cerrar** (gris, arriba y aparte, para no pulsarlo sin querer). Cronómetro grande y nivel de audio. Nada más: ni título, ni «con quién», ni ajustes. Cada cambio de estado **vibra** con un patrón distinto (`navigator.vibrate`: 1 pulso al grabar, 2 al pausar, 1 largo al parar), para saber qué ha pasado sin mirar. Cerrar mientras graba pregunta «Guardar / Seguir grabando / Descartar». | Decidido. Es la lectura literal del punto 5 de la sección 1. |
| D39 | **Control sin tocar la pantalla, vía 1: auriculares y pantalla de bloqueo con Media Session.** Las teclas de volumen **no llegan a una página web** en Android: no hay forma de capturarlas desde una PWA. Lo que sí llega es el botón de **auriculares o manos libres Bluetooth** (play/pause) a través de `navigator.mediaSession`, siempre que la página sea la sesión de medios activa. Para serlo, mientras se graba se reproduce en bucle un **audio silencioso**; con eso aparece además la **tarjeta de medios del sistema** (barra de notificaciones y pantalla de bloqueo) con Play/Pausa, que mapeamos a **Seguir/Pausa** de la grabación. El título de la tarjeta enseña «Grabando entrevista · 12:34». | [SUPUESTO, **lo primero que se prueba en F18a**, en el móvil del usuario] Chrome en Android mantiene la sesión de medios con el audio silencioso y entrega `play`/`pause` desde el manos libres y la pantalla de bloqueo. Plan B (D40). |
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
  resumen       TEXT NOT NULL DEFAULT '',
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
| `PUT /entrevistas/{id}` | Editar título, con quién, transcripción y resumen |
| `DELETE /entrevistas/{id}` | Borra la entrevista y su audio |
| `GET /entrevistas/{id}/audio` | El audio, con `Range` |
| `POST /entrevistas/{id}/transcribir` | Arranca o reanuda la transcripción (503 sin `LLM_API_KEY`) |
| `POST /entrevistas/{id}/resumir` | Arranca el resumen (409 sin transcripción) |
| `POST /entrevistas/{id}/nota` | Crea la nota con el resumen en el mismo proyecto |

## 6. Pantallas (mock 18 en adelante)

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
- Con `?grabar=1` (D41) la app abre aquí directamente, pide el micrófono y, si ya lo tenía, **espera el toque en Grabar**; no arranca sola, para no grabar por un toque accidental en el acceso directo. [SUPUESTO] Es lo que quiere el usuario. Plan B: `?grabar=ya` que arranca solo, como segundo acceso directo.

### 6.3 Entrevista (detalle)

Reproductor, estado, **Transcribir** (con avance «3 de 12»), transcripción con hablantes, **Resumir**, resumen, «Guardar como nota», «📋 Copiar», Editar, Mover y Borrar.

## 7. Fases

| Fase | Qué | Se prueba |
|---|---|---|
| F18a | **Captura**: pantalla 6.2 con la máquina de estados (D42), `MediaRecorder` + IndexedDB (D28), Wake Lock, vibración, Media Session con audio silencioso (D39), notificación con botones (D40), `shortcuts` en el manifest y `?grabar=1` (D41), consentimiento (D36). **Sin backend**: al parar, de momento «Descargar .webm». | **Primero D39 en el móvil del usuario**: si el manos libres y la pantalla de bloqueo no pausan, se documenta y se sigue solo con D40. Después la guía de la sección 9. |
| F18b | **Guardar**: migración 1 → 2, tabla, `POST`/`GET`/`DELETE`/`audio`, fichero en el volumen, subida con avance y reintento, pestaña y lista (6.1), «Subir audio», reproductor. | Grabar 2 min en el móvil, cerrar la pestaña a mitad y recuperar lo grabado; `deploy.yml` sube un audio pequeño y lo baja igual. |
| F19 | **Transcribir**: `ffmpeg` en la imagen, trocear (D31), `llm.rs::transcribir`, estado y avance, reintento por trozo, editar el texto. **Primero la prueba de D30** (un trozo real por el proxy). | Una conversación de dos personas de ~10 min; `deploy.yml` transcribe 5 s de audio sintético si hay clave. |
| F20 | **Resumen** y «Guardar como nota», copiar. | El resumen de F19 trae acuerdos y tareas reconocibles. |
| F21 | Exportar (F11) con entrevistas, detalles de la lista, borrar audio tras transcribir si se decide (D29). | — |
| F22 (opcional) | **App nativa mínima de Android** (Kotlin): solo graba y sube a `POST /entrevistas` con el token; da lo que la PWA no puede (D41): mosaico de ajustes rápidos, widget, grabar con la pantalla bloqueada, teclas de volumen con la app delante, servicio en primer plano que no muere. Todo lo demás sigue en la PWA. | Solo si tras usar F18 el usuario echa en falta justo eso. Necesita instalación por APK (sin Play). |

F18a se puede publicar sola en Pages (no toca `app/`) y es donde se resuelven los supuestos del móvil; F18b es el primer PR de backend. Cada fase termina con push, verificación por workflow, entrada en bitácora y resumen de sesión, como marca `CLAUDE.md`.

## 8. Preguntas al usuario (antes de F18a)

Todas con respuesta supuesta; con silencio se sigue con ella.

1. ¿Cuánto dura una entrevista típica? (¿15 min, 1 h, más?) Supuesto: **hasta 1 h**. Cambia el tamaño del volumen y el coste.
2. ¿Casi siempre **dos personas**, o también reuniones de varias? Supuesto: dos.
3. ¿Te vale el **resumen** como tercer paso, o preferías otro (solo tareas, acta formal)? Supuesto: resumen (D33).
4. ¿Borrar el audio al transcribir para ahorrar espacio, o guardarlo siempre? Supuesto: guardarlo, y se revisa en F21.
5. ¿Tienes **auriculares o manos libres Bluetooth** con botón? Si no, D39 aporta solo la tarjeta de la pantalla de bloqueo y D40 pasa a ser lo principal.
6. ¿El acceso directo debe **empezar a grabar solo** o esperar el toque en Grabar? Supuesto: esperar (6.2).

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
