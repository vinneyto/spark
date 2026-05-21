use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use glam::Vec3A;
use serde::Deserialize;
use spark_lib::{
    decoder::{ChunkReceiver, MultiDecoder},
    gsplat::GsplatArray,
    ply::PlyEncoder,
    spz::SpzEncoder,
};

#[derive(Debug, Deserialize)]
struct ClipConfig {
    input: String,
    output: Option<String>,
    output_format: Option<OutputFormat>,
    keep: Option<KeepMode>,
    opacity_min: Option<f32>,
    clip: ClipShape,
}

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
        center: [f32; 3],
        axis: [f32; 3],
        radius: f32,
        half_height: f32,
    },
}

impl ClipShape {
    fn contains(&self, point: Vec3A) -> bool {
        match self {
            ClipShape::Cylinder {
                center,
                axis,
                radius,
                half_height,
            } => {
                let center = Vec3A::from_array(*center);
                let axis_v = Vec3A::from_array(*axis);
                let axis_len = axis_v.length();
                if axis_len <= 1.0e-12 {
                    return false;
                }
                let axis_n = axis_v / axis_len;
                let v = point - center;
                let h = v.dot(axis_n);
                if h.abs() > *half_height {
                    return false;
                }
                let radial2 = v.length_squared() - h * h;
                radial2 <= radius * radius
            }
        }
    }
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
    eprintln!("Usage: build-clip [--dry-run] <config.json>");
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        return Ok(());
    }

    let mut dry_run = false;
    let mut config_path: Option<String> = None;

    for arg in args {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            s if s.starts_with('-') => {
                eprintln!("Unknown option: {s}");
                print_usage();
                return Ok(());
            }
            _ => {
                if config_path.is_some() {
                    eprintln!("Only one config path is allowed");
                    print_usage();
                    return Ok(());
                }
                config_path = Some(arg);
            }
        }
    }

    let config_path = config_path.context("missing config path")?;
    let config_raw = std::fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read config: {config_path}"))?;
    let config: ClipConfig = serde_json::from_str(&config_raw)
        .with_context(|| format!("failed to parse JSON config: {config_path}"))?;

    let keep_mode = config.keep.unwrap_or(KeepMode::Inside);
    let opacity_min = config.opacity_min.unwrap_or(0.0);
    if opacity_min < 0.0 || opacity_min > 1.0 {
        bail!("opacity_min must be in [0,1], got {opacity_min}");
    }

    match &config.clip {
        ClipShape::Cylinder {
            axis,
            radius,
            half_height,
            ..
        } => {
            let axis_len = Vec3A::from_array(*axis).length();
            if axis_len <= 1.0e-12 {
                bail!("clip.axis must be non-zero for cylinder");
            }
            if *radius <= 0.0 {
                bail!("clip.radius must be > 0");
            }
            if *half_height <= 0.0 {
                bail!("clip.half_height must be > 0");
            }
        }
    }

    let input_path = &config.input;
    let mut decoder = MultiDecoder::new(GsplatArray::new(), None, Some(input_path));
    read_file_chunks(input_path, &mut decoder)
        .with_context(|| format!("failed to decode input: {input_path}"))?;

    let mut splats = decoder.into_splats();
    let total = splats.len();

    splats.retain(|splat| {
        if splat.opacity() < opacity_min {
            return false;
        }
        let inside = config.clip.contains(splat.center);
        match keep_mode {
            KeepMode::Inside => inside,
            KeepMode::Outside => !inside,
        }
    });

    let kept = splats.len();
    let removed = total.saturating_sub(kept);

    println!("Input: {input_path}");
    println!("Total splats: {total}");
    println!("Kept splats: {kept}");
    println!("Removed splats: {removed}");
    println!("Mode: {:?}", keep_mode);

    if dry_run {
        println!("Dry-run enabled: no file written");
        return Ok(());
    }

    let output_format = config.output_format.unwrap_or(OutputFormat::Ply);
    let output_path = config.output.unwrap_or_else(|| {
        derive_output_path(&PathBuf::from(input_path), output_format)
    });

    match output_format {
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
