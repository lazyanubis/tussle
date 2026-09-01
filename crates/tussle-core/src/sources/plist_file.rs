//! Bounded reads for preference plists.

use std::io::{Cursor, Read};
use std::path::Path;

use crate::ScanError;
use plist::stream::{Event, Reader};

/// Preference files should be small. This cap prevents a malicious symlink,
/// special file, or corrupted plist from making a scan allocate without bound.
const MAX_PLIST_BYTES: u64 = 16 * 1024 * 1024;
/// Bound structural complexity independently from byte size. This prevents a
/// compact but deeply nested plist from exhausting the stack when converted
/// into plist::Value and limits work spent on extremely wide documents.
const MAX_PLIST_DEPTH: usize = 128;
const MAX_PLIST_EVENTS: usize = 100_000;

pub(super) fn parse_value(path: &Path) -> Result<plist::Value, ScanError> {
    let file = std::fs::File::open(path).map_err(|source| ScanError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let bytes = read_limited(file, path, MAX_PLIST_BYTES)?;
    parse_value_bytes(&bytes, path)
}

fn read_limited<R: Read>(reader: R, path: &Path, limit: u64) -> Result<Vec<u8>, ScanError> {
    let mut reader = reader.take(limit.saturating_add(1));
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|source| ScanError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    if bytes.len() as u64 > limit {
        return Err(ScanError::Schema {
            path: path.to_path_buf(),
            message: format!("plist exceeds the {limit}-byte safety limit"),
        });
    }

    Ok(bytes)
}

fn parse_value_bytes(bytes: &[u8], path: &Path) -> Result<plist::Value, ScanError> {
    let mut depth = 0usize;
    let mut events = Vec::new();

    for event in Reader::new(Cursor::new(bytes)) {
        let event = event.map_err(|error| schema_error(path, format!("plist parse: {error}")))?;
        if events.len() == MAX_PLIST_EVENTS {
            return Err(schema_error(
                path,
                format!("plist exceeds the {MAX_PLIST_EVENTS}-event safety limit"),
            ));
        }

        match &event {
            Event::StartArray(_) | Event::StartDictionary(_) => {
                depth += 1;
                if depth > MAX_PLIST_DEPTH {
                    return Err(schema_error(
                        path,
                        format!("plist exceeds the {MAX_PLIST_DEPTH}-level nesting safety limit"),
                    ));
                }
            }
            Event::EndCollection => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    schema_error(path, "plist contains an unmatched collection end".into())
                })?;
            }
            _ => {}
        }
        events.push(event);
    }

    if depth != 0 {
        return Err(schema_error(
            path,
            "plist ended before all collections were closed".into(),
        ));
    }

    plist::Value::from_events(events.into_iter().map(Ok::<_, plist::Error>))
        .map_err(|error| schema_error(path, format!("plist parse: {error}")))
}

fn schema_error(path: &Path, message: String) -> ScanError {
    ScanError::Schema {
        path: path.to_path_buf(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn accepts_input_at_the_limit() {
        let bytes = read_limited(Cursor::new(b"1234"), Path::new("fixture.plist"), 4)
            .expect("input at the limit should be accepted");
        assert_eq!(bytes, b"1234");
    }

    #[test]
    fn rejects_input_larger_than_the_limit() {
        let error = read_limited(Cursor::new(b"12345"), Path::new("fixture.plist"), 4)
            .expect_err("oversized input should be rejected");
        assert!(matches!(
            error,
            ScanError::Schema { message, .. } if message.contains("4-byte safety limit")
        ));
    }

    #[test]
    fn rejects_excessively_nested_plists_before_building_the_value() {
        let mut xml = String::from(r#"<?xml version="1.0"?><plist version="1.0">"#);
        xml.push_str(&"<array>".repeat(MAX_PLIST_DEPTH + 1));
        xml.push_str(&"</array>".repeat(MAX_PLIST_DEPTH + 1));
        xml.push_str("</plist>");

        let error = parse_value_bytes(xml.as_bytes(), Path::new("nested.plist"))
            .expect_err("excessive nesting should be rejected");
        assert!(matches!(
            error,
            ScanError::Schema { message, .. } if message.contains("nesting safety limit")
        ));
    }
}
