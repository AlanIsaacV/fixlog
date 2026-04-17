Valida el estado actual del proyecto ejecutando en orden:

1. `cargo build --all` — debe compilar sin errores ni warnings.
2. `cargo test --all` — todos los tests deben pasar.
3. `cargo clippy --all-targets --all-features -- -D warnings` — sin warnings de clippy.
4. `cargo fmt --all --check` — código formateado.

Reporta el resultado en formato conciso:

```
✅ build
✅ test (N/M passed)
❌ clippy: <breve resumen de issues>
✅ fmt
```

Si hay fallos, muestra solo lo esencial (primeras 10 líneas del error de cada comando fallido). No intentes arreglar nada automáticamente. Pregúntame si quiero que los arregles.
