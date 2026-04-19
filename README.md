# fixlog

Parser y viewer de logs FIX (Financial Information eXchange) en Rust. Pensado para procesar millones de mensajes de forma eficiente, con zero-copy desde el archivo hasta la salida y un TUI interactivo local.

**Estado actual**: Fases 1–4 cerradas + rediseño TUI (Fases A/B, 2026-04-18) + Fase 5 parcial. CLI con `sniff`, `parse`, `stats`, `grep` (con `--follow`), `sessions`, `orders`, `histogram` y TUI interactivo (`fixlog tui`) sobre FIX 4.4, FIXT.1.1, FIX 5.0, FIX 5.0 SP1 y FIX 5.0 SP2.

## Qué puede hacer hoy

- **Auto-detectar formato** de un log: separador (`SOH`, `|`, `^`, `;`), prefijos de línea (timestamps, logback), line endings.
- **Parsear sin asumir un formato único**: un mismo binario procesa logs QuickFIX puros, logs re-renderizados con `|`, logs envueltos en prefijos variables.
- **Resolver tags a nombres**: tag `54` → `Side`, valor `1` → `BUY`. Múltiples diccionarios (FIX 4.4, FIXT.1.1, FIX 5.0, FIX 5.0 SP1, FIX 5.0 SP2) con selección automática según `BeginString` y `ApplVerID`.
- **Resumir un archivo** de millones de mensajes en segundos: sesiones, rango temporal, top MsgTypes.
- **Indexar en paralelo** con rayon: ~1 GiB/s de throughput en 1M mensajes; soporte append-only para tailing incremental.
- **Filtrar con DSL** tipo grep: `35=D AND 55=AAPL`, `55~^MS`, con `--follow` estilo `tail -f`.
- **Hot-tag pre-filter**: filtros AND de igualdades sobre tags indexados se resuelven en ~156 µs sobre 1M mensajes (≈3000× más rápido que full-scan).
- **Análisis de sesiones**: agrupar mensajes por par canónico `(SenderCompID, TargetCompID)`, mostrar conteos por dirección, rango de `MsgSeqNum` y detectar gaps.
- **Order lifecycle**: reconstruir el ciclo completo de una orden por `ClOrdID` (tag 11), siguiendo `F` (cancel) y `G` (replace) aunque el ClOrdID cambie, con un gráfico Gantt ASCII.
- **Histograma temporal**: distribución de mensajes por segundo/ms/μs/minuto con sparkline ASCII y top-N de picos de tráfico.
- **TUI interactivo** (ratatui): lista virtual con columnas semánticas (`TIME · MESSAGE · CLIENT ORDER ID · STATUS · DETAIL`), detalle resuelto con navegación por campo, filtro live con preview, búsqueda, overlays para sesiones / órdenes / diff / bookmarks / histograma, filtrado desde el campo bajo cursor (`f`/`x`), toggle de header/trailer (`c`), toggle de heartbeats (`H`), yank al clipboard, tailing en vivo.
- **Diff entre mensajes**, bookmarks vim-style (`m<letra>` / `'<letra>`) y **export** multi-formato (`csv` / `json` / `fix` / `pretty`) directo desde el TUI.
- **Emitir JSON** línea-por-línea para piping a `jq` u otras herramientas.

## Instalación

Requisitos: Rust estable reciente (probado con 1.95, edition 2024).

```sh
git clone <repo> fixlog
cd fixlog
cargo build --release -p fixlog-cli
```

El binario queda en `target/release/fixlog`. Opcionalmente:

```sh
cargo install --path crates/fixlog-cli
```

## Uso

Los ejemplos asumen que hiciste `alias fixlog=./target/release/fixlog` o instalaste con `cargo install`.

### `fixlog sniff` — detectar formato

```sh
$ fixlog sniff fixtures/synthetic/with_timestamp_prefix.log
File:           fixtures/synthetic/with_timestamp_prefix.log
Separator:      SOH (0x01)
Line prefix:    fixed 24 bytes
Line ending:    LF (\n)
Encoding:       UTF-8 / ASCII
Msg boundary:   line
```

### `fixlog parse` — imprimir mensajes

Formato `pretty` (por defecto) con nombres de tags y labels de enums:

```sh
$ fixlog parse fixtures/synthetic/minimal_4.4.log --first 1
Message @ offset 0 (95 bytes) Logon
      8  BeginString     = FIX.4.4
      9  BodyLength      = 73
     35  MsgType         = A (Logon)
     34  MsgSeqNum       = 1
     49  SenderCompID    = SENDER
     52  SendingTime     = 20260416-13:30:00.000
     56  TargetCompID    = TARGET
     98  EncryptMethod   = 0 (NONE_OTHER)
    108  HeartBtInt      = 30
    141  ResetSeqNumFlag = Y (YES_RESET_SEQUENCE_NUMBERS)
     10  CheckSum        = 161
```

Formato `json` (JSONL, un objeto por línea) — pipe-friendly:

```sh
$ fixlog parse my.log --format json | jq '. | select(.msg_type_name == "NewOrderSingle") | .tags[] | select(.name == "Symbol").value' | sort -u
```

Opciones:
- `--first N` — procesar solo los primeros `N` mensajes.
- `--format pretty|json` — formato de salida.

### `fixlog stats` — resumen ejecutivo

```sh
$ fixlog stats my-fix44-trading.log
File:            my-fix44-trading.log
Messages parsed: 5419
Parse errors:    0
Time range:      20260416-13:30:04.713 .. 20260416-19:42:09.279
Sessions:        2
        3354  BROKER → OMS
        2065  OMS → BROKER
Message types:   6 (6 shown)
        3124  8    ExecutionReport
        1535  D    NewOrderSingle
         489  0    Heartbeat
         268  F    OrderCancelRequest
           2  A    Logon
           1  9    OrderCancelReject
```

### `fixlog sessions` — sesiones y gaps de secuencia

Agrupa mensajes por par canónico `(SenderCompID, TargetCompID)` (las dos direcciones colapsan en una entrada) y detecta huecos en `MsgSeqNum` por dirección.

```sh
$ fixlog sessions fixtures/real/fix44-om.log
session                          msgs   seq-range       top types        gaps
BROKER ↔ OMS                     5419   1..5419         8=3124 D=1535    0
```

Opciones:
- `--format pretty|json` — JSONL (uno por sesión) útil para pipear a `jq`.

Exit code `1` si el log no contiene sesiones resolubles.

### `fixlog orders` — lifecycle de una orden

Sin `--id` lista las top-N cadenas de órdenes con más eventos; con `--id <clordid>` imprime el timeline completo + Gantt ASCII del ciclo de vida, siguiendo `F` (cancel) y `G` (replace) aunque el ClOrdID cambie a mitad de la cadena.

```sh
$ fixlog orders my.log --id ABC123
ClOrdID: ABC123
events: 7

  offset  ordinal  type  exec_type  ord_status  at
  2048    42       D     —          —           20260416-13:30:04.713
  2161    43       8     PendingNew New         20260416-13:30:04.720
  2274    44       8     New        New         20260416-13:30:04.742
  ...

Gantt:  N  X  N  P  P  F

$ fixlog orders my.log --limit 5
```

Opciones:
- `--id <clordid>` — imprimir solo ese ciclo.
- `--limit N` — top-N órdenes cuando no se pasa `--id` (default: 20).
- `--format pretty|json`.

### `fixlog histogram` — histograma temporal

Distribución de mensajes por bucket de tiempo (basado en `SendingTime`, tag 52) con sparkline ASCII escalado por percentil 95 y lista de picos.

```sh
$ fixlog histogram my.log --bucket 500ms --width 100 --peaks 3
bucket:  500ms
bins:    42389  (dropped: 12)

▁▁▂▃▄▅▇█▇▆▅▃▂▂▁ … (100 cols)

peaks:
   2026-04-16 13:31:24.500    487 msgs
   2026-04-16 13:31:25.000    441 msgs
   2026-04-16 13:31:25.500    398 msgs
```

Opciones:
- `--bucket <dur>` — tamaño de bucket; soporta `Ns`, `Nms`, `Nus`, `Nm` (default: `1s`).
- `--width <cols>` — ancho de la sparkline en columnas (default: 80).
- `--peaks <N>` — cuántos picos mostrar (default: 5).

### `fixlog grep` — filtrar con DSL

Gramática mínima del filtro (ver `crates/fixlog-query`): `<tag><op><value>` combinados con `AND` / `OR` / `NOT` y paréntesis. Operadores: `=`, `!=`, `~` (regex). Precedencia: `NOT > AND > OR`.

```sh
# Todos los NewOrderSingle.
fixlog grep my.log --filter "35=D"

# NewOrderSingle para AAPL.
fixlog grep my.log --filter "35=D AND 55=AAPL"

# ExecutionReports de brokers con prefijo MS (regex).
fixlog grep my.log --filter "35=8 AND 49~^MS" --format json

# Tailing en vivo con filtro.
fixlog grep live.log --filter "35=3" --follow
```

Exit codes estilo `grep(1)`: `0` si matchea ≥ 1, `1` si 0 matches.

### `fixlog tui` — viewer interactivo

Abre el log en una UI de terminal con lista virtual, panel de detalle, filtro live y tailing.

```sh
fixlog tui my.log                        # viewer básico
fixlog tui my.log --filter "35=D"        # pre-aplica un filtro
fixlog tui live.log --follow             # tailing en vivo
```

Dentro del TUI: `:help` (o `:h`) abre un overlay con el cheatsheet completo de shortcuts y comandos.

#### Layout

```text
fixlog /path/to/my.log — 42/5419 (5419)               ← title bar
+-----------------------------+--------------------------+
| time   message  clordid  st | tag  name   type raw dec |
|  ...   D  BRK→OMS  ABC12  — | 35   MsgType char D  ... |
| >...   D  BRK→OMS  ABC13  — | 55   Symbol  str  AAPL   |
|  ...   8  OMS→BRK  ABC12 Nw | 54   Side    char 1  BUY |
|  ...                        | ...                      |
+-----------------------------+--------------------------+
[follow] list  filter: 35=D           42/1535 (5419)     ← status bar
:filter 35=D▏                                            ← command bar (cuando está activa)
```

El panel activo (list / detail) se controla con `Tab` / `Shift+Tab`. En modo raw (`r`) el detalle ocupa todo el ancho y muestra los bytes FIX crudos envueltos, facilitando la selección con el mouse.

#### Modos de entrada

| Modo      | Cómo se entra       | Qué hace                                     |
|-----------|---------------------|----------------------------------------------|
| Normal    | arranque / `Esc`    | Navegación + toggles vim-like                |
| Command   | `:`                 | Escritura de comandos (`:filter`, `:export`…) |
| Search    | `/`                 | Buscar por expresión DSL                      |

#### Navegación (modo normal)

| Tecla                 | Acción                                                        |
|-----------------------|---------------------------------------------------------------|
| `j` / `↓`             | cursor abajo (list o detail según focus)                      |
| `k` / `↑`             | cursor arriba                                                 |
| `g`                   | primer elemento (drop a Browse)                               |
| `G`                   | último elemento (re-entra a Follow si aplica)                 |
| `Ctrl+D` / `PageDown` | media página abajo                                            |
| `Ctrl+U` / `PageUp`   | media página arriba                                           |
| `F`                   | alternar Follow / Browse                                      |
| `Tab` / `Shift+Tab`   | mover focus entre list y detail                               |
| `←` / `→`             | scroll horizontal del panel enfocado                          |
| `0`                   | reset de scrolls (list h, detail h, detail v)                 |

Cualquier movimiento que no sea `G` deja la vista en **Browse**. En Browse el cursor queda fijo aunque lleguen mensajes nuevos; aparece `⬇ N new` en la status bar.

#### Detail panel (cuando focus = Detail)

| Tecla                 | Acción                                                        |
|-----------------------|---------------------------------------------------------------|
| `j` / `k` / `g` / `G` | mover el cursor por campo dentro del mensaje                  |
| `Ctrl+D` / `Ctrl+U`   | media página de campos                                        |
| `f`                   | filtro desde el campo bajo cursor: `AND tag=value`            |
| `x`                   | filtro desde el campo bajo cursor: `AND NOT tag=value`        |

`f` y `x` respetan la DSL: valores con caracteres especiales se cuotan y escapan automáticamente.

#### Toggles

| Tecla | Toggle                                                                      |
|-------|-----------------------------------------------------------------------------|
| `c`   | ocultar / mostrar tags comunes de header+trailer (8,9,10,34,35,49,52,56)    |
| `H`   | ocultar / mostrar Heartbeats (compone `AND NOT 35=0` en el filtro efectivo) |
| `r`   | alternar vista raw FIX (SOH → `\|`, envuelto) ↔ tabla de campos resueltos   |

#### Búsqueda (`/`)

Misma gramática que el filtro; mueve el cursor al siguiente match, `n`/`N` iteran reutilizando la expresión ya compilada.

| Tecla   | Acción                                          |
|---------|-------------------------------------------------|
| `/expr` | iniciar búsqueda                                |
| `Enter` | ir al primer match                              |
| `n`     | siguiente match                                 |
| `N`     | match anterior                                  |
| `Esc`   | cancelar sin mover cursor                       |

#### Secuencias de dos teclas

| Secuencia    | Efecto                                                            |
|--------------|-------------------------------------------------------------------|
| `yy`         | yank: bytes crudos del mensaje (SOH rendered como `\|`)           |
| `yY`         | yank: pretty-printed (tabla con tags resueltos)                   |
| `dd`         | setear diff slot A al mensaje bajo cursor                          |
| `dD`         | setear diff slot B y abrir el overlay de diff                     |
| `m<letra>`   | set bookmark (`a`–`z`, `A`–`Z`) al mensaje bajo cursor            |
| `'<letra>`   | saltar al bookmark (si está en el filtro actual)                   |
| `O`          | abrir overlay de order lifecycle (tag 11 del mensaje bajo cursor) |

#### Overlays

| Overlay      | Se abre con               | Cerrar | Acción                                      |
|--------------|---------------------------|--------|---------------------------------------------|
| Help         | `:help` / `:h`            | `Esc`  | cheatsheet de shortcuts (scrollable)        |
| Sessions     | `:sessions`               | `Esc`  | `j`/`k` mueven; `Enter` aplica `49=X AND 56=Y` |
| Orders       | `O` o `:orders [id]`      | `Esc`  | timeline + Gantt; coloreado por `ExecType`  |
| Histogram    | `:histogram [bucket]`     | `Esc`  | sparkline + top-20 picos                    |
| Marks        | `:marks`                  | `Esc`  | lista de bookmarks                          |
| Diff         | `dd` + `dD` (ambos slots) | `Esc`  | tabla lado a lado, resaltado por diferencia |

#### Comandos (modo Command)

| Comando                        | Efecto                                                                                          |
|--------------------------------|-------------------------------------------------------------------------------------------------|
| `:q` / `:quit`                 | salir                                                                                           |
| `:h` / `:help`                 | abrir el overlay de help                                                                        |
| `:filter <expr>` / `:f <expr>` | aplicar filtro (live preview al escribir)                                                       |
| `:filter`                      | limpiar el filtro activo                                                                        |
| `:sessions`                    | abrir overlay de sesiones                                                                       |
| `:orders [id]`                 | abrir timeline de una orden; sin `id` usa el tag 11 del mensaje bajo cursor                     |
| `:histogram [bucket]`          | abrir histograma; `bucket` soporta `Ns` / `Nms` / `Nus` / `Nm` (default: `1s`)                  |
| `:marks`                       | abrir overlay de bookmarks                                                                      |
| `:export <fmt> <path>`         | exportar `visible` a archivo; `fmt` ∈ {`csv`, `json`, `fix`, `pretty`}                          |
| `:diff clear`                  | resetear los dos slots de diff                                                                  |
| `↑` / `↓` (dentro de `:`)      | historial de comandos                                                                           |
| `Esc`                          | cancelar (revierte el preview live si aplica; cierra el overlay si hay uno abierto)             |

**Salir**: `q`, `:q`, o `Ctrl+C`.

#### Colores por MsgType

| MsgType | Etiqueta        | Color    |
|---------|-----------------|----------|
| `D`     | NewOrderSingle  | verde    |
| `8`     | ExecutionReport | azul     |
| `3` / `j` | Reject / BusinessReject | rojo |
| `0`     | Heartbeat       | gris     |
| otros   | —               | default  |

### Verbosidad

```sh
fixlog -v  parse my.log   # tracing nivel info en stderr
fixlog -vv parse my.log   # tracing nivel debug (incluye checksum mismatches, etc.)
```

También respeta `RUST_LOG` si quieres control fino: `RUST_LOG=fixlog_parser=debug fixlog parse my.log`.

## Formatos soportados

| Aspecto | Soporte |
|---------|---------|
| Versiones FIX | 4.4, FIXT.1.1 (session) + 5.0 / 5.0 SP1 / 5.0 SP2 (application) |
| Separador | SOH (`\x01`), `\|`, `^`, `;` — autodetectado |
| Prefijos de línea | cualquiera (timestamp, logback, PID…) — el parser escanea `8=FIX` |
| Line ending | LF, CRLF |
| Encoding | UTF-8 / ASCII (bytes no-UTF-8 se muestran lossy) |
| Checksums inválidos | **no-fatales**: se emite el mensaje y se loggea en `-vv` |

Añadir una versión FIX extra es ~10 líneas: descargar el XML de QuickFIX/J a `dictionaries/` y añadirlo a `DICTIONARIES` en `crates/fixlog-dict/build.rs`.

## Arquitectura

Workspace Cargo con crates desacoplados:

```
crates/
├── fixlog-format/   # Sniffer del formato (separador, prefijo, encoding)
├── fixlog-parser/   # Parser zero-copy → RawMessage
├── fixlog-dict/     # Diccionarios FIX (build.rs genera desde XML)
├── fixlog-index/    # Índice offset-based + hot-tags + paralelo con rayon
├── fixlog-query/    # DSL de filtros (AST + parser + evaluador)
├── fixlog-core/     # Facade re-exportando los anteriores
├── fixlog-cli/      # Binario `fixlog` con sniff/parse/stats/grep/tui
└── fixlog-tui/      # Lib ratatui (montada por `fixlog tui`)
```

El parser no sabe nada del diccionario — emite pares `(tag, &[u8])` y el resolver decora con nombres. Esto permite reindexar millones de mensajes sin materializar strings. El TUI renderiza <1 ms por frame en 1M mensajes.

Más detalles en [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) (intención de diseño), [`docs/agent/`](docs/agent/) (estado actual, organizado para LLMs) y [`docs/agent/crates/tui.md`](docs/agent/crates/tui.md) (internals del TUI).

## Performance

Métricas de referencia (Darwin 25.3.0, criterion `--quick`):

| Bench                                         | Throughput / tiempo        |
|-----------------------------------------------|----------------------------|
| `parse_real/fixt11_md` (8.7 MB)               | ~1.1 GiB/s                 |
| `parse_real/fix44_om` (2.1 MB)                | ~314 MiB/s                 |
| `index_amplified/parallel_40MiB`              | ~1.08 GiB/s (**5.1×** vs single-thread) |
| `tui_bootstrap/1M_messages`                   | ~123 ms                    |
| `tui_frame/list_detail_status_200x50`         | **~737 µs** (~22× bajo el target de 16 ms) |
| `tui_filter/apply_35eqD_1M` (hot-tag pre-filter, Fase 4) | **~156 µs** (~3000× vs ~477 ms pre-Fase 4) |
| `analysis/session_build_1M`                   | ~1.16 s                    |
| `analysis/order_lookup_1M` (real fix44-om)    | ~5 µs                      |
| `analysis/histogram_build_1M`                 | ~500 ms                    |

Los números se re-miden al cerrar cada fase y viven en [`docs/agent/state.md`](docs/agent/state.md).

## Desarrollo

```sh
cargo test --all                             # 293 tests
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all
cargo run -p fixlog-cli -- sniff <file>      # ejecutar sin instalar
cargo run -p fixlog-cli -- tui <file>        # lanzar el TUI

# Benches.
cargo bench -p fixlog-parser --bench parse
cargo bench -p fixlog-index  --bench index
cargo bench -p fixlog-tui    --bench frame
```

Fixtures:
- `fixtures/synthetic/` — logs pequeños versionados para tests golden.
- `fixtures/real/` — gitignored, logs reales para validación manual.

Ver [`fixtures/README.md`](fixtures/README.md) para detalles del corpus.

## Roadmap

- **Fase 1** ✓: CLI + parser + diccionarios + sniffer.
- **Fase 2** ✓: indexación paralela + DSL de filtros + `grep --follow`.
- **Fase 3** ✓: TUI ratatui con lista virtual, detalle, filtro live, búsqueda, follow/browse, yank.
- **Fase 4** ✓: análisis avanzado — session tracking, order lifecycle view, diff, bookmarks, export, hot-tag pre-filter.
- **Fase A/B** ✓ (2026-04-18): rediseño del TUI — columnas semánticas, toggles `c`/`H`/`r`, focus entre paneles, filter-from-detail (`f`/`x`), navegación por campo.
- **Fase 5** (parcial): polish. **Done**: crate `fixlog-render`, diccionarios FIX 5.0 / 5.0 SP1, `QueryExpr: Clone`, analysis pipe-separated. **Backlog**: caché de índices (`<file>.fixlog-idx`), config persistente (`~/.config/fixlog/config.toml`), flag `--strict`, diccionarios híbridos, multi-file/tabs, repeating groups, symbolic query names.

Detalle completo en [`docs/ROADMAP.md`](docs/ROADMAP.md). Plan atómico de cada fase en `docs/PHASE{1,2,3,4,5}_PLAN.md`.

## Licencia

MIT OR Apache-2.0.
