use alloc::vec::Vec;

pub const BROADCAST: [u8; 6] = [0xFF; 6];
pub const IPV4: u16 = 0x0800;
pub const ARP: u16 = 0x0806;

pub struct Frame<'a> {
    pub dst: [u8; 6],
    pub src: [u8; 6],
    pub ethertype: u16,
    pub payload: &'a [u8],
}

pub fn parse(frame: &[u8]) -> Option<Frame<'_>> {
    if frame.len() < 14 {
        return None;
    }
    Some(Frame {
        dst: frame[0..6].try_into().ok()?,
        src: frame[6..12].try_into().ok()?,
        ethertype: u16::from_be_bytes([frame[12], frame[13]]),
        payload: &frame[14..],
    })
}

pub fn build(dst: [u8; 6], src: [u8; 6], ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(14 + payload.len());
    frame.extend_from_slice(&dst);
    frame.extend_from_slice(&src);
    frame.extend_from_slice(&ethertype.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}
