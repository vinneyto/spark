use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use glam::{Quat, Vec3A};
use serde::Deserialize;
use spark_lib::{
    decoder::{ChunkReceiver, MultiDecoder},
    gsplat::GsplatArray,
    ply::PlyEncoder,
    spz::SpzEncoder,
    tsplat::{Tsplat, TsplatArray},
};

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum OutputFormat {
    Ply,
    Spz,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum KeepMode {
    Inside,
    Outside,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClipShape {
    Cylinder {
        radius: f32,
        height: f32,
        position: [f32; 3],
        quaternion: [f32; 4],
    },
}

impl ClipShape {
    fn contains(&self, point: Vec3A) -> bool {
        match self {
            ClipShape::Cylinder {
                radius,
                height,
                position,
                quaternion,
            } => {
                let center = Vec3A::from_array(*position);
                let q = Quat::from_array(*quaternion).normalize();
                let local = q.inverse() * (point - center);

                let half_height = *height * 0.5;
                if local.y.abs() > half_height {
                    return false;
                }

                let radial2 = local.x * local.x + local.z * local.z;
                radial2 <= radius * radius
            }
        }
    }
}

#[derive(Debug)]
struct CliArgs {
    input: String,
    output: Option<String>,
    clipping_json: String,
    output_format: OutputFormat,
    keep: KeepMode,
    opacity_min: f32,
    dry_run: bool,
}

fn read_file_chunks(filename: &str, decoder: &mut impl ChunkReceiver) -> anyhow::Result<()> {
    const CHUNK_SIZE: usize = 1024 * 1024;
    let mut reader = BufReader::new(File::open(filename)?);
    let mut buffer = vec![0u8; CHUNK_SIZE];
    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        decoder.push(&buffer[..bytes_read])?;
    }
    decoder.finish()
}

fn derive_output_path(input: &Path, output_format: OutputFormat) -> String {
    let ext = match output_format {
        OutputFormat::Ply => "ply",
        OutputFormat::Spz => "spz",
    };

    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("out")
        .to_owned();

    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    parent
        .join(format!("{stem}-clipped.{ext}"))
        .to_string_lossy()
        .into_owned()
}

fn print_usage() {
    eprintln!("Usage: build-clip [--dry-run] <input.ply|input.spz> [--output <path>] [--clipping-json <clip.json>] [--output-format ply|spz] [--keep inside|outside] [--opacity-min <0..1>]");
}

fn parse_args() -> anyhow::Result<CliArgs> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        bail!("missing input path")
    }

    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut clipping_json: Option<String> = None;
    let mut output_format = OutputFormat::Ply;
    let mut keep = KeepMode::Inside;
    let mut opacity_min = 0.0f32;
    let mut dry_run = false;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--output" => {
                i += 1;
                if i >= args.len() {
                    bail!("--output requires a value");
                }
                output = Some(args[i].clone());
                i += 1;
            }
            "--clipping-json" => {
                i += 1;
                if i >= args.len() {
                    bail!("--clipping-json requires a value");
                }
                clipping_json = Some(args[i].clone());
                i += 1;
            }
            "--output-format" => {
                i += 1;
                if i >= args.len() {
                    bail!("--output-format requires a value");
                }
                output_format = match args[i].as_str() {
                    "ply" => OutputFormat::Ply,
                    "spz" => OutputFormat::Spz,
                    other => bail!("unsupported --output-format: {other}"),
                };
                i += 1;
            }
            "--keep" => {
                i += 1;
                if i >= args.len() {
                    bail!("--keep requires a value");
                }
                keep = match args[i].as_str() {
                    "inside" => KeepMode::Inside,
                    "outside" => KeepMode::Outside,
                    other => bail!("unsupported --keep mode: {other}"),
                };
                i += 1;
            }
            "--opacity-min" => {
                i += 1;
                if i >= args.len() {
                    bail!("--opacity-min requires a value");
                }
                opacity_min = args[i]
                    .parse::<f32>()
                    .with_context(|| format!("invalid --opacity-min: {}", args[i]))?;
                i += 1;
            }
            s if s.starts_with('-') => {
                bail!("unknown option: {s}");
            }
            value => {
                if input.is_some() {
                    bail!("only one input path is allowed");
                }
                input = Some(value.to_string());
                i += 1;
            }
        }
    }

    let input = input.context("missing input path")?;
    let clipping_json = clipping_json.context("missing required option --clipping-json")?;

    Ok(CliArgs {
        input,
        output,
        clipping_json,
        output_format,
        keep,
        opacity_min,
        dry_run,
    })
}

fn main() -> anyhow::Result<()> {
    let args = parse_args()?;

    if args.opacity_min < 0.0 || args.opacity_min > 1.0 {
        bail!("--opacity-min must be in [0,1], got {}", args.opacity_min);
    }

    let clipping_raw = std::fs::read_to_string(&args.clipping_json)
        .with_context(|| format!("failed to read clipping json: {}", args.clipping_json))?;
    let clip: ClipShape = serde_json::from_str(&clipping_raw)
        .with_context(|| format!("failed to parse clipping json: {}", args.clipping_json))?;

    match &clip {
        ClipShape::Cylinder {
            radius,
            height,
            quaternion,
            ..
        } => {
            if *radius <= 0.0 {
                bail!("radius must be > 0");
            }
            if *height <= 0.0 {
                bail!("height must be > 0");
            }
            let q = Quat::from_array(*quaternion);
            if q.length_squared() <= 1.0e-12 {
                bail!("quaternion must be non-zero");
            }
        }
    }

    let mut decoder = MultiDecoder::new(GsplatArray::new(), None, Some(&args.input));
    read_file_chunks(&args.input, &mut decoder)
        .with_context(|| format!("failed to decode input: {}", args.input))?;

    let mut splats = decoder.into_splats();
    let total = splats.len();

    splats.retain(|splat| {
        if splat.opacity() < args.opacity_min {
            return false;
        }
        let inside = clip.contains(splat.center.into());
        match args.keep {
            KeepMode::Inside => inside,
            KeepMode::Outside => !inside,
        }
    });

    let kept = splats.len();
    let removed = total.saturating_sub(kept);

    println!("Input: {}", args.input);
    println!("Total splats: {total}");
    println!("Kept splats: {kept}");
    println!("Removed splats: {removed}");
    println!("Mode: {:?}", args.keep);

    if args.dry_run {
        println!("Dry-run enabled: no file written");
        return Ok(());
    }

    let output_path = args.output.unwrap_or_else(|| {
        derive_output_path(&PathBuf::from(&args.input), args.output_format)
    });

    match args.output_format {
        OutputFormat::Ply => {
            let encoder = PlyEncoder::new(splats);
            let bytes = encoder.encode().context("failed to encode PLY")?;
            let mut writer = BufWriter::new(
                File::create(&output_path)
                    .with_context(|| format!("failed to create output file: {output_path}"))?,
            );
            writer
                .write_all(&bytes)
                .with_context(|| format!("failed to write output file: {output_path}"))?;
            println!("Wrote {output_path} ({} bytes)", bytes.len());
        }
        OutputFormat::Spz => {
            let encoder = SpzEncoder::new(splats);
            let bytes = encoder.encode().context("failed to encode SPZ")?;
            let mut writer = BufWriter::new(
                File::create(&output_path)
                    .with_context(|| format!("failed to create output file: {output_path}"))?,
            );
            writer
                .write_all(&bytes)
                .with_context(|| format!("failed to write output file: {output_path}"))?;
            println!("Wrote {output_path} ({} bytes)", bytes.len());
        }
    }

    Ok(())
}
