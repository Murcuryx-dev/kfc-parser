use std::fs;
use std::env;
use std::path::{Path, PathBuf};

fn main() -> std::io::Result<()> {
    generate_definitions_file()?;

    // The id of this build of the loader: a cache written by another build is not
    // trusted. It is a fingerprint of the loader's source and not the time of the
    // build, so that building the same source again gives the same files.
    println!("cargo:rustc-env=BUILD_ID={:016x}", source_fingerprint()?);

    Ok(())
}

fn generate_definitions_file() -> std::io::Result<()> {
    let definitions_dir = Path::new("definitions");

    println!("cargo:rerun-if-changed={}", definitions_dir.display());

    let mut lua_files = Vec::new();

    collect_lua_files(definitions_dir, &mut lua_files)?;

    lua_files.sort();

    for file in &lua_files {
        println!("cargo:rerun-if-changed={}", file.display());
    }

    let mut combined = String::new();

    combined.push_str("--- @meta\n");

    for file in lua_files {
        let content = fs::read_to_string(&file)?;

        if let Some(pos) = content.find('\n') {
            if content[..pos].trim().contains("@meta") {
                combined.push_str(&content[pos + 1..]);
            } else {
                combined.push_str(&content);
            }
        } else {
            combined.push_str(&content);
        }
    }

    let out_dir = env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join("definitions.lua");

    let need_write = match fs::read_to_string(&out_path) {
        Ok(existing) => existing != combined,
        Err(_) => true,
    };

    if need_write {
        std::fs::write(&out_path, combined.as_bytes())?;
    }

    Ok(())
}

fn collect_lua_files(
    dir: &Path,
    out: &mut Vec<PathBuf>
) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();

        if p.is_dir() {
            collect_lua_files(&p, out)?;
        } else if p.extension().and_then(|s| s.to_str()) == Some("lua") {
            out.push(p);
        }
    }
    Ok(())
}

/// A fingerprint of every source file of the workspace's crates and of the lock file.
/// Line endings do not count, so a checkout on Windows gives the same value.
fn source_fingerprint() -> std::io::Result<u64> {
    let crates_dir = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .expect("the crate is inside the crates directory")
        .to_path_buf();
    let lock_file = crates_dir.parent()
        .expect("the crates directory is inside the workspace")
        .join("Cargo.lock");

    println!("cargo:rerun-if-changed={}", crates_dir.display());
    println!("cargo:rerun-if-changed={}", lock_file.display());

    let mut files = Vec::new();

    collect_source_files(&crates_dir, &mut files)?;
    files.sort();

    if lock_file.is_file() {
        files.push(lock_file);
    }

    let mut hash: u64 = 0xcbf29ce484222325;
    let mut add = |bytes: &[u8]| {
        for byte in bytes {
            if *byte != b'\r' {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    };

    for file in files {
        let name = file.strip_prefix(&crates_dir).unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");

        add(name.as_bytes());
        add(&fs::read(&file)?);
    }

    Ok(hash)
}

fn collect_source_files(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();

        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "target") {
                collect_source_files(&path, files)?;
            }
        } else if path.extension().is_some_and(|ext| ext == "rs" || ext == "lua" || ext == "toml" || ext == "def") {
            files.push(path);
        }
    }

    Ok(())
}
