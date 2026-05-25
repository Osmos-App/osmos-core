/// Returns the BLAKE3 hex digest of `data`.
pub fn blake3_hex(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

/// Returns the BLAKE3 hex digest of a file at `path`, reading it in 64 KiB chunks.
pub fn blake3_file(path: &std::path::Path) -> crate::Result<String> {
    use std::io::Read;
    let mut hasher = blake3::Hasher::new();
    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}
