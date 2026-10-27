//! MPC1 admission control frames, as the transport writes and reads them (`src/control_codec.cpp`): the joiner sends
//! its ticket in an admission request, the host answers with the result. Over ICE they travel inside sealed control
//! frames (`pair_crypto.cpp`), so only a peer holding the pair key gets this far; the parser still trusts nothing.
//!
//! Header: "MPC1", version 1, the type, two zero bytes and the payload's length (u32), big-endian. A request's
//! payload is the ticket's length (u16) and the ticket; a result's is admitted (0 or 1), the code's length, the
//! admission id's length, a zero byte, then the code and the id (64 hex characters when admitted).

pub const CONTROL_MAGIC: &[u8; 4] = b"MPC1";
pub const CONTROL_VERSION: u8 = 1;
pub const ADMISSION_REQUEST: u8 = 1;
pub const ADMISSION_RESULT: u8 = 2;
pub const HEADER_BYTES: usize = 12;
pub const MAX_CONTROL_FRAME: usize = 4608;
const MAX_TICKET: usize = 4096;
const MAX_FIELD: usize = 64;

/// What an admission result frame carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionResult {
    pub admitted: bool,
    /// 1 to 64 bytes: `ok`, or why the host refused.
    pub code: Vec<u8>,
    /// Up to 64 bytes; the 64-hex admission id when admitted.
    pub admission_id: Vec<u8>,
}

fn frame(kind: u8, payload: &[u8]) -> Option<Vec<u8>> {
    if payload.is_empty() || payload.len() > MAX_CONTROL_FRAME - HEADER_BYTES {
        return None;
    }
    let mut out = Vec::with_capacity(HEADER_BYTES + payload.len());
    out.extend_from_slice(CONTROL_MAGIC);
    out.push(CONTROL_VERSION);
    out.push(kind);
    out.extend_from_slice(&[0, 0]);
    // At most 4596 by the check above
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

fn unframe(input: &[u8], kind: u8) -> Option<&[u8]> {
    if input.len() < HEADER_BYTES
        || input.len() > MAX_CONTROL_FRAME
        || &input[..4] != CONTROL_MAGIC
        || input[4] != CONTROL_VERSION
        || input[5] != kind
        || input[6..8] != [0, 0]
    {
        return None;
    }
    let declared = u32::from_be_bytes([input[8], input[9], input[10], input[11]]) as usize;
    let payload = &input[HEADER_BYTES..];
    (declared != 0 && declared == payload.len()).then_some(payload)
}

/// `EncodeAdmissionRequest`; `None` for an empty ticket or one over 4096 bytes.
pub fn encode_admission_request(ticket: &[u8]) -> Option<Vec<u8>> {
    if ticket.is_empty() || ticket.len() > MAX_TICKET {
        return None;
    }
    let mut payload = Vec::with_capacity(2 + ticket.len());
    payload.extend_from_slice(&(ticket.len() as u16).to_be_bytes());
    payload.extend_from_slice(ticket);
    frame(ADMISSION_REQUEST, &payload)
}

/// `DecodeAdmissionRequest`: the ticket bytes, unchecked (the ticket parser's job).
pub fn decode_admission_request(input: &[u8]) -> Option<&[u8]> {
    let payload = unframe(input, ADMISSION_REQUEST)?;
    if payload.len() < 3 {
        return None;
    }
    let size = usize::from(u16::from_be_bytes([payload[0], payload[1]]));
    (size != 0 && size <= MAX_TICKET && payload.len() == size + 2).then(|| &payload[2..])
}

/// `EncodeAdmissionResult`.
pub fn encode_admission_result(result: &AdmissionResult) -> Option<Vec<u8>> {
    let code = &result.code;
    let id = &result.admission_id;
    if code.is_empty() || code.len() > MAX_FIELD || id.len() > MAX_FIELD {
        return None;
    }
    let mut payload = Vec::with_capacity(4 + code.len() + id.len());
    // Both at most 64 by the check above
    payload.extend_from_slice(&[u8::from(result.admitted), code.len() as u8, id.len() as u8, 0]);
    payload.extend_from_slice(code);
    payload.extend_from_slice(id);
    frame(ADMISSION_RESULT, &payload)
}

/// `DecodeAdmissionResult`.
pub fn decode_admission_result(input: &[u8]) -> Option<AdmissionResult> {
    let payload = unframe(input, ADMISSION_RESULT)?;
    if payload.len() < 5 || payload[0] > 1 || payload[3] != 0 {
        return None;
    }
    let code_size = usize::from(payload[1]);
    let id_size = usize::from(payload[2]);
    if code_size == 0 || code_size > MAX_FIELD || id_size > MAX_FIELD || payload.len() != 4 + code_size + id_size {
        return None;
    }
    let admitted = payload[0] != 0;
    let admission_id = payload[4 + code_size..].to_vec();
    if admitted && admission_id.len() != 64 {
        return None;
    }
    Some(AdmissionResult {
        admitted,
        code: payload[4..4 + code_size].to_vec(),
        admission_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_round_trips_with_the_transports_layout() {
        let frame = encode_admission_request(b"mpt1.abc.def").unwrap();
        assert_eq!(&frame[..12], b"MPC1\x01\x01\x00\x00\x00\x00\x00\x0e");
        assert_eq!(&frame[12..14], &[0, 12]);
        assert_eq!(decode_admission_request(&frame), Some(&b"mpt1.abc.def"[..]));
        assert_eq!(encode_admission_request(b""), None);
        assert_eq!(encode_admission_request(&[b'a'; 4097]), None);
        assert!(encode_admission_request(&[b'a'; 4096]).is_some());
    }

    #[test]
    fn a_result_round_trips_and_an_admission_needs_its_id() {
        let admitted = AdmissionResult {
            admitted: true,
            code: b"ok".to_vec(),
            admission_id: vec![b'a'; 64],
        };
        let frame = encode_admission_result(&admitted).unwrap();
        assert_eq!(&frame[12..16], &[1, 2, 64, 0]);
        assert_eq!(decode_admission_result(&frame), Some(admitted.clone()));
        let short = AdmissionResult {
            admission_id: vec![b'a'; 63],
            ..admitted
        };
        assert_eq!(decode_admission_result(&encode_admission_result(&short).unwrap()), None);
        let refused = AdmissionResult {
            admitted: false,
            code: b"wrong_session".to_vec(),
            admission_id: Vec::new(),
        };
        assert_eq!(
            decode_admission_result(&encode_admission_result(&refused).unwrap()),
            Some(refused)
        );
    }

    #[test]
    fn frames_that_bend_the_header_are_refused() {
        let frame = encode_admission_request(b"ticket").unwrap();
        let bent = |at: usize, value: u8| {
            let mut copy = frame.clone();
            copy[at] = value;
            decode_admission_request(&copy).map(<[u8]>::to_vec)
        };
        assert_eq!(bent(0, b'X'), None); // magic
        assert_eq!(bent(4, 2), None); // version
        assert_eq!(bent(5, ADMISSION_RESULT), None); // type
        assert_eq!(bent(7, 1), None); // reserved
        assert_eq!(bent(11, 9), None); // declared length
        assert_eq!(bent(13, 5), None); // ticket length
        assert_eq!(decode_admission_request(&frame[..frame.len() - 1]), None);
        let mut long = frame.clone();
        long.resize(MAX_CONTROL_FRAME + 1, 0);
        assert_eq!(decode_admission_request(&long), None);
        assert_eq!(decode_admission_request(&[]), None);
    }
}
