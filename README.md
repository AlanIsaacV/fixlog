# fixlog

Parser y viewer de logs FIX (Financial Information eXchange) en Rust. Pensado para procesar millones de mensajes de forma eficiente, con zero-copy desde el archivo hasta la salida y un TUI interactivo local.

**Estado actual**: Fases 1–3 cerradas. CLI con `sniff`, `parse`, `stats`, `grep` (con `--follow`) y TUI interactivo (`fixlog tui`) sobre FIX 4.4, FIXT.1.1 y FIX 5.0SP2.

## Qué puede hacer hoy

- **Auto-detectar formato** de un log: separador (`SOH`, `|`, `^`, `;`), prefijos de línea (timestamps, logback), line endings.
- **Parsear sin asumir un formato único**: un mismo binario procesa logs QuickFIX puros, logs re-renderizados con `|`, logs envueltos en prefijos variables.
- **Resolver tags a nombres**: tag `54` → `Side`, valor `1` → `BUY`. Múltiples diccionarios (FIX 4.4, FIXT.1.1, FIX 5.0SP2) con selección automática según `BeginString` y `ApplVerID`.
- **Resumir un archivo** de millones de mensajes en segundos: sesiones, rango temporal, top MsgTypes.
- **Indexar en paralelo** con rayon: ~1 GiB/s de throughput en 1M mensajes; soporte append-only para tailing incremental.
- **Filtrar con DSL** tipo grep: `35=D AND 55=AAPL`, `55~^MS`, con `--follow` estilo `tail -f`.
- **TUI interactivo** (ratatui): lista virtual con detalle resuelto, filtro live, navegación vim, yank al clipboard, tailing en vivo.
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

#### Layout

```text
fixlog /path/to/my.log — 42/5419 (5419)         ← title bar
+-------------------------------+---------------------+
| #ord offset  type sndr→tgt raw | tag  name  type raw decoded
|    #41  4200  D   BRK→OMS  ... | 35   MsgType  char D   NewOrderSingle
| >  #42  4310  D   BRK→OMS  ... | 55   Symbol   str  AAPL
|    #43  4420  8   OMS→BRK  ... | 54   Side     char 1   BUY
|    ...                         | ...
+-------------------------------+---------------------+
[follow] sep:Soh  filter: 35=D           42/1535 (5419)    ← status bar
:filter 35=D▏                                              ← command bar (cuando está activa)
```

#### Keybindings

**Navegación** (modo normal):

| Tecla            | Acción                                  |
|------------------|-----------------------------------------|
| `j` / `↓`        | cursor abajo                            |
| `k` / `↑`        | cursor arriba                           |
| `g`              | primer mensaje                          |
| `G`              | último mensaje (activa Follow)          |
| `Ctrl+D` / `PageDown` | media página abajo                |
| `Ctrl+U` / `PageUp`   | media página arriba                |
| `F`              | alternar Follow / Browse                |

Cualquier movimiento que no sea `G` deja la vista en **Browse**. En Browse el cursor queda fijo aunque lleguen mensajes nuevos; aparece `⬇ N new` en la status bar.

**Comandos** (con `:`):

| Comando                | Efecto                                           |
|------------------------|--------------------------------------------------|
| `:q` / `:quit`         | salir                                            |
| `:help` / `:h`         | mostrar cheatsheet en la status bar              |
| `:filter <expr>` / `:f <expr>` | aplicar filtro (live preview al escribir) |
| `:filter`              | limpiar el filtro activo                         |
| `↑` / `↓` (en comando) | historial                                        |
| `Esc`                  | cancelar (revierte el preview live si aplica)    |

**Búsqueda** (con `/`): misma gramática que el filtro; mueve el cursor al siguiente match.

| Tecla   | Acción                                          |
|---------|-------------------------------------------------|
| `/expr` | iniciar búsqueda                                |
| `Enter` | ir al primer match                              |
| `n`     | siguiente match                                 |
| `N`     | match anterior                                  |
| `Esc`   | cancelar sin mover cursor                       |

**Yank al clipboard**:

| Secuencia | Contenido copiado                                 |
|-----------|---------------------------------------------------|
| `yy`      | bytes crudos del mensaje (SOH rendered como `\|`) |
| `yY`      | pretty-printed (tabla con tags resueltos)         |

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
| Versiones FIX | 4.4, FIXT.1.1 (session) + 5.0SP2 (application) |
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
| `tui_filter/apply_35eqD_1M` (full scan)       | ~477 ms                    |

Los números se re-miden al cerrar cada fase y viven en [`docs/agent/state.md`](docs/agent/state.md).

## Desarrollo

```sh
cargo test --all                             # 189 tests
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
- **Fase 4**: análisis avanzado — session tracking, order lifecycle view, diff, bookmarks, export.
- **Fase 5**: polish — caché de índices serializado, config persistente (`~/.config/fixlog/config.toml`), diccionarios híbridos, multi-file/tabs.

Detalle completo en [`docs/ROADMAP.md`](docs/ROADMAP.md). Plan atómico de cada fase en `docs/PHASE{1,2,3}_PLAN.md`.

## Licencia

MIT OR Apache-2.0.
