//! Bounded, in-memory support reports. Never retain raw document diagnostics.
use std::{cell::RefCell, collections::VecDeque, io::{self, Read}, process::Command};
use serde::Serialize;

#[derive(Serialize)]
pub struct Failure { pub message: String, pub details: Option<String> }
struct Trace { header: String, stage: String, events: VecDeque<String>, dropped_events: usize }
thread_local! { static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) }; }
pub struct Session;
impl Session {
    pub fn begin(operation: &str, quality: &str, measurement: bool, target: Option<u64>) -> Self {
        let timestamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let header=format!("PDF Tools {} support report\nReport: {}\nTime (Unix UTC): {timestamp}\nPlatform: {}/{}\nDiagnostics format: 2\nOperation: {operation}\nCompression: {quality}\nPrepare compressed output: {measurement}\nTarget bytes: {target:?}\n",env!("CARGO_PKG_VERSION"),uuid::Uuid::new_v4(),std::env::consts::OS,std::env::consts::ARCH);
        TRACE.with(|slot| *slot.borrow_mut()=Some(Trace {header,stage:"Starting".into(),events:VecDeque::new(),dropped_events:0}));
        Self
    }
    pub fn fail(&self, message: String) -> Failure {
        let trace=TRACE.with(|slot| slot.borrow_mut().take());
        let details=if message.starts_with("Cancelled") { None } else { trace.map(|trace| {
            let omitted=if trace.dropped_events>0 {format!("Earlier report events omitted: {}\n",trace.dropped_events)} else {String::new()};
            format!("{}Failed step: {}\nError: {}\n\n{}{}\n\nFixed diagnostic categories and recognized engine phrases only; filenames, paths, passwords, object values and document-derived strings are excluded. Unrecognized diagnostic text is counted and omitted. Nothing was uploaded or written to a log file.",trace.header,trace.stage,message,omitted,trace.events.into_iter().collect::<Vec<_>>().join("\n"))
        }) };
        Failure {message,details}
    }
}
impl Drop for Session { fn drop(&mut self) { TRACE.with(|slot| *slot.borrow_mut()=None); } }
pub fn unavailable() -> Failure { Failure {message:"PDF processing failed.".into(),details:Some(format!("PDF Tools {}\nPlatform: {}/{}\nThe processing task ended unexpectedly before a report could be completed.",env!("CARGO_PKG_VERSION"),std::env::consts::OS,std::env::consts::ARCH))} }
pub fn stage(value: &str) { TRACE.with(|slot| {if let Some(trace)=slot.borrow_mut().as_mut(){
    if trace.stage!=value {record(trace,format!("Step: {value}"));trace.stage=value.to_owned();}
}}); }
pub fn engine_version(major:u32,minor:u32,patch:u32) {event(format!("Engine: qpdf {major}.{minor}.{patch}"));}
pub fn compression_engine() {event("Compressor: compress-pdf 0.1.0".into());}
pub fn reconnected_widgets(count: usize) {event(format!("Form repair: reconnected {count} equivalent duplicate widget(s); saved field values verified."));}
pub fn strict_validation() {event("Validation policy: warnings reject a generated or repaired copy.".into());}
pub fn input_bytes(bytes:u64) { TRACE.with(|slot| {if let Some(trace)=slot.borrow_mut().as_mut(){trace.header.push_str(&format!("Input: {bytes} bytes; page count not yet determined\n"));}}); }
pub fn inputs(values: impl Iterator<Item=(u64,u32)>) {
    TRACE.with(|slot| {if let Some(trace)=slot.borrow_mut().as_mut(){for (index,(bytes,pages)) in values.enumerate(){trace.header.push_str(&format!("Input {}: {bytes} bytes, {pages} pages\n",index+1));}}});
}
fn record(trace: &mut Trace, value: String) {
    if trace.events.len()==100 {trace.events.pop_front();trace.dropped_events+=1;}
    trace.events.push_back(value);
}
fn event(value: String) { TRACE.with(|slot| {if let Some(trace)=slot.borrow_mut().as_mut(){record(trace,value);}}); }
pub struct Capture {pub bytes:Vec<u8>, pub truncated:bool}
pub fn read_bounded(mut input: impl Read, limit: usize) -> io::Result<Capture> {
    let mut bytes=Vec::new();let mut chunk=[0u8;8192];let mut truncated=false;
    loop {let count=input.read(&mut chunk)?;if count==0 {break;}let keep=count.min(limit.saturating_sub(bytes.len()));bytes.extend_from_slice(&chunk[..keep]);truncated|=keep<count;}
    Ok(Capture {bytes,truncated})
}
pub fn action(command: &Command) -> &'static str {
    if matches!(std::path::Path::new(command.get_program()).file_name().and_then(|name| name.to_str()), Some("compress-pdf" | "compress-pdf.exe")) {return "PDF compression";}
    if command.get_args().any(|arg| arg == "--prepare-forms") {return "PDFium form appearance preparation";}
    if command.get_program().to_string_lossy().ends_with("pdf-image-worker") || command.get_program().to_string_lossy().ends_with("pdf-image-worker.exe") {return "Image compression worker";}
    for arg in command.get_args() {let arg=arg.to_string_lossy();
        if arg=="--is-encrypted" {return "qpdf encryption probe (exit 2 means unencrypted)";}
        if arg=="--requires-password" {return "qpdf password probe";}
        if arg=="--check" {return "qpdf validation";}
        if arg=="--json" {return "qpdf document inspection";}
        if arg=="--show-npages" {return "qpdf page count";}
        if arg=="--version" {return "qpdf version";}
        if arg.starts_with("--job-json-file=") {return "qpdf processing job";}
        if arg.starts_with("--update-from-json=") {return "qpdf bookmark/image update";}
    }
    "qpdf preparation"
}
// Match fixed engine phrases rather than copying free-form tokens from the PDF.
// QPDF can echo invalid object strings and filenames in otherwise useful errors.
fn safe_messages(raw: &[u8]) -> Vec<String> {
    const KNOWN: &[&str]=&[
        "damaged pdf file", "can't find startxref", "attempting to reconstruct cross-reference table",
        "expected endstream", "expected endobj", "unexpected eof", "unexpected end of file",
        "eof while reading", "unable to find /root dictionary", "unable to recover",
        "attempting to recover stream length", "recovered stream length", "stream length mismatch",
        "incorrect data check", "incorrect header check", "invalid distance code",
        "invalid distance too far back", "invalid code lengths set", "invalid stored block lengths",
        "invalid block type", "inflate", "error decoding stream data", "errors while decoding",
        "unable to decode", "unsupported filter", "unknown filter", "unrecognized filter",
        "expected dictionary", "expected array", "expected name", "expected integer",
        "expected string", "expected object", "invalid object", "invalid xref",
        "operation succeeded with warnings", "file is encrypted", "invalid password",
        "permission denied", "no space left on device", "read-only file system",
        "image engine could not read this pdf", "image engine could not start",
        "this pdf is too complex for image compression", "could not encode a compressed image",
        "could not write a compressed image", "invalid image dictionary", "image data unavailable",
        "image index exceeds the supported limit", "could not prepare compressed images",
        "could not write compressed images", "image workspace unavailable", "invalid image index",
        "out of memory", "bad_alloc", "cannot allocate memory", "failed to allocate", "insufficient memory",
        "dictionary has duplicated key", "unknown token while reading object", "treating unknown token type as null",
        "invalid character", "not a dictionary", "not an array", "loop detected",
    ];
    // All labels and matching phrases are application constants. Never copy
    // even numeric-looking context: engine prefixes may include customer IDs.
    const CATEGORIES: &[(&str, &str, &[&str])]=&[
        ("form.orphan_widget", "Form widget is not connected to the form-field tree", &["widget annotation is not reachable from /acroform"]),
        ("form.missing_field_type", "Form field has no usable field type", &["field has no /ft", "field is missing /ft"]),
        ("form.appearance", "Form appearance is missing or invalid", &["invalid appearance", "missing appearance", "no appearance stream"]),
        ("object.dictionary_type", "Dictionary operation encountered the wrong object type", &["operation for dictionary attempted on object of type", "operation for dictionary object attempted on object of wrong type"]),
        ("object.array_type", "Array operation encountered the wrong object type", &["operation for array attempted on object of type", "operation for array object attempted on object of wrong type"]),
        ("object.string_type", "String operation encountered the wrong object type", &["operation for string attempted on object of type"]),
        ("object.number_type", "Numeric operation encountered the wrong object type", &["operation for integer attempted on object of type", "operation for numeric attempted on object of type"]),
        ("object.duplicate_key", "Dictionary contains a duplicate key", &["dictionary has duplicated key", "duplicate dictionary key"]),
        ("object.unknown_token", "Object contains an unrecognized token", &["unknown token while reading object", "treating unknown token type as null"]),
        ("object.count_mismatch", "Declared object count does not match the object table", &["reported number of objects"]),
        ("xref.reconstruction", "Cross-reference table needs reconstruction", &["can't find startxref", "attempting to reconstruct cross-reference table", "file is damaged", "damaged pdf file"]),
        ("stream.length", "Stream length or termination needs recovery", &["attempting to recover stream length", "recovered stream length", "stream length mismatch", "expected endstream"]),
        ("stream.filter", "Stream uses an unsupported or unrecognized filter", &["unsupported filter", "unknown filter", "unrecognized filter"]),
        ("structure.loop", "Document structure contains a reference loop", &["loop detected"]),
        ("system.storage", "Local storage is full, read-only or inaccessible", &["no space left on device", "read-only file system", "permission denied"]),
        ("system.memory", "Engine could not allocate memory", &["out of memory", "bad_alloc", "cannot allocate memory", "failed to allocate", "insufficient memory"]),
    ];
    let mut groups: Vec<(String,usize)>=Vec::new();
    let mut total=0;let mut classified=0;let mut omitted=0;
    for line in String::from_utf8_lossy(raw).lines().filter(|line|!line.trim().is_empty()) {
        total+=1;
        let lower=line.to_ascii_lowercase();
        let category=CATEGORIES.iter().find(|(_,_,phrases)|phrases.iter().any(|phrase|lower.contains(phrase)));
        let messages:Vec<String>=if let Some((code,label,_))=category {
            vec![format!("[{code}] {label}")]
        } else {
            KNOWN.iter().filter(|text|lower.contains(**text)).map(|text|format!("[engine.recognized] {text}")).collect()
        };
        if messages.is_empty() {omitted+=1;continue;}
        classified+=1;
        for message in messages {
            if let Some((_,count))=groups.iter_mut().find(|(existing,_)|existing==&message) {*count+=1;}
            else {groups.push((message,1));}
        }
    }
    if total==0 {return Vec::new();}
    let mut output=vec![format!("  Diagnostic lines: {total}; classified: {classified}; omitted for privacy: {omitted}.")];
    for (label,count) in groups {output.push(format!("  {label}: {count} line(s)."));}
    if omitted>0 {output.push(format!("  {omitted} unrecognized diagnostic line(s) omitted for privacy."));}
    output
}

pub fn command_result(action: &str, result: &str, stderr: &Capture) {
    event(format!("{action}: {result}"));
    for line in safe_messages(&stderr.bytes) {event(line);}
    if stderr.truncated {event("  Diagnostic capture capped at 64 KiB; excess output drained and discarded.".into());}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_keep_causes_without_echoing_customer_data() {
        let raw=b"WARNING: /Users/Mortgage/Client-123456789.pdf (object 7 0, offset 410): expected endstream, found 'SECRET_CLIENT_TEXT'\nqpdf: password='SECRET_PASSWORD'\n";
        let output=safe_messages(raw).join("\n");
        assert!(output.contains("[stream.length]")&&output.contains("1 line(s)"));
        assert!(!output.contains("object 7 0")&&!output.contains("offset 410"));
        for private in ["Users","Mortgage","Client","123456789","SECRET","PASSWORD",".pdf"] {assert!(!output.contains(private));}
    }
    #[test]
    fn repeated_warnings_are_counted_and_late_causes_are_not_dropped() {
        let raw=format!("{}{}{}", "SECRET_UNKNOWN_TEXT\n".repeat(80),
            "WARNING: PRIVATE_FILE object 987654321: widget annotation is not reachable from /AcroForm in the document catalog\n".repeat(54),
            "WARNING: PRIVATE_VALUE: operation for dictionary attempted on object of type null\n");
        let output=safe_messages(raw.as_bytes()).join("\n");
        assert!(output.contains("[form.orphan_widget] Form widget is not connected to the form-field tree: 54 line(s)."));
        assert!(output.contains("[object.dictionary_type]"));
        assert!(output.contains("Diagnostic lines: 135; classified: 55; omitted for privacy: 80."));
        assert!(output.lines().count()<6);
        for private in ["SECRET", "PRIVATE", "987654321", "type null"] {assert!(!output.contains(private));}
    }
    #[test]
    fn report_retention_and_capture_limits_are_explicit() {
        let session=Session::begin("compress","balanced",true,None);
        stage("Check input working copy");stage("Repair input working copy");stage("Validate output");
        let report=session.fail("Repair failed".into()).details.unwrap();
        assert!(report.contains("Step: Check input working copy")&&report.contains("Step: Repair input working copy"));
        drop(session);
        let session=Session::begin("compress","balanced",true,None);
        for _ in 0..110 {event("Synthetic prior step".into());}
        command_result("qpdf validation","exit 3",&Capture{bytes:b"widget annotation is not reachable from /AcroForm".to_vec(),truncated:true});
        let report=session.fail("Repair failed".into()).details.unwrap();
        assert!(report.contains("Earlier report events omitted:")&&report.contains("capture capped at 64 KiB"));
        assert!(report.contains("[form.orphan_widget]"));
    }
    #[test]
    fn real_orphan_widget_is_classified_without_field_data() {
        let directory=tempfile::tempdir().unwrap();let input=directory.path().join("PRIVATE_FILENAME.pdf");
        let objects=[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [] >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Annots [5 0 R] >>",
            "<< /Length 0 >>\nstream\n\nendstream",
            "<< /Type /Annot /Subtype /Widget /Rect [10 10 90 30] /FT /Tx /T (PRIVATE_FIELD) /V (PRIVATE_VALUE) /P 3 0 R >>",
        ];
        let mut pdf=b"%PDF-1.4\n".to_vec();let mut offsets=Vec::new();
        for (index,object) in objects.iter().enumerate() {
            offsets.push(pdf.len());pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n",index+1).as_bytes());
        }
        let xref=pdf.len();pdf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for offset in offsets {pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());}
        pdf.extend_from_slice(format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
        std::fs::write(&input,pdf).unwrap();
        let engine=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf");
        let session=Session::begin("compress","balanced",true,None);
        let error=crate::repair_snapshot_if_needed(&engine,&input,1,&std::sync::atomic::AtomicBool::new(false)).unwrap_err();
        let report=session.fail(error).details.unwrap();
        assert!(report.contains("[form.orphan_widget]"));
        assert!(report.contains("Step: Repair input working copy"));
        assert!(report.contains("could not be repaired safely"));
        assert!(report.contains("Validation policy: warnings reject"));
        assert!(!report.contains("PRIVATE")&&!report.contains(input.to_str().unwrap()));
    }
    #[test]
    #[ignore = "Requires an explicitly supplied local orphan-widget PDF; no document content is printed"]
    fn supplied_orphan_widget_report() {
        let input=std::path::PathBuf::from(std::env::var_os("PDF_TOOLS_DIAGNOSTIC_INPUT").expect("Supply a local PDF"));
        let before=crate::digest(&input,None).unwrap();
        let directory=tempfile::tempdir().unwrap();let snapshot=directory.path().join("working.pdf");
        std::fs::copy(&input,&snapshot).unwrap();
        let engine=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf");
        let session=Session::begin("compress","balanced",true,None);
        input_bytes(std::fs::metadata(&input).unwrap().len());
        let pages=crate::count_pages(&engine,&snapshot,None).unwrap();
        assert!(pages>0);
        let error=crate::checked(Command::new(&engine).arg("--check").arg(&snapshot),None).unwrap_err();
        let report=session.fail(error).details.unwrap();
        assert!(report.contains("[form.orphan_widget]"));
        assert!(!report.contains(input.to_str().unwrap()));
        assert_eq!(before,crate::digest(&input,None).unwrap());
        println!("{report}");
    }
    #[test]
    fn bounded_capture_drains_instead_of_closing_the_engine_pipe_early() {
        let data=vec![b'x';100000];let mut cursor=io::Cursor::new(data);let result=read_bounded(&mut cursor,1024).unwrap();
        assert_eq!(result.bytes.len(),1024);assert!(result.truncated);assert_eq!(cursor.position(),100000);
    }
    #[test]
    fn reports_are_scoped_and_cancellation_has_no_error_button() {
        let session=Session::begin("compress","balanced",true,None);stage("Validate output");
        command_result("qpdf validation","exit 3",&Capture{bytes:b"WARNING: incorrect data check".to_vec(),truncated:false});
        let report=session.fail("The PDF could not pass validation. No output was saved.".into()).details.unwrap();
        assert!(report.contains("Validate output")&&report.contains("exit 3")&&report.contains("incorrect data check"));drop(session);
        let session=Session::begin("merge","lossless",false,None);assert!(!session.fail("New failure".into()).details.unwrap().contains("incorrect data check"));drop(session);
        let session=Session::begin("compress","balanced",true,None);assert!(session.fail("Cancelled. No output was saved.".into()).details.is_none());
    }
    #[test]
    fn real_engine_failure_reaches_the_copyable_report() {
        use std::io::Write;
        let mut input=tempfile::NamedTempFile::new().unwrap();input.write_all(b"%PDF-1.4\nbroken synthetic input\n").unwrap();
        let engine=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf");
        let session=Session::begin("compress","balanced",true,None);stage("Validate output");
        let error=crate::checked(Command::new(engine).arg("--check").arg(input.path()),None).unwrap_err();
        let report=session.fail(error).details.unwrap();
        assert!(report.contains("qpdf validation: exit 2")&&report.contains("[xref.reconstruction]"));
        assert!(!report.contains(input.path().to_str().unwrap()));
    }
}
