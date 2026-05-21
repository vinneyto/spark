# build-clip

Minimal Rust CLI scaffold for clipping splats from a JSON config.

## Usage

```bash
cd rust
cargo run -p build-clip -- --dry-run ./build-clip/examples/cylinder.json
cargo run -p build-clip -- ./build-clip/examples/cylinder.json
```

## Config schema

See `clip.schema.json`.

Supported now:
- `clip.type = "cylinder"`
- `keep = "inside" | "outside"`
- `opacity_min` (optional)
- `output_format = "ply" | "spz"`

## Example config

```json
{
  "input": "scene.ply",
  "output": "scene-clipped.ply",
  "output_format": "ply",
  "keep": "inside",
  "opacity_min": 0.0,
  "clip": {
    "type": "cylinder",
    "center": [0.0, 0.0, 0.0],
    "axis": [0.0, 1.0, 0.0],
    "radius": 2.5,
    "half_height": 4.0
  }
}
```
