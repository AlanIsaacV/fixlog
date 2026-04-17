# Roadmap — fixlog

Plan de desarrollo incremental en 5 fases. Cada fase es un MVP funcional que aporta valor por sí solo. El proyecto puede detenerse al final de cualquier fase y seguir siendo útil.

## Contexto base

- **Casos de uso**: post-mortem (explorar logs históricos) y tailing en vivo (seguir logs que crecen). Ambos por igual.
- **Versiones FIX soportadas**: 4.4, 5.0, 5.0SP1, 5.0SP2, FIXT.1.1.
- **Formatos de entrada**: varios, con auto-detección mediante sniffer.
- **Diccionarios**: Fase 1 embebidos (opción A), Fase 5 migración a híbrido (opción C) con override desde `~/.config/fixlog/dictionaries/`.
- **Corpus de pruebas**: logs reales del usuario en `fixtures/` + logs sintéticos para edge cases.

---

## Fase 1 — Core Parser (CLI sin TUI)

**Objetivo**: binario CLI que lee logs FIX de formato variable y los parsea correctamente. Validado contra fixtures reales antes de tocar UI.

### Features

- **Format sniffer** (`fixlog-format`):
  - Detecta separador: `\x01` (SOH), `|`, `^A`, `<SOH>` literal, `;`.
  - Detecta prefijo por línea (timestamp, log level, thread id) vía regex.
  - Detecta encoding (ASCII vs UTF-8 con/sin BOM) y line endings (`\n` vs `\r\n`).
  - Detecta si un mensaje cabe en una línea o cruza múltiples.
  - Toma decisiones con las primeras ~1000 líneas, no el archivo completo.
  - Output: struct `LogFormat` que consume el parser.

- **Parser agnóstico a versión** (`fixlog-parser`):
  - Zero-copy: trabaja con `&[u8]` apuntando al buffer original.
  - Emite `RawMessage { tags: Vec<(u32, &[u8])> }` sin interpretar.
  - Validación: BeginString (tag 8), BodyLength (tag 9), CheckSum (tag 10).
  - Mensajes malformados se loggean con `tracing::warn!` y se saltan, no crashean.
  - Soporta remover prefijo configurado por el sniffer.

- **Dictionary resolver** (`fixlog-dict`):
  - Diccionarios embebidos para FIX 4.4, 5.0, 5.0SP1, 5.0SP2, FIXT.1.1.
  - `build.rs` parsea los XML de QuickFIX y genera código Rust estático.
  - Resolver recibe `RawMessage` → devuelve `ResolvedMessage` con nombres de tags, tipos y valores enum decodificados.
  - Selecciona diccionario según tag 8 + tag 1128 (ApplVerID) para FIXT.
  - Fallback a "raw" si no hay match para un tag.

- **CLI** (`fixlog-cli`):
  - `fixlog sniff <file>`: reporta el formato detectado (separador, prefijo, encoding).
  - `fixlog parse <file> [--first N] [--format json|pretty]`: imprime los primeros N mensajes.
  - `fixlog stats <file>`: resumen con total mensajes, conteo por tipo (tag 35), rango temporal, sesiones detectadas.

### Criterios de aceptación

- `cargo test --all` pasa con fixtures reales.
- El sniffer identifica correctamente los formatos presentes en `fixtures/`.
- Benchmark baseline establecido: mensajes/seg en un archivo de referencia.
- Parser tolera input corrupto sin crashear (fuzzing básico con `cargo-fuzz` opcional).

---

## Fase 2 — Indexación y búsqueda (CLI)

**Objetivo**: consultas rápidas sobre archivos grandes con soporte para tailing.

### Features

- **Indexación paralela** (`fixlog-index`):
  - Pasada inicial con `rayon`: divide el archivo en chunks, encuentra offsets de inicio de mensaje, arma índice primario `Vec<MessageOffset>`.
  - Índice secundario configurable para tags "hot" (por defecto: 35, 49, 56, 11, 34, 37): `HashMap<(Tag, Value), Vec<MessageIndex>>`.
  - Usa `memmap2` para evitar cargar a RAM.

- **Índice append-only**:
  - Método `append_from_offset(&mut self, from: u64)` para procesar solo el delta cuando el archivo crece.
  - Invariantes documentadas para concurrencia.

- **File watcher**:
  - `notify` crate detecta cambios de tamaño → dispara `append_from_offset`.
  - Detecta rotación de logs (cambio de inode típico con logrotate) y reabre.

- **DSL de filtros** (`fixlog-query`):
  - Parser de expresiones tipo `35=D AND 55=AAPL`, `35=8 OR 35=9`, `54=1 AND NOT 59=0`.
  - Operadores: `=`, `!=`, `~` (regex), `AND`, `OR`, `NOT`, paréntesis.
  - AST evaluable contra `RawMessage` sin re-parsear.

- **CLI extendida**:
  - `fixlog grep <file> --filter "expr"`.
  - `fixlog grep ... --follow`: estilo `tail -f`.
  - `fixlog grep ... --json`: output estructurado para piping a `jq`.
  - `fixlog index <file>`: construye el índice y lo guarda para reusar.

### Criterios de aceptación

- Indexación paralela mide >X MB/s en el benchmark (establecer X en Fase 1).
- Filtros complejos devuelven resultados en <100ms sobre archivo de 1GB.
- Tailing funciona correctamente con logrotate y append concurrente.

---

## Fase 3 — TUI básico (ratatui)

**Objetivo**: experiencia interactiva equivalente a fixparser.targetcompid.com, completamente local.

### Features

- **Layout**:
  - Panel principal: lista virtual de mensajes (solo renderiza visibles).
  - Panel lateral/inferior: detalle del mensaje seleccionado.
  - Barra de estado: versión FIX detectada, filtro activo, contador de mensajes.
  - Barra de comandos (estilo vim `:`).

- **Modo dual de scroll**:
  - Follow mode: anclado al final, auto-scroll con cada mensaje nuevo.
  - Browse mode: cursor libre, mensajes nuevos no interrumpen.
  - Tecla `F` alterna.
  - Indicador `⬇ N new` cuando llegan mensajes en browse mode.

- **Navegación vim-like**:
  - `j/k` o flechas: arriba/abajo.
  - `g/G`: inicio/fin.
  - `Ctrl+D/U`: media página.
  - `/`: buscar.
  - `n/N`: siguiente/anterior match.
  - `:`: modo comando.
  - `q`: salir.

- **Vista de detalle**:
  - Tabla con columnas: tag, nombre, tipo, valor crudo, valor decodificado.
  - Ejemplo: `54 | Side | char | 1 | Buy`.
  - Scroll independiente si el mensaje es largo.

- **Resaltado por tipo de mensaje**:
  - NewOrderSingle (D) en verde.
  - ExecutionReport (8) en azul.
  - Reject (3) / BusinessMessageReject (j) en rojo.
  - Heartbeat (0) en gris.
  - Colores configurables (Fase 5).

- **Filtro live**:
  - Barra de filtro interactiva, escribes expresión y filtra en tiempo real.
  - Historial de filtros con `↑/↓`.

- **Yank (copy al clipboard)**:
  - `yy`: copia mensaje crudo.
  - `yY`: copia mensaje formateado.

- **Indicador de versión**:
  - Barra muestra `FIX.4.4 | FIXT.1.1→5.0SP2 | Mixed` según lo detectado.

### Criterios de aceptación

- TUI responde fluido (<16ms por frame) sobre archivo de 1M mensajes.
- Follow mode mantiene sincronía con tailing sin saltos visuales.
- Filtros live no bloquean la UI (procesamiento async o en worker thread).

---

## Fase 4 — Análisis avanzado

**Objetivo**: herramientas de debugging que un trader/engineer realmente necesita.

### Features

- **Session tracking**:
  - Agrupar mensajes por `(SenderCompID, TargetCompID)`.
  - Métricas in/out, mensajes por tipo.
  - Detección de gaps en `MsgSeqNum` (tag 34) con resaltado.
  - Latencia entre Request y Response.

- **Order lifecycle view**:
  - Selecciona un `ClOrdID` (tag 11) y ve timeline completo.
  - Cadena: NewOrderSingle → PartialFills → Fill/Cancel/Reject.
  - Tiempos entre eventos.
  - Visualización tipo Gantt simple en ASCII.

- **Diff view**:
  - Selecciona dos mensajes → muestra diferencias tag por tag lado a lado.
  - Útil para comparar ExecutionReports sucesivos.

- **Bookmarks**:
  - `m<letra>`: marca mensaje con letra.
  - `'<letra>`: salta al marcado.
  - Lista de bookmarks con `:marks`.

- **Export**:
  - Mensajes filtrados a CSV, JSON, FIX crudo, pretty text.
  - `:export csv /ruta/output.csv`.

- **Estadísticas temporales**:
  - Histograma msg/s con ASCII chart embebido en TUI.
  - Detección de picos de tráfico.

### Criterios de aceptación

- Order lifecycle reconstruye correctamente cadenas complejas (replace, cancel, partial fills).
- Session tracking maneja múltiples sesiones mezcladas en un mismo log.
- Export preserva información sin pérdidas.

---

## Fase 5 — Extensibilidad y polish

**Objetivo**: herramienta production-ready.

### Features

- **Diccionarios híbridos (migración A → C)**:
  - Embebidos como fallback.
  - Override desde `~/.config/fixlog/dictionaries/FIX44.xml` etc.
  - Permite tags custom del broker/exchange.
  - Hot reload opcional con file watcher.

- **Repeating groups**:
  - Parseo correcto de `NoMDEntries`, `NoAllocs`, `NoPartyIDs`, etc.
  - Nota: puede adelantarse a Fase 1 si aparecen en fixtures reales.

- **Caché de índices**:
  - Serialización con `rkyv` a `<archivo>.fixlog-idx`.
  - Hash del archivo para invalidar cache si cambió.
  - Reabrir log de 10GB en <1s si el cache es válido.

- **Configuración persistente**:
  - `~/.config/fixlog/config.toml`.
  - Colores, tags hot, filtros guardados, keybindings.
  - Perfiles por sesión/broker.

- **Multi-file / multi-tab**:
  - Abrir varios logs simultáneamente.
  - Tabs en el TUI (`gt/gT` estilo vim).
  - Diff entre archivos de sesiones distintas.

- **Scripting opcional**:
  - Rhai embebido para reglas custom del usuario.
  - Ejemplo: alertar si `Cumulativamente >100 rejects en 1min`.

### Criterios de aceptación

- Cache de índice valida correctamente por hash (cambio de archivo invalida).
- Configuración custom sobrevive upgrades.
- Documentación completa en `docs/USAGE.md`.

---

## Transversales (todas las fases)

- Tests unitarios por módulo, integration tests con fixtures.
- Benchmarks `criterion` con baseline versionado.
- Logging estructurado con `tracing`, verbosity con `-v`, `-vv`.
- CI con GitHub Actions: build + test + clippy + fmt en Linux/macOS/Windows.
- `cargo-deny` para verificar licencias y vulnerabilidades conocidas.
- Publicación: `fixlog-parser` y `fixlog-dict` en crates.io como librerías reutilizables.
