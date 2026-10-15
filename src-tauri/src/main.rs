#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod workspace;
mod image_import;
mod size_split;
mod destination;
mod diagnostics;
#[cfg(test)]
mod acceptance;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, Manager};
use tauri_plugin_updater::UpdaterExt;

const MAX_BYTES: u64 = 250 * 1024 * 1024;
const MAX_PREVIEW_CHUNK: u64 = 1024 * 1024;
const MAX_DOCUMENTS: usize = 100;
const MAX_PAGES: u32 = 10000;
const MAX_OUTPUTS: usize = 500;

#[derive(Clone)]
struct Selected {
    path: PathBuf,
    bytes: u64,
    pages: u32,
    digest: [u8; 32],
    source_digest: [u8; 32],
    working: Option<Arc<tempfile::NamedTempFile>>,
    features: Features,
    locked: bool,
}
impl Selected { fn working_path(&self) -> &Path { self.working.as_ref().map_or(self.path.as_path(), |file| file.path()) } }
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Features {
    forms: bool, signatures: bool, tags: bool, attachments: usize,
    needs_appearances: bool, unsupported_forms: bool,
    bookmarks: Vec<Bookmark>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Bookmark { title: String, page: Option<u32>, children: Vec<Bookmark> }

struct Store {
    workspace: workspace::Workspace,
    documents: Mutex<HashMap<String, Selected>>,
    dropped_paths: Mutex<HashSet<PathBuf>>,
    operation: Mutex<()>,
    job: Mutex<Option<(String, Arc<AtomicBool>)>>,
    results: Mutex<HashMap<String, SavedOutput>>,
    compression_preview: Mutex<Option<Arc<CompressionPreview>>>,
    pending_update: Mutex<Option<(String, tauri_plugin_updater::Update)>>,
}
impl Store {
    fn new() -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self { workspace: workspace::Workspace::new()?, documents: Mutex::default(), dropped_paths: Mutex::default(), operation: Mutex::default(), job: Mutex::default(), results: Mutex::default(), compression_preview: Mutex::default(), pending_update: Mutex::default() }))
    }
}
struct CompressionPreview {
    id: String,
    ids: Vec<String>,
    compression: Compression,
    target_bytes: Option<u64>,
    outputs: Vec<tempfile::NamedTempFile>,
    digests: Vec<[u8; 32]>,
    sizes: Vec<u64>,
    recovered: bool,
    // Keep the private directory alive until the cached outputs are released.
    _workspace: tempfile::TempDir,
}
#[derive(Clone)]
struct SavedOutput { path: PathBuf, files: Vec<PathBuf> }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentInfo {
    id: String,
    name: String,
    bytes: u64,
    pages: u32,
    preview_bytes: u64,
    locked: bool,
    features: Features,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    id: String,
    name: String,
    files: usize,
    input_bytes: u64,
    output_bytes: u64,
    file_bytes: Vec<u64>,
    recovered: bool,
}
#[derive(Clone, Serialize)]
struct Progress {
    message: String,
    cancellable: bool,
}
#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Merge,
    Compress,
    Split,
    Organize,
}
#[derive(Clone, Copy, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Compression {
    #[default]
    Lossless,
    Balanced,
    Small,
}
#[derive(Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum SplitMode {
    #[default]
    EveryPage,
    Groups,
    Ranges,
    FileSize,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageEdit {
    page: u32,
    rotation: u16,
    #[serde(default)]
    document_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    job_id: String,
    operation: Operation,
    ids: Vec<String>,
    #[serde(default)]
    compression: Compression,
    #[serde(default)]
    split_mode: SplitMode,
    #[serde(default)]
    pages_per_file: u32,
    #[serde(default)]
    ranges: String,
    #[serde(default)]
    pages: Vec<PageEdit>,
    #[serde(default)]
    measure_only: bool,
    #[serde(default)]
    preview_id: Option<String>,
    #[serde(default)] target_bytes: Option<u64>,
    #[serde(default)] max_output_bytes: Option<u64>,
    #[serde(default)] output_names: Vec<String>,
    #[serde(default)] flatten_forms: bool,
    #[serde(default)] acknowledge_changes: bool,
    #[serde(default)] merge_bookmarks: bool,
}
struct JobLease {
    store: Arc<Store>,
    id: String,
}
impl Drop for JobLease {
    fn drop(&mut self) {
        if let Ok(mut job) = self.store.job.lock() {
            if job.as_ref().is_some_and(|(id, _)| id == &self.id) {
                *job = None;
            }
        }
    }
}
fn cancelled(cancel: Option<&AtomicBool>) -> Result<(), String> {
    if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        Err("Cancelled. No output was saved.".into())
    } else {
        Ok(())
    }
}
fn progress(channel: &Channel<Progress>, message: impl Into<String>, cancellable: bool) {
    let message=message.into();
    diagnostics::stage(&message);
    let _ = channel.send(Progress {
        message,
        cancellable,
    });
}
fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
fn engine(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    if let Some(value) = std::env::var_os("PDF_TOOLS_QPDF") {
        let path = PathBuf::from(value);
        if path.is_absolute() && path.is_file() {
            return Ok(path);
        }
        return Err("PDF_TOOLS_QPDF must point to an absolute qpdf executable path.".into());
    }
    let filename = if cfg!(windows) { "qpdf.exe" } else { "qpdf" };
    let path = app
        .path()
        .resource_dir()
        .map_err(|_| "Cannot locate application resources.")?
        .join("engine")
        .join(filename);
    if !path.is_file() {
        return Err("The local PDF engine is missing. Install a complete copy of PDF Tools, or ask the developer to finish the engine bundle.".into());
    }
    Ok(path)
}
// No shell or persistent logs. Drain both pipes; retain only bounded captures.
fn run(command: &mut Command, cancel: Option<&AtomicBool>) -> Result<(i32, Vec<u8>), String> {
    cancelled(cancel)?;
    let action=diagnostics::action(command);
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child=command.spawn().map_err(|error| {
        diagnostics::command_result(action,&format!("start failed, OS error {:?}",error.raw_os_error()),&diagnostics::Capture{bytes:vec![],truncated:false});
        "Cannot start the local PDF engine."
    })?;
    let stdout=child.stdout.take().ok_or("Cannot read PDF engine results.")?;
    let stderr=child.stderr.take().ok_or("Cannot read PDF engine diagnostics.")?;
    let reader=thread::spawn(move || diagnostics::read_bounded(stdout,2*1024*1024));
    let errors=thread::spawn(move || diagnostics::read_bounded(stderr,64*1024));
    let started=Instant::now();
    let status=loop {
        let stopped=cancelled(cancel).is_err();
        if stopped || started.elapsed()>=Duration::from_secs(600) {
            let _=child.kill();let _=child.wait();let _=reader.join();
            if let Ok(Ok(capture))=errors.join(){diagnostics::command_result(action,if stopped {"cancelled"} else {"timed out after 600 seconds"},&capture);}
            return Err(if stopped {"Cancelled. No output was saved."} else {"PDF processing stopped after an engine error or a ten-minute time limit. Try a smaller document."}.into());
        }
        match child.try_wait() {
            Ok(Some(status))=>break status,
            Ok(None)=>thread::sleep(Duration::from_millis(50)),
            Err(error)=>{
                let _=child.kill();let _=child.wait();let _=reader.join();
                if let Ok(Ok(capture))=errors.join(){diagnostics::command_result(action,&format!("wait failed, OS error {:?}",error.raw_os_error()),&capture);}
                return Err("PDF processing stopped after an engine error. No output was saved.".into());
            }
        }
    };
    let capture=errors.join().ok().and_then(Result::ok).unwrap_or(diagnostics::Capture{bytes:vec![],truncated:true});
    let code=status.code().unwrap_or(-1);
    let mut result=format!("exit {code}, {} ms",started.elapsed().as_millis());
    #[cfg(unix)] {use std::os::unix::process::ExitStatusExt;if let Some(signal)=status.signal(){result.push_str(&format!(", signal {signal}"));}}
    diagnostics::command_result(action,&result,&capture);
    let output=reader.join().map_err(|_| "Cannot read PDF engine results.")?.map_err(|_| "Cannot read PDF engine results.")?;
    cancelled(cancel)?;
    Ok((code,output.bytes))
}
fn checked(command: &mut Command, cancel: Option<&AtomicBool>) -> Result<Vec<u8>, String> {
    let (code, output) = run(command, cancel)?;
    if code != 0 {
        return Err(if code == 3 {
            "The PDF could not pass validation. No output was saved."
        } else {
            "The PDF engine could not process this file. No output was saved."
        }
        .into());
    }
    Ok(output)
}
fn check_version(engine: &Path) -> Result<(), String> {
    let output = checked(Command::new(engine).arg("--version"), None)?;
    let text = String::from_utf8_lossy(&output);
    let version = text
        .split_whitespace()
        .nth(2)
        .ok_or("Cannot identify qpdf version.")?;
    let mut parts = version
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    diagnostics::engine_version(major,minor,parts.next().unwrap_or(0));
    if major < 12 || (major == 12 && minor < 1) {
        return Err("qpdf 12.1 or newer is required.".into());
    }
    Ok(())
}
fn digest(path: &Path, cancel: Option<&AtomicBool>) -> Result<[u8; 32], String> {
    let mut file = File::open(path).map_err(|_| "Cannot read an input document.")?;
    let mut hash = Sha256::new();
    let mut chunk = [0u8; 65536];
    let mut total = 0u64;
    loop {
        cancelled(cancel)?;
        let count = file
            .read(&mut chunk)
            .map_err(|_| "Cannot read an input document.")?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_BYTES {
            return Err("Choose a PDF no larger than 250 MB.".into());
        }
        hash.update(&chunk[..count]);
    }
    Ok(hash.finalize().into())
}
fn count_pages(engine: &Path, path: &Path, cancel: Option<&AtomicBool>) -> Result<u32, String> {
    let output = checked(Command::new(engine).arg("--show-npages").arg(path), cancel)?;
    String::from_utf8_lossy(&output)
        .trim()
        .parse::<u32>()
        .map_err(|_| "Cannot count PDF pages.".into())
}
fn inspect(engine: &Path, path: &Path, cancel: Option<&AtomicBool>) -> Result<Selected, String> {
    let path = fs::canonicalize(path).map_err(|_| "Cannot access this document.")?;
    let metadata = fs::metadata(&path).map_err(|_| "Cannot inspect this document.")?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err("Choose a PDF no larger than 250 MB.".into());
    }
    let mut signature = [0u8; 5];
    File::open(&path)
        .and_then(|mut file| file.read_exact(&mut signature))
        .map_err(|_| "Cannot read this document.")?;
    if &signature != b"%PDF-" {
        return Err("The selected file does not have a PDF header.".into());
    }
    let (encrypted, _) = run(
        Command::new(engine).arg("--is-encrypted").arg(&path),
        cancel,
    )?;
    if encrypted != 2 {
        return Err("Encrypted PDFs are not supported. Choose an unencrypted working copy.".into());
    }
    let features = inspect_features(engine, &path, cancel)?;
    let pages = count_pages(engine, &path, cancel)?;
    if pages == 0 || pages > MAX_PAGES {
        return Err("Choose a document with 1 to 10,000 pages.".into());
    }
    Ok(Selected {
        digest: digest(&path, cancel)?,
        source_digest: digest(&path, cancel)?,
        working: None, features, locked: false,
        path,
        bytes: metadata.len(),
        pages,
    })
}
fn inspect_json(engine: &Path, path: &Path, cancel: Option<&AtomicBool>) -> Result<serde_json::Value, String> {
    let json_file = tempfile::NamedTempFile::new_in(path.parent().ok_or("Cannot inspect PDF information.")?).map_err(|_| "Cannot inspect PDF information.")?;
    checked(Command::new(engine).args(["--json", "--json-stream-data=none"]).arg(path).arg(json_file.path()), cancel)?;
    if json_file.as_file().metadata().map_err(|_| "Cannot inspect PDF information.")?.len() > 64 * 1024 * 1024 { return Err("Document structure is too complex.".into()); }
    serde_json::from_reader(json_file.as_file()).map_err(|_| "Cannot inspect PDF information.".into())
}
fn bookmarks(values: &serde_json::Value) -> Vec<Bookmark> {
    values.as_array().map(|items| items.iter().map(|item| Bookmark {
        title: item["title"].as_str().unwrap_or("").to_owned(),
        page: item["destpageposfrom1"].as_u64().and_then(|n| u32::try_from(n).ok()),
        children: bookmarks(&item["kids"]),
    }).collect()).unwrap_or_default()
}
fn inspect_features(engine: &Path, path: &Path, cancel: Option<&AtomicBool>) -> Result<Features, String> {
    let json = inspect_json(engine, path, cancel)?;
    let objects = json.pointer("/qpdf/1").and_then(|v| v.as_object()).ok_or("Cannot inspect document structure.")?;
    let root = objects.get("trailer").and_then(|v| v.pointer("/value/~1Root")).and_then(|v| v.as_str()).ok_or("Cannot inspect document catalog.")?;
    let catalog = objects.get(&format!("obj:{root}")).and_then(|v|v.get("value")).ok_or("Cannot inspect document catalog.")?;
    let fields = json.pointer("/acroform/fields").and_then(|v| v.as_array()).ok_or("Cannot inspect forms.")?;
    let signatures = fields.iter().any(|field| field["fieldtype"] == "/Sig") || catalog.get("/Perms").is_some();
    Ok(Features {
        forms: !fields.is_empty(), signatures, tags: catalog.get("/StructTreeRoot").is_some(),
        attachments: json["attachments"].as_object().map_or(0, |items| items.len()),
        needs_appearances: json.pointer("/acroform/needappearances").and_then(|v| v.as_bool()).unwrap_or(false),
        unsupported_forms: catalog.pointer("/~1AcroForm/~1XFA").is_some() || objects.values().any(|obj| obj.pointer("/value/~1XFA").is_some()),
        bookmarks: bookmarks(&json["outlines"]),
    })
}
fn document_info(id: String, document: &Selected) -> DocumentInfo {
    DocumentInfo { id, name: name(&document.path), bytes: document.bytes, pages: document.pages,
        preview_bytes: document.working.as_ref().and_then(|file| file.as_file().metadata().ok()).map_or(document.bytes, |m| m.len()),
        locked: document.locked, features: document.features.clone() }
}
fn copy_into(source: &Path, destination: &File, cancel: Option<&AtomicBool>) -> Result<(), String> {
    copy_bytes(source, destination, cancel, Some(MAX_BYTES))
}
// Outputs can combine many valid inputs. The per-input import cap must not
// reject a verified merged document during navigation rewriting or saving.
fn copy_output(source: &Path, destination: &File, cancel: Option<&AtomicBool>) -> Result<(), String> {
    copy_bytes(source, destination, cancel, None)
}
fn copy_bytes(source: &Path, destination: &File, cancel: Option<&AtomicBool>, limit: Option<u64>) -> Result<(), String> {
    let mut input = File::open(source).map_err(|_| "Cannot read an input document.")?;
    let mut output = destination
        .try_clone()
        .map_err(|_| "Cannot open the working copy.")?;
    output
        .set_len(0)
        .map_err(|_| "Cannot prepare a working copy.")?;
    output
        .seek(SeekFrom::Start(0))
        .map_err(|_| "Cannot prepare a working copy.")?;
    let mut chunk = [0u8; 65536];
    let mut total = 0u64;
    loop {
        cancelled(cancel)?;
        let count = input
            .read(&mut chunk)
            .map_err(|_| "Cannot read an input document.")?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if limit.is_some_and(|limit| total > limit) {
            return Err("An input document exceeds 250 MB. No output was saved.".into());
        }
        output
            .write_all(&chunk[..count])
            .map_err(|_| "Cannot write a working copy. Check available disk space.")?;
    }
    output
        .sync_all()
        .map_err(|_| "Cannot finish writing a working copy.")?;
    Ok(())
}
fn parse_range(value: &str, total: u32) -> Result<Vec<u32>, String> {
    if value.trim().is_empty() {
        return Err("Enter a page range.".into());
    }
    let mut result = Vec::new();
    for segment in value.split(',') {
        let parts: Vec<&str> = segment.trim().split('-').map(str::trim).collect();
        if parts.is_empty()
            || parts.len() > 2
            || parts
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()))
        {
            return Err("Use page numbers or ranges such as 1-3,5.".into());
        }
        let first = parts[0]
            .parse::<u32>()
            .map_err(|_| "Invalid page number.")?;
        let last = parts
            .last()
            .unwrap()
            .parse::<u32>()
            .map_err(|_| "Invalid page number.")?;
        if first == 0 || last < first || last > total {
            return Err(format!(
                "Use page numbers between 1 and {total}, with ranges in ascending order."
            ));
        }
        if result.len() + (last - first + 1) as usize > MAX_PAGES as usize {
            return Err("Choose at most 10,000 pages per output.".into());
        }
        result.extend(first..=last);
    }
    Ok(result)
}
fn split_plan(request: &Request, total: u32) -> Result<Vec<Vec<u32>>, String> {
    let result = match request.split_mode {
        SplitMode::FileSize => vec![(1..=total).collect()],
        SplitMode::Ranges => {
            if request.ranges.len() > 20000 {
                return Err("The page range list is too long.".into());
            }
            let ranges: Vec<&str> = request.ranges.split(';').collect();
            if ranges.len() > MAX_OUTPUTS {
                return Err("Create at most 500 PDFs at once.".into());
            }
            ranges
                .into_iter()
                .map(|range| parse_range(range, total))
                .collect::<Result<Vec<_>, _>>()?
        }
        SplitMode::EveryPage | SplitMode::Groups => {
            let size = if matches!(request.split_mode, SplitMode::EveryPage) {
                1
            } else {
                request.pages_per_file
            };
            if size == 0 || size > total {
                return Err(format!("Use a group size between 1 and {total}."));
            }
            if total.div_ceil(size) as usize > MAX_OUTPUTS {
                return Err("Create at most 500 PDFs at once. Choose a larger group size.".into());
            }
            (1..=total)
                .step_by(size as usize)
                .map(|start| (start..=total.min(start + size - 1)).collect())
                .collect()
        }
    };
    if result.iter().map(Vec::len).sum::<usize>() > 20000 {
        return Err("Choose at most 20,000 output pages in total.".into());
    }
    Ok(result)
}
fn range_argument(pages: &[u32]) -> String {
    // Collapse consecutive pages to keep Windows command lines short.
    let mut chunks = Vec::new();
    let mut index = 0;
    while index < pages.len() {
        let first = pages[index];
        let mut last = first;
        while index + 1 < pages.len() && pages[index + 1] == last + 1 {
            index += 1;
            last = pages[index];
        }
        chunks.push(if first == last {
            first.to_string()
        } else {
            format!("{first}-{last}")
        });
        index += 1;
    }
    chunks.join(",")
}
// Only private snapshots reach this function. Repair compressed streams once,
// then require a clean check and the original page count before using the copy.
fn repair_snapshot_if_needed(
    engine: &Path, snapshot: &Path, expected: usize, cancel: &AtomicBool,
) -> Result<bool, String> {
    let (code, _) = run(Command::new(engine).arg("--check").arg(snapshot), Some(cancel))?;
    if code == 0 { return Ok(false); }
    if code != 3 { return Err("This PDF contains damaged data and could not be repaired. No output was saved.".into()); }
    let candidate = tempfile::Builder::new().suffix(".pdf")
        .tempfile_in(snapshot.parent().ok_or("Working folder unavailable.")?)
        .map_err(|_| "Cannot create a repair copy.")?;
    let (repair_code, _) = run(
        Command::new(engine).args(["--recompress-flate", "--compress-streams=y", "--object-streams=generate"])
            .arg(snapshot).arg(candidate.path()), Some(cancel),
    )?;
    if ![0, 3].contains(&repair_code) { return Err("This PDF could not be repaired. No output was saved.".into()); }
    validate_output(engine, candidate.path(), expected, cancel)
        .map_err(|error| if error.starts_with("Cancelled") { error } else { "This PDF could not be repaired safely. No output was saved.".into() })?;
    let destination = File::options().read(true).write(true).open(snapshot)
        .map_err(|_| "Cannot update the working copy.")?;
    copy_into(candidate.path(), &destination, Some(cancel))?;
    Ok(true)
}

fn validate_output(
    engine: &Path,
    output: &Path,
    expected: usize,
    cancel: &AtomicBool,
) -> Result<u64, String> {
    diagnostics::stage("Validate output");
    checked(
        Command::new(engine).arg("--check").arg(output),
        Some(cancel),
    )?;
    if count_pages(engine, output, Some(cancel))? as usize != expected {
        return Err("Output pages do not match the requested pages. No output was saved.".into());
    }
    let bytes = fs::metadata(output)
        .map_err(|_| "Cannot inspect output size.")?
        .len();
    if bytes == 0 {
        return Err("The PDF engine produced an empty file. No output was saved.".into());
    }
    Ok(bytes)
}
fn image_runtime(engine: &Path) -> Result<(PathBuf, PathBuf), String> {
    let directory = engine.parent().ok_or("Image engine unavailable.")?;
    let worker = directory.join(if cfg!(windows) {
        "pdf-image-worker.exe"
    } else {
        "pdf-image-worker"
    });
    let library = directory.join(if cfg!(windows) {
        "pdfium.dll"
    } else {
        "libpdfium.dylib"
    });
    if !worker.is_file() || !library.is_file() {
        return Err("The image engine is missing. Install a complete copy of PDF Tools.".into());
    }
    Ok((worker, library))
}
fn optimize_images(
    engine: &Path,
    output: &tempfile::NamedTempFile,
    workspace: &Path,
    compression: Compression,
    pages: usize,
    tuning: Option<(u8, u32)>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    diagnostics::stage("Compress images");
    let (worker, library) = image_runtime(engine)?;
    let directory = tempfile::Builder::new()
        .prefix("images-")
        .tempdir_in(workspace)
        .map_err(|_| "Cannot create an image workspace.")?;
    let index = directory.path().join("index.json");
    let mut prefix = std::ffi::OsString::from("--json-stream-prefix=");
    prefix.push(directory.path().join("stream"));
    checked(
        Command::new(engine)
            .args(["--json-output=2", "--json-stream-data=file"])
            .arg(prefix)
            .arg(output.path())
            .arg(&index),
        Some(cancel),
    )?;
    let patch = directory.path().join("patch.json");
    let config = directory.path().join("operation.json");
    let operation = serde_json::json!({
        "library": library, "document": output.path(), "index": index, "patch": patch,
        "quality": tuning.map(|t| t.0), "dpi": tuning.map(|t| t.1),
        "directory": directory.path(), "preset": if compression == Compression::Balanced { "balanced" } else { "small" },
    });
    fs::write(
        &config,
        serde_json::to_vec(&operation).map_err(|_| "Cannot prepare image compression.")?,
    )
    .map_err(|_| "Cannot prepare image compression.")?;
    let result = checked(Command::new(worker).arg(config), Some(cancel)).map_err(|error| {
        if error.starts_with("Cancelled") {
            error
        } else {
            "Image compression could not finish. No output was saved.".into()
        }
    })?;
    let stats: serde_json::Value =
        serde_json::from_slice(&result).map_err(|_| "Invalid image compression result.")?;
    let images = stats["images"]
        .as_u64()
        .ok_or("Invalid image compression result.")?;
    if images == 0 {
        return Ok(());
    }
    let candidate = tempfile::Builder::new()
        .suffix(".pdf")
        .tempfile_in(directory.path())
        .map_err(|_| "Cannot create compressed output.")?;
    let mut update = std::ffi::OsString::from("--update-from-json=");
    update.push(patch);
    checked(
        Command::new(engine)
            .arg(update)
            .args(["--object-streams=generate", "--compression-level=9"])
            .arg(output.path())
            .arg(candidate.path()),
        Some(cancel),
    )?;
    let size = validate_output(engine, candidate.path(), pages, cancel)?;
    if size
        < fs::metadata(output.path())
            .map_err(|_| "Cannot inspect output size.")?
            .len()
    {
        copy_into(candidate.path(), output.as_file(), Some(cancel))?;
    }
    Ok(())
}

fn navigation_documents(documents: &[Selected], filename_bookmarks: bool) -> Vec<Selected> {
    let mut navigation = documents.to_vec();
    if filename_bookmarks {
        for document in &mut navigation {
            let title = document.path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            document.features.bookmarks = vec![Bookmark { title, page: Some(1), children: document.features.bookmarks.clone() }];
        }
    }
    navigation
}

fn preserve_navigation(engine: &Path, output: &tempfile::NamedTempFile, documents: &[Selected], layout: &[(usize,u32,u16)], workspace: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let json = inspect_json(engine, output.path(), Some(cancel))?;
    let original = json.pointer("/qpdf/1").and_then(|v| v.as_object()).ok_or("Cannot check document navigation.")?;
    let root_ref = original["trailer"].pointer("/value/~1Root").and_then(|v| v.as_str()).ok_or("Cannot check document catalog.")?;
    let root_key = format!("obj:{root_ref}");
    let mut root = original[&root_key]["value"].clone();
    let identity = documents.len() == 1 && layout.len() == documents[0].pages as usize && layout.iter().enumerate().all(|(i,(d,p,_))| *d == 0 && *p == i as u32+1);
    let mut changes = serde_json::Map::new();
    if !identity {
        if let Some(root) = root.as_object_mut() { root.remove("/StructTreeRoot"); root.remove("/MarkInfo"); root.remove("/Metadata"); root.remove("/Dests"); }
        // Named destinations from a partial primary document can target discarded pages.
        if let Some(names_ref) = root.get("/Names").and_then(|v| v.as_str()) {
            let key = format!("obj:{names_ref}");
            if let Some(mut names) = original.get(&key).cloned() { if let Some(dict) = names["value"].as_object_mut() { dict.remove("/Dests"); changes.insert(key, names); } }
        }
        for page in json["pages"].as_array().ok_or("Cannot inspect output pages.")? {
            let key = format!("obj:{}",page["object"].as_str().ok_or("Invalid output page.")?);
            if let Some(mut obj) = original.get(&key).cloned() { if let Some(dict) = obj["value"].as_object_mut() { dict.remove("/StructParents"); changes.insert(key, obj); } }
        }
    }
    let page_refs: Vec<String> = json["pages"].as_array().ok_or("Cannot inspect output pages.")?.iter().map(|page| page["object"].as_str().unwrap_or("").to_string()).collect();
    #[derive(Clone)] struct Outline { title: String, destination: Option<String>, children: Vec<Outline> }
    fn map_bookmarks(items: &[Bookmark], document: usize, layout: &[(usize,u32,u16)], refs: &[String]) -> Vec<Outline> {
        items.iter().filter_map(|item| {
            let destination = item.page.and_then(|page| layout.iter().position(|(d,p,_)| *d == document && *p == page)).map(|i| refs[i].clone());
            let children = map_bookmarks(&item.children,document,layout,refs);
            if destination.is_none() && children.is_empty() { None } else { Some(Outline { title:item.title.clone(),destination,children }) }
        }).collect()
    }
    let outlines: Vec<_> = documents.iter().enumerate().flat_map(|(i,doc)| map_bookmarks(&doc.features.bookmarks,i,layout,&page_refs)).collect();
    let mut next = json.pointer("/qpdf/0/maxobjectid").and_then(|v|v.as_u64()).ok_or("Cannot inspect output objects.")? + 1;
    let outline_root = format!("{} 0 R",next); next+=1;
    fn emit(items: &[Outline], parent: &str, next: &mut u64, changes: &mut serde_json::Map<String,serde_json::Value>) -> Option<(String,String,usize)> {
        if items.is_empty() { return None; }
        let ids: Vec<_> = items.iter().map(|_| { let id=format!("{} 0 R",*next); *next+=1; id }).collect();
        let mut count=items.len();
        for (i,item) in items.iter().enumerate() {
            let mut obj=serde_json::json!({"/Title":format!("u:{}",item.title),"/Parent":parent});
            if i>0 { obj["/Prev"]=ids[i-1].clone().into(); } if i+1<ids.len() {obj["/Next"]=ids[i+1].clone().into();}
            if let Some(dest)=&item.destination { obj["/Dest"]=serde_json::json!([dest,"/Fit"]); }
            if let Some((first,last,children))=emit(&item.children,&ids[i],next,changes) { obj["/First"]=first.into(); obj["/Last"]=last.into(); obj["/Count"]=children.into(); count+=children; }
            changes.insert(format!("obj:{}",ids[i]),serde_json::json!({"value":obj}));
        }
        Some((ids[0].clone(),ids.last().unwrap().clone(),count))
    }
    if let Some((first,last,count))=emit(&outlines,&outline_root,&mut next,&mut changes) {
        changes.insert(format!("obj:{outline_root}"),serde_json::json!({"value":{"/Type":"/Outlines","/First":first,"/Last":last,"/Count":count}})); root["/Outlines"]=outline_root.into();
    } else if let Some(root)=root.as_object_mut() { root.remove("/Outlines"); }
    changes.insert(root_key,serde_json::json!({"value":root}));
    let patch=tempfile::Builder::new().suffix(".json").tempfile_in(workspace).map_err(|_| "Cannot preserve navigation.")?;
    serde_json::to_writer(patch.as_file(), &serde_json::json!({"qpdf":[{"jsonversion":2},changes]})).map_err(|_| "Cannot preserve navigation.")?;
    let candidate=tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace).map_err(|_| "Cannot preserve navigation.")?;
    let mut argument=std::ffi::OsString::from("--update-from-json="); argument.push(patch.path());
    checked(Command::new(engine).arg(argument).arg("--object-streams=generate").arg(output.path()).arg(candidate.path()),Some(cancel))?;
    validate_output(engine,candidate.path(),layout.len(),cancel)?;
    copy_output(candidate.path(),output.as_file(),Some(cancel))
}

fn record_result(
    store: &Store,
    path: PathBuf,
    output_paths: Vec<PathBuf>,
    files: usize,
    input_bytes: u64,
    output_bytes: u64,
    recovered: bool,
) -> Result<Saved, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let mut results = store
        .results
        .lock()
        .map_err(|_| "The PDF was saved, but its folder cannot be shown.")?;
    if results.len() >= 100 {
        results.clear();
    }
    let saved = Saved {
        id: id.clone(),
        name: name(&path),
        files,
        input_bytes,
        output_bytes,
        file_bytes: Vec::new(),
        recovered,
    };
    results.insert(id, SavedOutput { path, files: output_paths });
    Ok(saved)
}

#[tauri::command]
async fn engine_status(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = engine(&app)?;
        check_version(&engine)?;
        let (worker, library) = image_runtime(&engine)?;
        checked(
            Command::new(worker).arg("--check-runtime").arg(library),
            None,
        )
        .map_err(|_| "The image engine could not start. Install a complete copy of PDF Tools.")?;
        Ok(())
    })
    .await
    .map_err(|_| "Engine check failed.")?
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportResult {
    documents: Vec<DocumentInfo>,
    rejected: Vec<ImportFailure>,
}
#[derive(Serialize)]
struct ImportFailure { name: String, reason: String }

fn select_path(store: &Store, engine: &Path, path: &Path) -> Result<Selected, String> {
        let path = fs::canonicalize(path).map_err(|_| "Cannot access this file.")?;
        let file = tempfile::Builder::new().suffix(".pdf").tempfile_in(store.workspace.path()).map_err(|_| "Cannot create a working copy.")?;
        copy_into(&path, file.as_file(), None)?;
        let source_digest = digest(file.path(), None)?;
        if image_import::supported(&path) {
            let converted = tempfile::Builder::new().suffix(".pdf").tempfile_in(store.workspace.path()).map_err(|_| "Cannot create image page.")?;
            image_import::convert(file.path(), converted.path())?;
            let mut document = inspect(engine, converted.path(), None)?;
            document.path = path; document.bytes = file.as_file().metadata().map_err(|_| "Cannot inspect image.")?.len();
            document.source_digest = source_digest; document.working = Some(Arc::new(converted));
            return Ok(document);
        }
        let (encrypted, _) = run(Command::new(engine).arg("--is-encrypted").arg(file.path()), None)?;
        if encrypted == 0 {
            let (requires_password, _) = run(Command::new(engine).arg("--requires-password").arg(file.path()), None)?;
            if requires_password != 2 && requires_password != 3 {
                let bytes = file.as_file().metadata().map_err(|_| "Cannot inspect PDF.")?.len();
                return Ok(Selected { path, bytes, pages: 0, digest: source_digest, source_digest, working: Some(Arc::new(file)), features: Features::default(), locked: true });
            }
            let readable = tempfile::Builder::new().suffix(".pdf").tempfile_in(store.workspace.path()).map_err(|_| "Cannot open PDF.")?;
            checked(Command::new(engine).arg("--decrypt").arg(file.path()).arg(readable.path()), None)?;
            let mut document = inspect(engine, readable.path(), None)?;
            document.path = path; document.bytes = file.as_file().metadata().map_err(|_| "Cannot inspect PDF.")?.len();
            document.source_digest = source_digest; document.working = Some(Arc::new(readable));
            return Ok(document);
        }
        let mut document = inspect(engine, file.path(), None)?;
        document.path = path; document.working = Some(Arc::new(file));
        Ok(document)
}
fn select_paths(store: &Store, engine: &Path, paths: Vec<PathBuf>, allow_images: bool) -> Result<ImportResult, String> {
    if paths.len() > MAX_DOCUMENTS { return Err("Choose at most 100 files at once.".into()); }
    let mut result = ImportResult::default();
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let identity = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen.insert(identity) { continue; }
        let pdf = path.extension().is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("pdf"));
        let document = if !path.is_file() {
            Err("Choose a readable file, not a folder.".into())
        } else if !pdf && !(allow_images && image_import::supported(&path)) {
            Err(if allow_images { "Choose a PDF, JPEG, PNG, or WebP image." } else { "Choose a PDF file." }.into())
        } else { select_path(store, engine, &path) };
        match document {
            Ok(document) => selected.push(document),
            Err(reason) => result.rejected.push(ImportFailure { name: name(&path), reason }),
        }
    }
    let mut stored = store.documents.lock().map_err(|_| "Document selection is unavailable.")?;
    for document in selected {
        let existing = stored.iter().find(|(_, old)| old.path == document.path && old.digest == document.digest).map(|(id, _)| id.clone());
        if existing.is_none() && stored.len() >= MAX_DOCUMENTS {
            result.rejected.push(ImportFailure { name: name(&document.path), reason: "Clear unused documents before adding more than 100 files.".into() });
            continue;
        }
        let id = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        result.documents.push(document_info(id.clone(), &document));
        stored.insert(id, document);
    }
    Ok(result)
}
#[tauri::command]
async fn import_dropped_documents(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Store>>,
    paths: Vec<String>,
    allow_images: bool,
) -> Result<ImportResult, String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = store
            .operation
            .try_lock()
            .map_err(|_| "Another operation is in progress.")?;
        if paths.is_empty() || paths.len() > MAX_DOCUMENTS {
            return Err("Drop between 1 and 100 files.".into());
        }
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        {
            let mut allowed = store
                .dropped_paths
                .lock()
                .map_err(|_| "File drop is unavailable.")?;
            if paths.iter().any(|path| !allowed.contains(path)) {
                return Err("Drop the files into the app again.".into());
            }
            for path in &paths {
                allowed.remove(path);
            }
        }
        let engine = engine(&app)?;
        check_version(&engine)?;
        select_paths(&store, &engine, paths, allow_images)
    })
    .await
    .map_err(|_| "PDF import failed.")?
}

#[tauri::command]
async fn choose_documents(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Store>>,
    multiple: bool,
    allow_images: bool,
) -> Result<ImportResult, String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = store
            .operation
            .try_lock()
            .map_err(|_| "Another operation is in progress.")?;
        let engine = engine(&app)?;
        check_version(&engine)?;
        let dialog = if allow_images { rfd::FileDialog::new().add_filter("PDFs and images", &["pdf", "jpg", "jpeg", "png", "webp"]) } else { rfd::FileDialog::new().add_filter("PDF documents", &["pdf"]) };
        let paths = if multiple {
            dialog.pick_files()
        } else {
            dialog.pick_file().map(|path| vec![path])
        };
        let Some(paths) = paths else {
            return Ok(ImportResult::default());
        };
        select_paths(&store, &engine, paths, allow_images)
    })
    .await
    .map_err(|_| "Document selection failed.")?
}
#[tauri::command]
fn retain_documents(state: tauri::State<'_, Arc<Store>>, ids: Vec<String>) -> Result<(), String> {
    let _busy = state
        .operation
        .try_lock()
        .map_err(|_| "Another operation is in progress.")?;
    let keep: HashSet<_> = ids.into_iter().collect();
    state
        .documents
        .lock()
        .map_err(|_| "Document selection is unavailable.")?
        .retain(|id, _| keep.contains(id));
    Ok(())
}
#[tauri::command]
async fn unlock_document(app: tauri::AppHandle, state: tauri::State<'_, Arc<Store>>, id: String, password: String) -> Result<DocumentInfo, String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = store.operation.try_lock().map_err(|_| "Another operation is in progress.")?;
        if password.len() > 1024 || password.contains(['\n', '\r', '\0']) { return Err("Invalid password.".into()); }
        let document = store.documents.lock().map_err(|_| "Document unavailable.")?.get(&id).cloned().ok_or("Document unavailable.")?;
        if !document.locked { return Ok(document_info(id, &document)); }
        let engine = engine(&app)?;
        let mut secret = tempfile::NamedTempFile::new_in(store.workspace.path()).map_err(|_| "Cannot unlock PDF.")?;
        secret.write_all(password.as_bytes()).map_err(|_| "Cannot unlock PDF.")?;
        let output = tempfile::Builder::new().suffix(".pdf").tempfile_in(store.workspace.path()).map_err(|_| "Cannot unlock PDF.")?;
        let mut argument = std::ffi::OsString::from("--password-file="); argument.push(secret.path());
        checked(Command::new(&engine).arg(argument).arg("--decrypt").arg(document.working_path()).arg(output.path()), None).map_err(|_| "Incorrect password or unsupported encryption.")?;
        let mut unlocked = inspect(&engine, output.path(), None)?;
        unlocked.path = document.path; unlocked.bytes = document.bytes; unlocked.source_digest = document.source_digest;
        unlocked.working = Some(Arc::new(output));
        let info = document_info(id.clone(), &unlocked);
        store.documents.lock().map_err(|_| "Document unavailable.")?.insert(id, unlocked);
        Ok(info)
    }).await.map_err(|_| "Cannot unlock PDF.")?
}
#[tauri::command]
async fn read_document_range(state: tauri::State<'_, Arc<Store>>, id: String, begin: u64, end: u64, preview_id: Option<String>, output_index: Option<usize>) -> Result<tauri::ipc::Response, String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if end <= begin || end - begin > MAX_PREVIEW_CHUNK { return Err("Invalid preview range.".into()); }
        let mut file = if let Some(preview_id) = preview_id {
            let preview = store.compression_preview.lock().map_err(|_| "Preview unavailable.")?.as_ref().filter(|p| p.id == preview_id).cloned().ok_or("Preview expired.")?;
            preview.outputs.get(output_index.unwrap_or(0)).ok_or("Preview unavailable.")?.reopen().map_err(|_| "Preview unavailable.")?
        } else {
            let document = store.documents.lock().map_err(|_| "Document unavailable.")?.get(&id).cloned().ok_or("Document unavailable.")?;
            if document.locked { return Err("Unlock the PDF first.".into()); }
            File::open(document.working_path()).map_err(|_| "Preview unavailable.")?
        };
        if end > file.metadata().map_err(|_| "Preview unavailable.")?.len() { return Err("Invalid preview range.".into()); }
        file.seek(SeekFrom::Start(begin)).map_err(|_| "Preview unavailable.")?;
        let mut bytes = vec![0; (end - begin) as usize]; file.read_exact(&mut bytes).map_err(|_| "Preview unavailable.")?;
        Ok(tauri::ipc::Response::new(bytes))
    }).await.map_err(|_| "Preview unavailable.")?
}
#[tauri::command]
fn cancel_job(state: tauri::State<'_, Arc<Store>>, job_id: String) -> Result<(), String> {
    let job = state.job.lock().map_err(|_| "Job status is unavailable.")?;
    if let Some((id, cancel)) = job.as_ref() {
        if id == &job_id {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}
#[tauri::command]
fn discard_compression_preview(state: tauri::State<'_, Arc<Store>>) -> Result<(), String> {
    let _busy = state
        .operation
        .try_lock()
        .map_err(|_| "Another operation is in progress.")?;
    *state
        .compression_preview
        .lock()
        .map_err(|_| "Compression preview is unavailable.")? = None;
    Ok(())
}
fn make_size_part(engine: &Path, input: &Path, documents: &[Selected], first: u32, last: u32, workspace: &Path, cancel: &AtomicBool) -> Result<(tempfile::NamedTempFile, u64), String> {
    cancelled(Some(cancel))?;
    let output = tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace).map_err(|_| "Cannot create temporary output.")?;
    let path_text = |path: &Path| path.to_str().map(str::to_owned).ok_or_else(|| "Choose a folder with a valid Unicode name.".to_owned());
    let job = serde_json::json!({ "inputFile": path_text(input)?, "outputFile": path_text(output.path())?, "removeUnreferencedResources": "yes", "objectStreams": "generate", "pages": [{ "file": path_text(input)?, "range": format!("{first}-{last}") }] });
    let config = tempfile::Builder::new().suffix(".json").tempfile_in(workspace).map_err(|_| "Cannot prepare the PDF operation.")?;
    serde_json::to_writer(config.as_file(), &job).map_err(|_| "Cannot prepare the PDF operation.")?;
    let mut argument = std::ffi::OsString::from("--job-json-file="); argument.push(config.path());
    let (code, _) = run(Command::new(engine).arg(argument), Some(cancel))?;
    if code != 0 && code != 3 { return Err("Cannot split this PDF by file size.".into()); }
    validate_output(engine, output.path(), (last-first+1) as usize, cancel)?;
    let layout: Vec<_> = (first..=last).map(|page| (0,page,0)).collect();
    preserve_navigation(engine, &output, documents, &layout, workspace, cancel)?;
    let size = validate_output(engine, output.path(), (last-first+1) as usize, cancel)?;
    output.as_file().sync_all().map_err(|_| "Cannot finish writing the PDF.")?;
    Ok((output, size))
}
#[tauri::command]
async fn process_documents(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Store>>,
    request: Request,
    on_progress: Channel<Progress>,
) -> Result<Option<Saved>, diagnostics::Failure> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let operation=match request.operation {Operation::Merge=>"merge",Operation::Compress=>"compress",Operation::Split=>"split",Operation::Organize=>"organize"};
        let quality=match request.compression {Compression::Lossless=>"original quality",Compression::Balanced=>"balanced",Compression::Small=>"smaller"};
        let report=diagnostics::Session::begin(operation,quality,request.measure_only,request.target_bytes);
        let result=(|| -> Result<Option<Saved>,String> {
        let _busy = store.operation.try_lock().map_err(|_| "Another operation is in progress.")?;
        uuid::Uuid::parse_str(&request.job_id).map_err(|_| "Invalid job identifier.")?;
        let cancel = Arc::new(AtomicBool::new(false));
        *store.job.lock().map_err(|_| "Job status is unavailable.")? = Some((request.job_id.clone(), cancel.clone()));
        let _lease = JobLease { store: store.clone(), id: request.job_id.clone() };
        if request.measure_only {
            *store.compression_preview.lock().map_err(|_| "Compression preview is unavailable.")? = None;
        }
        if request.measure_only && request.operation != Operation::Compress {
            return Err("Only compression supports size measurement.".into());
        }
        match request.operation {
            Operation::Merge if (1..=MAX_DOCUMENTS).contains(&request.ids.len()) => (),
            Operation::Compress if (1..=MAX_DOCUMENTS).contains(&request.ids.len()) => (),
            Operation::Split if request.ids.len() == 1 => (),
            Operation::Organize if (1..=MAX_DOCUMENTS).contains(&request.ids.len()) => (),
            _ => return Err("Merge needs at least two PDFs. Split and Organize need one PDF.".into()),
        }
        if request.ids.iter().collect::<HashSet<_>>().len() != request.ids.len() { return Err("A document was selected twice.".into()); }
        let documents: Vec<Selected> = {
            let stored = store.documents.lock().map_err(|_| "Document selection is unavailable.")?;
            request.ids.iter().map(|id| stored.get(id).cloned().ok_or_else(|| "A selected document is no longer available.".to_string())).collect::<Result<_, _>>()?
        };
        if documents.iter().map(|doc| doc.pages as usize).sum::<usize>() > 20000 { return Err("Choose at most 20,000 input pages.".into()); }
        let multiple_outputs = request.operation == Operation::Split || (request.operation == Operation::Compress && documents.len() > 1);
        let mut plan = if request.operation == Operation::Split { split_plan(&request, documents[0].pages)? } else { Vec::new() };
        if matches!(request.split_mode, SplitMode::FileSize) && (request.operation != Operation::Split || !request.max_output_bytes.is_some_and(|limit| (100_000..=250_000_000).contains(&limit))) {
            return Err("Choose a file size limit between 0.1 and 250 MB.".into());
        }
        if let Some(target) = request.target_bytes {
            if request.operation != Operation::Compress || !(100_000..=MAX_BYTES).contains(&target) || request.compression == Compression::Lossless { return Err("Choose a target between 0.1 and 250 MB with image compression.".into()); }
        }
        if matches!(request.operation, Operation::Organize | Operation::Merge) && !request.pages.is_empty() {
            if request.pages.len() > 20000 { return Err("Choose at most 20,000 output pages.".into()); }
            for page in &request.pages {
                let index = request.ids.iter().position(|id| id == &page.document_id).unwrap_or(0);
                if (!page.document_id.is_empty() && !request.ids.contains(&page.document_id)) || page.page == 0 || page.page > documents[index].pages || ![0, 90, 180, 270].contains(&page.rotation) { return Err("Invalid page selection or rotation.".into()); }
            }
        } else if request.operation == Operation::Organize { return Err("Keep at least one page.".into()); }
        if documents.iter().any(|doc| doc.locked) { return Err("Unlock the PDFs first.".into()); }
        if documents.iter().any(|doc| doc.features.unsupported_forms) { return Err("XFA forms are unsupported. Use a flattened copy.".into()); }
        let page_operation = request.operation != Operation::Compress;
        if page_operation && documents.iter().any(|doc| doc.features.forms) && !request.flatten_forms { return Err("Select Flatten forms before saving.".into()); }
        if page_operation && documents.iter().any(|doc| doc.features.needs_appearances) { return Err("Save the filled form in its original application before flattening.".into()); }
        if documents.iter().any(|doc| doc.features.signatures) && !request.acknowledge_changes { return Err("Confirm digital signature removal before saving.".into()); }
        diagnostics::inputs(documents.iter().map(|document|(document.bytes,document.pages)));
        let engine = engine(&app)?; check_version(&engine)?;
        let suffix = match request.operation { Operation::Merge => "merged", Operation::Compress => "compressed", Operation::Split => "split", Operation::Organize => "organized" };
        let filename = if request.operation == Operation::Merge { "merged.pdf".into() } else { format!("{}-{suffix}.pdf", documents[0].path.file_stem().unwrap_or_default().to_string_lossy()) };
        if !request.measure_only {
            progress(&on_progress, if multiple_outputs { "Choose an output folder." } else { "Choose where to save a new PDF." }, false);
        }
        let destination = if request.measure_only { Some(std::env::temp_dir()) } else if multiple_outputs { rfd::FileDialog::new().pick_folder() } else {
            rfd::FileDialog::new().set_file_name(&filename).add_filter("PDF document", &["pdf"]).save_file()
        };
        let Some(mut destination) = destination else { return Ok(None); };
        if !request.measure_only && !multiple_outputs {
            if destination.extension().and_then(|value| value.to_str()).is_none_or(|value| !value.eq_ignore_ascii_case("pdf")) { destination.set_extension("pdf"); }
            if fs::symlink_metadata(&destination).is_ok() { return Err("Choose a new filename. Existing files are never overwritten.".into()); }
        }
        let parent = if request.measure_only || multiple_outputs { destination.as_path() } else { destination.parent().ok_or("Choose a valid output folder.")? };
        let selected_destination = destination::Destination::capture(parent)?;
        // Measurement uses private OS temporary storage; saving uses the destination filesystem.
        let workspace = store.workspace.job()?;
        let mut snapshots = Vec::new();
        let mut recovered_inputs = Vec::new();
        for (index, document) in documents.iter().enumerate() {
            progress(&on_progress, format!("Preparing document {} of {}...", index + 1, documents.len()), true);
            cancelled(Some(&cancel))?;
            let snapshot = tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace.path()).map_err(|_| "Cannot create a working copy.")?;
            copy_into(document.working_path(), snapshot.as_file(), Some(&cancel))?;
            let verified = inspect(&engine, snapshot.path(), Some(&cancel))?;
            if verified.digest != document.digest || verified.pages != document.pages || verified.bytes != fs::metadata(document.working_path()).map_err(|_| "Working copy unavailable.")?.len() { return Err("An input document changed. Remove it and add it again.".into()); }
            progress(&on_progress, format!("Checking document {} of {}...", index + 1, documents.len()), true);
            recovered_inputs.push(repair_snapshot_if_needed(&engine, snapshot.path(), document.pages as usize, &cancel)
                .map_err(|error| if error.starts_with("Cancelled") { error } else { format!("Document {}: {error}", index + 1) })?);
            if document.features.signatures || (page_operation && document.features.forms) {
                let prepared = tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace.path()).map_err(|_| "Cannot prepare forms.")?;
                let mut command = Command::new(&engine);
                if document.features.signatures { command.arg("--remove-restrictions"); }
                if page_operation && document.features.forms { command.arg("--flatten-annotations=all").arg("--remove-acroform"); }
                checked(command.arg(snapshot.path()).arg(prepared.path()), Some(&cancel))?;
                validate_output(&engine, prepared.path(), document.pages as usize, &cancel)?;
                copy_into(prepared.path(), snapshot.as_file(), Some(&cancel))?;
            }
            snapshots.push(snapshot);
        }
        let sized_parts = if request.operation == Operation::Split && matches!(request.split_mode, SplitMode::FileSize) {
            let parts = size_split::partition(documents[0].pages, request.max_output_bytes.unwrap(), MAX_OUTPUTS, |first, last| {
                progress(&on_progress, format!("Measuring pages {first}-{last}..."), true);
                make_size_part(&engine, snapshots[0].path(), &documents, first, last, workspace.path(), &cancel)
            })?;
            plan = parts.iter().map(|part| (part.first..=part.last).collect()).collect();
            parts
        } else { Vec::new() };
        let mut sized_parts = sized_parts.into_iter();
        let input_bytes = documents.iter().map(|doc| doc.bytes).sum::<u64>();
        let jobs = if request.operation == Operation::Split { plan.len() } else if multiple_outputs { documents.len() } else { 1 };
        let cached = if request.operation == Operation::Compress && !request.measure_only {
            if let Some(id) = &request.preview_id {
                let preview = store.compression_preview.lock().map_err(|_| "Compression preview is unavailable.")?
                    .as_ref().filter(|preview| &preview.id == id && preview.ids == request.ids && preview.compression == request.compression && preview.target_bytes == request.target_bytes)
                    .cloned().ok_or("Compression settings changed. Wait for the new size measurement.")?;
                Some(preview)
            } else { None }
        } else { None };
        let mut outputs = Vec::new(); let mut output_bytes = 0u64; let mut sizes = Vec::new();
        let mut recovered = recovered_inputs.iter().any(|repaired| *repaired) || cached.as_ref().is_some_and(|preview| preview.recovered);
        for index in 0..jobs {
            progress(&on_progress, if jobs == 1 { "Processing the PDF locally...".into() } else { format!("Creating PDF {} of {jobs}...", index + 1) }, true);
            if let Some(part) = sized_parts.next() {
                output_bytes += part.bytes; sizes.push(part.bytes); outputs.push(part.output);
                continue;
            }
            let output = tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace.path()).map_err(|_| "Cannot create temporary output.")?;
            if let Some(preview) = &cached {
                copy_into(preview.outputs[index].path(), output.as_file(), Some(&cancel))?;
                if digest(output.path(), Some(&cancel))? != preview.digests[index] {
                    return Err("The measured output changed. Check compression again.".into());
                }
                output_bytes += preview.sizes[index]; sizes.push(preview.sizes[index]); outputs.push(output);
                continue;
            }
            let path_text = |path: &Path| -> Result<String, String> {
                path.to_str().map(str::to_owned).ok_or_else(|| "Choose an output folder with a valid Unicode name.".into())
            };
            // QPDFJob JSON avoids Windows command-length limits for large page plans.
            // Drop unused page resources and pack structural objects without changing content.
            let mut job = serde_json::json!({ "outputFile": path_text(output.path())?, "removeUnreferencedResources": "yes", "objectStreams": "generate" });
            let expected = match request.operation {
                Operation::Merge | Operation::Organize => {
                    job["inputFile"] = path_text(snapshots[0].path())?.into();
                    let edits: Vec<(usize, u32, u16)> = if request.pages.is_empty() {
                        documents.iter().enumerate().flat_map(|(i, doc)| (1..=doc.pages).map(move |p| (i,p,0))).collect()
                    } else { request.pages.iter().map(|p| (request.ids.iter().position(|id| id == &p.document_id).unwrap_or(0), p.page, p.rotation)).collect() };
                    job["pages"] = edits.iter().map(|(i,p,_)| Ok(serde_json::json!({ "file": path_text(snapshots[*i].path())?, "range": p.to_string() }))).collect::<Result<Vec<_>, String>>()?.into();
                    let rotations: Vec<String> = [90,180,270].iter().filter_map(|r| { let positions: Vec<u32> = edits.iter().enumerate().filter(|(_,(_,_,rotation))| rotation == r).map(|(i,_)| i as u32+1).collect(); if positions.is_empty() { None } else { Some(format!("+{r}:{}",range_argument(&positions))) } }).collect();
                    if !rotations.is_empty() { job["rotate"] = serde_json::json!(rotations); }
                    if documents.len() > 1 { job["copyAttachmentsFrom"] = snapshots.iter().enumerate().skip(1).map(|(i,file)| Ok(serde_json::json!({"file": path_text(file.path())?, "prefix": format!("document-{}-",i+1)}))).collect::<Result<Vec<_>, String>>()?.into(); }
                    edits.len()
                }
                Operation::Compress => {
                    job["inputFile"] = path_text(snapshots[index].path())?.into();
                    job["compressStreams"] = "y".into();
                    job["recompressFlate"] = "".into();
                    job["compressionLevel"] = "9".into();
                    documents[index].pages as usize
                }
                Operation::Split => {
                    job["inputFile"] = path_text(snapshots[0].path())?.into();
                    job["pages"] = serde_json::json!([{ "file": path_text(snapshots[0].path())?, "range": range_argument(&plan[index]) }]);
                    plan[index].len()
                }
            };
            let config = tempfile::Builder::new().suffix(".json").tempfile_in(workspace.path()).map_err(|_| "Cannot prepare the PDF operation.")?;
            serde_json::to_writer(config.as_file(), &job).map_err(|_| "Cannot prepare the PDF operation.")?;
            config.as_file().sync_all().map_err(|_| "Cannot finish preparing the PDF operation.")?;
            let mut argument = std::ffi::OsString::from("--job-json-file="); argument.push(config.path());
            let mut command = Command::new(&engine); command.arg(argument);
            // qpdf exit 3 means warnings, not a failed operation. Accept the candidate
            // only if its independent, strict structure/stream check below succeeds.
            let (code, _) = run(&mut command, Some(&cancel))?;
            if ![0, 3].contains(&code) {
                return Err("The PDF engine could not process this file. No output was saved.".into());
            }
            progress(&on_progress, format!("Checking output {} of {jobs}...", index + 1), true);
            let mut size = validate_output(&engine, output.path(), expected, &cancel)?;
            if page_operation {
                let layout: Vec<(usize,u32,u16)> = if request.operation == Operation::Split { plan[index].iter().map(|p| (0,*p,0)).collect() }
                    else if request.pages.is_empty() { documents.iter().enumerate().flat_map(|(i,doc)| (1..=doc.pages).map(move |p| (i,p,0))).collect() }
                    else { request.pages.iter().map(|p| (request.ids.iter().position(|id| id == &p.document_id).unwrap_or(0),p.page,p.rotation)).collect() };
                let navigation = navigation_documents(&documents, request.operation == Operation::Merge && request.merge_bookmarks);
                preserve_navigation(&engine, &output, &navigation, &layout, workspace.path(), &cancel)?;
                size = validate_output(&engine, output.path(), expected, &cancel)?;
            }
            if request.operation == Operation::Compress && request.compression != Compression::Lossless {
                progress(&on_progress, format!("Compressing images in PDF {} of {jobs}...", index + 1), true);
                if let Some(target) = request.target_bytes {
                    let baseline = tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace.path()).map_err(|_| "Cannot prepare compression.")?;
                    copy_into(output.path(), baseline.as_file(), Some(&cancel))?;
                    for tuning in [(80,300),(78,240),(75,200),(70,150)] {
                        if size <= target { break; }
                        let attempt = tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace.path()).map_err(|_| "Cannot prepare compression.")?;
                        copy_into(baseline.path(), attempt.as_file(), Some(&cancel))?;
                        optimize_images(&engine, &attempt, workspace.path(), request.compression, expected, Some(tuning), &cancel)?;
                        let attempt_size = validate_output(&engine, attempt.path(), expected, &cancel)?;
                        if attempt_size < size { copy_into(attempt.path(), output.as_file(), Some(&cancel))?; size = attempt_size; }
                    }
                } else { optimize_images(&engine, &output, workspace.path(), request.compression, expected, None, &cancel)?; }
                size = validate_output(&engine, output.path(), expected, &cancel)?;
            }
            if request.operation == Operation::Compress && !documents[index].features.signatures && !recovered_inputs[index] && size >= fs::metadata(snapshots[index].path()).map_err(|_| "Working copy unavailable.")?.len() {
                copy_into(snapshots[index].path(), output.as_file(), Some(&cancel))?; size = fs::metadata(snapshots[index].path()).map_err(|_| "Working copy unavailable.")?.len();
            } else { recovered |= code == 3; }
            output.as_file().sync_all().map_err(|_| "Cannot finish writing the PDF.")?;
            output_bytes += size; sizes.push(size); outputs.push(output);
        }
        cancelled(Some(&cancel))?;
        if request.measure_only {
            let id = uuid::Uuid::new_v4().to_string();
            let digests = outputs.iter().map(|output| digest(output.path(), Some(&cancel))).collect::<Result<Vec<_>, _>>()?;
            cancelled(Some(&cancel))?;
            let measured = Saved { id: id.clone(), name: String::new(), files: jobs, input_bytes, output_bytes, file_bytes: sizes.clone(), recovered };
            *store.compression_preview.lock().map_err(|_| "Compression preview is unavailable.")? = Some(Arc::new(CompressionPreview {
                id, ids: request.ids, compression: request.compression, target_bytes: request.target_bytes, outputs, digests, sizes, recovered, _workspace: workspace,
            }));
            return Ok(Some(measured));
        }
        progress(&on_progress, "Saving verified output...", false);
        selected_destination.verify()?;
        // After this point, a short publication step completes without cancellation.
        if multiple_outputs {
            let prefix = if request.operation == Operation::Split { "Split PDFs-" } else { "Compressed PDFs-" };
            let folder = tempfile::Builder::new().prefix(prefix).tempdir_in(parent).map_err(|_| "Cannot create an output folder.")?;
            let mut output_paths = Vec::new();
            for (index, output) in outputs.into_iter().enumerate() {
                let filename = if request.operation == Operation::Compress {
                    let stem: String = documents[index].path.file_stem().unwrap_or_default().to_string_lossy().chars().take(50).collect();
                    format!("{:03}-{stem}-compressed.pdf", index + 1)
                } else { let fallback = format!("part-{:03}",index+1); let label = request.output_names.get(index).map_or(fallback.clone(), |s| workspace::safe_name(s, &fallback)); format!("{:03}-{label}.pdf",index+1) };
                let path = folder.path().join(filename);
                let staged = tempfile::Builder::new().suffix(".pdf").tempfile_in(folder.path()).map_err(|_| "Cannot save PDF.")?;
                copy_output(output.path(), staged.as_file(), None)?;
                staged.persist_noclobber(&path).map_err(|_| "Cannot save PDFs. No output folder was retained.")?;
                output_paths.push(path);
            }
            selected_destination.verify()?;
            let path = folder.keep();
            Ok(Some(record_result(&store, path, output_paths, jobs, input_bytes, output_bytes, recovered)?))
        } else {
            let output = outputs.pop().ok_or("No PDF was produced.")?;
            let staged = tempfile::Builder::new().prefix(".pdf-tools-save-").tempfile_in(parent).map_err(|_| "Cannot save PDF.")?;
            copy_output(output.path(), staged.as_file(), None)?;
            selected_destination.verify()?;
            staged.persist_noclobber(&destination).map_err(|_| "Cannot save the PDF. Choose a new filename and a writable folder.")?;
            Ok(Some(record_result(&store, destination.clone(), vec![destination], 1, input_bytes, output_bytes, recovered)?))
        }
        })();
        result.map_err(|message| report.fail(message))
    }).await.map_err(|_| diagnostics::unavailable())?
}
#[tauri::command]
async fn import_result_documents(app: tauri::AppHandle, state: tauri::State<'_, Arc<Store>>, result_id: String) -> Result<ImportResult, String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = store.operation.try_lock().map_err(|_| "Another operation is in progress.")?;
        let paths = store.results.lock().map_err(|_| "Saved output is unavailable.")?
            .get(&result_id).map(|result| result.files.clone()).ok_or("Saved output is no longer available in this session.")?;
        if paths.len() > MAX_DOCUMENTS { return Err("Add at most 100 output PDFs at a time using Add PDFs.".into()); }
        let engine = engine(&app)?;
        select_paths(&store, &engine, paths, false)
    }).await.map_err(|_| "Cannot open the saved output.")?
}
#[tauri::command]
async fn reveal_result(
    state: tauri::State<'_, Arc<Store>>,
    result_id: String,
) -> Result<(), String> {
    let path = state
        .results
        .lock()
        .map_err(|_| "Saved output is unavailable.")?
        .get(&result_id)
        .map(|result| result.path.clone())
        .ok_or("Saved output is no longer available in this session.")?;
    tauri::async_runtime::spawn_blocking(move || {
        if !path.exists() {
            return Err("The saved output was moved or removed.".into());
        }
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut value = Command::new("/usr/bin/open");
            if !path.is_dir() {
                value.arg("-R");
            }
            value.arg(&path);
            value
        };
        #[cfg(target_os = "windows")]
        let mut command = {
            let root = std::env::var_os("WINDIR").ok_or("Cannot locate Windows Explorer.")?;
            let mut value = Command::new(PathBuf::from(root).join("explorer.exe"));
            value.arg(if path.is_dir() {
                path.as_os_str().to_owned()
            } else {
                format!("/select,{}", path.display()).into()
            });
            value
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut command = {
            let mut value = Command::new("xdg-open");
            value.arg(if path.is_dir() {
                path.clone()
            } else {
                path.parent()
                    .ok_or("Cannot locate output folder.")?
                    .to_path_buf()
            });
            value
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "Cannot open the saved output folder.")?
            .wait()
            .map_err(|_| "Cannot open the saved output folder.")?;
        Ok(())
    })
    .await
    .map_err(|_| "Cannot show the saved output.")?
}
#[tauri::command]
async fn open_release_notes(app: tauri::AppHandle) -> Result<(), String> {
    // Only the installed version's release page can be opened, never a caller-supplied URL.
    let url = format!("https://github.com/el-jsn/offline-pdf-tools/releases/tag/v{}", app.package_info().version);
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        let mut command = { let mut command = Command::new("/usr/bin/open"); command.arg(&url); command };
        #[cfg(target_os = "windows")]
        let mut command = { let mut command = Command::new("cmd.exe"); command.args(["/C", "start", "", &url]); command };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut command = { let mut command = Command::new("xdg-open"); command.arg(&url); command };
        let status = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .status().map_err(|_| "Cannot open release notes in your browser.")?;
        if !status.success() { return Err("Cannot open release notes in your browser.".into()); }
        Ok(())
    }).await.map_err(|_| "Cannot open release notes in your browser.")?
}

#[tauri::command]
fn set_view_title(window: tauri::Window, tool: String) -> Result<(), String> {
    let title = match tool.as_str() {
        "merge" => "Merge PDFs",
        "compress" => "Compress PDF",
        "split" => "Split PDF",
        "organize" => "Organize pages",
        "compare" => "Compare PDFs",
        _ => return Err("Unknown PDF tool.".into()),
    };
    window
        .set_title(title)
        .map_err(|_| "Cannot update the window title.".into())
}

#[tauri::command]
async fn confirm_app_update(version: String) -> Result<bool, String> {
    if version.len() > 64
        || !version
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || b".-+".contains(&value))
    {
        return Err("Invalid update version.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        rfd::MessageDialog::new()
            .set_title("Install update")
            .set_description(format!(
                "Install PDF Tools {version}? The app will restart and clear unsaved selections."
            ))
            .set_buttons(rfd::MessageButtons::OkCancel)
            .set_level(rfd::MessageLevel::Info)
            .show()
            == rfd::MessageDialogResult::Ok
    })
    .await
    .map_err(|_| "Could not confirm the update.".into())
}
#[derive(Serialize)]
struct UpdateInfo {
    id: String,
    version: String,
}
#[tauri::command]
async fn check_app_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Store>>,
) -> Result<Option<UpdateInfo>, String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Network checks do not hold the PDF operation lock.
        *store
            .pending_update
            .lock()
            .map_err(|_| "Update status is unavailable.")? = None;
        let updater = app
            .updater_builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| "Updates are unavailable.")?;
        let update = tauri::async_runtime::block_on(updater.check()).map_err(|error| {
            match error {
                tauri_plugin_updater::Error::ReleaseNotFound => "No update release could be found.",
                tauri_plugin_updater::Error::TargetNotFound(_)
                | tauri_plugin_updater::Error::TargetsNotFound(_) => {
                    "No update has been published for this computer."
                }
                tauri_plugin_updater::Error::Reqwest(_) => {
                    "The update service could not be reached. Try again later."
                }
                _ => "The update release could not be read. Try again later.",
            }
            .to_string()
        })?;
        if let Some(mut update) = update {
            update.timeout = Some(Duration::from_secs(1200));
            let info = UpdateInfo {
                id: uuid::Uuid::new_v4().to_string(),
                version: update.version.clone(),
            };
            *store
                .pending_update
                .lock()
                .map_err(|_| "Update status is unavailable.")? = Some((info.id.clone(), update));
            Ok(Some(info))
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|_| "Update check failed.")?
}
#[tauri::command]
async fn install_app_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Store>>,
    update_id: String,
    on_progress: Channel<Progress>,
) -> Result<(), String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = store
            .operation
            .try_lock()
            .map_err(|_| "Finish the current operation before installing an update.")?;
        let update = {
            let mut pending = store
                .pending_update
                .lock()
                .map_err(|_| "Update status is unavailable.")?;
            if pending.as_ref().is_none_or(|(id, _)| id != &update_id) {
                return Err("Check for updates again before installing.".into());
            }
            pending.take().ok_or("No update is available.")?.1
        };
        progress(&on_progress, "Downloading update...", false);
        let mut downloaded = 0u64;
        tauri::async_runtime::block_on(update.download_and_install(
            |chunk, total| {
                downloaded += chunk as u64;
                if let Some(total) = total.filter(|total| *total > 0) {
                    progress(
                        &on_progress,
                        format!(
                            "Downloading update: {}%",
                            (downloaded.saturating_mul(100) / total).min(100)
                        ),
                        false,
                    );
                }
            },
            || progress(&on_progress, "Verifying and installing update...", false),
        ))
        .map_err(|_| "Update could not be installed. Check for updates and try again.")?;
        progress(&on_progress, "Restarting...", false);
        app.restart();
    })
    .await
    .map_err(|_| "Update installation failed.")?
}

#[tauri::command]
fn set_app_icon_appearance(app: tauri::AppHandle, dark: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let bytes: &'static [u8] = if dark {
            include_bytes!("../icons/icon-dark.png")
        } else {
            include_bytes!("../icons/icon.png")
        };
        app.run_on_main_thread(move || {
            use objc2::{AllocAnyThread, MainThreadMarker};
            use objc2_app_kit::{NSApplication, NSImage};
            use objc2_foundation::NSData;
            if let Some(main_thread) = MainThreadMarker::new() {
                let data = NSData::with_bytes(bytes);
                if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
                    let application = NSApplication::sharedApplication(main_thread);
                    // AppKit requires application icon changes on its main thread.
                    unsafe { application.setApplicationIconImage(Some(&image)); }
                }
            }
        }).map_err(|_| "Cannot update the app icon.".to_owned())?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, dark);
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Store::new().expect("Cannot initialize private storage"))
        // Tauri routes the main window's webview drops through WindowEvent.
        .on_window_event(|window, event| {
            #[cfg(target_os = "macos")]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" && window.hide().is_ok() {
                    api.prevent_close();
                }
            }
            if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                let state = window.state::<Arc<Store>>();
                if let Ok(mut allowed) = state.dropped_paths.lock() {
                    allowed.clear();
                    if paths.len() <= MAX_DOCUMENTS {
                        allowed.extend(paths.iter().cloned());
                    }
                };
            }
        })
        .invoke_handler(tauri::generate_handler![
            engine_status,
            choose_documents,
            import_dropped_documents,
            retain_documents,
            read_document_range,
            unlock_document,
            cancel_job,
            process_documents,
            discard_compression_preview,
            reveal_result,
            import_result_documents,
            open_release_notes,
            set_app_icon_appearance,
            set_view_title,
            check_app_update,
            install_app_update,
            confirm_app_update
        ])
        .build(tauri::generate_context!())
        .expect("Cannot start PDF Tools")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { has_visible_windows: false, .. } = &event {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            if matches!(event, tauri::RunEvent::Exit) {
                let state = app.state::<Arc<Store>>();
                if let Ok(mut docs) = state.documents.lock() { docs.clear(); }
                state.workspace.cleanup();
                if let Ok(mut preview) = state.compression_preview.lock() {
                    *preview = None;
                };
            }
        });
}
#[cfg(test)]
mod stream_repair_tests {
    use super::*;
    const CONTENT: &[u8] = b"BT /F1 12 Tf 72 720 Td (Synthetic credit report fixture) Tj ET\n";
    const DAMAGED: &[u8] = &[120,156,114,10,81,208,119,51,84,48,52,82,8,73,83,48,55,2,34,3,133,144,20,5,141,224,202,188,146,140,212,146,204,100,133,228,162,212,148,204,18,133,162,212,130,252,162,18,133,180,204,138,146,210,162,84,77,133,144,44,5,215,16,46,0,0,0,0,255,255];
    const VALID: &[u8] = &[120,156,115,10,81,208,119,51,84,48,52,82,8,73,83,48,55,2,34,3,133,144,20,5,141,224,202,188,146,140,212,146,204,100,133,228,162,212,148,204,18,133,162,212,130,252,162,18,133,180,204,138,146,210,162,84,77,133,144,44,5,215,16,46,0,51,230,19,31];
    fn engine_path() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf") }
    fn fixture(stream: &[u8], path: &Path) {
        let objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            [format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", stream.len()).into_bytes(), stream.to_vec(), b"\nendstream".to_vec()].concat(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec(); let mut offsets = vec![0];
        for (i, object) in objects.iter().enumerate() { offsets.push(pdf.len()); pdf.extend_from_slice(format!("{} 0 obj\n", i+1).as_bytes()); pdf.extend(object); pdf.extend_from_slice(b"\nendobj\n"); }
        let start = pdf.len(); pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len()+1).as_bytes());
        for offset in offsets.iter().skip(1) { pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes()); }
        pdf.extend_from_slice(format!("trailer\n<< /Root 1 0 R /Size {} >>\nstartxref\n{}\n%%EOF\n", objects.len()+1, start).as_bytes()); fs::write(path,pdf).unwrap();
    }
    #[test]
    fn repairs_unfinished_flate_and_preserves_content() {
        let directory = tempfile::tempdir().unwrap(); let path = directory.path().join("working.pdf"); fixture(DAMAGED, &path);
        let engine=engine_path(); let cancel=AtomicBool::new(false);
        assert_eq!(run(Command::new(&engine).arg("--check").arg(&path),Some(&cancel)).unwrap().0,3);
        assert!(repair_snapshot_if_needed(&engine,&path,1,&cancel).unwrap());
        assert_eq!(run(Command::new(&engine).arg("--check").arg(&path),Some(&cancel)).unwrap().0,0);
        let json: serde_json::Value=serde_json::from_slice(&checked(Command::new(&engine).arg("--json").arg(&path),Some(&cancel)).unwrap()).unwrap();
        let object=json["pages"][0]["contents"][0].as_str().unwrap().split_whitespace().next().unwrap();
        let decoded=checked(Command::new(&engine).arg(format!("--show-object={object}")).arg("--filtered-stream-data").arg(&path),Some(&cancel)).unwrap();
        assert_eq!(decoded,CONTENT);
    }
    #[test]
    fn clean_snapshot_remains_byte_identical() {
        let directory=tempfile::tempdir().unwrap(); let path=directory.path().join("clean.pdf");fixture(VALID,&path);let before=fs::read(&path).unwrap();
        assert!(!repair_snapshot_if_needed(&engine_path(),&path,1,&AtomicBool::new(false)).unwrap());assert_eq!(before,fs::read(path).unwrap());
    }
    #[test]
    fn failed_validation_leaves_snapshot_unchanged() {
        let directory=tempfile::tempdir().unwrap();let path=directory.path().join("working.pdf");fixture(DAMAGED,&path);let before=fs::read(&path).unwrap();
        assert!(repair_snapshot_if_needed(&engine_path(),&path,2,&AtomicBool::new(false)).is_err());assert_eq!(before,fs::read(path).unwrap());
    }
    #[test]
    fn cancellation_leaves_snapshot_unchanged() {
        let directory=tempfile::tempdir().unwrap();let path=directory.path().join("working.pdf");fixture(DAMAGED,&path);let before=fs::read(&path).unwrap();
        let result=repair_snapshot_if_needed(&engine_path(),&path,1,&AtomicBool::new(true));assert!(result.unwrap_err().starts_with("Cancelled"));assert_eq!(before,fs::read(path).unwrap());
    }
    #[test]
    fn fatal_stream_damage_is_rejected() {
        let directory=tempfile::tempdir().unwrap();let path=directory.path().join("working.pdf");fixture(b"not-a-compressed-stream",&path);let before=fs::read(&path).unwrap();
        assert!(repair_snapshot_if_needed(&engine_path(),&path,1,&AtomicBool::new(false)).is_err());assert_eq!(before,fs::read(path).unwrap());
    }
    #[test]
    fn repaired_snapshot_works_in_each_pdf_operation() {
        let directory=tempfile::tempdir().unwrap();let input=directory.path().join("working.pdf");fixture(DAMAGED,&input);let engine=engine_path();let cancel=AtomicBool::new(false);
        repair_snapshot_if_needed(&engine,&input,1,&cancel).unwrap();
        let source=input.to_str().unwrap();
        for (index, (args, pages)) in [
            (vec!["--empty","--pages",source,"1-z",source,"1","--"],2),
            (vec!["--empty","--pages",source,"1","--"],1),
            (vec!["--rotate=+90:1","--empty","--pages",source,"1","--"],1),
            (vec!["--recompress-flate","--compress-streams=y",source],1),
        ].iter().enumerate() {
            let output=directory.path().join(format!("result-{index}.pdf"));checked(Command::new(&engine).args(args).arg(&output),Some(&cancel)).unwrap();validate_output(&engine,&output,*pages,&cancel).unwrap();
        }
    }
    #[test]
    #[ignore = "Requires an explicitly supplied local PDF; never stores it in the repository"]
    fn local_document_repair() {
        let original=PathBuf::from(std::env::var_os("PDF_TOOLS_LOCAL_CHECK").expect("Supply a local PDF path"));
        let engine=engine_path(); let cancel=AtomicBool::new(false);
        let before=digest(&original,Some(&cancel)).unwrap(); let pages=count_pages(&engine,&original,Some(&cancel)).unwrap();
        let directory=tempfile::tempdir().unwrap(); let working=directory.path().join("working.pdf");fs::copy(&original,&working).unwrap();
        assert!(repair_snapshot_if_needed(&engine,&working,pages as usize,&cancel).unwrap());
        let output=directory.path().join("merged.pdf");
        checked(Command::new(&engine).args(["--empty","--pages"]).arg(&working).arg("1-z").arg(&working).args(["1","--"]).arg(&output),Some(&cancel)).unwrap();
        validate_output(&engine,&output,pages as usize+1,&cancel).unwrap();
        assert_eq!(before,digest(&original,Some(&cancel)).unwrap());
    }

    #[test]
    fn encrypted_pdf_without_open_password_is_readable() {
        let directory=tempfile::tempdir().unwrap(); let input=directory.path().join("input.pdf"); let encrypted=directory.path().join("encrypted.pdf");fixture(VALID,&input);let engine=engine_path();
        checked(Command::new(&engine).args(["--encrypt","","synthetic-owner-password","256","--"]).arg(&input).arg(&encrypted),None).unwrap();
        let store=Store::new().unwrap();let selected=select_paths(&store,&engine,vec![encrypted],false).unwrap().documents;
        assert_eq!(selected.len(),1);assert!(!selected[0].locked);assert_eq!(selected[0].pages,1);
    }
    #[test]
    fn encrypted_pdf_requiring_password_stays_locked() {
        let directory=tempfile::tempdir().unwrap();let input=directory.path().join("input.pdf");let encrypted=directory.path().join("encrypted.pdf");fixture(VALID,&input);let engine=engine_path();
        checked(Command::new(&engine).args(["--encrypt","synthetic-open-password","synthetic-owner-password","256","--"]).arg(&input).arg(&encrypted),None).unwrap();
        let store=Store::new().unwrap();let selected=select_paths(&store,&engine,vec![encrypted],false).unwrap().documents;assert!(selected[0].locked);
    }
    #[test]
    fn navigation_remaps_bookmarks_after_page_reordering() {
        let directory=tempfile::tempdir().unwrap();let input=directory.path().join("input.pdf");fixture(VALID,&input);let engine=engine_path();let cancel=AtomicBool::new(false);
        let source=directory.path().join("two-pages.pdf");checked(Command::new(&engine).args(["--empty","--pages"]).arg(&input).args(["1,1","--"]).arg(&source),Some(&cancel)).unwrap();
        let mut document=inspect(&engine,&source,Some(&cancel)).unwrap();document.features.bookmarks=vec![Bookmark {title:"Income".into(),page:Some(1),children:vec![]},Bookmark {title:"Statements".into(),page:Some(2),children:vec![]}];
        let output=tempfile::Builder::new().suffix(".pdf").tempfile_in(directory.path()).unwrap();
        checked(Command::new(&engine).args(["--empty","--pages"]).arg(&source).args(["2,1","--"]).arg(output.path()),Some(&cancel)).unwrap();
        preserve_navigation(&engine,&output,&[document],&[(0,2,0),(0,1,0)],directory.path(),&cancel).unwrap();
        let result=inspect_features(&engine,output.path(),Some(&cancel)).unwrap();assert_eq!(result.bookmarks.len(),2);assert_eq!(result.bookmarks[0].page,Some(2));assert_eq!(result.bookmarks[1].page,Some(1));
    }
    #[test]
    fn filename_bookmarks_preserve_children_and_map_document_starts() {
        let directory = tempfile::tempdir().unwrap(); let engine = engine_path(); let cancel = AtomicBool::new(false);
        let first = directory.path().join("Income.pdf"); let second = directory.path().join("Statements.pdf");
        fixture(VALID, &first); fixture(VALID, &second);
        let mut documents = vec![inspect(&engine,&first,None).unwrap(),inspect(&engine,&second,None).unwrap()];
        documents[0].features.bookmarks = vec![Bookmark {title:"Existing section".into(),page:Some(1),children:vec![]}];
        let before = digest(&first,None).unwrap();
        let output = tempfile::Builder::new().suffix(".pdf").tempfile_in(directory.path()).unwrap();
        checked(Command::new(&engine).args(["--empty","--pages"]).arg(&first).arg("1").arg(&second).args(["1","--"]).arg(output.path()),None).unwrap();
        let navigation = navigation_documents(&documents,true);
        preserve_navigation(&engine,&output,&navigation,&[(0,1,0),(1,1,0)],directory.path(),&cancel).unwrap();
        let result = inspect_features(&engine,output.path(),None).unwrap();
        assert_eq!(result.bookmarks.len(),2); assert_eq!(result.bookmarks[0].title,"Income"); assert_eq!(result.bookmarks[1].title,"Statements");
        assert_eq!(result.bookmarks[1].page,Some(2)); assert_eq!(result.bookmarks[0].children[0].title,"Existing section");
        assert_eq!(documents[0].features.bookmarks[0].title,"Existing section"); assert_eq!(digest(&first,None).unwrap(),before);
    }
    #[test]
    fn saved_output_continuation_records_only_published_paths() {
        let directory = tempfile::tempdir().unwrap(); let store = Store::new().unwrap();
        let first = directory.path().join("001-first.pdf"); let unrelated = directory.path().join("unrelated.pdf");
        fixture(VALID,&first); fixture(VALID,&unrelated);
        let saved = record_result(&store,directory.path().to_owned(),vec![first.clone()],1,100,100,false).unwrap();
        let files = store.results.lock().unwrap().get(&saved.id).unwrap().files.clone();
        let result = select_paths(&store,&engine_path(),files,false).unwrap();
        assert_eq!(result.documents.len(),1); assert_eq!(result.documents[0].name,"001-first.pdf");
    }
    #[test]
    fn another_instance_does_not_delete_live_working_files() {
        let first=workspace::Workspace::new().unwrap();let marker=first.path().join("synthetic-marker");fs::write(&marker,b"fixture").unwrap();
        let second=workspace::Workspace::new().unwrap();assert!(marker.is_file());drop(second);assert!(marker.is_file());
    }

}

#[cfg(test)]
mod import_batch_tests {
    use super::*;
    fn engine() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf") }
    fn fixture(directory: &Path) -> (PathBuf, PathBuf) {
        let image = directory.join("image.png");
        image::RgbImage::from_pixel(32, 16, image::Rgb([24, 80, 160])).save(&image).unwrap();
        let pdf = directory.join("valid.PDF");
        image_import::convert(&image, &pdf).unwrap();
        (pdf, image)
    }
    #[test]
    fn valid_files_survive_corrupt_and_unsupported_neighbors() {
        let directory = tempfile::tempdir().unwrap();
        let (pdf, image) = fixture(directory.path());
        let bad = directory.path().join("broken.pdf"); fs::write(&bad, b"invalid PDF").unwrap();
        let other = directory.path().join("notes.txt"); fs::write(&other, b"synthetic note").unwrap();
        let original_pdf = fs::read(&pdf).unwrap(); let original_image = fs::read(&image).unwrap();
        let store = Store::new().unwrap();
        let result = select_paths(&store, &engine(), vec![pdf.clone(), bad, image.clone(), other], true).unwrap();
        assert_eq!(result.documents.iter().map(|doc| doc.name.as_str()).collect::<Vec<_>>(), ["valid.PDF", "image.png"]);
        assert_eq!(result.rejected.iter().map(|failure| failure.name.as_str()).collect::<Vec<_>>(), ["broken.pdf", "notes.txt"]);
        assert!(result.documents.iter().all(|doc| doc.pages == 1 && !doc.locked));
        assert_eq!(store.documents.lock().unwrap().len(), 2);
        assert_eq!(fs::read(pdf).unwrap(), original_pdf); assert_eq!(fs::read(image).unwrap(), original_image);
        assert!(result.rejected.iter().all(|failure| !failure.reason.contains(directory.path().to_str().unwrap())));
    }
    #[test]
    fn rejected_batch_preserves_existing_selection_and_releases_failed_snapshots() {
        let directory = tempfile::tempdir().unwrap(); let (pdf, _) = fixture(directory.path());
        let store = Store::new().unwrap();
        let selected = select_paths(&store, &engine(), vec![pdf], false).unwrap();
        let before = fs::read_dir(store.workspace.path()).unwrap().count();
        let bad = directory.path().join("broken.pdf"); fs::write(&bad, b"invalid PDF").unwrap();
        let result = select_paths(&store, &engine(), vec![bad], false).unwrap();
        assert!(result.documents.is_empty()); assert_eq!(result.rejected.len(), 1);
        assert!(store.documents.lock().unwrap().contains_key(&selected.documents[0].id));
        assert_eq!(fs::read_dir(store.workspace.path()).unwrap().count(), before);
    }
    #[test]
    fn duplicates_do_not_consume_capacity_and_existing_files_still_import_at_limit() {
        let directory = tempfile::tempdir().unwrap(); let (pdf, _) = fixture(directory.path());
        let store = Store::new().unwrap();
        let result = select_paths(&store, &engine(), vec![pdf.clone(); 100], false).unwrap();
        assert_eq!(result.documents.len(), 1); assert!(result.rejected.is_empty());
        {
            let mut docs = store.documents.lock().unwrap(); let source = docs.values().next().unwrap().clone();
            for index in 1..100 { docs.insert(format!("fixture-{index}"), source.clone()); }
        }
        let second = directory.path().join("second.pdf"); fs::copy(&pdf, &second).unwrap();
        let result = select_paths(&store, &engine(), vec![pdf, second], false).unwrap();
        assert_eq!(result.documents.len(), 1); assert_eq!(result.rejected.len(), 1);
        assert_eq!(result.rejected[0].name, "second.pdf");
        assert_eq!(store.documents.lock().unwrap().len(), 100);
    }
    #[test]
    fn pdf_only_tools_reject_images_and_folders_without_losing_valid_pdf() {
        let directory = tempfile::tempdir().unwrap(); let (pdf, image) = fixture(directory.path());
        let store = Store::new().unwrap();
        let result = select_paths(&store, &engine(), vec![image, directory.path().to_path_buf(), pdf], false).unwrap();
        assert_eq!(result.documents.len(), 1); assert_eq!(result.rejected.len(), 2);
        assert_eq!(result.documents[0].name, "valid.PDF");
        assert!(select_paths(&store, &engine(), vec![PathBuf::new(); 101], true).is_err());
        assert_eq!(store.documents.lock().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod size_split_pdf_tests {
    use super::*;
    #[test]
    fn finished_pdfs_fit_and_preserve_every_page_with_navigation() {
        let directory = tempfile::tempdir().unwrap();
        let engine = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf");
        let mut inputs = Vec::new();
        for (index, width) in [240, 256, 280].into_iter().enumerate() {
            let mut seed = index as u32 + 42;
            let image = image::RgbImage::from_fn(width, 280, |_,_| {
                let mut pixel = [0;3];
                for channel in &mut pixel { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; *channel = (seed >> 24) as u8; }
                image::Rgb(pixel)
            });
            let source = directory.path().join(format!("image-{index}.png")); image.save(&source).unwrap();
            let pdf = directory.path().join(format!("input-{index}.pdf")); image_import::convert(&source, &pdf).unwrap(); inputs.push(pdf);
        }
        let input = directory.path().join("input.pdf");
        let mut command = Command::new(&engine); command.args(["--empty", "--pages"]);
        for pdf in &inputs { command.arg(pdf).arg("1"); }
        checked(command.arg("--").arg(&input), None).unwrap();
        let before = digest(&input, None).unwrap(); let mut doc = inspect(&engine, &input, None).unwrap();
        doc.features.bookmarks = (1..=3).map(|page| Bookmark { title: format!("Page {page}"), page: Some(page), children: vec![] }).collect();
        let workspace = tempfile::tempdir().unwrap(); let cancel = AtomicBool::new(false);
        let parts = size_split::partition(3, 250000, MAX_OUTPUTS, |first,last| make_size_part(&engine, &input, std::slice::from_ref(&doc), first,last, workspace.path(), &cancel)).unwrap();
        assert_eq!(parts.len(), 3);
        for (index, part) in parts.iter().enumerate() {
            assert_eq!(part.first, index as u32 + 1); assert_eq!(part.last, part.first);
            assert_eq!(part.bytes, fs::metadata(part.output.path()).unwrap().len()); assert!(part.bytes <= 250000);
            validate_output(&engine, part.output.path(), 1, &cancel).unwrap();
            let json: serde_json::Value = serde_json::from_slice(&checked(Command::new(&engine).arg("--json").arg(part.output.path()), None).unwrap()).unwrap();
            assert_eq!(json["pages"][0]["images"][0]["width"].as_u64().unwrap(), [240,256,280][index]);
            assert_eq!(json["outlines"][0]["title"].as_str().unwrap(), format!("Page {}", index+1));
        }
        drop(parts); assert_eq!(fs::read_dir(workspace.path()).unwrap().count(), 0);
        let failed = size_split::partition(3, 100000, MAX_OUTPUTS, |first,last| make_size_part(&engine, &input, std::slice::from_ref(&doc), first,last, workspace.path(), &cancel));
        assert!(failed.err().unwrap().starts_with("Page 1 exceeds"));
        assert_eq!(fs::read_dir(workspace.path()).unwrap().count(), 0);
        assert_eq!(digest(&input, None).unwrap(), before);
    }
}
