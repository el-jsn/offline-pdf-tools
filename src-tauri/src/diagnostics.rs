//! Bounded, in-memory support reports. Never retain raw document diagnostics.
use std::{cell::RefCell, collections::VecDeque, io::{self, Read}, process::Command};
use serde::Serialize;

#[derive(Serialize)]
pub struct Failure { pub message: String, pub details: Option<String> }
struct Trace { header: String, stage: String, events: VecDeque<String> }
thread_local! { static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) }; }
pub struct Session;
impl Session {
    pub fn begin(operation: &str, quality: &str, measurement: bool, target: Option<u64>) -> Self {
        let timestamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let header=format!("PDF Tools {} support report\nReport: {}\nTime (Unix UTC): {timestamp}\nPlatform: {}/{}\nOperation: {operation}\nCompression: {quality}\nSize preview: {measurement}\nTarget bytes: {target:?}\n",env!("CARGO_PKG_VERSION"),uuid::Uuid::new_v4(),std::env::consts::OS,std::env::consts::ARCH);
        TRACE.with(|slot| *slot.borrow_mut()=Some(Trace {header,stage:"Starting".into(),events:VecDeque::new()}));
        Self
    }
    pub fn fail(&self, message: String) -> Failure {
        let trace=TRACE.with(|slot| slot.borrow_mut().take());
        let details=if message.starts_with("Cancelled") { None } else { trace.map(|trace| format!("{}Failed step: {}\nError: {}\n\n{}\n\nRecognized engine messages only; filenames, paths, passwords and document-derived strings are excluded. Unrecognized diagnostic text is omitted. Nothing was uploaded or written to a log file.",trace.header,trace.stage,message,trace.events.into_iter().collect::<Vec<_>>().join("\n"))) };
        Failure {message,details}
    }
}
impl Drop for Session { fn drop(&mut self) { TRACE.with(|slot| *slot.borrow_mut()=None); } }
pub fn unavailable() -> Failure { Failure {message:"PDF processing failed.".into(),details:Some(format!("PDF Tools {}\nPlatform: {}/{}\nThe processing task ended unexpectedly before a report could be completed.",env!("CARGO_PKG_VERSION"),std::env::consts::OS,std::env::consts::ARCH))} }
pub fn stage(value: &str) { TRACE.with(|slot| {if let Some(trace)=slot.borrow_mut().as_mut(){trace.stage=value.to_owned();}}); }
pub fn engine_version(major:u32,minor:u32,patch:u32) {event(format!("Engine: qpdf {major}.{minor}.{patch}"));}
pub fn input_bytes(bytes:u64) { TRACE.with(|slot| {if let Some(trace)=slot.borrow_mut().as_mut(){trace.header.push_str(&format!("Input: {bytes} bytes; page count not yet determined\n"));}}); }
pub fn inputs(values: impl Iterator<Item=(u64,u32)>) {
    TRACE.with(|slot| {if let Some(trace)=slot.borrow_mut().as_mut(){for (index,(bytes,pages)) in values.enumerate(){trace.header.push_str(&format!("Input {}: {bytes} bytes, {pages} pages\n",index+1));}}});
}
fn event(value: String) { TRACE.with(|slot| {if let Some(trace)=slot.borrow_mut().as_mut(){if trace.events.len()==100 {trace.events.pop_front();} trace.events.push_back(value);}}); }
pub struct Capture {pub bytes:Vec<u8>, pub truncated:bool}
pub fn read_bounded(mut input: impl Read, limit: usize) -> io::Result<Capture> {
    let mut bytes=Vec::new();let mut chunk=[0u8;8192];let mut truncated=false;
    loop {let count=input.read(&mut chunk)?;if count==0 {break;}let keep=count.min(limit.saturating_sub(bytes.len()));bytes.extend_from_slice(&chunk[..keep]);truncated|=keep<count;}
    Ok(Capture {bytes,truncated})
}
pub fn action(command: &Command) -> &'static str {
    if command.get_program().to_string_lossy().ends_with("pdf-image-worker") || command.get_program().to_string_lossy().ends_with("pdf-image-worker.exe") {return "Image compression worker";}
    for arg in command.get_args() {let arg=arg.to_string_lossy();
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
    ];
    let mut output=Vec::new();let mut omitted=0;
    for line in String::from_utf8_lossy(raw).lines().take(64) {
        let lower=line.to_ascii_lowercase();let messages:Vec<_>=KNOWN.iter().filter(|text| lower.contains(**text)).copied().collect();
        if messages.is_empty(){if !line.trim().is_empty(){omitted+=1;}continue;}
        // Only structural object identifiers/offsets; never arbitrary quoted values.
        let mut context=String::new();
        for marker in ["object ","offset "] {
            if let Some((_,tail))=lower.split_once(marker) {
                let numbers:String=tail.chars().take_while(|c| c.is_ascii_digit()||*c==' ').take(24).collect();
                if !numbers.trim().is_empty(){context.push_str(&format!("{marker}{}; ",numbers.trim()));}
            }
        }
        output.push(format!("  {context}{}",messages.join("; ")));
    }
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
        assert!(output.contains("expected endstream")&&output.contains("object 7 0")&&output.contains("offset 410"));
        for private in ["Users","Mortgage","Client","123456789","SECRET","PASSWORD",".pdf"] {assert!(!output.contains(private));}
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
        assert!(report.contains("qpdf validation: exit 2")&&report.contains("can't find startxref"));
        assert!(!report.contains(input.path().to_str().unwrap()));
    }
}
