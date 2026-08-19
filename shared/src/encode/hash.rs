use std::{fs::File, io, path::Path};

pub fn hash_file(path: &Path) -> std::result::Result<blake3::Hash, io::Error> {
    let file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(file)?;
    Ok(hasher.finalize())
}
