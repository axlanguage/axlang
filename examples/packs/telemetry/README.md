# Ax External Pack Example

This directory is a complete local-registry pack example:

- `registry/acme.telemetry/pack.axpack` declares the pack metadata, `telemetry.track` operation, effect, and native C source.
- `registry/acme.telemetry/native.c` exports the expected `ax_pack_acme_telemetry_track` symbol.
- `ax.toml` records the consuming project dependency.
- `app.ax` imports and calls the pack.

Run it from this directory:

```bash
ax pack list --registry registry
ax pack find telemetry.track --registry registry
ax pack info acme.telemetry --registry registry
ax check app.ax --registry registry
ax graph app.ax --registry registry
ax build app.ax -o .ax-out/telemetry_app --registry registry
.ax-out/telemetry_app
```
