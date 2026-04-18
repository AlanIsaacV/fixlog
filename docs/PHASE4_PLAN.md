# Fase 4 — Análisis avanzado

Plan atómico y ordenado para la Fase 4. Se arranca con Fase 3 cerrada (`fixlog-tui` con ratatui + crossterm, 189 tests verdes, frame ~737 µs sobre 1M mensajes). Cada tarea tiene objetivo, archivos afectados, criterio de aceptación y dependencias. Ejecutar en orden, una por sesión.

**Contexto**: Fases 1–3 entregaron parser zero-copy, índice paralelo, tailing, DSL de filtros y TUI interactivo. Fase 4 añade las capas de **análisis semántico** que un trader/engineer realmente usa en debugging: agrupar por sesión, reconstruir ciclos de órdenes, comparar mensajes, marcar y exportar. El objetivo no es reemplazar `grep`/`filter`; es que el usuario haga preguntas sobre **patrones de negocio** (¿gaps de MsgSeqNum?, ¿este `ClOrdID` se rejectó o se llenó?) sin salir del TUI.

**Convención de estado**:

- `[ ]` pendiente
- `[~]` en curso
- `[x]` completa

---

## Resumen de entregables

1. Nuevo crate `fixlog-analysis` (library): `SessionMap`, `OrderTimeline`, `Histogram` — análisis puro sobre `LogIndex` + `&[u8]`. Depende de `fixlog-core`; no depende de `fixlog-tui`.
2. Subcomandos CLI: `fixlog sessions <file>`, `fixlog orders <file> [--id CLORDID]`, `fixlog histogram <file>`.
3. TUI views: panel de sesiones (`:sessions`), lifecycle overlay (`O` / `:orders`), diff view (dos slots vía `dd`/`dD`), bookmarks (`m<letra>`/`'<letra>`/`:marks`), export (`:export <fmt> <path>`), histograma temporal (`:histogram` toggle).
4. Optimización: **hot-tag pre-filter** en `fixlog-tui/state::apply_filter` — reduce `tui_filter/apply_35eqD_1M` de ~477 ms a <100 ms cuando la expresión es AND-de-igualdades sobre hot tags.
5. Stretch (gated): symbolic query names (`MsgType=NewOrderSingle`) vía feature flag + build.rs extension. **Default: diferido a Fase 5** (decisión confirmada en P4-T14).

**Baselines de performance a respetar** (no regresar ±5%, de `state.md` 2026-04-17):

- `parse_real/fixt11_md` ~1.1 GiB/s, `parse_real/fix44_om` ~314 MiB/s.
- `index_amplified/parallel_40MiB` ~1.08 GiB/s (5.1× vs single-thread).
- `tui_frame/list_detail_status_200x50` ~737 µs (budget <16 ms).
- `tui_bootstrap/1M_messages` ~123 ms.

**Nuevos targets Fase 4**:

- `analysis/session_build_1M` <1 s (single-pass sobre `LogIndex` + re-parse por timestamp).
- `analysis/order_lookup_1M` <50 ms (usa `SecondaryIndex::lookup(11, <clordid>)`).
- `tui_filter/apply_35eqD_1M` <100 ms (hot-tag pre-filter activo).

---

## Invariantes que NO romper

Heredadas de Fases 1–3. Siguen vigentes. Si una PR las rompe, re-hacer — no flexibilizar.

- **Parser zero-copy**: `RawMessage.raw` / `tags` siguen referenciando el `&[u8]` del mmap. En análisis se materializa sólo cuando el resultado cruza la frontera del mmap (sessions map, timeline owned).
- **`LogIndex.consumed` ≠ `buf.len()`**: `SessionMap::append_from_offset` (si existe para follow) parte de `consumed`, no de `file_size`.
- **`append_from_offset(buf, from)` exige `from == self.consumed`**. Análisis incremental bajo `--follow` respeta esto.
- **Query DSL es tag-numérico** en el core de `fixlog-query`. Symbolic names viven detrás de `--features symbolic-names` en un adapter — no se filtra ni se muta el AST.
- **`fixlog-parser` no depende de `fixlog-dict`**. `fixlog-analysis` sí puede (y debe) depender de dict para resolver MsgTypes nombrados en outputs de CLI.
- **Re-mmap en `--follow`, `Arc<Mmap>` swappable**. `SessionMap` y `OrderTimeline` almacenan ordinals (`u32`) o datos materializados (`Vec<u8>`), nunca `&[u8]` del mmap entre frames.
- **Mensajes corruptos = warn + skip**. Builders de análisis saltan silenciosamente los ordinals que fallan `parse_one`; `tracing::warn!` una vez por ordinal, no por cada intento.
- **TUI**: `#![deny(unsafe_code)]` con `#[allow]` sólo en `io.rs`. Event loop síncrono. No branch sobre `KeyCode` en `app.rs` — añadir `Action` en `input.rs`.
- **Multi-key vim sequences vía `pending_prefix`** (patrón documentado en `docs/agent/patterns.md`). Diff (`dd`/`dD`) y bookmarks (`m<letra>`/`'<letra>`) reutilizan ese estado — no introducir un timer ni un `InputMode::Bookmark`.

---

## P4-T01 · Stub del crate `fixlog-analysis`

- **Estado**: `[ ]`
- **Depende de**: Fase 3 cerrada.
- **Objetivo**: crate library con tipos públicos compilables; sin lógica.
- **Archivos**:
  - `crates/fixlog-analysis/Cargo.toml`: deps `fixlog-core`, `smallvec`, `thiserror`, `tracing`. Sin `serde`, sin `tokio`.
  - `crates/fixlog-analysis/src/lib.rs`:
    - `pub mod sessions;` (stub), `pub mod orders;` (stub), `pub mod histogram;` (stub).
    - `pub enum AnalysisError` con `thiserror` (variantes: `Parse(ParseError)`, `MissingTag { tag: u32, ordinal: u32 }`).
  - `crates/fixlog-analysis/src/sessions.rs`: `pub struct SessionKey { sender: Vec<u8>, target: Vec<u8> }`, `pub struct SessionStats { … }`, `pub struct SessionMap { … }` — sólo firmas, `todo!()` en bodies.
  - `crates/fixlog-analysis/src/orders.rs`: `pub struct OrderEvent { ordinal: u32, msg_type: Vec<u8>, sending_time_ns: Option<u128>, raw_slot: OrderSlot }`, `pub struct OrderTimeline { clordid: Vec<u8>, events: Vec<OrderEvent> }`.
  - `crates/fixlog-analysis/src/histogram.rs`: `pub struct Bin { start_ns: u128, end_ns: u128, count: u32 }`, `pub struct Histogram { bucket_ns: u64, bins: Vec<Bin> }`.
  - Workspace: añadir `crates/fixlog-analysis` a `members` en `/Cargo.toml`.
- **Plan Mode**: **requerido** — los tipos públicos de este crate son contrato para CLI y TUI; cualquier cambio posterior es breaking.
- **Criterio de aceptación**: `cargo build -p fixlog-analysis` y `cargo clippy -p fixlog-analysis -- -D warnings` limpios. `cargo test --all` sigue en 189 tests.

---

## P4-T02 · Session tracker (build + gap detection)

- **Estado**: `[ ]`
- **Depende de**: P4-T01.
- **Objetivo**: `SessionMap::build(index, buf, format) -> SessionMap` agrega por `(SenderCompID 49, TargetCompID 56)` con métricas y detección de gaps en `MsgSeqNum (34)`.
- **Archivos**:
  - `crates/fixlog-analysis/src/sessions.rs`:
    ```rust
    pub struct SessionStats {
        pub in_count: u32,      // msg dir unknown from the log alone; interpret as per-key
        pub out_count: u32,
        pub by_msg_type: HashMap<SmallVec<[u8; 2]>, u32>,
        pub seq_min: Option<u32>,
        pub seq_max: Option<u32>,
        pub gaps: Vec<SeqGap>,          // (from_seq, to_seq, ordinal_before, ordinal_after)
        pub ordinals: Vec<u32>,         // for follow/drill-down
    }
    pub struct SessionMap { by_key: HashMap<SessionKey, SessionStats>, by_ordinal: Vec<SessionKey> }
    impl SessionMap {
        pub fn build(index: &LogIndex, buf: &[u8], format: &LogFormat) -> Self;
        pub fn append_from(&mut self, index: &LogIndex, buf: &[u8], format: &LogFormat, from_ordinal: u32);
    }
    ```
  - **Algoritmo**:
    1. Single-pass sobre `index.messages[from..]`. Para cada ordinal, `parse_one(&buf[start..])` (reusa la ruta de Fase 1).
    2. Extrae 49 / 56 / 34 / 35 con `find_tag` (scan lineal sobre `tags`).
    3. `entry((49,56)).or_default()` → incrementa `by_msg_type[35]`, actualiza `seq_min/seq_max`, appendea ordinal.
    4. **Dirección (in/out)**: heurística — si el primer 49 de la sesión aparece como 49 siempre, el otro par es `out`. Documentar en el módulo que "in/out" aquí significa "rol dentro del par", no direccionalidad real (que el log no provee).
    5. **Gaps**: tras el pass, ordenar `ordinals` por `seq_num` vía re-lookup, detectar huecos `seq[i+1] != seq[i]+1`.
  - Tests unitarios sobre fixture sintético con 2 pares de `(49,56)` y 1 gap inyectado; `fixtures/real/fix44-om.log` → 1 sesión (`KALDMA1TRI ↔ BVLORSG`), 0 gaps esperados.
- **Decisión**: **sin hot-tag pre-pass**. El single-pass sobre `index.messages` ya es O(n) y el re-parse es barato (parser ~300 MiB/s sobre OM); usar `SecondaryIndex::lookup` añadiría constante sin reducir asintóticamente.
- **Criterio de aceptación**: test sobre fixture real cuenta las sesiones conocidas; gap test sintético encuentra exactamente el gap inyectado. `append_from` produce el mismo `SessionMap` que un `build` fresco (equivalencia).

---

## P4-T03 · Order lifecycle reconstruction

- **Estado**: `[ ]`
- **Depende de**: P4-T01 (tipos), P4-T02 (patrón de build-pass).
- **Objetivo**: `OrderTimeline::build(index, buf, format, clordid) -> Option<OrderTimeline>` — reconstruye la cadena de un `ClOrdID` (tag 11) cruzando con `OrderID` (tag 37) para incluir mensajes que sólo referencian el `37` después del primer ack.
- **Archivos**:
  - `crates/fixlog-analysis/src/orders.rs`:
    ```rust
    pub struct OrderEvent {
        pub ordinal: u32,
        pub msg_type: SmallVec<[u8; 2]>,
        pub sending_time: Option<SystemTime>,  // parsed from tag 52
        pub exec_type: Option<SmallVec<[u8; 2]>>,  // tag 150
        pub ord_status: Option<SmallVec<[u8; 2]>>, // tag 39
        pub cum_qty: Option<SmallVec<[u8; 16]>>,   // tag 14 (raw ASCII)
    }
    pub struct OrderTimeline {
        pub clordid: Vec<u8>,
        pub order_ids: SmallVec<[Vec<u8>; 2]>,  // tag 37 values observed
        pub events: Vec<OrderEvent>,            // sorted by ordinal
    }
    ```
  - **Algoritmo**:
    1. `index.secondary.lookup(11, clordid)` → ordinals iniciales.
    2. Re-parse de cada ordinal; recolectar tag 37 (OrderID) en `order_ids`.
    3. Segunda pasada: `index.secondary.lookup(37, order_id)` para cada `order_id` observado → expande eventos (execution reports posteriores al ack pueden omitir 11).
    4. Dedup ordinals, ordenar por ordinal (los offsets ya son orden cronológico).
    5. Para cada ordinal, `parse_one` → extrae 35, 52, 150, 39, 14.
    6. Tag 52 parseado vía helper pure en `fixlog-analysis::util::parse_sending_time` (formato `YYYYMMDD-HH:MM:SS[.sss]`).
  - **Decisión — cancel/replace (F/G)**: `fix44-om.log` no contiene F/G. Añadir `fixtures/synthetic/order_lifecycle.log` con la secuencia `D → 8(PendingNew) → 8(New) → F → 8(PendingCancel) → 8(Cancelled)` y otra con `D → 8(New) → G → 8(Replaced) → 8(PartialFill) → 8(Fill)` como golden test. Subtarea explícita dentro de esta tarea.
- **Criterio de aceptación**:
  - Sobre `fix44-om.log` y un ClOrdID conocido (extraído de `fixlog grep --filter "35=D" --first 1`): timeline tiene ≥ 2 eventos en orden, primer evento es `35=D`.
  - Sobre fixture sintético con cancel: timeline contiene los 6 eventos esperados con tipos correctos.
  - `lookup` sobre ClOrdID inexistente devuelve `None`, no panic.

---

## P4-T04 · Temporal histogram helper

- **Estado**: `[ ]`
- **Depende de**: P4-T01.
- **Objetivo**: `Histogram::build(index, buf, format, bucket: Duration) -> Histogram` con sparkline ASCII renderer.
- **Archivos**:
  - `crates/fixlog-analysis/src/histogram.rs`:
    ```rust
    impl Histogram {
        pub fn build(index: &LogIndex, buf: &[u8], format: &LogFormat, bucket: Duration) -> Self;
        pub fn render_sparkline(&self, width: usize) -> String; // unicode blocks ▁▂▃▄▅▆▇█
        pub fn peaks(&self, k: usize) -> Vec<&Bin>;             // top-k buckets
    }
    ```
  - **Algoritmo**:
    1. Single-pass sobre `index.messages`, parse tag 52 con el helper de P4-T03.
    2. Bucketizar: `bin_idx = (t - t_min) / bucket`; grow `bins` on demand.
    3. `render_sparkline`: mapea counts a chars `' ▁▂▃▄▅▆▇█' ` por percentil (no por max bruto — evita que un pico aplaste todo).
  - Tests sobre fixture real: histograma de `fix44-om.log` con bucket 1s; verificar `bins.iter().map(|b| b.count).sum() == 5419`.
- **Criterio de aceptación**: histograma cubre el rango temporal del log sin huecos espurios; sparkline legible para widths 40/80/200.

---

## P4-T05 · `fixlog sessions <file>` CLI subcommand

- **Estado**: `[ ]`
- **Depende de**: P4-T02.
- **Objetivo**: tabla ASCII con una fila por `(SenderCompID, TargetCompID)`; columnas `| session | msgs | by-msg-type | seq range | gaps |`.
- **Archivos**:
  - `crates/fixlog-cli/src/commands/sessions.rs`:
    - `run(args: SessionsArgs)`: mmap + sniff + `build_from_bytes_parallel` + `SessionMap::build` + render tabla via `BufWriter<StdoutLock>`.
    - Resolver MsgType via `fixlog_core::dict::msg_type_label` para mostrar `D=NewOrderSingle` en la columna by-msg-type.
  - `crates/fixlog-cli/src/main.rs`: nueva variante `Command::Sessions(SessionsArgs)`.
  - `crates/fixlog-cli/src/commands/mod.rs`: `pub mod sessions;`.
- **Decisión — JSON**: añadir `--format json` desde la primera iteración (simétrico con `parse`/`grep`). `--format pretty` default.
- **Criterio de aceptación**:
  - `fixlog sessions fixtures/real/fix44-om.log` imprime al menos una sesión con 5419 mensajes.
  - Exit code 0 si hay ≥ 1 sesión; 1 si el archivo no tiene mensajes válidos.
  - `--format json` produce JSONL válido (`jq empty`).

---

## P4-T06 · `fixlog orders <file> [--id CLORDID] [--limit N]` CLI subcommand

- **Estado**: `[ ]`
- **Depende de**: P4-T03.
- **Objetivo**:
  - Sin `--id`: lista los primeros `N` `ClOrdID` (default 20) ordenados por nº de eventos descendente.
  - Con `--id`: imprime el timeline completo + Gantt ASCII horizontal.
- **Archivos**:
  - `crates/fixlog-cli/src/commands/orders.rs`.
  - Gantt helper en `fixlog-analysis::orders::render_gantt(timeline, width) -> String`:
    - Una línea por evento; posición proporcional a `(sending_time - first_time) / total_range`.
    - Carácter por tipo: `N` (NewOrderSingle), `X` (ExecutionReport), `C` (Cancel), `R` (Reject), `?` (other).
    - Width default 60 chars.
- **Criterio de aceptación**:
  - `fixlog orders fixtures/real/fix44-om.log` lista al menos 10 ClOrdIDs.
  - `fixlog orders fixtures/real/fix44-om.log --id <known>` imprime timeline con ≥ 2 eventos + Gantt.
  - `--id <desconocido>` → exit 1, mensaje claro "no events found for ClOrdID=…".

---

## P4-T07 · `fixlog histogram <file>` CLI subcommand

- **Estado**: `[ ]`
- **Depende de**: P4-T04.
- **Objetivo**: imprime sparkline ASCII + tabla de buckets con counts.
- **Archivos**:
  - `crates/fixlog-cli/src/commands/histogram.rs`.
  - Args: `--bucket <duration>` (default 1s, parser manual `1s/500ms/1m`), `--width <cols>` (default 80), `--peaks N` (default 5).
- **Criterio de aceptación**:
  - Output sobre `fix44-om.log` muestra sparkline + counts agregados == 5419.
  - `--peaks 3` resalta los 3 buckets con más tráfico en la salida.

---

## P4-T08 · TUI sessions panel (`:sessions`)

- **Estado**: `[ ]`
- **Depende de**: P4-T02, P3-T06 (command bar).
- **Objetivo**: overlay con tabla de sesiones sobre la lista; `Esc` cierra, `Enter` aplica filtro `49=<sender> AND 56=<target>` y vuelve a la lista.
- **Archivos**:
  - `crates/fixlog-tui/src/state.rs`: extender `ViewMode` con variante `ViewMode::Sessions` (mantener Follow/Browse como booleano paralelo si ya se usan juntos — **decisión en la tarea**: separar `ViewMode` del `PanelFocus`). Alternativa preferida: nuevo `pub enum Overlay { None, Sessions, Orders(OrderTimeline), Diff { a: u32, b: u32 }, Histogram, Marks }` en `AppState.overlay`.
  - `crates/fixlog-tui/src/view/sessions.rs`: render del overlay (centered rect, `ratatui::widgets::Clear` + Table).
  - `crates/fixlog-tui/src/command.rs`: añadir variante `Sessions` al enum `Command`, dispatch en `execute`.
  - `crates/fixlog-tui/src/input.rs`: nuevas `Action::SessionsCursorDown/Up`, `Action::SessionsApplyFilter`, `Action::OverlayClose`.
- **Comportamiento**:
  - `:sessions` computa `SessionMap::build` on-demand (no cachear hasta que aparezca lag visible; 5K mensajes → instantáneo).
  - Gaps resaltados con color rojo vía `theme::color_for_gap(count) -> Option<Color>`.
  - `Enter` sobre una fila setea `state.filter_text = Some("49=X AND 56=Y")` + `apply_filter`.
- **Criterio de aceptación**:
  - Sobre fixture real abre overlay, navega, aplica filtro y reduce `visible` a la sesión seleccionada.
  - Re-abrir `:sessions` con filtro activo muestra **sólo** las sesiones tocadas por el filtro (scope intersection).
  - Perf: `SessionMap::build` sobre 1M mensajes <1 s (bench en P4-T15).

---

## P4-T09 · TUI order lifecycle view

- **Estado**: `[ ]`
- **Depende de**: P4-T03, P4-T08 (patrón Overlay).
- **Objetivo**: keybind `O` sobre una fila seleccionada (o `:orders [id]`) abre overlay con timeline + Gantt ASCII del `ClOrdID` (tag 11) del mensaje actual.
- **Archivos**:
  - `crates/fixlog-tui/src/view/orders.rs`: tabla de eventos `| ordinal | time | msg_type | Δ prev | status |` + Gantt row al tope.
  - `crates/fixlog-tui/src/input.rs`: `Action::OpenOrderTimeline`.
- **Comportamiento**:
  - `O` extrae tag 11 del ordinal bajo cursor. Si no hay 11 (p.ej. Heartbeat), status bar muestra `"no ClOrdID in this message"`.
  - Timeline usa `theme::color_for_lifecycle_stage(exec_type)`: PendingNew → gray, New → blue, PartialFill → yellow, Fill → green, Cancel/Reject → red.
  - `Esc` cierra; `j/k` scroll dentro del overlay si excede height.
- **Criterio de aceptación**:
  - `O` sobre un `35=D` en `fix44-om.log` abre timeline con ≥ 2 eventos y Gantt legible.
  - `O` sobre un Heartbeat no crashea, muestra status warning.

---

## P4-T10 · Diff view (dos slots)

- **Estado**: `[ ]`
- **Depende de**: P3-T05 (detail panel / ResolvedMessageOwned), P3-T12 (patrón pending_prefix).
- **Objetivo**: seleccionar dos mensajes vía `dd`/`dD`, abrir panel side-by-side con diferencias tag por tag.
- **Archivos**:
  - `crates/fixlog-tui/src/state.rs`: `pub diff_slots: [Option<u32>; 2]`.
  - `crates/fixlog-tui/src/input.rs`: `Action::DiffSlotA`, `Action::DiffSlotB`. Mapping: `d` setea `pending_prefix = Some('d')`; `d` completa → `DiffSlotA`; `D` completa → `DiffSlotB` y abre overlay si ambos slots están llenos.
  - `crates/fixlog-tui/src/view/diff.rs`: dos columnas lado a lado, una fila por tag (unión de ambos); coloring: igual → dim, sólo-A → yellow, sólo-B → cyan, distinto → red.
- **Comportamiento**:
  - `dd` toggle del slot A; `dD` toggle del slot B; cuando ambos están llenos y el usuario presiona `dD` (o entra `:diff`), se abre el overlay. Documentar en `tui.md` + `patterns.md`.
  - `Esc` cierra el overlay; los slots se mantienen hasta `:diff clear`.
- **Criterio de aceptación**:
  - Pick dos ExecutionReports consecutivos del mismo ClOrdID → diff muestra diferencias en tags 14 (CumQty), 32 (LastQty), 151 (LeavesQty) resaltadas en rojo.
  - Pick dos mensajes idénticos (imposible en log real — usar ordinal A = B) → status `"diff: both slots same"`, overlay no abre.

---

## P4-T11 · Bookmarks (`m<letra>` / `'<letra>` / `:marks`)

- **Estado**: `[ ]`
- **Depende de**: P3-T12 (pending_prefix), P3-T06 (command bar).
- **Objetivo**: marcar la posición actual con una letra, saltar con `'<letra>`, listar con `:marks`.
- **Archivos**:
  - `crates/fixlog-tui/src/state.rs`: `pub bookmarks: HashMap<char, u32>` — ordinal-keyed (NO offset; los ordinals son absolutos en `index.messages`).
  - `crates/fixlog-tui/src/input.rs`: mapping:
    - `m` setea `pending_prefix = Some('m')`; cualquier char letra completa → `Action::SetMark(c)`.
    - `'` setea `pending_prefix = Some('\'')`; cualquier char letra completa → `Action::JumpMark(c)`.
  - `crates/fixlog-tui/src/command.rs`: variante `Marks` → imprime tabla en overlay (reusar `view/overlay_list.rs` genérico si aparece redundancia con sessions/orders).
  - `crates/fixlog-tui/src/view/marks.rs`: overlay con tabla `| letra | ordinal | preview |`.
- **Comportamiento**:
  - `SetMark('a')` guarda `bookmarks['a'] = visible[cursor]`.
  - `JumpMark('a')` busca el ordinal en `visible`; si no está (por filtro activo), status `"mark 'a' not in filtered view"`.
  - Persistencia a disco: **out of scope** (Fase 5 `~/.config/fixlog/config.toml`).
- **Criterio de aceptación**:
  - `ma`, `G`, `'a` devuelve el cursor a la posición marcada.
  - `:marks` lista entradas con la letra y ordinal correspondiente.
  - Bookmark en ordinal filtrado out → status warn, cursor no se mueve.

---

## P4-T12 · Export (`:export <fmt> <path>`)

- **Estado**: `[ ]`
- **Depende de**: P3-T06 (command bar).
- **Objetivo**: escribe los mensajes en `state.visible` (respetando filtro activo) al path indicado, en formato `csv | json | fix | pretty`.
- **Archivos**:
  - `crates/fixlog-tui/src/export.rs`:
    - `pub fn export(state: &AppState, fmt: ExportFormat, path: &Path) -> Result<usize, ExportError>` — retorna nº de mensajes escritos.
    - `ExportFormat::Csv`: header `ordinal,offset,msg_type,sender,target,seq_num,sending_time`, una fila por mensaje. CSV manual (sin `csv` crate — 4 campos fijos escapados).
    - `ExportFormat::Json`: reusa `write_jsonl` vía `fixlog-render`.
    - `ExportFormat::Fix`: copia los bytes crudos `&buf[start..start+len]` + `\n` entre mensajes.
    - `ExportFormat::Pretty`: reusa `write_pretty` vía `fixlog-render`.
  - `crates/fixlog-tui/src/command.rs`: nueva variante `Export { fmt: ExportFormat, path: PathBuf }`; parser valida fmt ∈ {csv,json,fix,pretty}.
- **Decisión — refactor del renderer**:
  - **Elegido**: extraer `write_pretty` / `write_jsonl` / `write_json_string` de `crates/fixlog-cli/src/commands/parse.rs` a un nuevo micro-crate `fixlog-render` (≤ 200 LOC). Deps: sólo `fixlog-core`. Consumido por `fixlog-cli` y `fixlog-tui`.
  - **Rechazado — promover a `fixlog-core::render`**: `fixlog-core` debe mantenerse como facade de dominio, no de I/O.
  - **Rechazado — duplicar en `fixlog-tui`**: la deuda en `state.md` ya menciona `io.rs` duplicado; no queremos doble duplicación.
  - **Rechazado — dejar en `fixlog-cli` y que `fixlog-tui` dependa de él**: ciclo imposible, `fixlog-cli` ya depende de `fixlog-tui` (P3-T14).
- **Criterio de aceptación**:
  - `:export json /tmp/out.jsonl` con filtro `35=D` produce N líneas; `wc -l == state.visible.len()`; `jq empty /tmp/out.jsonl` pasa.
  - `:export fix /tmp/out.log` produce archivo que re-parseado con `fixlog parse --first 1000` da los mismos mensajes.
  - Path no escribible → status error con `ExportError::Io`, no crashea TUI.
  - `state.visible.len() > 100000` → status warn antes de escribir ("exporting 150000 msgs, this may freeze the UI…"); el usuario confirma con Enter.

---

## P4-T13 · TUI temporal histogram overlay (`:histogram`)

- **Estado**: `[ ]`
- **Depende de**: P4-T04 (helper), P4-T08 (patrón Overlay).
- **Objetivo**: overlay con sparkline ASCII + tabla de picos; `Esc` cierra.
- **Archivos**:
  - `crates/fixlog-tui/src/view/histogram.rs`: render usando `ratatui::widgets::Sparkline` si calza, o `Paragraph` con `Histogram::render_sparkline` custom si necesitamos percentiles.
  - `crates/fixlog-tui/src/command.rs`: variante `Histogram { bucket: Duration }`, default 1s.
- **Criterio de aceptación**:
  - `:histogram` sobre `fix44-om.log` renderiza sparkline cubriendo el rango temporal del log; suma de bins coincide con `visible.len()`.
  - `:histogram 500ms` muestra granularidad distinta.
  - Si no hay tag 52 parseable en ningún mensaje: overlay muestra "no timestamps available".

---

## P4-T14 · Hot-tag pre-filter (TUI `state::apply_filter`)

- **Estado**: `[ ]`
- **Depende de**: P2-T03 (SecondaryIndex), P3-T09 (apply_filter).
- **Objetivo**: cuando el `QueryExpr` se reduce a **AND-de-igualdades-sobre-hot-tags** (p.ej. `35=D AND 49=BROKER1`), construir `visible` via intersección de `SecondaryIndex::lookup` en vez de full scan.
- **Archivos**:
  - `crates/fixlog-query/src/ast.rs`: nuevo método `pub fn hot_equalities(&self) -> Option<Vec<(u32, &[u8])>>` — devuelve `Some(vec)` sólo si el AST es una conjunción puramente de `Pred(Eq)`; `None` si tiene `Or`, `Not`, `Re`, `Ne`, o paréntesis con estructura no plana.
  - `crates/fixlog-tui/src/state.rs::apply_filter`:
    ```rust
    if let Some(exp) = &state.filter {
        if let Some(equalities) = exp.hot_equalities() {
            if equalities.iter().all(|(t, _)| state.index.secondary.has_tag(*t)) {
                state.visible = intersect_lookups(&state.index.secondary, &equalities);
                return Ok(());
            }
        }
        // fallback: full scan as today
    }
    ```
  - `crates/fixlog-index/src/secondary.rs`: exponer `pub fn has_tag(&self, tag: u32) -> bool`.
  - Bench: `crates/fixlog-tui/benches/frame.rs::tui_filter/apply_35eqD_1M` debe caer de ~477 ms a <100 ms.
- **Decisión**:
  - **Cuándo gated-off**: si `hot_equalities()` devuelve `None` **o** cualquier tag no está en `SecondaryIndex`. No intentar optimizaciones parciales (inicia con subset + scan para el resto) en Fase 4; demasiada complejidad para poco valor incremental.
  - **Verificación de corrección**: test unitario que compara `visible` del path hot-tag vs full-scan sobre el mismo expresión + fixture — deben ser idénticos (mismos ordinals, mismo orden).
- **Criterio de aceptación**:
  - Bench `tui_filter/apply_35eqD_1M` <100 ms.
  - Expresiones con `~` / `OR` / no-hot-tag siguen usando full scan y pasan tests de P3-T09.
  - No regresa `tui_frame` ni `tui_bootstrap`.

---

## P4-T15 · Symbolic query names — gated stretch (decide en la tarea)

- **Estado**: `[ ]` (diferido a Fase 5 por default)
- **Depende de**: P4-T14 (para asegurar que el AST base no cambia), decisión del usuario.
- **Objetivo**: aceptar `MsgType=NewOrderSingle AND Side=Buy` en la barra de filtro.
- **Decisión por default**: **diferido a Fase 5**. Razones:
  1. Requiere extender `fixlog-dict::build.rs` con tablas reverse (`msg_type_value_by_name`, `field_value_by_label`) — crece el generated code.
  2. Requiere un adapter en `fixlog-query` (pre-processor de texto, recomendado por research) o una variante AST nueva.
  3. Fase 4 ya es densa en features de análisis; mezclar reversa de dict con análisis diluye el foco.
- **Si se aprueba en Fase 4**:
  - `crates/fixlog-dict/build.rs`: generar `pub fn msg_type_value_by_name(version: FixVersion, name: &str) -> Option<&'static [u8]>` + `pub fn field_enum_value_by_label(version, tag, label) -> Option<&'static [u8]>`.
  - `crates/fixlog-query/src/symbolic.rs` (behind `feature = "symbolic-names"`, default-off): `pub fn expand_symbols(input: &str, chain: DictChain) -> Result<String, SymbolicError>` — pre-procesador que reemplaza `MsgType=NewOrderSingle` → `35=D` antes de llamar a `parse`.
  - `fixlog-tui` puede opt-in la feature a través de su Cargo.toml con `fixlog-query = { features = ["symbolic-names"] }`.
  - **El core de `fixlog-query` no depende de `fixlog-dict`** — la dep vive en el módulo `symbolic`, sólo compilado con la feature.
- **Criterio de aceptación (si se hace en Fase 4)**:
  - `:filter MsgType=NewOrderSingle` equivale a `:filter 35=D` en resultados.
  - Sin la feature, el módulo no se compila y `fixlog-query` queda idéntico al de Fase 3.
  - Test de round-trip: expand + parse + eval == parse(numérico-directo) + eval.

---

## P4-T16 · Benches + state.md update

- **Estado**: `[ ]`
- **Depende de**: todas las anteriores que generen medidas.
- **Objetivo**: validar targets nuevos (sessions/orders/histogram/hot-tag) y confirmar que baselines no regresan.
- **Archivos**:
  - `crates/fixlog-analysis/benches/analysis.rs`: `session_build_1M`, `order_lookup_1M`, `histogram_build_1M` — reusan patrón de amplificación de `fixlog-parser/benches/parse.rs` (ver deuda documentada: extraer `amplify()` a un helper compartido — subtarea opcional aquí).
  - `crates/fixlog-tui/benches/frame.rs`: bench `tui_filter/apply_35eqD_1M` debe reflejar el valor pos-optimización.
  - Re-correr `cargo bench -p fixlog-parser --bench parse` y `-p fixlog-index --bench index` → confirmar ±5%.
  - Actualizar `docs/agent/state.md` con la tabla de Fase 4 completada y las nuevas métricas.
- **Criterio de aceptación**:
  - Todos los targets nuevos cumplidos (ver header).
  - `state.md` refleja la realidad post-Fase 4 (patrón "autoritativo sobre PHASE\*\_PLAN").

---

## P4-T17 · Agent docs

- **Estado**: `[ ]`
- **Depende de**: P4-T16.
- **Objetivo**: docs LLM-oriented para el nuevo crate + nuevas features TUI.
- **Archivos**:
  - `docs/agent/crates/analysis.md`: internals (SessionMap, OrderTimeline, Histogram), invariantes, algoritmos, números de bench.
  - `docs/agent/crates/tui.md`: extender con overlays, bookmarks, diff, export.
  - `docs/agent/crates/cli.md`: añadir `sessions`, `orders`, `histogram`.
  - `docs/agent/INDEX.md`: nueva entrada para `analysis.md` en la tabla de routing.
  - `docs/agent/patterns.md`: añadir sección "Overlay state" (extensión del pending_prefix pattern) y "Análisis derivado del LogIndex" (cómo escribir futuros analizadores).
- **Criterio de aceptación**:
  - Nuevo file `analysis.md` cubre los 3 módulos con ejemplos de uso.
  - `INDEX.md` routing incluye entradas "Session tracking" y "Order lifecycle".

---

## Criterios de "Fase 4 completa"

- P4-T01 a P4-T14 + P4-T16 + P4-T17 en `[x]`. P4-T15 (symbolic names) queda `[ ]` con estado `diferido a Fase 5` a menos que el usuario lo apruebe explícitamente.
- `cargo test --all` pasa — baseline 189 + nuevos tests; objetivo ≥ 230 tests.
- `cargo clippy --all-targets --all-features -- -D warnings` pasa.
- `cargo fmt --all --check` pasa.
- Baselines Fase 1/2/3 no regresan (±5%).
- `tui_filter/apply_35eqD_1M` <100 ms.
- `session_build_1M` <1 s, `order_lookup_1M` <50 ms, `histogram_build_1M` <500 ms.
- `docs/agent/state.md` actualizado por tarea (no al final).
- Commit de cierre: `chore(phase4): close Fase 4 — Análisis avanzado`.
- Tag git `v0.4.0-phase4` (usuario decide cuándo).

---

## Riesgos y decisiones deferidas

- **Symbolic names (P4-T15)**: default-diferido. Ver razonamiento arriba.
- **Overlay state machine**: `AppState.overlay: Option<Overlay>` es un enum creciente (Sessions, Orders, Diff, Marks, Histogram). Si el patrón se vuelve frágil (p.ej. stacking de overlays), considerar un `Vec<Overlay>` o máquina de estados dedicada. No sobre-diseñar en Fase 4.
- **Dirección in/out en sessions**: el log no distingue incoming vs outgoing; lo que reportamos es la agrupación por rol dentro del par `(49,56)`. Si Fase 5 añade un `--direction` flag con heurística (p.ej. por `PossDupFlag` o por config), actualizar la doc.
- **Cancel/replace en fixtures**: `fix44-om.log` no tiene F/G. Añadimos fixture sintético en P4-T03; si aparece fixture real con F/G, usarlo como golden.
- **Timestamps nanosecond precision**: tag 52 puede llegar sin fracción (`YYYYMMDD-HH:MM:SS`) o con ms/ns. El parser soporta hasta ns; valores más cortos se interpretan con ceros al final. Documentar en el módulo.
- **Histogram bucket size < resolución de tag 52**: si el usuario pide `--bucket 1us` y el log sólo tiene precisión de segundo, todos los mensajes caen en el primer microsegundo del segundo. No warn — es trabajo del usuario elegir bucket razonable.
- **Diff entre mensajes de versiones FIX distintas**: el resolver usa chain distinto para cada uno; el diff debe mostrar ambos por su propia resolución (no forzar un chain común). Edge case: un tag que cambia de significado entre versiones aparece como "distinto" en rojo aunque el raw sea el mismo — aceptable, es información útil.
- **Export de archivos muy grandes**: `:export` es bloqueante en el event loop (no tokio). Para `state.visible.len() > 100k`, el TUI se congela durante el write. Solución: warning + confirm en P4-T12.
- **Hot-tag pre-filter y `Ne` / paréntesis**: `hot_equalities()` sólo matchea conjunciones planas de `Eq`. `NOT 35=D` o `35=D AND (55=AAPL OR 55=MSFT)` caen al full-scan. Documentar en `crates/query.md`.
- **`fixlog-render` como nuevo crate**: pequeño pero real. Si el usuario prefiere duplicar los 200 LOC entre cli y tui, revisar antes de arrancar P4-T12.

---

## Notas sobre ejecución

- **Una tarea atómica por sesión**; `/clear` antes de la siguiente.
- **Plan Mode obligatorio** para P4-T01 (API pública del crate de análisis), P4-T02 (shape de `SessionMap`), P4-T03 (shape de `OrderTimeline`), P4-T14 (modificación a AST via `hot_equalities`). El resto puede avanzar directo.
- Invocar `/validate` antes de cerrar cada tarea.
- Invocar `/fixture-check` al cerrar P4-T02, P4-T03, P4-T05, P4-T06, P4-T08, P4-T09 (tareas que tocan el camino mmap → análisis → render contra fixtures reales).
- Commit al cerrar cada tarea con conventional commit (`feat(analysis):`, `feat(cli):`, `feat(tui):`, `refactor(render):`, `test:`, `bench:`).
- Al cerrar cada tarea: actualizar `docs/agent/state.md` en ese mismo commit.
