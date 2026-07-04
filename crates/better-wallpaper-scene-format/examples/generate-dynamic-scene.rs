use std::{env, fs, path::PathBuf};

fn sized_string(value: &str) -> Vec<u8> {
    let mut bytes = (value.len() as u32).to_le_bytes().to_vec();
    bytes.extend_from_slice(value.as_bytes());
    bytes
}

fn animated_texture() -> Vec<u8> {
    let mut bytes = b"TEXV0005\0TEXI0001\0".to_vec();
    bytes.extend_from_slice(&0_u32.to_le_bytes()); // observed RGBA byte layout
    bytes.extend_from_slice(&4_u32.to_le_bytes()); // animated/GIF flag
    for value in [4_u32, 2, 4, 2, 0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(b"TEXB0002\0");
    for value in [1_u32, 1, 4, 2, 0, 0, 32] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    // Two 2x2 frames: cyan followed by orange. The unequal channels make
    // accidental R/B swaps visible in both automated and manual checks.
    for _row in 0..2 {
        for _ in 0..2 {
            bytes.extend_from_slice(&[0, 220, 255, 255]);
        }
        for _ in 0..2 {
            bytes.extend_from_slice(&[255, 90, 0, 255]);
        }
    }
    bytes.extend_from_slice(b"TEXS0002\0");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    for (number, x) in [(0_u32, 0.0_f32), (1, 2.0)] {
        bytes.extend_from_slice(&number.to_le_bytes());
        bytes.extend_from_slice(&0.5_f32.to_le_bytes());
        for value in [x, 0.0, 2.0, 2.0, 0.0, 0.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

fn package(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = sized_string("PKGV0001");
    bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    let mut offset = 0_u32;
    for (name, data) in entries {
        bytes.extend_from_slice(&sized_string(name));
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in entries {
        bytes.extend_from_slice(data);
    }
    bytes
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: generate-dynamic-scene <output-directory>")?;
    fs::create_dir_all(&output)?;
    let scene = br#"{"general":{"orthogonalprojection":{"width":4,"height":2}},"objects":[{"id":"animated","name":"Animated color check","image":"models/animated.json","origin":"2 1 0","size":"4 2"}]}"#.to_vec();
    let model = br#"{"material":"materials/animated.json"}"#.to_vec();
    let material =
        br#"{"passes":[{"blending":"opaque","shader":"genericimage4","textures":["animated"]}]}"#
            .to_vec();
    let pkg = package(&[
        ("scene.json", scene),
        ("models/animated.json", model),
        ("materials/animated.json", material),
        ("materials/animated.tex", animated_texture()),
    ]);
    fs::write(output.join("scene.pkg"), pkg)?;
    fs::write(
        output.join("project.json"),
        br#"{"title":"Better Wallpaper dynamic color check","type":"scene","file":"scene.json"}"#,
    )?;
    println!("generated {}", output.display());
    Ok(())
}
