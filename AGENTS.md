# Manifiesto de Arquitectura: Modularidad Cohesiva y Componentes Reutilizables

Este documento establece la filosofía, los estándares y los patrones para construir aplicaciones web, móviles y de escritorio con **módulos cohesivos, límites explícitos y piezas componibles**. Está orientado al trabajo de agentes de IA, sin sustituir las especificaciones del producto ni las instrucciones del proyecto.

**Objetivo:** que una funcionalidad pueda comprenderse, modificarse y verificarse leyendo principalmente su propio módulo y los contratos que consume. La modularidad es obligatoria; la atomización extrema no.

> **Regla central:** mantener juntas las piezas que cambian por la misma razón y separar las que tienen responsabilidades, dependencias o ciclos de vida diferentes.

---

## 1. Filosofía Central: Modularidad y Componibilidad con Propósito

1. **Cohesión antes que fragmentación.** Cada módulo representa una capacidad o responsabilidad reconocible. Un archivo puede contener varias funciones estrechamente relacionadas; no debe acumular responsabilidades ajenas.
2. **Límites explícitos y bajo acoplamiento.** Los módulos colaboran mediante contratos públicos pequeños. No acceden a detalles internos de otros módulos ni forman dependencias circulares.
3. **Composición sobre duplicación de responsabilidades.** Reutilizar piezas existentes que representen el mismo concepto. Extraer nuevas piezas cuando aporten una frontera útil, no únicamente porque sea posible hacerlo.
4. **UI presentacional desacoplada.** Los componentes visuales reutilizables reciben datos y configuración, y comunican interacciones. La integración con red, almacenamiento o IPC pertenece a la capa de integración de la funcionalidad.
5. **Comportamientos headless.** Separar interacciones complejas o reutilizables de su presentación: posicionamiento, arrastre, atajos, observadores y gestión de foco. Mantener locales los manejadores triviales.
6. **Dependencias hacia contratos y dominio.** La lógica de negocio no importa implementaciones concretas de base de datos, transporte ni interfaz gráfica. Las implementaciones externas se conectan en un punto explícito de composición.
7. **Reutilización deliberada, no universal.** Una regla de negocio puede ser específica de su dominio. No convertirla en una abstracción genérica para consumidores hipotéticos.
8. **Verificación por límites.** Cada comportamiento modificado debe poder verificarse; los cambios de contratos o dependencias compartidas requieren comprobar también a sus consumidores.

### 1.1 Alcance y aplicación

- En proyectos nuevos, preferir **monolito modular organizado por funcionalidad**. No crear microservicios, paquetes independientes ni comunicación distribuida solamente para conseguir separación lógica.
- En proyectos existentes, respetar las convenciones compatibles con estos principios. No reorganizar todo el repositorio como efecto secundario de una tarea localizada.
- Los árboles, nombres y patrones de este documento son ejemplos, no una plantilla que deba generarse completa. Adaptarlos al lenguaje, framework y tamaño real del proyecto.
- No imponer JavaScript, Rust, un sistema de estilos, una biblioteca de estado ni un framework propio. Aprovechar las capacidades del stack elegido.
- Aplicar estas reglas dentro de las instrucciones vigentes de la herramienta y del repositorio. Resolver contradicciones relevantes antes de ampliar el alcance; no inventar una jerarquía alternativa de instrucciones.

### 1.2 Dirección de dependencias

En estos esquemas, `A → B` significa **A depende de B**, no el orden de ejecución:

```text
UI / Integración de la funcionalidad → API o casos de uso de la funcionalidad
UI / Integración de la funcionalidad → Componentes presentacionales → Primitivas y tokens
Componentes presentacionales         → Comportamientos headless, cuando corresponda

Adaptadores de entrada → Casos de uso → Dominio
Casos de uso          → Puertos que necesitan
Adaptadores de salida → Puertos que implementan
Punto de composición  → Casos de uso + adaptadores concretos
```

**Ni el dominio ni los casos de uso dependen de los adaptadores concretos.** El orden de llamadas en ejecución no cambia esta regla. No crear capas intermedias vacías para reproducir el esquema.

---

## 2. Nivel de Modularidad: Criterios del Estándar

| Dimensión | Regla aplicable |
| :--- | :--- |
| **Organización** | Primero por capacidad o funcionalidad; dentro de ella, separar responsabilidades cuando lo justifique su complejidad. |
| **Utilidades** | Agrupar funciones cohesivas por concepto. Un archivo por función es válido, pero no obligatorio. |
| **Componentes UI** | Separar presentación, integración y comportamientos complejos sin exigir una cadena completa de componentes. |
| **Casos de uso** | Hacer reconocible cada acción de negocio. Separar archivos cuando tengan lógica, dependencias o pruebas propias. |
| **Transformaciones** | Usar pipelines cuando existan etapas reales. Mantener funciones directas para secuencias sencillas. |
| **Estilos** | Tokens compartidos y estilos con ámbito controlado; sin correspondencia obligatoria de un CSS por archivo de código. |
| **Estado** | Propietario explícito por recurso o funcionalidad, una fuente de verdad y actualizaciones coherentes. |
| **Reutilización** | Compartir conceptos estables con consumidores reales o una necesidad actual documentada. |
| **Tamaño** | Evaluar responsabilidades y contexto necesario; no imponer límites universales de líneas, funciones o archivos. |

---

## 3. Desglose de la Arquitectura Modular

### Organización principal: funcionalidades y contratos

```text
src/
├── app/                     # Arranque, composición, rutas y configuración global
├── modules/
│   ├── catalog/
│   │   ├── public.*          # Contrato público, o equivalente idiomático del lenguaje
│   │   ├── domain/           # Reglas y tipos propios del catálogo
│   │   ├── application/      # Casos de uso y puertos necesarios
│   │   ├── adapters/         # Persistencia, transporte e integraciones
│   │   ├── ui/               # Vistas, componentes y estado de esta funcionalidad
│   │   └── tests/            # O pruebas junto al código, según el proyecto
│   └── media/
│       └── ...
├── shared/                  # Solo conceptos genuinamente compartidos
│   ├── ui/                  # Primitivas visuales y comportamientos reutilizables
│   └── primitives/          # Tipos y utilidades independientes del negocio
└── infrastructure/          # Recursos técnicos compartidos cuando sean necesarios
```

**No crear todas estas carpetas por adelantado.** En una funcionalidad pequeña pueden bastar `domain.*`, `use-cases.*`, un adaptador y sus pruebas. Separar más cuando aparezcan responsabilidades distintas. Si frontend y backend viven en árboles distintos, aplicar los mismos límites dentro de cada uno.

#### Reglas entre módulos

- Consumir únicamente la superficie pública de otro módulo, mediante los mecanismos del lenguaje. No exportar todos sus archivos para aparentar una API pública.
- Cada módulo es propietario de sus reglas, operaciones y estado. No modificar directamente tablas, almacenes o estructuras internas de otro módulo.
- Una base de datos compartida no elimina la propiedad de los datos. Resolver consultas transversales mediante contratos o modelos de lectura explícitos, no acceso incidental a detalles internos.
- No mover código a `shared/` para evitar decidir a quién pertenece. `shared/` no depende de módulos de negocio y no debe convertirse en un contenedor genérico.
- Ante un ciclo, revisar responsabilidades, dirección de dependencia o un coordinador explícito. No ocultarlo con eventos globales o importaciones diferidas.
- Los cambios de contratos incluyen sus consumidores y pruebas. Una implementación local puede cambiar sin alterar su interfaz pública.

### 3.1 Frontend: Sistema de Diseño Atómico y UI por Funcionalidad

Conservar el modelo **Átomos → Moléculas → Organismos → Layouts / Vistas** como vocabulario de composición, no como una obligación de crear cinco carpetas o cinco niveles para cada pantalla.

| Pieza | Responsabilidad y ubicación |
| :--- | :--- |
| **Átomos** | Botón, icono, input, badge, spinner. Primitivas visuales sin reglas de negocio. |
| **Moléculas** | Buscador, selector o menú que reúnen primitivas bajo una responsabilidad concreta. |
| **Organismos** | Secciones completas. Permanecen dentro de su funcionalidad cuando conocen conceptos de negocio. |
| **Layouts** | Distribución espacial y estructura, sin lógica de negocio innecesaria. |
| **Vistas / integración** | Coordinación de datos, navegación y acciones mediante las interfaces de la funcionalidad. |

#### Reglas de los componentes

- Los componentes presentacionales reutilizables no acceden directamente a APIs, bases de datos, IPC ni almacenes globales de negocio. Reciben datos y emiten callbacks o eventos definidos.
- Las vistas pueden coordinar carga de datos mediante los mecanismos del framework y los adaptadores del módulo. No deben implementar protocolos de transporte ni concentrar las reglas de negocio.
- Un componente específico como `CatalogItemCard` pertenece a `catalog/ui/` hasta que exista una razón real para compartirlo. No necesita ser útil en cualquier aplicación.
- Extraer subcomponentes cuando tengan responsabilidad, comportamiento, reutilización o complejidad propios. No crear componentes que solo reenvíen propiedades sin aportar semántica ni comportamiento.
- Encapsular el ciclo de vida: retirar listeners, observadores, suscripciones y recursos al desmontar o destruir la pieza. Usar las convenciones del framework; en UI imperativa puede exponerse `{ el, update, destroy }`.
- Mantener semántica accesible, etiquetas, navegación por teclado y gestión de foco en los componentes que lo requieren. Los estados de carga, vacío y error forman parte de la funcionalidad.

### 3.2 Frontend: Comportamientos Headless

Ejemplos: `useFloatingPosition`, `useClickOutside`, `useKeyboardShortcuts`, `useIntersection`, `useDragAndDrop` y `useLongPress`.

- Separar el cálculo puro de los efectos sobre DOM, eventos y suscripciones. No exigir pureza a una pieza encargada precisamente de gestionar esos efectos.
- Recibir referencias, configuración y dependencias necesarias. Evitar buscar nodos de la aplicación por identificadores globales.
- Definir activación, actualización y limpieza. No dejar listeners, timers ni observadores activos después de su ciclo de vida.
- Mantener el comportamiento local al módulo hasta que haya una razón para compartirlo.
- Reutilizar implementaciones existentes adecuadas antes de construir motores propios de posicionamiento, arrastre o accesibilidad.

Un callback sencillo no necesita un módulo headless. Un comportamiento extraído sí debe tener un contrato entendible y verificable.

### 3.3 Utilidades Cohesivas y Efectos Explícitos

Evitar `utils.*` o `helpers.*` como contenedores de funciones no relacionadas. Usar nombres que expliquen el concepto:

```text
format/
├── bytes.*       # Formato de tamaños y operaciones directamente relacionadas
├── duration.*    # Formato y tratamiento de duraciones
└── dates.*       # Funciones cohesivas de presentación de fechas
```

- Una función pequeña usada solo dentro de un archivo puede permanecer privada en él.
- Separar una utilidad cuando tenga uso independiente, complejidad propia o un límite claro de prueba. No extraer únicamente por cantidad de líneas.
- Mantener puras las transformaciones que no requieren efectos. Pasar dependencias variables, como la hora actual, cuando deban controlarse para verificar el resultado.
- Identificar explícitamente adaptadores de DOM, almacenamiento, temporizadores o red; no tratarlos como utilidades puras.
- No unificar dos reglas de negocio distintas porque hoy su código sea parecido. Compartir una regla cuando deba cambiar de forma coordinada en sus consumidores.

### 3.4 Frontend: Estado por Responsabilidad y Señales

- Mantener local el estado efímero de un componente. Elevarlo solo hasta el propietario común que lo necesita.
- Agrupar el estado de una funcionalidad bajo un propietario claro. Separarlo del estado realmente global, como tema o conectividad.
- Evitar duplicar datos derivados o mantener copias locales de datos remotos sin una estrategia de sincronización.
- Elegir señales, stores o mecanismos reactivos del stack existente. No desarrollar `createSignal` ni un event bus propios por defecto.
- Preservar invariantes: datos que deban actualizarse juntos necesitan una operación coherente, aunque estén representados por varias señales.
- Suscribir cada consumidor a lo necesario y verificar el comportamiento de actualización con el framework elegido; no asumir que dividir archivos garantiza renderizados selectivos.
- Usar eventos para notificaciones justificadas, con contratos y limpieza claros. No sustituir todos los flujos directos por un bus global.

### 3.5 Sistema de Estilos: Tokens y Ámbito Controlado

Conservar tokens compartidos para **color, espaciado, tipografía, elevación y movimiento**.

- Mantener los estilos específicos cerca de su componente o funcionalidad, según las convenciones del stack.
- Usar el mecanismo de aislamiento acordado: estilos con ámbito, módulos CSS, convenciones de nombres o el sistema de utilidades del proyecto.
- No asumir que mover `.btn` a otro archivo evita colisiones. Controlar explícitamente su ámbito o nomenclatura.
- Reservar estilos globales para tokens, reset, tipografía base y convenciones deliberadamente globales.
- Centralizar decisiones visuales repetidas, no cada declaración aislada. No exigir un archivo CSS por átomo ni extraer una abstracción por dos reglas coincidentes.
- Mantener comportamiento responsive, temas y preferencias de movimiento dentro de la responsabilidad correspondiente, evitando capas de sobrescrituras arbitrarias.

### 3.6 Backend: Dominio, Casos de Uso, Puertos y Pipelines

Organizar primero por capacidad: catálogo, medios, usuarios u otros dominios del producto. Dentro de cada módulo:

- **Dominio:** entidades, invariantes y reglas propias del negocio.
- **Casos de uso:** acciones del sistema y coordinación de reglas, efectos y límites transaccionales.
- **Puertos / traits / interfaces:** contratos pequeños definidos según las necesidades del consumidor, cuando se necesite aislar una dependencia externa.
- **Adaptadores:** implementaciones de persistencia, red, archivos, IPC y demás integraciones.
- **Composición:** conexión explícita de dependencias concretas con casos de uso.

#### Granularidad de los casos de uso

Separar `get_item_details`, `search_catalog` y `download_media_stream` cuando tengan comportamiento o dependencias diferentes. Las operaciones triviales y estrechamente relacionadas pueden compartir archivo sin perder su identidad.

No crear automáticamente una interfaz, fábrica, servicio, comando, pipeline y repositorio por cada acción. Una función con dependencias explícitas puede ser suficiente. Las interfaces deben proteger límites reales, no envolver toda función interna.

#### Pipelines componibles

Usarlos cuando el problema requiera etapas diferenciadas de transformación, procesamiento en flujo o políticas comunes de ejecución. Ejemplo conceptual:

```text
Entrada → Decodificación según el formato real → Validación → Transformación → Salida
```

Cada etapa declara sus entradas, salidas, errores y efectos. El orden de descifrado, descompresión o parseo debe corresponder al formato procesado, no a un ejemplo copiado.

Para una secuencia sencilla, preferir llamadas explícitas. Introducir un ejecutor genérico de pipelines solo cuando sus capacidades sean necesarias para el caso actual.

#### Estado y recursos

Definir propietarios, límites y liberación de cachés, streams, tareas y archivos temporales. En operaciones concurrentes, definir cómo se preservan los invariantes y quién coordina los cambios. No introducir múltiples locks o micro-almacenes por defecto.

Los errores deben conservar su significado entre capas; traducirlos en los adaptadores de entrada. No ocultar fallos con valores por defecto que aparenten éxito.

---

## 4. Guía Práctica: ¿Cuándo Extraer una Pieza?

Antes de crear un archivo, componente, hook, clase o módulo, aplicar este **test de extracción**:

1. **Responsabilidad:** ¿representa un concepto o comportamiento que puede nombrarse sin recurrir a «helper», «manager» o «common» genéricos?
2. **Independencia:** ¿tiene dependencias, ciclo de vida, cambios, pruebas o consumidores que justifican tratarlo por separado?
3. **Localidad:** ¿la extracción permite comprender su consumidor con menos detalle interno, sin obligar a seguir una cadena de intermediarios triviales?
4. **Contrato:** ¿puede exponerse una interfaz pequeña sin revelar casi toda la estructura interna de su consumidor?

**Extraer cuando exista una responsabilidad clara y un beneficio concreto.** La posibilidad de escribir una prueba aislada o de reutilizar algo algún día no basta por sí sola.

Si no hay ese beneficio, mantener la pieza privada dentro de su unidad cohesiva. Si mezcla responsabilidades o efectos distintos, separar aunque todavía tenga un solo consumidor.

### Ubicación de la pieza

```text
¿Solo pertenece a una funcionalidad? → Dentro de ese módulo.
¿Es detalle de una sola implementación? → Privada y cercana a su consumidor.
¿La comparten consumidores reales bajo el mismo contrato? → Evaluar shared/.
¿Orquesta varias funcionalidades? → Coordinador explícito en el nivel adecuado.
```

### Señales para revisar los límites

- Un cambio pequeño exige abrir numerosos archivos sin relación clara.
- Dos módulos se modifican siempre juntos y sus contratos exponen detalles internos.
- Una pieza acumula variantes y condicionales para servir a consumidores incompatibles.
- Un archivo mezcla reglas, transporte, almacenamiento y presentación.

Estas señales requieren revisar el diseño, no dividir o fusionar automáticamente. **No optimizar por número de archivos; optimizar por responsabilidad y superficie de cambio.**

---

## 5. Catálogo de Piezas Reutilizables Bajo Demanda

Este catálogo conserva las piezas del manifiesto como **opciones**, no como una lista de infraestructura que deba implementarse al iniciar cada proyecto.

| Pieza | Cuándo incorporarla |
| :--- | :--- |
| **Botones, inputs, iconos y tokens** | Cuando formen parte de la interfaz y necesiten un contrato visual consistente. |
| **Posicionamiento, clic externo y atajos** | Cuando existan interacciones que lo requieran; reutilizar primero la solución adecuada del stack. |
| **Formato de bytes y duración** | Cuando el producto presente esos datos. |
| **Debounce / throttle** | Cuando el comportamiento de entrada o la frecuencia de eventos lo justifiquen. |
| **Builder DOM / señales** | Solo si se ha elegido una UI que los necesite y no los proporcione ya su infraestructura. |
| **BoundedCache** | Cuando exista una necesidad de caché con límites, política de invalidación y propietario definidos. |
| **Pipeline** | Cuando haya etapas y requisitos de ejecución que merezcan esa abstracción. |
| **WorkdirManager** | Cuando se creen archivos temporales cuyo ciclo de vida deba administrarse. |
| **EventChannel** | Cuando haya notificaciones asíncronas con productores, consumidores y semántica claros. |

**Antes de construir:** comprobar el repositorio, el lenguaje y las dependencias existentes. No desarrollar un framework interno para cumplir este catálogo. No implementar criptografía, saneamiento de HTML ni otros mecanismos de seguridad improvisados como micro-utilidades.

---

## 6. Invariantes de Calidad y Anti-Patrones Prohibidos

| Práctica a evitar | Alternativa exigida |
| :--- | :--- |
| **Componentes o servicios «Dios»** | Separar responsabilidades reconocibles; conservar juntos los detalles cohesivos. |
| **Una función = un archivo por obligación** | Extraer según responsabilidad y beneficio, no según una cuota de granularidad. |
| **Dependencias circulares o acceso a internos ajenos** | Contratos públicos y dirección de dependencias explícita. |
| **`shared/` como contenedor de todo** | Propiedad por módulo y promoción deliberada de conceptos compartidos. |
| **Abstracciones para futuros imaginados** | Resolver el requisito actual con puntos de extensión justificados. |
| **Reglas de negocio duplicadas que deben evolucionar juntas** | Un propietario y un contrato compartido. |
| **Eventos globales como pegamento universal** | Llamadas explícitas o eventos con propósito y contrato documentados. |
| **Acceso incidental al DOM global** | Referencias explícitas; overlays o portales mediante una integración controlada. |
| **Reescrituras ajenas a la tarea** | Cambios acotados; separar las mejoras no necesarias para el objetivo actual. |
| **Validación aparente** | Evidencia de comprobaciones ejecutadas y límites declarados. |

### 6.1 Correcciones bajo revisión con receipt

Cuando una revisión nativa devuelve una corrección, el orden es obligatorio y no se improvisa:

1. **Registrar el forecast de líneas de corrección antes de editar.**
2. Aplicar únicamente el fix aceptado dentro de ese presupuesto.
3. Ejecutar y capturar la validación dirigida del fix.
4. Finalizar la revisión y recién entonces validar/commitear el árbol aprobado.

> No editar entre la recepción del hallazgo y el forecast. Ese cambio invalida el binding del candidato congelado y bloquea la continuidad de la revisión.

### 6.2 Verificación proporcional al cambio

- Probar reglas y transformaciones sin infraestructura cuando sea posible. Verificar también errores, entradas inválidas y límites relevantes.
- Probar adaptadores con comprobaciones de integración apropiadas. Los mocks por sí solos no sustituyen la validación de persistencia o transporte.
- Al cambiar un contrato, comprobar los consumidores afectados. Al tocar código compartido, ampliar la validación más allá del módulo editado.
- Para flujos críticos, comprobar la integración entre módulos y el recorrido de usuario correspondiente. No declarar una funcionalidad completa solo porque pasen sus pruebas unitarias.
- Ejecutar formato, análisis estático, comprobación de tipos, compilación y pruebas según los comandos disponibles y el alcance del cambio.
- No inventar comandos ni resultados. Informar qué se ejecutó, qué resultado tuvo y qué no se pudo verificar. No silenciar fallos ni eliminar pruebas para obtener una ejecución verde.
- Validar entradas y permisos en las fronteras donde corresponda. No confiar únicamente en restricciones de UI; no exponer secretos ni datos sensibles en logs o errores.

### 6.3 Flujo de Trabajo para Agentes

**Antes de editar:** leer las instrucciones aplicables, la especificación y el módulo afectado. Identificar su API pública, consumidores, pruebas y comandos reales. Inspeccionar el estado de trabajo para no sobrescribir cambios ajenos.

**Delimitar:** establecer objetivo, criterios de aceptación, módulos afectados y cambios de contrato necesarios. Si el proyecto usa SDD u otro proceso de especificación, utilizar sus artefactos existentes; no crear un proceso paralelo.

**Implementar:** hacer el cambio mínimo que resuelva completamente el requisito, no el menor número artificial de líneas. Reutilizar piezas adecuadas y aplicar el test de extracción. No añadir dependencias, capas ni refactors no relacionados sin una necesidad concreta.

**Coordinar:** cuando trabajen varios agentes, asignar responsabilidades y acordar contratos antes de editar superficies compartidas. No modificar simultáneamente el mismo contrato, migración o configuración sin coordinación. La validación local no sustituye una comprobación del resultado integrado.

**Verificar:** ejecutar las comprobaciones pertinentes de la sección 6.2. Si hay una revisión nativa activa, respetar la secuencia de la sección 6.1; no alterar un candidato congelado fuera de ese flujo.

**Entregar:** resumir qué cambió, qué contratos fueron afectados y qué evidencia de validación existe. Declarar limitaciones y riesgos pendientes. No hacer commits, publicar ni desplegar salvo autorización de la tarea o del flujo aprobado.

Cuando un módulo lo necesite, mantener documentación breve sobre su responsabilidad, API pública, dependencias permitidas y cómo probarlo. No duplicar todo este manifiesto en cada carpeta ni documentar trivialidades como si fueran subsistemas.

---

> **Máxima de Modularidad:** una pieza merece independencia cuando tiene una responsabilidad y un contrato propios, no simplemente porque pueda moverse a otro archivo.
>
> **Máxima para Agentes:** cada cambio debe requerir comprender lo necesario para hacerlo correctamente, sin arrastrar detalles internos de partes no relacionadas del sistema.
