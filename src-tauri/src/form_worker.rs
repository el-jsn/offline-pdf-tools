//! Runs only in the isolated worker. Never executes document JavaScript or actions.
use pdfium_render::prelude::*;
use std::{fs::File, io::Write, path::Path};
#[repr(C)]
struct Output {
    callback: FPDF_FILEWRITE,
    file: File,
    bytes: u64,
}
unsafe extern "C" fn write_block(
    this: *mut FPDF_FILEWRITE,
    data: *const std::ffi::c_void,
    size: std::os::raw::c_ulong,
) -> i32 {
    let output = &mut *(this as *mut Output);
    output.bytes = output.bytes.saturating_add(size as u64);
    if output.bytes > 512 * 1024 * 1024 {
        return 0;
    }
    i32::from(
        output
            .file
            .write_all(std::slice::from_raw_parts(data as *const u8, size as usize))
            .is_ok(),
    )
}
pub fn prepare(library: &Path, input: &Path, output: &Path) -> Result<(), &'static str> {
    let bindings =
        Pdfium::bind_to_library(library.to_path_buf()).map_err(|_| "Form engine unavailable.")?;
    let input = input.to_str().ok_or("Form working copy unavailable.")?;
    let file = File::options()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|_| "Cannot create form working copy.")?;
    let mut writer = Output {
        callback: FPDF_FILEWRITE {
            version: 1,
            WriteBlock: Some(write_block),
        },
        file,
        bytes: 0,
    };
    unsafe {
        bindings.FPDF_InitLibrary();
        let document = bindings.FPDF_LoadDocument(input, None);
        if document.is_null() {
            bindings.FPDF_DestroyLibrary();
            return Err("Cannot open form working copy.");
        }
        // No JS platform, network callbacks, actions, or XFA initialization.
        let mut info: FPDF_FORMFILLINFO = std::mem::zeroed();
        info.version = 2;
        info.xfa_disabled = 1;
        let form = bindings.FPDFDOC_InitFormFillEnvironment(document, &mut info);
        let result = (|| {
            if form.is_null() {
                return Err("Cannot initialize form appearances.");
            }
            let count = bindings.FPDF_GetPageCount(document);
            if !(1..=10000).contains(&count) {
                return Err("Invalid form page count.");
            }
            for index in 0..count {
                let page = bindings.FPDF_LoadPage(document, index);
                if page.is_null() {
                    return Err("Cannot prepare a form page.");
                }
                // PDFium regenerates AcroForm appearances from saved values while
                // loading the page's form controls when NeedAppearances is set.
                bindings.FORM_OnAfterLoadPage(page, form);
                bindings.FORM_OnBeforeClosePage(page, form);
                bindings.FPDF_ClosePage(page);
            }
            if bindings.FPDF_SaveAsCopy(document, &mut writer.callback, 2) == 0 {
                return Err("Cannot save form appearances.");
            }
            writer
                .file
                .flush()
                .map_err(|_| "Cannot save form appearances.")?;
            Ok(())
        })();
        if !form.is_null() {
            bindings.FPDFDOC_ExitFormFillEnvironment(form);
        }
        bindings.FPDF_CloseDocument(document);
        bindings.FPDF_DestroyLibrary();
        result
    }
}
