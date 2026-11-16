use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

pub(super) fn is_narration_signature(signature: &str) -> bool {
    let Ok(decoded) = STANDARD.decode(signature) else {
        return false;
    };
    let Some(message) = length_delimited_field(&decoded, 2) else {
        return false;
    };
    let Some(metadata) = length_delimited_field(message, 1) else {
        return false;
    };
    length_delimited_field(metadata, 8) == Some(b"narration".as_slice())
}

fn length_delimited_field(bytes: &[u8], wanted_field: u64) -> Option<&[u8]> {
    let mut cursor = 0;
    while cursor < bytes.len() {
        let key = read_varint(bytes, &mut cursor)?;
        let field_number = key >> 3;
        let wire_type = key & 0b111;
        if field_number == 0 {
            return None;
        }
        if wire_type == 2 {
            let length = usize::try_from(read_varint(bytes, &mut cursor)?).ok()?;
            let end = cursor.checked_add(length)?;
            let value = bytes.get(cursor..end)?;
            if field_number == wanted_field {
                return Some(value);
            }
            cursor = end;
            continue;
        }
        if field_number == wanted_field {
            return None;
        }
        match wire_type {
            0 => {
                read_varint(bytes, &mut cursor)?;
            }
            1 => cursor = cursor.checked_add(8)?,
            5 => cursor = cursor.checked_add(4)?,
            _ => return None,
        }
        if cursor > bytes.len() {
            return None;
        }
    }
    None
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut value = 0;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}
