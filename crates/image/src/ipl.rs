pub const IPL_ROM_SIZE: usize = 0x200000;
const SCRAMBLE_START: usize = 0x100;
const SCRAMBLE_SIZE: usize = 0x1AFE00;

pub fn scramble(data: &mut [u8]) {
    assert_eq!(data.len(), IPL_ROM_SIZE);

    let region = &mut data[SCRAMBLE_START..SCRAMBLE_START + SCRAMBLE_SIZE];

    let mut acc: u8 = 0;
    let mut nacc: u8 = 0;

    let mut t: u16 = 0x2953;
    let mut u: u16 = 0xD9C2;
    let mut v: u16 = 0x3FF1;

    let mut x: u8 = 1;

    let mut it = 0;
    while it < region.len() {
        let t0 = (t & 1) as u8;
        let t1 = ((t >> 1) & 1) as u8;
        let u0 = (u & 1) as u8;
        let u1 = ((u >> 1) & 1) as u8;
        let v0 = (v & 1) as u8;

        x ^= t1 ^ v0;
        x ^= u0 | u1;
        x ^= (t0 ^ u1 ^ v0) & (t0 ^ u0);

        if t0 == u0 {
            v >>= 1;
            if v0 != 0 {
                v ^= 0xB3D0;
            }
        }

        if t0 == 0 {
            u >>= 1;
            if u0 != 0 {
                u ^= 0xFB10;
            }
        }

        t >>= 1;
        if t0 != 0 {
            t ^= 0xA740;
        }

        nacc += 1;
        acc = acc.wrapping_shl(1) + x;
        if nacc == 8 {
            region[it] ^= acc;
            nacc = 0;
            it += 1;
        }
    }
}

pub fn is_encoded(data: &[u8]) -> bool {
    if data.len() < 0x104 {
        return true;
    }

    let w = u32::from_be_bytes(data[0x100..0x104].try_into().unwrap());
    w != 0x3C800011
}
