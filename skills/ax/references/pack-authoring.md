# Ax Pack Authoring

## Manifest

```txt
name = "acme.telemetry"
version = "1.0.0"
syntax = []
operations = ["telemetry.track"]
effects = ["telemetry.write"]
native = ["native.c"]
```

`operations` is optional for older manifests. When present, Ax treats it as the pack's callable API and rejects undeclared external calls during semantic checking.

## Native Symbols

External operation calls lower to C symbols:

```txt
telemetry.track() -> ax_pack_acme_telemetry_track()
```

Keep symbols `void` for v1 pack calls unless the compiler has explicit typing for the operation.

## Local Registry Layout

```txt
registry/
  acme.telemetry/
    pack.axpack
    native.c
```

Use:

```bash
ax pack list --registry ./registry
ax pack find telemetry --registry ./registry
ax pack info acme.telemetry --registry ./registry
ax pack install acme.telemetry --registry ./registry
ax build app.ax -o app --registry ./registry
```

The skill can scaffold this layout:

```bash
scripts/new-pack.sh ./registry acme.telemetry telemetry.write track
```

which creates a `telemetry.track()` operation backed by:

```c
void ax_pack_acme_telemetry_track(void);
```

Run the end-to-end pack smoke when changing pack tooling:

```bash
scripts/pack-smoke.sh .
```

Use the checked-in pack example when you need a stable reference project:

```bash
cd examples/packs/telemetry
../../../target/release/ax pack list --registry registry
../../../target/release/ax check app.ax --registry registry
../../../target/release/ax build app.ax -o .ax-out/telemetry_app --registry registry
.ax-out/telemetry_app
```

## Remote Registry Layout

Serve `index.axpack`:

```txt
acme.telemetry 1.0.0
```

and manifests at:

```txt
https://registry.example.com/acme.telemetry/pack.axpack
```

Remote native files are fetched relative to the manifest URL and cached under `.ax-out/pack-native/`.

## Validation Checklist

- `ax pack list --registry <source>` lists the pack.
- `ax pack find <query> --registry <source>` finds the pack by name, syntax, operation, native source, or effect.
- `ax pack info <pack> --registry <source>` shows manifest details, including operations.
- `ax pack install <pack> --registry <source>` updates `ax.toml`.
- `ax check` accepts declared effects.
- `ax graph` shows inferred pack effects.
- `ax build` links the expected `ax_pack_*` symbols.
- The produced binary calls the native pack symbol at runtime.
