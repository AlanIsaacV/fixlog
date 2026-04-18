# Fase 3 — TUI básico (ratatui)

Plan atómico y ordenado para la Fase 3. Se arranca con Fase 2 cerrada (index paralelo, query DSL, `grep --follow`; P2-T10 diferido). Cada tarea tiene objetivo, archivos afectados, criterio de aceptación y dependencias. Ejecutar en orden, una por sesión.

**Convención de estado**:

- `[ ]` pendiente
- `[~]` en curso
- `[x]` completa

---

## Resumen de entregables

1. Nuevo crate `fixlog-tui` (library) con `ratatui` + `crossterm`: event loop, estado, renderizado.
2. Binario `fixlog-tui` (thin wrapper sobre la lib) y subcomando `fixlog tui <file>` en `fixlog-cli`.
3. Layout: lista virtual de mensajes + panel de detalle + barra de estado + barra de comando.
4. Modo dual: **follow** (anclado al final, auto-scroll) vs **browse** (cursor libre), toggle con `F`. Indicador `⬇ N new` en browse.
5. Navegación vim-like: `j/k`, `g/G`, `Ctrl+D/U`, `/` + `n/N`, `:`, `q`.
6. Filtro live reutilizando `fixlog-query`, con historial `↑/↓`.
7. Yank al clipboard (`yy` raw, `yY` pretty) vía `arboard`.
8. Resaltado por `MsgType`: `D` verde, `8` azul, `3/j` rojo, `0` gris.
9. Tailing: re-mmap + `append_from_offset` al detectar crecimiento; rotación/truncado reusa rutas de Fase 2.

**Baseline de performance a respetar** (no regresar, de `state.md` 2026-04-17):

- `parse_real/fixt11_md` ~1.1 GiB/s, `parse_real/fix44_om` ~314 MiB/s (criterion `--quick`).
- `index_amplified/parallel_40MiB` ~1.08 GiB/s (**5.1×** vs single-thread).
- Objetivo nuevo Fase 3: **<16 ms por frame** en archivo de 1M mensajes (~budget de 60 fps, aunque sólo repintamos on-demand).

---

## Invariantes que NO romper

Todas heredadas y vigentes. Si una PR las rompe, re-hacer — no flexibilizar.

- **Parser zero-copy**: `RawMessage.raw` y `RawMessage.tags` siguen referenciando el `&[u8]` del mmap. No `String` ni `Vec<u8>` en el hot path de rendering de la lista.
- **`LogIndex.consumed` ≠ `buf.len()`**: es el byte inmediatamente después del último mensaje *exitoso*. Cualquier re-render/append del TUI parte de `consumed`, no de `file_size`.
- **`append_from_offset(buf, from)` exige `from == self.consumed`**. El watcher del TUI debe leer `consumed` ANTES de disparar el append y pasarlo tal cual.
- **Query DSL es tag-numérico**: la barra de filtro acepta `35=D AND 55=AAPL`, no `MsgType=NewOrderSingle`. El adapter symbolic name queda fuera de scope (Fase 5 o adapter thin posterior).
- **`fixlog-parser` no depende de `fixlog-dict`**. El TUI usa ambos, pero la dirección de la dependencia no cambia.
- **Re-mmap en `--follow`**: nunca leer a RAM. El follower re-abre `memmap2::Mmap` cuando cambia el tamaño; el mmap viejo se dropea en orden, así el `&[u8]` viejo no sobrevive al nuevo.
- **Mensajes corruptos = warn + skip**. El renderer de la lista puede encontrar una fila con `ParseError`; se salta igual que en `parse_all`, se loggea con `tracing::warn!`, nunca crashea el TUI.

---

## P3-T01 · Stub del crate `fixlog-tui`

- **Estado**: `[ ]`
- **Depende de**: Fase 2 cerrada.
- **Objetivo**: crate library con tipos públicos y dependencias declaradas; compila sin lógica.
- **Archivos**:
  - `crates/fixlog-tui/Cargo.toml`:
    - deps: `fixlog-core`, `ratatui = "0.29"` (o la última estable), `crossterm = "0.28"`, `anyhow`, `tracing`, `arboard = "3"`, `regex` (si hace falta para el filtro — probablemente ya via `fixlog-query`).
    - No añadir `tokio`/async.
  - `crates/fixlog-tui/src/lib.rs`:
    - `pub struct TuiConfig { pub path: PathBuf, pub follow: bool, pub initial_filter: Option<String> }`.
    - `pub fn run(cfg: TuiConfig) -> anyhow::Result<()>` — stub con `todo!()`.
    - `pub enum TuiError` con `thiserror` (o reusar `anyhow` internamente si no hay consumidores externos que necesiten variantes).
  - Añadir miembro al workspace en `/Cargo.toml`.
- **Plan Mode**: **requerido** — define la API pública (`TuiConfig`, `run`, re-exports) antes de codear.
- **Criterio de aceptación**: `cargo build -p fixlog-tui` y `cargo clippy -p fixlog-tui -- -D warnings` limpios. `cargo test --all` sigue en 89 tests (no rompió nada).

---

## P3-T02 · Event loop + setup/teardown del terminal

- **Estado**: `[ ]`
- **Depende de**: P3-T01
- **Objetivo**: `run(cfg)` abre el terminal en raw mode, pinta un frame vacío con "fixlog" + path, cierra limpio en `q` o `Ctrl+C`.
- **Archivos**:
  - `crates/fixlog-tui/src/app.rs` — `pub struct App` con campos mínimos (path, should_quit, terminal size).
  - `crates/fixlog-tui/src/terminal.rs` — helpers `enter()` / `leave()` que envuelven `enable_raw_mode`, `EnterAlternateScreen`, `DisableMouseCapture` y el panic hook para restaurar el terminal si algo revienta.
  - `crates/fixlog-tui/src/event.rs` — wrapper sobre `crossterm::event::poll` con timeout ~250 ms (el timeout permite procesar ticks para `--follow` sin bloquear). `enum Event { Key(KeyEvent), Tick, Resize }`.
- **Invariantes**:
  - `leave()` debe correrse incluso si `run()` devuelve `Err`. Patrón: struct RAII `TerminalGuard` cuyo `Drop` restaura.
  - Panic hook: `std::panic::set_hook` que primero llama a `leave()` y luego invoca el hook default — nunca dejar la terminal rota.
- **Criterio de aceptación**:
  - Lanzar `fixlog-tui <any-fixture>` abre el alternate screen, muestra un frame vacío con `path`, cierra con `q`.
  - `Ctrl+C` también cierra limpio (handler de `SIGINT` opcional — mínimo respetar el default).
  - Panicking dentro del loop restaura la terminal (test manual: inyectar `panic!`).

---

## P3-T03 · Modelo de estado (`AppState`) y vista de datos

- **Estado**: `[ ]`
- **Depende de**: P3-T02
- **Objetivo**: tipos de dominio que representan "qué mensaje está visible", "cuál está seleccionado", modo follow/browse. Sin render todavía.
- **Archivos**:
  - `crates/fixlog-tui/src/state.rs`:
    ```rust
    pub struct AppState {
        pub mmap: Arc<Mmap>,           // buffer fuente (re-mmap en follow)
        pub format: LogFormat,
        pub index: LogIndex,
        pub filter: Option<Expr>,      // fixlog_query::Expr; None => sin filtro
        pub visible: Vec<u32>,         // ordinals en index.messages que pasan el filtro
        pub cursor: usize,             // posición dentro de `visible`
        pub viewport_top: usize,       // primera fila visible
        pub mode: ViewMode,            // Follow | Browse
        pub new_since_browse: u32,     // contador para "⬇ N new"
        pub status: StatusMessage,
    }
    pub enum ViewMode { Follow, Browse }
    ```
  - `crates/fixlog-tui/src/state/init.rs` — `fn bootstrap(path: &Path) -> Result<AppState>`:
    1. Mmap el archivo.
    2. Sniff `LogFormat` sobre `head(mmap, 64*1024)` (reutilizar helper de `fixlog-cli/io.rs` o duplicar — decisión en la tarea).
    3. `build_from_bytes_parallel(&mmap, &format)`.
    4. `visible = (0..index.messages.len() as u32).collect()` (sin filtro inicial) o filtrar si `cfg.initial_filter`.
    5. `cursor = visible.len().saturating_sub(1)`, `mode = Follow`.
- **Decisiones**:
  - `Arc<Mmap>` para poder swappear el buffer en follow sin invalidar referencias tomadas por render (se reemplaza entre frames).
  - `visible: Vec<u32>` en vez de re-evaluar el filtro cada frame. Se recomputa cuando el filtro cambia o llega una ventana de mensajes nuevos.
- **Criterio de aceptación**:
  - Tests unitarios sobre `bootstrap` con los 2 fixtures reales: `visible.len()` coincide con `state.md` (5419 / 8229).
  - `AppState` no contiene `String` ni `Vec<u8>` derivados de los mensajes — sólo ordinals + refs al mmap.

---

## P3-T04 · Lista virtual de mensajes

- **Estado**: `[ ]`
- **Depende de**: P3-T03
- **Objetivo**: panel principal renderiza sólo las filas visibles del viewport. Cada fila: `#ordinal  offset  MsgType  SenderCompID→TargetCompID  (primeros N bytes del raw)`.
- **Archivos**:
  - `crates/fixlog-tui/src/view/list.rs`:
    - `pub fn render(frame: &mut Frame, area: Rect, state: &AppState)`.
    - Para cada fila visible `i in viewport_top..viewport_top+area.height`:
      - `ord = state.visible[i]`
      - `offset = state.index.messages[ord as usize].start`
      - Parse lazy: `parse_one(&state.mmap[offset..])` — O(1) fields extraction, reusa la ruta de Fase 1.
      - Extraer tags 35, 49, 56 con `tags.iter().find(|(t,_)| *t == X)`.
- **Reglas de rendering**:
  - No alloc `String` por fila: usar `ratatui::text::Span::raw(&str)` con slices del mmap convertidos vía `std::str::from_utf8` (lossy con `from_utf8_unchecked` prohibido; usar `String::from_utf8_lossy` sólo en la celda concreta que lo necesita y sólo para filas visibles — ~50 copias, no 1M).
  - Fallar a mostrar `?` si la fila no parsea (no panic, sólo `tracing::warn!` una vez por ordinal).
- **Criterio de aceptación**:
  - Archivo real `fix44-om.log`: lista muestra 5419 filas, scroll hasta el final sin errores visuales.
  - Medición informal (`std::time::Instant` alrededor del render): <2 ms por frame en viewport de 50 filas sobre `fixt11-md.log`.
  - Abrir archivo amplificado a 1M mensajes: `bootstrap` termina, renderiza, scroll Ctrl+D/U mantiene responsiveness.

---

## P3-T05 · Panel de detalle

- **Estado**: `[ ]`
- **Depende de**: P3-T04
- **Objetivo**: panel lateral (o inferior) muestra el mensaje seleccionado decodificado con el resolver del dict.
- **Archivos**:
  - `crates/fixlog-tui/src/view/detail.rs`:
    - Tabla con columnas: `Tag | Name | Type | Raw | Decoded`.
    - Usa `fixlog_core::{resolve, ResolvedMessage}`. La resolución allocates — OK aquí: sólo corre cuando cambia el cursor, no cada frame.
    - Cachear el `ResolvedMessage` actual en `AppState.detail_cache: Option<(u32, ResolvedMessageOwned)>` para no re-resolver en re-renders del mismo mensaje.
  - `ResolvedMessageOwned`: versión con `Vec<u8>` en los valores — porque el mmap puede ser reemplazado en follow, y guardar `&'a [u8]` ata el cache al buffer viejo. Materializar al resolver.
- **Layout**:
  - Split horizontal por defecto: lista 60% / detalle 40%. `Tab` alterna focus (scroll independiente en el detalle con `j/k` cuando el panel tiene focus).
- **Criterio de aceptación**:
  - Seleccionar un `ExecutionReport` real y ver `35 | MsgType | char | 8 | ExecutionReport`, `54 | Side | char | 1 | BUY`, etc.
  - Cursor en un mensaje con tag no diccionarizado muestra `?` en `Name` y el valor crudo — no crash.
  - Cursor en mensaje malformado muestra `<parse error: ...>` en una sola línea, no rompe el panel.

---

## P3-T06 · Barra de estado + barra de comando

- **Estado**: `[ ]`
- **Depende de**: P3-T05
- **Objetivo**: dos líneas en el fondo de la pantalla. Status muestra versión FIX, filtro activo, contador; command bar inicia con `:` y acepta comandos.
- **Archivos**:
  - `crates/fixlog-tui/src/view/status.rs`:
    - Left: `FIX.4.4 | FIXT.1.1→5.0SP2 | Mixed` según lo que haya visto el sniffer + los `BeginString` únicos en el index.
    - Center: `filter: <expr>` o `no filter`.
    - Right: `<cursor+1>/<visible.len()> (<total>)` — ej. `42/500 (5419)` si hay filtro.
  - `crates/fixlog-tui/src/view/command.rs`:
    - Edit line estilo vim. `:q` sale, `:help` muestra overlay, `:filter <expr>` setea filtro explícito.
    - Historial en memoria, `↑/↓` navegan entradas previas.
- **Reglas**:
  - Barra de comando oculta por default; se abre con `:` y se cierra con `Esc` o `Enter`.
  - Errores (filtro inválido, comando desconocido) se muestran en status bar con color rojo por 2 segundos (tick del event loop los limpia).
- **Criterio de aceptación**:
  - Filtro válido (`:filter 35=D`) reduce `visible.len()` y la barra lo refleja.
  - Filtro inválido (`:filter 35=`) muestra error parseable en status; `fixlog-query` ya da posición de error, mostrarla.
  - `:q` cierra la app igual que `q`.

---

## P3-T07 · Navegación vim-like

- **Estado**: `[ ]`
- **Depende de**: P3-T06
- **Objetivo**: keybindings completos de navegación en la lista.
- **Bindings**:
  - `j` / `↓` — cursor +1
  - `k` / `↑` — cursor -1
  - `g` — inicio
  - `G` — fin
  - `Ctrl+D` — media página abajo (`area.height / 2`)
  - `Ctrl+U` — media página arriba
  - `PageDown`/`PageUp` — página completa
  - `q` — salir
- **Archivos**:
  - `crates/fixlog-tui/src/input.rs`:
    - `fn handle_key(state: &mut AppState, key: KeyEvent) -> Action`.
    - `enum Action { None, Quit, Refilter, SwitchMode, ... }`.
  - Pruebas unitarias: setear `state.cursor`, disparar `j` 10 veces, verificar `cursor += 10` (clamped a `visible.len()-1`).
- **Reglas**:
  - Cualquier movimiento de cursor que no sea `G` auto-activa **Browse** mode (no queremos que `k` te saque del final y deje el auto-scroll activo).
  - `G` activa **Follow** mode.
- **Criterio de aceptación**:
  - Todos los bindings responden correctamente, clamped en bordes.
  - `cursor` y `viewport_top` nunca quedan fuera de rango (test unitario con `visible.len() = 0`, `= 1`, `= 10000`).

---

## P3-T08 · Modo Follow / Browse + indicador de mensajes nuevos

- **Estado**: `[ ]`
- **Depende de**: P3-T07
- **Objetivo**: toggle con `F`. En follow el cursor se pega al final; en browse el cursor es libre y se muestra `⬇ N new` cuando llegan mensajes.
- **Archivos**:
  - `crates/fixlog-tui/src/follow.rs`:
    - `fn on_index_grew(state: &mut AppState, new_msgs: usize)`:
      - En `Follow`: cursor salta al final, `new_since_browse = 0`.
      - En `Browse`: `new_since_browse += new_msgs`. Cursor no se mueve. Indicador visible en status/border.
  - `crates/fixlog-tui/src/view/list.rs`: añadir overlay `⬇ N new` cuando `mode == Browse && new_since_browse > 0`.
- **Bindings**:
  - `F` — toggle Follow/Browse. Al volver a Follow, `new_since_browse = 0`, cursor al final.
- **Criterio de aceptación**:
  - Sin `--follow` real: simular growth en un test llamando `on_index_grew(&mut state, 100)` — el indicador aparece en browse, no en follow.
  - Con `--follow` + fixture sintético que crece (test manual: `tee -a fixtures/real/fix44-om.log` a un fixture temporal): el contador sube, `F` lo resetea.

---

## P3-T09 · Filtro live (reutiliza `fixlog-query`)

- **Estado**: `[ ]`
- **Depende de**: P3-T08
- **Objetivo**: barra `/` abre filtro inline; cada keystroke recompila y re-evalúa; resultados visibles en tiempo real.
- **Archivos**:
  - `crates/fixlog-tui/src/filter.rs`:
    - `fn recompute_visible(state: &mut AppState)`:
      ```rust
      state.visible.clear();
      let Some(expr) = &state.filter else {
          state.visible.extend(0..state.index.messages.len() as u32);
          return;
      };
      for (ord, off) in state.index.messages.iter().enumerate() {
          let bytes = &state.mmap[off.start as usize .. (off.start + off.len as u64) as usize];
          if let Ok((msg, _)) = parse_one(bytes) {
              if expr.matches(&msg) { state.visible.push(ord as u32); }
          }
      }
      ```
    - Pre-filtro opcional usando `SecondaryIndex` si la expresión es una igualdad sobre un hot-tag (`35=D`). Si la expresión es un `AND` de igualdades hot-tag, intersecar los `Vec<u32>` de cada una. Fallback a full scan si la expresión tiene `OR`, `~`, o tags no-hot.
  - `crates/fixlog-tui/src/view/command.rs`: añadir modo "filter input" activado con `/`.
- **Historial**: `Vec<String>` en `AppState.filter_history`; `↑/↓` lo recorre dentro del input.
- **Criterio de aceptación**:
  - Sobre `fix44-om.log`: `/35=D` reduce `visible` al conteo esperado (validar contra `fixlog grep fixtures/real/fix44-om.log --filter "35=D" | wc -l`).
  - Filtro con error posicional muestra caret bajo la posición del error.
  - Re-evaluación sobre 1M mensajes con pre-filtro hot-tag: <100 ms (como el criterio P2).
  - Full-scan (filtro con `~`) sobre 1M mensajes: <1 s (aceptable por rareza del caso).

---

## P3-T10 · Búsqueda `/` + `n/N`

- **Estado**: `[ ]`
- **Depende de**: P3-T09
- **Objetivo**: `/<expr>` busca dentro del `visible` actual y mueve cursor al siguiente match; `n` siguiente, `N` anterior.
- **Decisión**:
  - Reutilizar `fixlog-query` como motor de búsqueda — es la misma idea de filter pero sin filtrar la lista (sólo mover cursor).
  - Alternativa: búsqueda literal (`memchr::memmem`) sobre `raw` del mensaje, más liviana para texto puro. Decidir en la tarea; por default usar query DSL para consistencia.
- **Archivos**:
  - `crates/fixlog-tui/src/search.rs`:
    - `pub struct SearchState { expr: Expr, last_direction: Dir, last_hit: Option<u32> }`.
    - `fn next_match(state: &mut AppState, dir: Dir) -> Option<u32>`.
- **Criterio de aceptación**:
  - `/` abre input, al confirmar avanza al siguiente match desde el cursor actual. `n` repite la dirección; `N` invierte.
  - Wrap-around al final del visible con mensaje en status ("search wrapped").
  - No modifica `visible` (a diferencia del filtro).

---

## P3-T11 · Resaltado por `MsgType`

- **Estado**: `[ ]`
- **Depende de**: P3-T04
- **Objetivo**: color por tipo de mensaje en la lista.
- **Mapeo default**:
  - `D` (NewOrderSingle) → verde
  - `8` (ExecutionReport) → azul
  - `3` (Reject) / `j` (BusinessMessageReject) → rojo
  - `0` (Heartbeat) → gris
  - otros → default terminal
- **Archivos**:
  - `crates/fixlog-tui/src/theme.rs`:
    - `pub struct Theme { msg_type_colors: HashMap<Vec<u8>, Color>, ... }`.
    - `pub fn default_theme() -> Theme`.
  - Rendering en `view/list.rs` aplica `Style::default().fg(theme.msg_type_colors.get(&msgtype).copied().unwrap_or(Color::Reset))`.
- **Decisión deferida**: configuración persistente del tema vive en Fase 5 (`~/.config/fixlog/config.toml`). Fase 3 sólo hardcodea defaults.
- **Criterio de aceptación**:
  - En `fix44-om.log` las filas `MsgType=D` se ven verdes; `MsgType=8` azules. Verificar visualmente.
  - No regresión: mensaje sin color definido se renderiza con default sin panic.

---

## P3-T12 · Yank al clipboard

- **Estado**: `[ ]`
- **Depende de**: P3-T05
- **Objetivo**: `yy` copia el mensaje crudo seleccionado; `yY` copia el pretty-printed.
- **Archivos**:
  - `crates/fixlog-tui/src/clipboard.rs`:
    - Wrapper sobre `arboard::Clipboard`. Soporta fallback con aviso cuando no hay display server (Linux headless/SSH sin X forwarding).
    - `fn copy_raw(state: &AppState) -> Result<()>`: copia `&state.mmap[offset..offset+len]` como UTF-8 lossy.
    - `fn copy_pretty(state: &AppState) -> Result<()>`: reutiliza el renderer pretty de `fixlog-cli/commands/parse.rs` — mover ese renderer a `fixlog-core` para compartir (o duplicar si es pequeño; decisión en la tarea).
- **Bindings**: `yy` (secuencia, timeout 500 ms), `yY`.
- **Criterio de aceptación**:
  - Copiar y pegar en editor externo un mensaje raw produce bytes idénticos al `xxd` del offset correspondiente en el archivo.
  - Copiar pretty produce el mismo output que `fixlog parse <file> --first 1 --format pretty` para ese mensaje.
  - Sin clipboard disponible: status bar muestra "clipboard unavailable" sin crash.

---

## P3-T13 · Integración `--follow` (tailing dentro del TUI)

- **Estado**: `[ ]`
- **Depende de**: P3-T08, P3-T09
- **Objetivo**: `fixlog-tui <file> --follow` se queda leyendo cambios del archivo; append al index, re-filter, refrescar lista.
- **Archivos**:
  - `crates/fixlog-tui/src/follow.rs` (extender):
    - Reusar la lógica de `fixlog-cli/src/commands/grep.rs` — idealmente **extraer** a `fixlog-core::follow` un helper reutilizable (`FileWatcher` con un canal de eventos `Grew { new_size } | Rotated | Truncated`).
    - Si la extracción complica el scope, duplicar en `fixlog-tui` y anotar en state.md como deuda.
- **Pipeline por evento**:
  - `Grew { new_size }`:
    1. Re-mmap (nuevo `Arc<Mmap>`).
    2. `state.index.append_from_offset(&new_mmap, state.index.consumed, &state.format)`.
    3. `state.mmap = new_mmap`.
    4. `filter::recompute_visible` (o sólo append si el filtro es puramente por MsgType/hot-tag y podemos evaluar el delta — optimización opcional).
    5. `follow::on_index_grew(state, delta)`.
  - `Rotated` / `Truncated`: rebuild total (`bootstrap`).
- **Criterio de aceptación**:
  - `fixlog-tui fixtures/real/fix44-om.log --follow` mientras se le hace `tee -a` con mensajes nuevos: las filas aparecen en <1 s, cursor se pega al final en follow mode.
  - Rotación simulada (`mv a b && touch a && cat b >> a`) no crashea; rebuild re-lee el archivo nuevo.
  - En browse mode el indicador `⬇ N new` sube y no se resetea hasta `F`.

---

## P3-T14 · Subcomando `fixlog tui <file>` en `fixlog-cli`

- **Estado**: `[ ]`
- **Depende de**: P3-T01..T13
- **Objetivo**: el binario `fixlog` sabe invocar el TUI vía `fixlog tui <file> [--filter EXPR] [--follow]`.
- **Archivos**:
  - `crates/fixlog-cli/Cargo.toml`: añadir `fixlog-tui` como dep.
  - `crates/fixlog-cli/src/commands/tui.rs`:
    ```rust
    pub fn run(args: TuiArgs) -> anyhow::Result<()> {
        fixlog_tui::run(fixlog_tui::TuiConfig {
            path: args.path,
            follow: args.follow,
            initial_filter: args.filter,
        })
    }
    ```
  - `crates/fixlog-cli/src/commands/mod.rs`: re-exportar.
  - `crates/fixlog-cli/src/main.rs`: variant `Command::Tui(TuiArgs)`.
- **Decisión**: mantener también un binario standalone `fixlog-tui` (con `src/main.rs` en el mismo crate o un binario gemelo) — útil para lanzarlo sin pasar por el subcomando. Decidir en la tarea; default: sólo subcomando para Fase 3, binario standalone diferido a Fase 5.
- **Criterio de aceptación**:
  - `fixlog tui fixtures/real/fix44-om.log` abre el TUI.
  - `fixlog tui fixtures/real/fix44-om.log --filter "35=D"` abre con filtro pre-aplicado y lista filtrada.
  - `fixlog --help` muestra `tui` entre los subcomandos.

---

## P3-T15 · Bench de frame budget y anti-regresión

- **Estado**: `[ ]`
- **Depende de**: P3-T14
- **Objetivo**: validar el criterio "<16 ms por frame en archivo de 1M mensajes".
- **Archivos**:
  - `crates/fixlog-tui/benches/frame.rs`:
    - Generar (o amplificar) un buffer sintético de 1M mensajes en `target/bench_data/` (reusar la lógica de amplificación del bench de `fixlog-index`).
    - Bench: bootstrap + N renders simulados (sin terminal real — usar `ratatui::backend::TestBackend` con tamaño fijo, p.ej. 200×50).
    - Medir render de la lista, render del detalle, evaluación de un filtro típico (`35=D`).
  - Correr `cargo bench -p fixlog-parser --bench parse` y `-p fixlog-index --bench index` para confirmar que no hay regresión (±5%).
- **Criterio de aceptación**:
  - Frame budget (list + detail + status): mediana <16 ms, p99 <32 ms sobre 1M mensajes. Documentar la medición en `docs/agent/crates/tui.md` y actualizar `state.md`.
  - Baseline de parser y de index no regresa: `parse_real/fixt11_md` ≥ 1.05 GiB/s, `index_amplified/parallel_40MiB` ≥ 1.0 GiB/s.

---

## P3-T16 · Docs y actualización de `state.md`

- **Estado**: `[ ]`
- **Depende de**: P3-T15
- **Objetivo**: agent docs para el nuevo crate y estado de fase.
- **Archivos**:
  - `docs/agent/crates/tui.md` — internals: layout, AppState, flujo de eventos, keybindings, invariantes, bench numbers.
  - `docs/agent/INDEX.md` — añadir entrada para `tui.md` en la tabla de routing.
  - `docs/agent/state.md` — sección "Completed (vs PHASE3_PLAN.md)" con la tabla de tareas, baseline de frame budget, nuevas decisiones deferidas.
  - Actualizar `CLAUDE.md` sólo si cambia el stack (p.ej. añadir `arboard` a la lista de dependencias mencionadas) — pero sin reescribir secciones.
- **Criterio de aceptación**:
  - `docs/agent/state.md` refleja fielmente lo implementado (reglas de "autoritativo sobre PHASE*_PLAN").
  - `tui.md` enumera todos los keybindings y las invariantes nuevas (p.ej. `ResolvedMessageOwned` vs borrowed).

---

## Criterios de "Fase 3 completa"

- P3-T01 a P3-T16 en `[x]`.
- `cargo test --all` pasa — suma nuevos tests de `fixlog-tui`; objetivo ≥ 100 tests (89 baseline + ~15 nuevos).
- `cargo clippy --all-targets --all-features -- -D warnings` pasa.
- `cargo fmt --all --check` pasa.
- Bench de parser y de index no regresan (±5%).
- Frame budget <16 ms mediana sobre 1M mensajes sintéticos.
- `docs/agent/state.md` actualizado por tarea (no al final).
- Commit de cierre: `chore(phase3): close Fase 3 — TUI básico`.
- Tag git `v0.3.0-phase3` (el usuario decide cuándo taggear; lo registramos cuando dé luz verde).

---

## Riesgos y decisiones deferidas

- **Symbolic names en el filtro** (`MsgType=NewOrderSingle`): **out of scope**. Mantiene `fixlog-query` tag-numérico. Adapter thin queda para una tarea posterior o Fase 5.
- **Theme persistente**: colores hardcoded en Fase 3; `~/.config/fixlog/config.toml` espera Fase 5.
- **Multi-file / tabs**: Fase 5.
- **Async / tokio**: no se introduce. El event loop es síncrono con `crossterm::event::poll(timeout)`; el watcher emite a un `std::sync::mpsc::channel`.
- **Mouse**: desactivado por default. Si el usuario lo pide, se puede añadir en una tarea posterior (click-to-select, scroll wheel).
- **Repeating groups en detalle**: siguen planas (una fila por `(tag, value)` en orden). Jerarquía visual (indent por `NoXxx`) es scope de Fase 4 ("order lifecycle view") o Fase 5.
- **Ownership del resolve en detalle** (`ResolvedMessageOwned`): materializar strings en el cache evita invalidar al re-mmappear. Costo: una alloc por cambio de cursor. Aceptable — el usuario no mueve el cursor 60 veces por segundo.
- **Pre-filtro con `SecondaryIndex`** en P3-T09: la intersección de hot-tags acelera `35=D AND 49=X` pero no `55=AAPL` (no hot por default). Si los usuarios piden 55 como hot, añadirlo a `HotTags::default_set()`; eso vive en `fixlog-index` y no requiere tocar el TUI.
- **Extracción de `FileWatcher`** a `fixlog-core` (P3-T13): beneficia reutilización entre `grep --follow` y `tui --follow`. Si complica el scope de la tarea, duplicar y dejar la deuda anotada en `state.md`.
- **Binario standalone `fixlog-tui`**: sólo subcomando `fixlog tui` en Fase 3. Binario independiente es trivial de añadir después si el usuario lo pide.
- **P2-T10 (`fixlog index` con caché)**: sigue diferido a Fase 5. La Fase 3 opera con `build_from_bytes_parallel` en memoria en cada apertura; para archivos >1 GB esto es aceptable (<1 s con paralelismo).

---

## Notas sobre ejecución

- **Una tarea atómica por sesión**; `/clear` antes de la siguiente.
- **Plan Mode obligatorio** para P3-T01 (API pública del crate), P3-T03 (tipos de estado), P3-T13 (integración follow). El resto puede avanzar directo si el plan está claro.
- Invocar `/validate` antes de cerrar cada tarea.
- Invocar `/fixture-check` al cerrar P3-T04, P3-T09, P3-T13 (las tareas que tocan el camino mmap → parser → render contra fixtures reales).
- Commit al cerrar cada tarea con conventional commit (`feat(tui):`, `feat(cli):`, `refactor(core):`, `test(tui):`, `bench(tui):`).
- Al cerrar cada tarea: actualizar `docs/agent/state.md` **en ese mismo commit** (no al final de la fase).
