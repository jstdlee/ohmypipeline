use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Separators the reader will consider. The winner is the one that splits the
/// first record into the most fields. This reads one record, not the file.
pub fn sniff_separator(header_line: &str) -> u8 {
    let candidates = *b",\t;|";
    let mut best = b',';
    let mut best_count = 0usize;
    for sep in candidates {
        let count = split_record(header_line, sep).len();
        if count > best_count {
            best_count = count;
            best = sep;
        }
    }
    best
}

pub fn csv_headers(path: &Path) -> Result<Vec<String>, String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let n = reader.read_line(&mut line).map_err(|err| err.to_string())?;
    if n == 0 {
        return Err("empty file".into());
    }
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    let sep = sniff_separator(&line);
    let headers = split_record(&line, sep);
    if headers.is_empty() {
        return Err("no header".into());
    }
    Ok(headers)
}

fn split_record(line: &str, sep: u8) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_quotes {
            if b == b'"' {
                if bytes.get(i + 1) == Some(&b'"') {
                    current.push('"');
                    i += 2;
                    continue;
                }
                in_quotes = false;
            } else {
                current.push(b as char);
            }
        } else if b == b'"' {
            in_quotes = true;
        } else if b == sep {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(b as char);
        }
        i += 1;
    }
    fields.push(current);
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_prefers_the_separator_that_splits_the_header() {
        assert_eq!(sniff_separator("amount,region,month"), b',');
        assert_eq!(sniff_separator("amount\tregion\tmonth"), b'\t');
        assert_eq!(sniff_separator("amount;region;month"), b';');
    }

    #[test]
    fn quoted_commas_stay_inside_the_field() {
        let fields = split_record("\"a,b\",c", b',');
        assert_eq!(fields, vec!["a,b".to_string(), "c".to_string()]);
    }
}
