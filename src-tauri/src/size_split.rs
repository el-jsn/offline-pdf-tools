pub struct Part<T> { pub first: u32, pub last: u32, pub output: T, pub bytes: u64 }

// Measure finished PDFs; shared resources make source/page-size estimates unreliable.
// Search for a fitting contiguous prefix, retaining only the last verified candidate.
pub fn partition<T>(total: u32, limit: u64, max_parts: usize, mut measure: impl FnMut(u32, u32) -> Result<(T, u64), String>) -> Result<Vec<Part<T>>, String> {
    if total == 0 || limit == 0 || max_parts == 0 { return Err("Choose a valid file size limit.".into()); }
    let (whole, bytes) = measure(1, total)?;
    if bytes <= limit { return Ok(vec![Part { first: 1, last: total, output: whole, bytes }]); }
    drop(whole);
    let mut hint = (u64::from(total) * limit / bytes.max(1)).clamp(1, u64::from(total)) as u32;
    let mut parts = Vec::new(); let mut first = 1;
    while first <= total {
        if parts.len() >= max_parts { return Err(format!("Create at most {max_parts} PDFs. Choose a larger file size limit.")); }
        let remaining = total - first + 1;
        let mut low = 0; let mut high = remaining + 1;
        let mut probe = hint.min(remaining); let mut best = None;
        loop {
            let last = first + probe - 1;
            let (output, bytes) = measure(first, last)?;
            if bytes <= limit {
                best = Some(Part { first, last, output, bytes }); low = probe;
                if low == remaining { break; }
            } else {
                drop(output); high = probe;
                if probe == 1 { return Err(format!("Page {first} exceeds the file size limit. Compress the PDF first or choose a larger limit.")); }
            }
            if high - low <= 1 { break; }
            probe = if high == remaining + 1 && low > 0 { (low.saturating_mul(2)).min(remaining) } else { low + (high-low) / 2 };
        }
        let part = best.ok_or("Cannot find a page group within the file size limit.")?;
        hint = (part.last - part.first + 2).max(1);
        first = part.last + 1; parts.push(part);
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uneven_pages_stay_in_order_and_each_finished_part_fits() {
        let sizes = [20, 50, 10, 80, 15, 30];
        let result = partition(6, 95, 500, |first,last| Ok(((), 5 + sizes[(first-1) as usize..last as usize].iter().sum::<u64>()))).unwrap();
        assert_eq!(result.iter().flat_map(|part| part.first..=part.last).collect::<Vec<_>>(), (1..=6).collect::<Vec<_>>());
        assert!(result.iter().all(|part| part.bytes <= 95));
        assert!(result.len() > 1);
    }
    #[test]
    fn small_document_remains_one_file_and_single_oversized_page_fails() {
        let result = partition(3, 100, 500, |_,_| Ok(((), 50))).unwrap(); assert_eq!(result.len(), 1);
        let error = partition(3, 100, 500, |first,last| Ok(((), if first <= 2 && last >= 2 { 200 } else { 20 }))).err().unwrap();
        assert!(error.starts_with("Page 2 exceeds"));
    }
    #[test]
    fn output_limit_counts_finished_parts_rather_than_binary_fragments() {
        let result = partition(1024, 100000, 500, |first,last| Ok(((), 30000 * u64::from(last-first+1)))).unwrap();
        assert_eq!(result.len(), 342);
        assert!(result.iter().all(|part| part.bytes <= 100000));
        assert_eq!(result.iter().flat_map(|part| part.first..=part.last).collect::<Vec<_>>(), (1..=1024).collect::<Vec<_>>());
    }
    #[test]
    fn bounds_and_measurement_failures_stop_without_returning_partial_results() {
        assert!(partition(8, 10, 2, |first,last| Ok(((), 8 * u64::from(last-first+1)))).is_err());
        let error = partition::<()>(2, 10, 500, |_,_| Err("Cancelled. No output was saved.".into())).err().unwrap();
        assert!(error.starts_with("Cancelled"));
        assert!(partition(0, 10, 500, |_,_| Ok(((), 1))).is_err());
    }
}
