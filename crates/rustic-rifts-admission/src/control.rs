// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The control frames two transports exchange to admit a joiner: the joiner's ticket, and the host's verdict. A
//! 12-byte big-endian header (magic, version, type, two zero bytes, payload length), then the payload.

/// The magic every control frame starts with.
pub const CONTROL_MAGIC: &[u8; 4] = b"RPC1";
/// The control frame format version.
pub const CONTROL_VERSION: u8 = 1;
/// The frame type of a joiner's admission request.
pub const ADMISSION_REQUEST: u8 = 1;
/// The frame type of the host's admission result.
pub const ADMISSION_RESULT: u8 = 2;
/// The header's size: magic, version, type, two zero bytes and the payload length.
pub const HEADER_BYTES: usize = 12;
/// The largest control frame accepted, in bytes.
pub const MAX_CONTROL_FRAME: usize = 4608;

const MAX_TICKET: usize = 4096;
const MAX_CODE: usize = 64;
const MAX_ADMISSION_ID: usize = 64;
/// An admitted result carries the 64-hex-digit admission id.
const ADMITTED_ID: usize = 64;

/// The host's verdict on a joiner's ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionResult {
    /// Whether the joiner was let in.
    pub admitted: bool,
    /// `ok` or the refusal code, 1 to 64 bytes.
    pub code: Vec<u8>,
    /// The 64-hex-digit admission id when admitted, else empty.
    pub admission_id: Vec<u8>,
}

fn frame(kind: u8, payload: &[u8]) -> Option<Vec<u8>> {
    let length = u32::try_from(payload.len()).ok()?;
    let mut out = Vec::with_capacity(HEADER_BYTES + payload.len());
    out.extend_from_slice(CONTROL_MAGIC);
    out.extend_from_slice(&[CONTROL_VERSION, kind, 0, 0]);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

/// The payload of a well-formed frame of `kind`.
fn payload_of(input: &[u8], kind: u8) -> Option<&[u8]> {
    if !(HEADER_BYTES..=MAX_CONTROL_FRAME).contains(&input.len()) {
        return None;
    }
    let (header, payload) = input.split_at(HEADER_BYTES);
    let declared = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);
    let header_ok = &header[..4] == CONTROL_MAGIC
        && header[4] == CONTROL_VERSION
        && header[5] == kind
        && header[6..8] == [0, 0]
        && declared != 0
        && usize::try_from(declared).ok() == Some(payload.len());
    header_ok.then_some(payload)
}

/// The request frame carrying `ticket`; `None` when it is empty or over 4096 bytes.
pub fn encode_admission_request(ticket: &[u8]) -> Option<Vec<u8>> {
    if ticket.is_empty() || ticket.len() > MAX_TICKET {
        return None;
    }
    let length = u16::try_from(ticket.len()).ok()?;
    let mut payload = Vec::with_capacity(2 + ticket.len());
    payload.extend_from_slice(&length.to_be_bytes());
    payload.extend_from_slice(ticket);
    frame(ADMISSION_REQUEST, &payload)
}

/// The ticket bytes, borrowed from `input`.
pub fn decode_admission_request(input: &[u8]) -> Option<&[u8]> {
    let payload = payload_of(input, ADMISSION_REQUEST)?;
    if payload.len() < 3 {
        return None;
    }
    let length = usize::from(u16::from_be_bytes([payload[0], payload[1]]));
    let ticket = &payload[2..];
    (length != 0 && length <= MAX_TICKET && length == ticket.len()).then_some(ticket)
}

/// Encodes what it is given; an admitted result with a short id encodes but will not decode.
pub fn encode_admission_result(result: &AdmissionResult) -> Option<Vec<u8>> {
    if result.code.is_empty() || result.code.len() > MAX_CODE || result.admission_id.len() > MAX_ADMISSION_ID {
        return None;
    }
    let code_len = u8::try_from(result.code.len()).ok()?;
    let id_len = u8::try_from(result.admission_id.len()).ok()?;
    let mut payload = Vec::with_capacity(4 + result.code.len() + result.admission_id.len());
    payload.extend_from_slice(&[u8::from(result.admitted), code_len, id_len, 0]);
    payload.extend_from_slice(&result.code);
    payload.extend_from_slice(&result.admission_id);
    frame(ADMISSION_RESULT, &payload)
}

/// The result a well-formed result frame carries; `None` for anything malformed.
pub fn decode_admission_result(input: &[u8]) -> Option<AdmissionResult> {
    let payload = payload_of(input, ADMISSION_RESULT)?;
    if payload.len() < 5 || payload[0] > 1 || payload[3] != 0 {
        return None;
    }
    let admitted = payload[0] == 1;
    let code_len = usize::from(payload[1]);
    let id_len = usize::from(payload[2]);
    let lengths_ok = (1..=MAX_CODE).contains(&code_len)
        && id_len <= MAX_ADMISSION_ID
        && payload.len() == 4 + code_len + id_len
        && (!admitted || id_len == ADMITTED_ID);
    if !lengths_ok {
        return None;
    }
    let (code, admission_id) = payload[4..].split_at(code_len);
    Some(AdmissionResult {
        admitted,
        code: code.to_vec(),
        admission_id: admission_id.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admitted() -> AdmissionResult {
        AdmissionResult {
            admitted: true,
            code: b"OK".to_vec(),
            admission_id: vec![b'a'; 64],
        }
    }

    #[test]
    fn request_example_bytes() {
        let frame = encode_admission_request(b"rpt1.abc.def").unwrap();
        assert_eq!(
            frame[..14],
            [
                0x52, 0x50, 0x43, 0x31, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0e, 0x00, 0x0c
            ]
        );
        assert_eq!(&frame[14..], b"rpt1.abc.def");
        assert_eq!(decode_admission_request(&frame), Some(&b"rpt1.abc.def"[..]));
    }

    #[test]
    fn request_ticket_sizes() {
        assert_eq!(encode_admission_request(b""), None);
        assert_eq!(encode_admission_request(&[b'x'; 4097]), None);
        let largest = encode_admission_request(&[b'x'; 4096]).unwrap();
        assert_eq!(largest.len(), 12 + 2 + 4096);
        assert_eq!(decode_admission_request(&largest).map(<[u8]>::len), Some(4096));
    }

    #[test]
    fn result_round_trips() {
        let frame = encode_admission_result(&admitted()).unwrap();
        assert_eq!(frame[..12], [0x52, 0x50, 0x43, 0x31, 1, 2, 0, 0, 0, 0, 0, 70]);
        assert_eq!(frame[12..16], [0x01, 0x02, 0x40, 0x00]);
        assert_eq!(decode_admission_result(&frame), Some(admitted()));

        let refusal = AdmissionResult {
            admitted: false,
            code: b"P2P_TICKET_EXPIRED".to_vec(),
            admission_id: Vec::new(),
        };
        let frame = encode_admission_result(&refusal).unwrap();
        assert_eq!(decode_admission_result(&frame), Some(refusal));

        let short = AdmissionResult {
            admission_id: vec![b'a'; 63],
            ..admitted()
        };
        let frame = encode_admission_result(&short).unwrap();
        assert_eq!(decode_admission_result(&frame), None);

        let no_code = AdmissionResult {
            code: Vec::new(),
            ..admitted()
        };
        assert_eq!(encode_admission_result(&no_code), None);
        let long_code = AdmissionResult {
            code: vec![b'C'; 65],
            ..admitted()
        };
        assert_eq!(encode_admission_result(&long_code), None);
        let long_id = AdmissionResult {
            admission_id: vec![b'a'; 65],
            ..admitted()
        };
        assert_eq!(encode_admission_result(&long_id), None);
    }

    #[test]
    fn bent_frames_are_refused() {
        let request = encode_admission_request(b"rpt1.abc.def").unwrap();
        let bent = |at: usize, value: u8| {
            let mut frame = request.clone();
            frame[at] = value;
            decode_admission_request(&frame).map(<[u8]>::to_vec)
        };
        assert_eq!(bent(0, b'M'), None, "magic");
        assert_eq!(bent(4, 2), None, "version");
        assert_eq!(bent(5, ADMISSION_RESULT), None, "type");
        assert_eq!(bent(6, 1), None, "reserved");
        assert_eq!(bent(7, 1), None, "reserved");
        assert_eq!(bent(11, 0x0f), None, "declared length");
        assert_eq!(bent(13, 0x0b), None, "ticket length");
        assert_eq!(bent(13, 0), None, "zero ticket length");
        assert_eq!(
            decode_admission_request(&request[..request.len() - 1]),
            None,
            "truncated"
        );
        assert_eq!(decode_admission_request(&[]), None, "empty");
        assert_eq!(decode_admission_request(&request[..12]), None, "header only");

        let mut oversize = encode_admission_request(&[b'x'; 4096]).unwrap();
        oversize.extend_from_slice(&[0; 500]);
        assert_eq!(decode_admission_request(&oversize), None);

        let result = encode_admission_result(&admitted()).unwrap();
        let bent_result = |at: usize, value: u8| {
            let mut frame = result.clone();
            frame[at] = value;
            decode_admission_result(&frame)
        };
        assert_eq!(bent_result(12, 2), None, "admitted flag");
        assert_eq!(bent_result(15, 1), None, "reserved payload byte");
        assert_eq!(bent_result(13, 0), None, "zero code length");
        assert_eq!(bent_result(5, ADMISSION_REQUEST), None, "type");
        assert_eq!(decode_admission_result(&request), None);
    }
}
