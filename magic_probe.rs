fn main() {
    // Reproduce the exact construct from fontcache.rs:258 — a &[u8] slice of
    // unknown static length matched against byte-string literal patterns.
    let ttf: Vec<u8> = vec![0x00, 0x01, 0x00, 0x00, 0xAA, 0xBB];
    let magic = &ttf[..4.min(ttf.len())]; // type: &[u8]
    let via_matches = matches!(magic, b"\x00\x01\x00\x00" | b"true" | b"OTTO" | b"ttcf");

    // The unambiguous form.
    const SIGS: [&[u8]; 4] = [&[0x00, 0x01, 0x00, 0x00], b"true", b"OTTO", b"ttcf"];
    let via_eq = SIGS.iter().any(|sig| magic == *sig);

    println!("matches!()={via_matches}  eq={via_eq}");
}
