# build-clip

Minimal Rust CLI for clipping splats by a JSON clipping shape.

## Usage

```bash
cd rust
cargo run -p build-clip -- ./path/to/file.ply --clipping-json ./build-clip/examples/cylinder.json
cargo run -p build-clip -- ./path/to/file.ply --output ./path/to/file-clipped.ply --clipping-json ./build-clip/examples/cylinder.json
```

`--output` is optional. If omitted, output is created next to input with `-clipped` suffix.

Optional flags:
- `--dry-run`
- `--keep inside|outside` (default: `inside`)
- `--opacity-min <0..1>` (default: `0`)
- `--output-format ply|spz` (default: `ply`)

## Clipping JSON schema

See `clip.schema.json`.

Current supported shape:
- `type = "cylinder"`

## Example clipping JSON

```json
{
  "type": "cylinder",
  "radius": 0.46,
  "height": 0.97,
  "position": [-0.0665, -0.0514, 0.1548],
  "quaternion": [0, 0, 0, 1]
}
```
