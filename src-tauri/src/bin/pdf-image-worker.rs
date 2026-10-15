#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[path = "../form_worker.rs"]
mod form_appearances;
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType};
use pdfium_render::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    library: PathBuf,
    document: PathBuf,
    index: PathBuf,
    patch: PathBuf,
    directory: PathBuf,
    preset: String,
    #[serde(default)]
    quality: Option<u8>,
    #[serde(default)]
    dpi: Option<u32>,
}
#[derive(Clone)]
struct Candidate {
    key: String,
    dict: Value,
    bytes: u64,
}
fn display_size(a: f64, b: f64, c: f64, d: f64) -> Option<(f64, f64)> {
    let width = a.hypot(b);
    let height = c.hypot(d);
    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
        .then_some((width, height))
}
fn largest_placement(first: Option<(f64, f64)>, next: Option<(f64, f64)>) -> Option<(f64, f64)> {
    match (first, next) {
        (Some((w, h)), Some((nw, nh))) => Some((w.max(nw), h.max(nh))),
        _ => None,
    }
}
fn target_dimensions(
    width: u32,
    height: u32,
    display_width: f64,
    display_height: f64,
    dpi: u32,
) -> (u32, u32) {
    let scale = ((display_width * dpi as f64 / 72.0) / width as f64)
        .max((display_height * dpi as f64 / 72.0) / height as f64)
        .min(1.0);
    (
        ((width as f64 * scale).ceil() as u32).clamp(1, width),
        ((height as f64 * scale).ceil() as u32).clamp(1, height),
    )
}
fn standard_page_units(objects: &serde_json::Map<String, Value>) -> bool {
    objects.values().all(|object| {
        let dict = &object["value"];
        dict["/Type"] != "/Page"
            || dict
                .get("/UserUnit")
                .is_none_or(|unit| unit.as_f64() == Some(1.0))
    })
}
struct Optimizer<'a> {
    directory: &'a Path,
    candidates: HashMap<[u8; 32], Vec<Candidate>>,
    visited: HashSet<[u8; 32]>,
    replacements: serde_json::Map<String, Value>,
    quality: u8,
    dpi: u32,
    examined: usize,
    placements: HashMap<[u8; 32], Option<(f64, f64)>>,
}
impl Optimizer<'_> {
    fn collect_placements(
        &mut self,
        object: PdfPageObject<'_>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 16 || self.examined > 200000 {
            return Err("This PDF is too complex for image compression.".into());
        }
        self.examined += 1;
        if let Some(form) = object.as_x_object_form_object() {
            for child in form.iter() {
                self.collect_placements(child, depth + 1)?;
            }
        } else if let Some(image) = object.as_image_object() {
            if let Ok(raw) = image.get_raw_image_data() {
                if raw.len() <= 32 * 1024 * 1024 {
                    let hash: [u8; 32] = Sha256::digest(&raw).into();
                    let size = if depth == 0 {
                        image.matrix().ok().and_then(|matrix| {
                            display_size(
                                matrix.a() as f64,
                                matrix.b() as f64,
                                matrix.c() as f64,
                                matrix.d() as f64,
                            )
                        })
                    } else {
                        None
                    };
                    self.placements
                        .entry(hash)
                        .and_modify(|existing| *existing = largest_placement(*existing, size))
                        .or_insert(size);
                }
            }
        }
        Ok(())
    }
    fn visit(&mut self, object: PdfPageObject<'_>, depth: usize) -> Result<(), String> {
        if depth > 16 || self.examined > 200000 {
            return Err("This PDF is too complex for image compression.".into());
        }
        self.examined += 1;
        if let Some(form) = object.as_x_object_form_object() {
            for child in form.iter() {
                self.visit(child, depth + 1)?;
            }
        } else if let Some(image) = object.as_image_object() {
            let Ok(width) = image.width() else {
                return Ok(());
            };
            let Ok(height) = image.height() else {
                return Ok(());
            };
            if width <= 0 || height <= 0 || (width as u64) * (height as u64) > 32_000_000 {
                return Ok(());
            }
            let Ok(raw) = image.get_raw_image_data() else {
                return Ok(());
            };
            if raw.len() > 32 * 1024 * 1024 {
                return Ok(());
            }
            let hash: [u8; 32] = Sha256::digest(&raw).into();
            if !self.visited.insert(hash) {
                return Ok(());
            }
            let Some(candidates) = self.candidates.get(&hash) else {
                return Ok(());
            };
            let first = &candidates[0];
            // Same bytes with different colour/decode dictionaries are ambiguous: preserve them.
            if candidates.iter().any(|item| {
                ["/ColorSpace", "/Decode", "/Width", "/Height", "/Filter"]
                    .iter()
                    .any(|key| item.dict.get(*key) != first.dict.get(*key))
            }) {
                return Ok(());
            }
            if first.dict["/Width"].as_u64() != Some(width as u64)
                || first.dict["/Height"].as_u64() != Some(height as u64)
            {
                return Ok(());
            }
            // Every direct placement must be understood before changing a shared stream.
            // Nested form transforms and ambiguous stream identities remain unchanged.
            if depth > 0 || candidates.len() != 1 {
                return Ok(());
            }
            let Some(Some((display_width, display_height))) = self.placements.get(&hash).copied()
            else {
                return Ok(());
            };
            let (target_width, target_height) = target_dimensions(
                width as u32,
                height as u32,
                display_width,
                display_height,
                self.dpi,
            );
            let Ok(decoded) = image.get_raw_image() else {
                return Ok(());
            };
            if decoded.width() != width as u32 || decoded.height() != height as u32 {
                return Ok(());
            }
            if let Some(rgba) = decoded.as_rgba8() {
                if rgba.pixels().any(|pixel| pixel[3] != 255) {
                    return Ok(());
                }
            }
            let resized = decoded
                .resize_exact(target_width, target_height, FilterType::Lanczos3)
                .to_rgb8();
            let mut encoded = Vec::new();
            JpegEncoder::new_with_quality(&mut encoded, self.quality)
                .encode(
                    &resized,
                    target_width,
                    target_height,
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|_| "Could not encode a compressed image.")?;
            if encoded.len() as u64 + 64 >= first.bytes {
                return Ok(());
            }
            let path = self
                .directory
                .join(format!("compressed-image-{}.jpg", self.replacements.len()));
            fs::write(&path, encoded).map_err(|_| "Could not write a compressed image.")?;
            for item in candidates {
                let mut dict = item.dict.clone();
                let map = dict.as_object_mut().ok_or("Invalid image dictionary.")?;
                for key in ["/Decode", "/DecodeParms", "/Length", "/SMaskInData"] {
                    map.remove(key);
                }
                map.insert("/Width".into(), target_width.into());
                map.insert("/Height".into(), target_height.into());
                map.insert("/ColorSpace".into(), "/DeviceRGB".into());
                map.insert("/BitsPerComponent".into(), 8.into());
                map.insert("/Filter".into(), "/DCTDecode".into());
                self.replacements.insert(
                    item.key.clone(),
                    json!({ "stream": { "dict": dict, "datafile": path } }),
                );
            }
        }
        Ok(())
    }
}
fn process(config: Config) -> Result<usize, String> {
    let directory =
        fs::canonicalize(&config.directory).map_err(|_| "Image workspace unavailable.")?;
    if fs::metadata(&config.index)
        .map_err(|_| "Image index unavailable.")?
        .len()
        > 32 * 1024 * 1024
    {
        return Err("Image index exceeds the supported limit.".into());
    }
    let index: Value =
        serde_json::from_slice(&fs::read(&config.index).map_err(|_| "Image index unavailable.")?)
            .map_err(|_| "Invalid image index.")?;
    let objects = index
        .pointer("/qpdf/1")
        .and_then(Value::as_object)
        .ok_or("Invalid image index.")?;
    let mut candidates: HashMap<[u8; 32], Vec<Candidate>> = HashMap::new();
    for (key, object) in objects {
        let Some(stream) = object.get("stream") else {
            continue;
        };
        let Some(dict) = stream.get("dict") else {
            continue;
        };
        if dict["/Subtype"] != "/Image"
            || dict["/BitsPerComponent"] != 8
            || dict.get("/SMask").is_some()
            || dict.get("/Mask").is_some()
            || dict["/ImageMask"] == true
        {
            continue;
        }
        let Some(width) = dict["/Width"].as_u64() else {
            continue;
        };
        let Some(height) = dict["/Height"].as_u64() else {
            continue;
        };
        if width == 0 || height == 0 || width.saturating_mul(height) > 32_000_000 {
            continue;
        }
        let Some(path) = stream["datafile"].as_str() else {
            continue;
        };
        let data = fs::canonicalize(path).map_err(|_| "Image data unavailable.")?;
        if !data.starts_with(&directory) {
            return Err("Image data is outside the private workspace.".into());
        }
        let bytes = fs::metadata(&data)
            .map_err(|_| "Image data unavailable.")?
            .len();
        if !(8 * 1024..=32 * 1024 * 1024).contains(&bytes) {
            continue;
        }
        let raw = fs::read(&data).map_err(|_| "Image data unavailable.")?;
        let hash: [u8; 32] = Sha256::digest(&raw).into();
        candidates.entry(hash).or_default().push(Candidate {
            key: key.clone(),
            dict: dict.clone(),
            bytes,
        });
    }
    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(&config.library).map_err(|_| "Image engine could not start.")?,
    );
    let document = pdfium
        .load_pdf_from_file(&config.document, None)
        .map_err(|_| "Image engine could not read this PDF.")?;
    let (quality, dpi) = match config.preset.as_str() {
        "balanced" => (80, 240),
        "small" => (65, 150),
        _ => return Err("Invalid image quality setting.".into()),
    };
    let mut optimizer = Optimizer {
        directory: &directory,
        candidates,
        visited: HashSet::new(),
        replacements: serde_json::Map::new(),
        quality: config.quality.unwrap_or(quality),
        dpi: config.dpi.unwrap_or(dpi),
        examined: 0,
        placements: HashMap::new(),
    };
    // Unusual page units need a physical scale not exposed by the image matrix.
    // Preserve these images until every placement's scale can be established.
    if standard_page_units(objects) {
        for page in document.pages().iter() {
            for object in page.objects().iter() {
                optimizer.collect_placements(object, 0)?;
            }
        }
        optimizer.examined = 0;
        for page in document.pages().iter() {
            for object in page.objects().iter() {
                optimizer.visit(object, 0)?;
            }
        }
    }
    let count = optimizer.replacements.len();
    fs::write(
        config.patch,
        serde_json::to_vec(&json!({ "qpdf": [index["qpdf"][0].clone(), optimizer.replacements] }))
            .map_err(|_| "Could not prepare compressed images.")?,
    )
    .map_err(|_| "Could not write compressed images.")?;
    Ok(count)
}
fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() == 5 && args[1] == "--prepare-forms" {
        match form_appearances::prepare(Path::new(&args[2]), Path::new(&args[3]), Path::new(&args[4])) {
            Ok(()) => println!("Form appearances prepared"),
            Err(message) => { eprintln!("{message}"); std::process::exit(2); }
        }
        return;
    }
    if args.len() == 3 && args[1] == "--check-runtime" {
        match Pdfium::bind_to_library(PathBuf::from(&args[2])) {
            Ok(bindings) => {
                let _pdfium = Pdfium::new(bindings);
                println!("PDFium image engine ready");
            }
            Err(_) => std::process::exit(2),
        }
        return;
    }
    let result = (|| {
        let path = args.get(1).ok_or("Missing image operation.")?;
        let bytes = fs::read(path).map_err(|_| "Image operation unavailable.")?;
        let config = serde_json::from_slice(&bytes).map_err(|_| "Invalid image operation.")?;
        process(config)
    })();
    match result {
        Ok(images) => println!("{}", json!({ "images": images })),
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod placement_tests {
    use super::*;
    #[test]
    fn shared_images_use_largest_width_and_height_without_upscaling() {
        let size = largest_placement(Some((72.0, 144.0)), Some((288.0, 72.0))).unwrap();
        assert_eq!(
            target_dimensions(2400, 2400, size.0, size.1, 240),
            (960, 960)
        );
        assert_eq!(target_dimensions(300, 300, size.0, size.1, 240), (300, 300));
    }
    #[test]
    fn rotation_preserves_physical_axis_lengths() {
        let straight = display_size(288.0, 0.0, 0.0, 144.0).unwrap();
        let rotated = display_size(0.0, 288.0, -144.0, 0.0).unwrap();
        assert_eq!(straight, rotated);
        assert_eq!(
            target_dimensions(2400, 1200, rotated.0, rotated.1, 240),
            (960, 480)
        );
    }
    #[test]
    fn one_unknown_placement_keeps_the_shared_image_unchanged() {
        assert_eq!(largest_placement(Some((100.0, 100.0)), None), None);
        assert_eq!(largest_placement(None, Some((100.0, 100.0))), None);
        assert_eq!(display_size(f64::NAN, 0.0, 0.0, 100.0), None);
        assert_eq!(display_size(0.0, 0.0, 0.0, 100.0), None);
    }
    #[test]
    fn nonstandard_or_unknown_page_units_preserve_images() {
        for unit in [json!(2), json!(0), json!("9 0 R"), Value::Null] {
            let objects = json!({ "obj:1 0 R": {"value": {"/Type":"/Page", "/UserUnit":unit}} });
            assert!(!standard_page_units(objects.as_object().unwrap()));
        }
        let objects = json!({ "obj:1 0 R": {"value": {"/Type":"/Page"}}, "obj:2 0 R": {"value": {"/Type":"/Page", "/UserUnit":1}} });
        assert!(standard_page_units(objects.as_object().unwrap()));
    }
}
