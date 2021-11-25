use png::{BitDepth, ColorType};
use qoi_rs::{ChannelCount, write_to_file};
use std::{fs::File, io::Result, path::{Path, PathBuf}};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let help = "Usage: <img.png> <img.qoi> OR <img.qoi> <img.png>";
    let source: PathBuf = args.next().expect(help).into();
    let dest: PathBuf = args.next().expect(help).into();

    let source_ext = source.extension().and_then(|e| e.to_str());
    let dest_ext = dest.extension().and_then(|e| e.to_str());

    match (source_ext, dest_ext) {
        (Some("png"), Some("qoi")) => {
            png_to_qoi(source, dest)
        },
        (Some("qoi"), Some("png")) => {
            todo!("QOI decoding")
        }
        _ => {
            eprintln!("{}", help);
            Ok(())
        }
    }
}

fn png_to_qoi(source: impl AsRef<Path>, dest: impl AsRef<Path>) -> Result<()> {
    let decoder = png::Decoder::new(File::open(source)?);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    let bytes = &buf[..info.buffer_size()];

    assert_eq!(info.bit_depth, BitDepth::Eight);
    let channels = match info.color_type {
        ColorType::Rgb => ChannelCount::Rgb,
        ColorType::Rgba => ChannelCount::Rgba,
        other => panic!("Unsupported color type {:?}, supports only RGB, RGBA", other),
    };

    write_to_file(dest, bytes, info.width as _, channels)
}