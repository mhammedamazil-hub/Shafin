//! DEFLATE (RFC 1951) compressor + decompressor, zlib framing (RFC 1950),
//! CRC-32 and Adler-32. Written in-tree so the project has no dependencies and
//! PNG output is byte-identical on every platform and thread count.
use vl_core::par;

// ---------------------------------------------------------------- checksums

fn crc_table() -> &'static [[u32; 256]; 8] {
    use std::sync::OnceLock;
    static T: OnceLock<Box<[[u32; 256]; 8]>> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = Box::new([[0u32; 256]; 8]);
        for n in 0..256u32 {
            let mut c = n;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            t[0][n as usize] = c;
        }
        for n in 0..256usize {
            let mut c = t[0][n];
            for k in 1..8 {
                c = t[0][(c & 0xff) as usize] ^ (c >> 8);
                t[k][n] = c;
            }
        }
        t
    })
}

/// CRC-32 (IEEE 802.3), slicing-by-8.
pub fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let t = crc_table();
    let mut c = !crc;
    let mut chunks = data.chunks_exact(8);
    for ch in &mut chunks {
        let lo = u32::from_le_bytes([ch[0], ch[1], ch[2], ch[3]]) ^ c;
        let hi = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]);
        c = t[7][(lo & 0xff) as usize]
            ^ t[6][((lo >> 8) & 0xff) as usize]
            ^ t[5][((lo >> 16) & 0xff) as usize]
            ^ t[4][(lo >> 24) as usize]
            ^ t[3][(hi & 0xff) as usize]
            ^ t[2][((hi >> 8) & 0xff) as usize]
            ^ t[1][((hi >> 16) & 0xff) as usize]
            ^ t[0][(hi >> 24) as usize];
    }
    for &b in chunks.remainder() {
        c = t[0][((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

pub fn crc32(data: &[u8]) -> u32 {
    crc32_update(0, data)
}

const ADLER_BASE: u32 = 65521;

pub fn adler32_update(adler: u32, data: &[u8]) -> u32 {
    let (mut a, mut b) = (adler & 0xffff, adler >> 16);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= ADLER_BASE;
        b %= ADLER_BASE;
    }
    (b << 16) | a
}

pub fn adler32(data: &[u8]) -> u32 {
    adler32_update(1, data)
}

/// Adler-32 of the concatenation A||B from adler(A), adler(B) and len(B).
pub fn adler32_combine(adler1: u32, adler2: u32, len2: u64) -> u32 {
    let rem = (len2 % ADLER_BASE as u64) as u32;
    let mut sum1 = adler1 & 0xffff;
    let mut sum2 = (rem as u64 * sum1 as u64 % ADLER_BASE as u64) as u32;
    sum1 += (adler2 & 0xffff) + ADLER_BASE - 1;
    sum2 += (adler1 >> 16) + (adler2 >> 16) + ADLER_BASE - rem;
    if sum1 >= ADLER_BASE {
        sum1 -= ADLER_BASE;
    }
    if sum1 >= ADLER_BASE {
        sum1 -= ADLER_BASE;
    }
    if sum2 >= ADLER_BASE << 1 {
        sum2 -= ADLER_BASE << 1;
    }
    if sum2 >= ADLER_BASE {
        sum2 -= ADLER_BASE;
    }
    sum1 | (sum2 << 16)
}

// ---------------------------------------------------------------- tables

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

// ---------------------------------------------------------------- inflate

const FAST_BITS: u32 = 10;

struct Huffman {
    fast: Vec<u16>, // (symbol << 4) | length, 0 = use the slow path
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Huffman, String> {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[l as usize] += 1;
        }
        count[0] = 0;
        // over-subscription check (incomplete codes are tolerated, as in zlib for single codes)
        let mut left: i32 = 1;
        for c in count.iter().skip(1) {
            left = (left << 1) - *c as i32;
            if left < 0 {
                return Err("invalid Huffman code (over-subscribed)".into());
            }
        }
        let mut offs = [0u16; 16];
        for len in 1..15 {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        // fast table
        let mut fast = vec![0u16; 1 << FAST_BITS];
        let mut next = [0u32; 16];
        let mut code = 0u32;
        for len in 1..16 {
            code = (code + count[len - 1] as u32) << 1;
            next[len] = code;
        }
        for (sym, &l) in lengths.iter().enumerate() {
            let l = l as u32;
            if l == 0 {
                continue;
            }
            let c = next[l as usize];
            next[l as usize] += 1;
            if l <= FAST_BITS {
                let rev = c.reverse_bits() >> (32 - l);
                let mut i = rev;
                while i < (1 << FAST_BITS) {
                    fast[i as usize] = ((sym as u16) << 4) | l as u16;
                    i += 1 << l;
                }
            }
        }
        Ok(Huffman { fast, count, symbol })
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    cnt: u32,
    overrun: u32, // bits consumed past the end of input
}

impl<'a> BitReader<'a> {
    #[inline]
    fn refill(&mut self) {
        while self.cnt <= 56 {
            let b = if self.pos < self.data.len() {
                self.data[self.pos]
            } else {
                self.overrun += 8;
                0
            };
            self.pos += 1;
            self.buf |= (b as u64) << self.cnt;
            self.cnt += 8;
        }
    }
    #[inline]
    fn bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        if self.cnt < n {
            self.refill();
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.cnt -= n;
        v
    }
    #[inline]
    fn decode(&mut self, h: &Huffman) -> Result<u16, String> {
        if self.cnt < 15 {
            self.refill();
        }
        let e = h.fast[(self.buf & ((1 << FAST_BITS) - 1)) as usize];
        if e != 0 {
            let l = (e & 15) as u32;
            self.buf >>= l;
            self.cnt -= l;
            return Ok(e >> 4);
        }
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= (self.buf & 1) as i32;
            self.buf >>= 1;
            self.cnt -= 1;
            let count = h.count[len] as i32;
            if code - count < first {
                return Ok(h.symbol[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("invalid Huffman code in stream".into())
    }
    /// True if more bits were consumed than the input provided.
    fn exhausted(&self) -> bool {
        self.overrun > self.cnt
    }
    fn align_byte(&mut self) {
        let drop = self.cnt % 8;
        self.buf >>= drop;
        self.cnt -= drop;
    }
}

fn fixed_tables() -> (Huffman, Huffman) {
    let mut l = [0u8; 288];
    for (i, v) in l.iter_mut().enumerate() {
        *v = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    (Huffman::new(&l).unwrap(), Huffman::new(&[5u8; 30]).unwrap())
}

/// Inflate a raw DEFLATE stream. `limit` bounds the output size (decompression-bomb guard).
/// Returns (output, number of input bytes consumed).
pub fn inflate_raw(data: &[u8], size_hint: usize, limit: usize) -> Result<(Vec<u8>, usize), String> {
    let mut out: Vec<u8> = Vec::with_capacity(size_hint.min(limit).min(1 << 30));
    let mut br = BitReader { data, pos: 0, buf: 0, cnt: 0, overrun: 0 };
    let mut fixed: Option<(Huffman, Huffman)> = None;
    loop {
        let last = br.bits(1);
        let kind = br.bits(2);
        match kind {
            0 => {
                br.align_byte();
                let len = br.bits(16) as usize;
                let nlen = br.bits(16) as usize;
                if len != (!nlen & 0xffff) {
                    return Err("stored block length mismatch".into());
                }
                if out.len() + len > limit {
                    return Err("decompressed data exceeds the size limit".into());
                }
                // bytes still buffered in the bit reader come first
                let mut remaining = len;
                while remaining > 0 && br.cnt >= 8 {
                    out.push(br.bits(8) as u8);
                    remaining -= 1;
                }
                if remaining > 0 {
                    if br.pos + remaining > data.len() {
                        return Err("unexpected end of deflate data".into());
                    }
                    out.extend_from_slice(&data[br.pos..br.pos + remaining]);
                    br.pos += remaining;
                }
            }
            1 | 2 => {
                let dynamic;
                let (lit, dist): (&Huffman, &Huffman) = if kind == 1 {
                    let f = fixed.get_or_insert_with(fixed_tables);
                    (&f.0, &f.1)
                } else {
                    let hlit = br.bits(5) as usize + 257;
                    let hdist = br.bits(5) as usize + 1;
                    let hclen = br.bits(4) as usize + 4;
                    if hlit > 286 || hdist > 30 {
                        return Err("bad dynamic block header".into());
                    }
                    let mut cl = [0u8; 19];
                    for &o in CL_ORDER.iter().take(hclen) {
                        cl[o] = br.bits(3) as u8;
                    }
                    let clh = Huffman::new(&cl)?;
                    let mut lengths = vec![0u8; hlit + hdist];
                    let mut i = 0;
                    while i < hlit + hdist {
                        let sym = br.decode(&clh)?;
                        match sym {
                            0..=15 => {
                                lengths[i] = sym as u8;
                                i += 1;
                            }
                            16 => {
                                if i == 0 {
                                    return Err("repeat with no previous length".into());
                                }
                                let prev = lengths[i - 1];
                                let rep = 3 + br.bits(2) as usize;
                                if i + rep > hlit + hdist {
                                    return Err("too many code lengths".into());
                                }
                                lengths[i..i + rep].fill(prev);
                                i += rep;
                            }
                            _ => {
                                let rep = if sym == 17 { 3 + br.bits(3) as usize } else { 11 + br.bits(7) as usize };
                                if i + rep > hlit + hdist {
                                    return Err("too many code lengths".into());
                                }
                                i += rep;
                            }
                        }
                        if br.exhausted() {
                            return Err("unexpected end of deflate data".into());
                        }
                    }
                    if lengths[256] == 0 {
                        return Err("missing end-of-block code".into());
                    }
                    dynamic = (Huffman::new(&lengths[..hlit])?, Huffman::new(&lengths[hlit..])?);
                    (&dynamic.0, &dynamic.1)
                };
                loop {
                    let sym = br.decode(lit)? as usize;
                    if sym < 256 {
                        if out.len() >= limit {
                            return Err("decompressed data exceeds the size limit".into());
                        }
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let s = sym - 257;
                        if s >= 29 {
                            return Err("invalid length symbol".into());
                        }
                        let len = LEN_BASE[s] as usize + br.bits(LEN_EXTRA[s] as u32) as usize;
                        let ds = br.decode(dist)? as usize;
                        if ds >= 30 {
                            return Err("invalid distance symbol".into());
                        }
                        let d = DIST_BASE[ds] as usize + br.bits(DIST_EXTRA[ds] as u32) as usize;
                        if d > out.len() {
                            return Err("distance too far back".into());
                        }
                        if out.len() + len > limit {
                            return Err("decompressed data exceeds the size limit".into());
                        }
                        let start = out.len() - d;
                        if d >= len {
                            out.extend_from_within(start..start + len);
                        } else {
                            for k in 0..len {
                                let b = out[start + k];
                                out.push(b);
                            }
                        }
                    }
                    if br.exhausted() {
                        return Err("unexpected end of deflate data".into());
                    }
                }
            }
            _ => return Err("invalid deflate block type".into()),
        }
        if br.exhausted() {
            return Err("unexpected end of deflate data".into());
        }
        if last == 1 {
            break;
        }
    }
    let consumed = br.pos - (br.cnt / 8) as usize;
    Ok((out, consumed.min(data.len())))
}

/// Inflate a zlib stream (header + deflate + Adler-32).
pub fn zlib_decompress(data: &[u8], size_hint: usize, limit: usize) -> Result<Vec<u8>, String> {
    if data.len() < 6 {
        return Err("zlib stream too short".into());
    }
    let (cmf, flg) = (data[0], data[1]);
    if cmf & 0x0f != 8 || (cmf as u32 * 256 + flg as u32) % 31 != 0 {
        return Err("bad zlib header".into());
    }
    if flg & 0x20 != 0 {
        return Err("zlib preset dictionaries are not supported".into());
    }
    let (out, used) = inflate_raw(&data[2..], size_hint, limit)?;
    let tail = 2 + used;
    if tail + 4 > data.len() {
        return Err("zlib stream is missing its checksum".into());
    }
    let want = u32::from_be_bytes([data[tail], data[tail + 1], data[tail + 2], data[tail + 3]]);
    if adler32(&out) != want {
        return Err("zlib checksum mismatch (corrupt data)".into());
    }
    Ok(out)
}

// ---------------------------------------------------------------- deflate

struct BitWriter {
    out: Vec<u8>,
    buf: u64,
    n: u32,
}

impl BitWriter {
    #[inline]
    fn put(&mut self, v: u32, bits: u32) {
        self.buf |= (v as u64) << self.n;
        self.n += bits;
        while self.n >= 8 {
            self.out.push(self.buf as u8);
            self.buf >>= 8;
            self.n -= 8;
        }
    }
    fn align(&mut self) {
        if self.n > 0 {
            self.out.push(self.buf as u8);
            self.buf = 0;
            self.n = 0;
        }
    }
}

/// Length-limited Huffman code lengths (heuristic of miniz/zlib family).
fn code_lengths(freq: &[u32], max_len: usize) -> Vec<u8> {
    let mut lens = vec![0u8; freq.len()];
    let mut syms: Vec<(u32, u16)> = freq.iter().enumerate().filter(|(_, f)| **f > 0).map(|(i, f)| (*f, i as u16)).collect();
    if syms.is_empty() {
        return lens;
    }
    if syms.len() == 1 {
        lens[syms[0].1 as usize] = 1;
        return lens;
    }
    syms.sort();
    let n = syms.len();
    // two-queue Huffman construction
    let mut weight: Vec<u64> = syms.iter().map(|s| s.0 as u64).collect();
    let mut parent = vec![usize::MAX; 2 * n - 1];
    weight.resize(2 * n - 1, 0);
    let (mut leaf, mut internal) = (0usize, n);
    for next in n..2 * n - 1 {
        let pick = |leaf: &mut usize, internal: &mut usize| -> usize {
            if *leaf < n && (*internal >= next || weight[*leaf] <= weight[*internal]) {
                *leaf += 1;
                *leaf - 1
            } else {
                *internal += 1;
                *internal - 1
            }
        };
        let a = pick(&mut leaf, &mut internal);
        let b = pick(&mut leaf, &mut internal);
        weight[next] = weight[a] + weight[b];
        parent[a] = next;
        parent[b] = next;
    }
    let mut depth = vec![0u32; 2 * n - 1];
    for i in (0..2 * n - 2).rev() {
        depth[i] = depth[parent[i]] + 1;
    }
    let mut num = [0i32; 64];
    for d in depth.iter().take(n) {
        num[(*d as usize).min(63)] += 1;
    }
    // enforce the maximum code size
    for i in max_len + 1..64 {
        num[max_len] += num[i];
        num[i] = 0;
    }
    let mut total: u64 = 0;
    for i in (1..=max_len).rev() {
        total += (num[i] as u64) << (max_len - i);
    }
    while total != 1u64 << max_len {
        num[max_len] -= 1;
        for i in (1..max_len).rev() {
            if num[i] != 0 {
                num[i] -= 1;
                num[i + 1] += 2;
                break;
            }
        }
        total -= 1;
    }
    let mut j = n;
    for (i, cnt) in num.iter().enumerate().take(max_len + 1).skip(1) {
        for _ in 0..*cnt {
            j -= 1;
            lens[syms[j].1 as usize] = i as u8;
        }
    }
    lens
}

/// Canonical codes, already bit-reversed for LSB-first output.
fn canonical_codes(lens: &[u8]) -> Vec<u16> {
    let mut count = [0u32; 16];
    for &l in lens {
        count[l as usize] += 1;
    }
    count[0] = 0;
    let mut next = [0u32; 16];
    let mut code = 0u32;
    for len in 1..16 {
        code = (code + count[len - 1]) << 1;
        next[len] = code;
    }
    lens.iter()
        .map(|&l| {
            if l == 0 {
                0
            } else {
                let c = next[l as usize];
                next[l as usize] += 1;
                (c.reverse_bits() >> (32 - l as u32)) as u16
            }
        })
        .collect()
}

#[inline]
fn dist_code(d: u32) -> u32 {
    let d = d - 1;
    if d < 4 {
        d
    } else {
        let l = 31 - d.leading_zeros();
        2 * l + ((d >> (l - 1)) & 1)
    }
}

fn len_code_table() -> &'static [u8; 259] {
    use std::sync::OnceLock;
    static T: OnceLock<[u8; 259]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u8; 259];
        for (len, e) in t.iter_mut().enumerate().skip(3) {
            let mut c = 28;
            while LEN_BASE[c] as usize > len {
                c -= 1;
            }
            *e = c as u8;
        }
        t
    })
}

struct Level {
    max_chain: u32,
    nice: usize,
    lazy: bool,
}

fn level_params(level: u8) -> Level {
    match level {
        0 | 1 => Level { max_chain: 4, nice: 16, lazy: false },
        2 | 3 => Level { max_chain: 12, nice: 32, lazy: false },
        4..=6 => Level { max_chain: 32, nice: 128, lazy: true },
        _ => Level { max_chain: 256, nice: 258, lazy: true },
    }
}

/// Token: literal (dist == 0, value = byte) or match (value = length, dist > 0).
#[derive(Clone, Copy)]
struct Token {
    value: u16,
    dist: u16,
}

fn write_block(bw: &mut BitWriter, tokens: &[Token], raw: &[u8], last: bool) {
    let lct = len_code_table();
    let mut lf = [0u32; 286];
    let mut df = [0u32; 30];
    for t in tokens {
        if t.dist == 0 {
            lf[t.value as usize] += 1;
        } else {
            lf[257 + lct[t.value as usize] as usize] += 1;
            df[dist_code(t.dist as u32) as usize] += 1;
        }
    }
    lf[256] = 1;
    if df.iter().filter(|f| **f > 0).count() < 2 {
        // keep the distance code complete (two codes of one bit) for strict decoders
        df[0] = df[0].max(1);
        df[1] = df[1].max(1);
    }
    let ll = code_lengths(&lf, 15);
    let dl = code_lengths(&df, 15);
    let hlit = (257..=286).rev().find(|&n| ll[n - 1] != 0).unwrap_or(257);
    let hdist = (1..=30).rev().find(|&n| dl[n - 1] != 0).unwrap_or(1);

    // run-length encode the code lengths
    let mut seq: Vec<u8> = Vec::with_capacity(hlit + hdist);
    seq.extend_from_slice(&ll[..hlit]);
    seq.extend_from_slice(&dl[..hdist]);
    let mut rle: Vec<(u8, u8)> = Vec::new(); // (symbol, extra value)
    let mut i = 0;
    while i < seq.len() {
        let v = seq[i];
        let mut run = 1;
        while i + run < seq.len() && seq[i + run] == v {
            run += 1;
        }
        let mut left = run;
        if v == 0 {
            while left >= 11 {
                let r = left.min(138);
                rle.push((18, (r - 11) as u8));
                left -= r;
            }
            if left >= 3 {
                rle.push((17, (left - 3) as u8));
                left = 0;
            }
            for _ in 0..left {
                rle.push((0, 0));
            }
        } else {
            rle.push((v, 0));
            left -= 1;
            while left >= 3 {
                let r = left.min(6);
                rle.push((16, (r - 3) as u8));
                left -= r;
            }
            for _ in 0..left {
                rle.push((v, 0));
            }
        }
        i += run;
    }
    let mut cf = [0u32; 19];
    for (s, _) in &rle {
        cf[*s as usize] += 1;
    }
    let cl = code_lengths(&cf, 7);
    let hclen = (4..=19).rev().find(|&n| cl[CL_ORDER[n - 1]] != 0).unwrap_or(4);

    // cost comparison against a stored block
    let mut bits: u64 = 3 + 5 + 5 + 4 + 3 * hclen as u64;
    for (s, _) in &rle {
        bits += cl[*s as usize] as u64 + match s {
            16 => 2,
            17 => 3,
            18 => 7,
            _ => 0,
        };
    }
    for (s, f) in lf.iter().enumerate() {
        let extra = if s >= 257 { LEN_EXTRA[s - 257] as u64 } else { 0 };
        bits += *f as u64 * (ll[s] as u64 + extra);
    }
    for (s, f) in df.iter().enumerate() {
        bits += *f as u64 * (dl[s] as u64 + DIST_EXTRA[s] as u64);
    }
    let stored_bits = (raw.len() as u64 + 5 * (raw.len() as u64 / 65535 + 1)) * 8 + 8;
    if stored_bits < bits {
        write_stored(bw, raw, last);
        return;
    }

    let lc = canonical_codes(&ll);
    let dc = canonical_codes(&dl);
    let cc = canonical_codes(&cl);
    bw.put(last as u32, 1);
    bw.put(2, 2);
    bw.put((hlit - 257) as u32, 5);
    bw.put((hdist - 1) as u32, 5);
    bw.put((hclen - 4) as u32, 4);
    for &o in CL_ORDER.iter().take(hclen) {
        bw.put(cl[o] as u32, 3);
    }
    for (s, x) in &rle {
        bw.put(cc[*s as usize] as u32, cl[*s as usize] as u32);
        match s {
            16 => bw.put(*x as u32, 2),
            17 => bw.put(*x as u32, 3),
            18 => bw.put(*x as u32, 7),
            _ => {}
        }
    }
    for t in tokens {
        if t.dist == 0 {
            bw.put(lc[t.value as usize] as u32, ll[t.value as usize] as u32);
        } else {
            let c = lct[t.value as usize] as usize;
            bw.put(lc[257 + c] as u32, ll[257 + c] as u32);
            bw.put(t.value as u32 - LEN_BASE[c] as u32, LEN_EXTRA[c] as u32);
            let d = dist_code(t.dist as u32) as usize;
            bw.put(dc[d] as u32, dl[d] as u32);
            bw.put(t.dist as u32 - DIST_BASE[d] as u32, DIST_EXTRA[d] as u32);
        }
    }
    bw.put(lc[256] as u32, ll[256] as u32);
}

fn write_stored(bw: &mut BitWriter, raw: &[u8], last: bool) {
    let mut chunks = raw.chunks(65535).peekable();
    if chunks.peek().is_none() {
        bw.put(last as u32, 1);
        bw.put(0, 2);
        bw.align();
        bw.out.extend_from_slice(&[0, 0, 0xff, 0xff]);
        return;
    }
    while let Some(c) = chunks.next() {
        let fin = last && chunks.peek().is_none();
        bw.put(fin as u32, 1);
        bw.put(0, 2);
        bw.align();
        let n = c.len() as u16;
        bw.out.extend_from_slice(&n.to_le_bytes());
        bw.out.extend_from_slice(&(!n).to_le_bytes());
        bw.out.extend_from_slice(c);
    }
}

const NIL: u32 = u32::MAX;
const HASH_BITS: u32 = 15;
const WINDOW: usize = 32768;
const BLOCK_TOKENS: usize = 1 << 16;

/// Compress one independent chunk into a byte-aligned run of deflate blocks.
/// When `last` is false the chunk ends with an empty stored block (sync flush),
/// so chunks can be concatenated - this is what makes parallel compression
/// deterministic: the output depends only on the chunk size, never on threads.
pub fn deflate_chunk(data: &[u8], level: u8, last: bool) -> Vec<u8> {
    let mut bw = BitWriter { out: Vec::with_capacity(data.len() / 2 + 64), buf: 0, n: 0 };
    if level == 0 || data.len() < 16 {
        write_stored(&mut bw, data, last);
        if !last {
            write_stored(&mut bw, &[], false);
        }
        return bw.out;
    }
    let p = level_params(level);
    let n = data.len();
    let mut head = vec![NIL; 1 << HASH_BITS];
    let mut prev = vec![NIL; n];
    let hash = |i: usize| -> usize {
        let v = (data[i] as u32) | ((data[i + 1] as u32) << 8) | ((data[i + 2] as u32) << 16);
        (v.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
    };
    let find = |pos: usize, head: &[u32], prev: &[u32], min_len: usize| -> (usize, usize) {
        let max_len = (n - pos).min(258);
        if max_len < 3 {
            return (0, 0);
        }
        let (mut best_len, mut best_dist) = (min_len, 0usize);
        let mut cand = head[hash(pos)];
        let mut chain = p.max_chain;
        while cand != NIL && chain > 0 {
            let c = cand as usize;
            if pos - c > WINDOW {
                break;
            }
            if best_len < max_len && data[c + best_len] == data[pos + best_len] {
                let mut l = 0;
                while l < max_len && data[c + l] == data[pos + l] {
                    l += 1;
                }
                if l > best_len {
                    best_len = l;
                    best_dist = pos - c;
                    if l >= p.nice || l == max_len {
                        break;
                    }
                }
            }
            cand = prev[c];
            chain -= 1;
        }
        if best_dist == 0 {
            (0, 0)
        } else {
            (best_len, best_dist)
        }
    };

    let mut tokens: Vec<Token> = Vec::with_capacity(BLOCK_TOKENS);
    let mut block_start = 0usize;
    let mut pos = 0usize;
    let insert = |i: usize, head: &mut [u32], prev: &mut [u32]| {
        if i + 2 < n {
            let h = hash(i);
            prev[i] = head[h];
            head[h] = i as u32;
        }
    };
    while pos < n {
        let (mut len, mut dist) = if pos + 2 < n { find(pos, &head, &prev, 2) } else { (0, 0) };
        insert(pos, &mut head, &mut prev);
        if len >= 3 && p.lazy && len < p.nice && pos + 3 < n {
            let (l2, d2) = find(pos + 1, &head, &prev, len);
            if l2 > len {
                tokens.push(Token { value: data[pos] as u16, dist: 0 });
                pos += 1;
                insert(pos, &mut head, &mut prev);
                len = l2;
                dist = d2;
            }
        }
        if len >= 3 {
            tokens.push(Token { value: len as u16, dist: dist as u16 });
            for k in 1..len {
                insert(pos + k, &mut head, &mut prev);
            }
            pos += len;
        } else {
            tokens.push(Token { value: data[pos] as u16, dist: 0 });
            pos += 1;
        }
        if tokens.len() >= BLOCK_TOKENS {
            write_block(&mut bw, &tokens, &data[block_start..pos], last && pos == n);
            tokens.clear();
            block_start = pos;
        }
    }
    if !tokens.is_empty() || (last && block_start == 0 && n == 0) {
        write_block(&mut bw, &tokens, &data[block_start..n], last);
    } else if last && block_start == n {
        // data ended exactly on a block boundary: the previous block already carried BFINAL
    }
    if !last {
        write_stored(&mut bw, &[], false);
    }
    bw.align();
    bw.out
}

pub const CHUNK: usize = 1 << 19;

/// zlib-compress `data`. Output is identical for every `threads` value.
pub fn zlib_compress(data: &[u8], level: u8, threads: usize) -> Vec<u8> {
    let nchunks = data.len().div_ceil(CHUNK).max(1);
    let parts = par::par_map(nchunks, threads, |i| {
        let s = i * CHUNK;
        let e = (s + CHUNK).min(data.len());
        deflate_chunk(&data[s..e], level, i + 1 == nchunks)
    });
    let mut out = Vec::with_capacity(parts.iter().map(|p| p.len()).sum::<usize>() + 6);
    out.extend_from_slice(&zlib_header(level));
    for p in parts {
        out.extend_from_slice(&p);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

pub fn zlib_header(level: u8) -> [u8; 2] {
    let flevel: u16 = match level {
        0 | 1 => 0,
        2..=5 => 1,
        6 => 2,
        _ => 3,
    };
    let cmf: u16 = 0x78;
    let mut flg = flevel << 6;
    flg += 31 - ((cmf * 256 + flg) % 31);
    if (cmf * 256 + flg) % 31 != 0 {
        flg -= 31;
    }
    [cmf as u8, flg as u8]
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::rng::Rng;

    #[test]
    fn checksums_known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        let mut r = Rng::new(1, 1);
        let a: Vec<u8> = (0..70_000).map(|_| r.next_u32() as u8).collect();
        let b: Vec<u8> = (0..12_345).map(|_| r.next_u32() as u8).collect();
        let mut ab = a.clone();
        ab.extend_from_slice(&b);
        assert_eq!(adler32_combine(adler32(&a), adler32(&b), b.len() as u64), adler32(&ab));
        // incremental crc == one-shot crc
        assert_eq!(crc32_update(crc32(&a), &b), crc32(&ab));
    }

    fn corpus() -> Vec<Vec<u8>> {
        let mut r = Rng::new(42, 7);
        let mut v: Vec<Vec<u8>> = vec![
            vec![],
            vec![7],
            b"hello hello hello hello hello hello".to_vec(),
            vec![0u8; 100_000],
            (0..300_000u32).map(|i| (i % 251) as u8).collect(),
            (0..200_000).map(|_| r.next_u32() as u8).collect(), // incompressible
            (0..400_000).map(|_| (r.below(4) * 60) as u8).collect(), // low entropy
        ];
        // image-like: smooth gradient with a little noise, bigger than one chunk
        v.push((0..1_500_000u32).map(|i| ((i / 3000) as u8).wrapping_add((r.below(3)) as u8)).collect());
        // text-like with long repeats
        let mut t = Vec::new();
        for i in 0..20_000 {
            t.extend_from_slice(format!("sphere {{ at: [{}, 0, {}], radius: 0.5 }}\n", i % 17, i % 5).as_bytes());
        }
        v.push(t);
        v
    }

    #[test]
    fn roundtrip_all_levels() {
        for data in corpus() {
            for level in [0u8, 1, 3, 6, 9] {
                let z = zlib_compress(&data, level, 2);
                let back = zlib_decompress(&z, data.len(), data.len() + 16).expect("inflate");
                assert_eq!(back.len(), data.len(), "level {level}");
                assert!(back == data, "roundtrip mismatch at level {level} (len {})", data.len());
            }
        }
    }

    #[test]
    fn thread_count_does_not_change_bytes() {
        let data = corpus().pop().unwrap();
        let a = zlib_compress(&data, 6, 1);
        let b = zlib_compress(&data, 6, 4);
        assert_eq!(a, b);
        assert!(a.len() < data.len() / 8, "text should compress well: {} -> {}", data.len(), a.len());
    }

    #[test]
    fn compression_is_effective() {
        let zeros = vec![0u8; 1_000_000];
        assert!(zlib_compress(&zeros, 6, 1).len() < 3000);
        let mut r = Rng::new(3, 3);
        let noise: Vec<u8> = (0..100_000).map(|_| r.next_u32() as u8).collect();
        assert!(zlib_compress(&noise, 6, 1).len() < noise.len() + 200, "incompressible data must fall back to stored blocks");
    }

    #[test]
    fn inflates_reference_streams() {
        // zlib.compress(b"hello hello hello hello", 9) from CPython 3.11 (fixed-Huffman block)
        let z = [0x78, 0xda, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01, 0x68, 0x03, 0x08, 0xb1];
        assert_eq!(zlib_decompress(&z, 0, 1000).unwrap(), b"hello hello hello hello");
        // stored block
        let s = [0x78, 0x01, 0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c', 0x02, 0x4d, 0x01, 0x27];
        assert_eq!(zlib_decompress(&s, 0, 1000).unwrap(), b"abc");
    }

    #[test]
    fn rejects_corrupt_and_oversized() {
        let data: Vec<u8> = (0..50_000u32).map(|i| (i % 97) as u8).collect();
        let mut z = zlib_compress(&data, 6, 1);
        assert!(zlib_decompress(&z, 0, 1000).is_err(), "size limit");
        let n = z.len();
        z[n - 1] ^= 0xff;
        assert!(zlib_decompress(&z, 0, 1 << 20).is_err(), "checksum");
        assert!(zlib_decompress(&z[..n / 2], 0, 1 << 20).is_err(), "truncated");
        assert!(zlib_decompress(&[0x78, 0x9c, 0xff, 0xff, 0xff, 0xff, 0, 0], 0, 1 << 20).is_err());
    }
}

#[cfg(test)]
mod interop_tests {
    use super::*;

    /// A stream produced by the reference zlib implementation (dynamic Huffman blocks).
    #[test]
    fn inflates_reference_zlib_dynamic_stream() {
        let raw = include_bytes!("../tests/data/ref_dynamic.bin");
        let z = include_bytes!("../tests/data/ref_dynamic.zlib");
        assert_eq!((z[2] >> 1) & 3, 2, "fixture must start with a dynamic block");
        let out = zlib_decompress(z, raw.len(), raw.len() + 1).unwrap();
        assert!(out == raw);
        // and our own compressor should be in the same league as zlib -9 on this data
        let ours = zlib_compress(raw, 9, 1);
        assert!(ours.len() as f64 <= z.len() as f64 * 1.08, "ours {} vs zlib {}", ours.len(), z.len());
    }
}
