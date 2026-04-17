Ejecuta el CLI actual contra cada archivo en fixtures/ y reporta resultados. No modifiques código.

Para cada archivo en `fixtures/real/` y `fixtures/synthetic/`:

1. Ejecuta `cargo run -p fixlog-cli --quiet -- sniff <archivo>` si el comando existe.
2. Ejecuta `cargo run -p fixlog-cli --quiet -- stats <archivo>` si el comando existe.
3. Ejecuta `cargo run -p fixlog-cli --quiet -- parse --first 3 <archivo>` si el comando existe.

Reporta en tabla:

```
archivo                           | sniff  | stats  | parse  | mensajes | errores
fixtures/synthetic/minimal_4.4    | ✅     | ✅     | ✅     | 10       | 0
fixtures/real/broker_xyz.log      | ✅     | ⚠️     | ❌     | 1247     | 3
```

Para cada archivo con ❌ o ⚠️, muestra los primeros 5 líneas del error o warning.

No hagas modificaciones. Solo reporta.

Si algún subcomando todavía no está implementado (tarea pendiente en PHASE1_PLAN.md), simplemente márcalo como `—` y sigue con el siguiente.
