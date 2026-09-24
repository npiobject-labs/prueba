# Plan: cabecera de una sola línea con menú

Fecha: 2026-09-24 · Estado: v2, **F27 hecha** (mock de diseño: `docs/mocks/021-menu.html`, build `PR-B1-20260924-029`; app: build `PR-B1-20260924-031`) · Continúa `plan-analisis.md` (hasta D67) · Fuente de verdad: este fichero.

## 1. Qué se pide (en palabras del usuario)

> Reorganiza el menú de la pantalla inicial del proyecto: todo lo que está en la primera barra, más la segunda de las tres opciones (proyectos, notas, entrevistas), más el buscador, pasen a formar parte de un menú, y en la parte visible, en la primera línea, solamente queden el sol/la luna, un icono de buscador y el propio menú que se despliega con sus opciones. Se trata de generar espacio al principio de la pantalla, dejando una sola línea arriba y llevando todo lo demás al menú. Una vez elegida una opción, el menú se contrae y no ocupa pantalla, y al principio de la información tiene que venir primero el título de lo que se ha elegido (proyectos, notas…) y luego información adicional como el número de notas o de entrevistas. Hazme un análisis crítico y analítico.

> (Tras el análisis) 1: no lo sé, elige el más usable y claro. 2: arriba. 3: primero mock.

Respuestas: (1) el título va **en la misma línea** que el menú, porque es lo más claro (D69); (2) el tema se queda **arriba**, en la barra; (3) **primero este mock**.

## 2. Medida de partida

Medido con Chromium a 412×780 en la app de hoy (build `PR-B1-20260924-028`), dentro de un proyecto:

| Zona | Alto | Fija al hacer scroll |
|---|---|---|
| Fila 1: título e iconos | ≈ 50 px | sí |
| Pestañas Proyectos · Notas · Entrevistas | 38 px | sí |
| Buscador y fila de Etiquetas / En todos | 76 px | sí |
| **Cabecera fija** | **185 px (24 %)** | sí |
| «Editar proyecto» y «＋ Subcarpeta» | ≈ 108 px | no |
| **Primera nota** | empieza en el **px 293** (38 %); caben 3 | — |

En el mock 21: cabecera fija de **56 px (7 %)**; la primera nota empieza en el px 110–170 según haya descripción y subcarpetas, con una o dos notas más a la vista. Cambiar de sección pasa de 1 toque a 2; buscar sigue en 1 (la lupa pone el cursor en el campo).

## 3. Decisiones y supuestos

| # | Decisión | Estado |
|---|---|---|
| D68 | **Una sola barra fija de 56 px**: ☰ menú · título · 🔍 · tema (🌗/☀️/🌙, como hoy). Desaparecen de la cabecera las pestañas, el buscador, la fila de etiquetas y los iconos ☑ 📄 ⚙. | Decidido (respuesta 2: el tema, arriba). |
| D69 | **El título dice siempre dónde estás**, en dos pisos dentro de la barra: arriba la sección en pequeño y en color (PROYECTOS, NOTAS, ENTREVISTAS, DOCUMENTOS, AJUSTES) y abajo el proyecto o «Todas las notas». En una subcarpeta, «Padre › Subcarpeta», y tocar el padre vuelve a él (sustituye a `#migas`). | Decidido (respuesta 1: lo más claro). |
| D70 | **Menú desplegable bajo la barra**, con velo, en tres bloques: *Ir a* (Proyectos, Notas del proyecto, Entrevistas, con cifras y la sección actual marcada), *Este proyecto* (Seleccionar notas, Documentos, Editar proyecto, Nueva subcarpeta; solo con un proyecto activo) y *General* (Ajustes). El icono ☰ se convierte en ✕ mientras está abierto. Se cierra al elegir, al tocar fuera, con Escape y con **Atrás** (abrirlo añade una entrada al historial, así que Atrás cierra el menú en vez de salir de la app). | [SUPUESTO] Mejor desplegable desde arriba que cajón lateral: sale donde está el dedo y no tapa el contenido entero. Plan B: cajón lateral con el mismo contenido. |
| D71 | **La lupa es de la pantalla**: en Notas abre texto, día, etiquetas y «En todos los proyectos»; en Proyectos, texto y orden. En Entrevistas, Documentos y Ajustes no hay búsqueda y la lupa no sale. Al abrirla el cursor ya está en el campo. | Decidido. |
| D72 | **Filtro puesto a la vista**: con el buscador cerrado, una tira bajo la barra lo resume («🔍 "reserva" · #app · 3 de 15 ✕») y la lupa queda en color. Tocar la tira reabre el buscador; ✕ lo quita todo. La lista nunca sale recortada sin explicación (lo aprendido en F10). | Decidido. |
| D73 | **Primera fila del contenido: cifras de lo que se ve**, en pequeño, y se va con el scroll. Notas de un proyecto: notas · entrevistas · subcarpetas · última actividad, y la descripción debajo. Todas las notas: notas y proyectos. Proyectos: proyectos · subcarpetas · archivados · notas. Entrevistas: entrevistas · minutos grabados · sin transcribir. Documentos: documentos · informes · encargos. | Decidido. |
| D74 | **La tira de subcarpetas solo sale si el proyecto tiene alguna**; «＋ Subcarpeta» y «Editar proyecto» pasan al menú (*Este proyecto*). | Decidido. |
| D75 | **Seleccionar**: la barra cambia a ✕ (salir) + «ELEGIR NOTAS / 3 de 15», sin lupa; la barra de abajo (Todas/Ninguna, Mover, Cancelar, Generar) sigue como hoy, y la pulsación larga sigue entrando en la selección. | Decidido. |
| D76 | **Cambiar de sección cuesta un toque más** (menú + opción). | [SUPUESTO] Se compensa con el espacio y con que el título dice siempre dónde se está. Plan B: deslizar a los lados para pasar de sección, o volver a una barra de tres pestañas abajo si se echa de menos. |

## 4. Qué se ve en el mock

`docs/mocks/021-menu.html`, navegable con datos de ejemplo; la banda de arriba tiene «Qué cambia» con la comparación de alturas.

1. **Notas de un proyecto** con subcarpetas: barra, cifras, descripción y tira de subcarpetas.
2. **Menú abierto**: los tres bloques con cifras.
3. **Buscador abierto** y **filtro puesto** con su tira resumen.
4. **Subcarpeta** con «Padre › Subcarpeta» en el título.
5. **Proyectos**, **Entrevistas** y **Todas las notas**, cada una con su título y sus cifras.
6. **Selección** con la barra cambiada.

Atajos: `?v=menu`, `buscar`, `filtro`, `sub`, `proyectos`, `entrevistas`, `elegir`, `todas`.

## 5. Fases

| Fase | Entregable |
|---|---|
| F27 | App (sin backend): barra única, menú, lupa contextual con tira de filtro, cifras en la primera fila, tira de subcarpetas condicional y selección con la barra cambiada. Se hace después de F26 (informes en la app). Pruebas con Chromium a 390 y 412 px, incluido Atrás con el menú abierto. |

## 6. Preguntas al usuario (contestadas: desplegable desde arriba, y sin lupa en Entrevistas)

1. ¿Menú **desplegable desde arriba** (el del mock) o **cajón lateral** (D70)?
2. ¿Te vale que en Entrevistas no haya lupa (D71)?

## 7. Cómo quedó (2026-09-24)

- **Los botones de siempre, dentro del menú**: las pestañas y los iconos ☑ 📄 ⚙ y «Editar proyecto» pasaron al menú con sus mismos ids (`#tabProy`, `#tabNotas`, `#tabEnt`, `#seleccionarBtn`, `#documentosBtn`, `#ajustesBtn`, `#cpEditar`), así que sus manejadores no cambiaron; el buscador de proyectos (`#pq`, `#porden`) se movió a la lupa. Desapareció el 📁 que llevaba oculto desde F13.
- **Cifras sin parpadeo**: la fila de cifras de la lista solo se pinta cuando la lista cargada es la del ámbito actual; al volver de una nota a la misma lista no desaparece y reaparece.
- **Seleccionar desde otra pantalla** (por ejemplo, desde Entrevistas) va primero a la lista del proyecto y entra en la selección cuando llega.
- **Aviso de grabación sin subir**: como Entrevistas ya no está a la vista, el botón ☰ lleva un punto rojo además del ⚠️ de dentro del menú.
- **Medida**: con la barra de 56 px, la lista de un proyecto con descripción y subcarpetas empieza hacia el px 190 (antes, el px 293 sin descripción) y la cabecera fija ocupa el 7 % de la pantalla en lugar del 24 %.
- **Pruebas**: 94 comprobaciones con Chromium a 390 y 412 px contra el backend local (barra, cifras, menú con Atrás, Escape y tocar fuera, lupa y tira en Notas y Proyectos, «en todos los proyectos», subcarpeta con migas y «Nueva subcarpeta en…», Entrevistas sin lupa, selección desde otra pantalla con ✕, Documentos, Ajustes, tema, sin proyecto elegido), más las regresiones de los informes (38) y de los plegables y el botón ＋. Sin errores de JS ni desbordes.

