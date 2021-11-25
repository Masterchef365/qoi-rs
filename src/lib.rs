use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Result, Seek, SeekFrom, Write},
    path::Path,
};

type Rgba = [u8; 4];

const COLOR_LUT_SIZE: usize = 64;
/// The pixel decoded if the first pixel is an RLE command
const DEFAULT_PREV_PIXEL: Rgba = [0, 0, 0, 0xFF];
const MAX_RUN_LENGTH: u32 = 0x2020;
const MAX_RUN_8_LENGTH: u32 = 33;
const MAGIC: &[u8; 4] = b"qoif";

const QOI_PADDING: usize = 4;
const QOI_INDEX: u8 = 0b00000000; // 00xxxxxx
const QOI_RUN_8: u8 = 0b01000000; // 010xxxxx
const QOI_RUN_16: u8 = 0b01100000; // 011xxxxx
const QOI_DIFF_8: u8 = 0b10000000; // 10xxxxxx
const QOI_DIFF_16: u8 = 0b11000000; // 110xxxxx
const QOI_DIFF_24: u8 = 0b11100000; // 1110xxxx
const QOI_COLOR: u8 = 0b11110000; // 1111xxxx

const QOI_MASK_2: u8 = 0b11000000; // 11000000
const QOI_MASK_3: u8 = 0b11100000; // 11100000
const QOI_MASK_4: u8 = 0b11110000; // 11110000

pub fn write_to_file(
    path: impl AsRef<Path>,
    data: &[u8],
    width: usize,
    channels: ChannelCount,
) -> Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    encode(&mut writer, data, width, channels)?;
    writer.flush()
}

pub fn read_from_file(
    path: impl AsRef<Path>,
    channels: ChannelCount,
) -> Result<(Vec<u8>, u16, u16)> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    decode(reader, channels)
}

#[derive(Copy, Clone, Debug)]
#[repr(usize)]
pub enum ChannelCount {
    Rgb = 3,
    Rgba = 4,
}

fn color_hash([r, g, b, a]: Rgba) -> u8 {
    r ^ g ^ b ^ a
}

fn subtract_pixels([rx, gx, bx, ax]: Rgba, [ry, gy, by, ay]: Rgba) -> [i32; 4] {
    return [
        rx as i32 - ry as i32,
        gx as i32 - gy as i32,
        bx as i32 - by as i32,
        ax as i32 - ay as i32,
    ];
}

pub fn encode<W: Write + Seek>(
    mut writer: W,
    data: &[u8],
    width: usize,
    channels: ChannelCount,
) -> Result<()> {
    let (width, height, total_pixels) = verify_and_calculate_dims(data, width, channels);

    let size_field_offset = encode_header(&mut writer, width, height)?;

    let mut image_data_len: usize = 0; // Length of image bytes written in bytes

    let mut run: u32 = 0; // Run length encoding run length
    let mut px_prev = DEFAULT_PREV_PIXEL; // Previous pixel
    let mut px = px_prev; // Current pixel
    let mut index = [[0; 4]; COLOR_LUT_SIZE];

    for (pixel_idx, pixel_data) in data.chunks_exact(channels as usize).enumerate() {
        // Copy pixel data
        px[..channels as usize].copy_from_slice(pixel_data);

        // Pixel matches the previous one, increase run length
        let pixel_matches_last = px == px_prev;
        if pixel_matches_last {
            run += 1;
        }

        // There is a run, and we've reached the max run length, the last pixel doesn't match, or we've reached the very last pixel (so we must dump any current run).
        if run > 0
            && (run == MAX_RUN_LENGTH || !pixel_matches_last || pixel_idx + 1 == total_pixels)
        {
            if run < MAX_RUN_8_LENGTH {
                // Write a short run length
                run -= 1;
                let message: u8 = QOI_RUN_8 | run as u8;
                image_data_len += writer.write(&[message])?;
            } else {
                // Write a long run length
                run -= MAX_RUN_8_LENGTH;
                image_data_len += writer.write(&[QOI_RUN_16 | (run >> 8) as u8, run as u8])?;
            }
            run = 0;
        }

        if !pixel_matches_last {
            let index_pos = color_hash(px) % 64;

            if px == index[index_pos as usize] {
                image_data_len += writer.write(&[QOI_INDEX | index_pos])?;
            } else {
                index[index_pos as usize] = px;
                let diff = subtract_pixels(px, px_prev);
                let [vr, vg, vb, va] = diff;

                let within_small_diff = diff.into_iter().all(|v| v > -16 && v < 17);

                if within_small_diff {
                    // Use difference encoding
                    if va == 0 && vr > -2 && vr < 3 && vg > -2 && vg < 3 && vb > -2 && vb < 3 {
                        // Use 2-bit difference encoding
                        image_data_len += writer.write(&[
                            QOI_DIFF_8 | (((vr + 1) << 4) | (vg + 1) << 2 | (vb + 1)) as u8
                        ])?;
                    } else if va == 0
                        && vr > -16
                        && vr < 17
                        && vg > -8
                        && vg < 9
                        && vb > -8
                        && vb < 9
                    {
                        // Use 5 or 4-bit difference encoding
                        image_data_len += writer.write(&[
                            QOI_DIFF_16 | (vr + 15) as u8,
                            (((vg + 7) << 4) | (vb + 7)) as u8,
                        ])?;
                    } else {
                        // Use 5-bit difference encoding
                        image_data_len += writer.write(&[
                            QOI_DIFF_24 | ((vr + 15) >> 1) as u8,
                            (((vr + 15) << 7) | ((vg + 15) << 2) | ((vb + 15) >> 3)) as u8,
                            (((vb + 15) << 5) | (va + 15)) as u8,
                        ])?;
                    }
                } else {
                    // Encode an entire pixel (but only the differing components)
                    let gate = |v: i32, x: u8| if v != 0 { x } else { 0 };

                    image_data_len += writer.write(&[QOI_COLOR
                        | gate(vr, 8)
                        | gate(vg, 4)
                        | gate(vb, 2)
                        | gate(va, 1)])?;

                    if vr != 0 {
                        image_data_len += writer.write(&[px[0]])?;
                    }
                    if vg != 0 {
                        image_data_len += writer.write(&[px[1]])?;
                    }
                    if vb != 0 {
                        image_data_len += writer.write(&[px[2]])?;
                    }
                    if va != 0 {
                        image_data_len += writer.write(&[px[3]])?;
                    }
                }
            }
        }

        px_prev = px;
    }

    // Padding
    image_data_len += writer.write(&[0; QOI_PADDING])?;

    // Seek and write the length to the header
    encode_size(writer, image_data_len as u32, size_field_offset)
}

/// Returns (width, height, total_pixels) and verifies that the image dimensions and channel count match the data
#[track_caller]
pub fn verify_and_calculate_dims(
    data: &[u8],
    width: usize,
    channels: ChannelCount,
) -> (u16, u16, usize) {
    // Check that the width and data length match up
    assert!(
        data.len() % (channels as usize) == 0,
        "Pixel count must be a multiple of channel count ({}).",
        channels as usize
    );
    assert!(
        data.len() % width == 0,
        "Pixel count must be a multiple of width"
    );
    let height = data.len() / (width as usize * channels as usize);

    let height: u16 = height.try_into().expect("Image height > 2^16");
    let width: u16 = width.try_into().expect("Image width > 2^16");
    let total_pixels = data.len() / 3;

    (width, height, total_pixels)
}

/// Returns the offset at which the file size will be written
fn encode_header<W: Write + Seek>(mut writer: W, width: u16, height: u16) -> Result<u64> {
    writer.write(MAGIC)?;
    writer.write(&width.to_le_bytes())?;
    writer.write(&height.to_le_bytes())?;
    let offset = writer.seek(SeekFrom::Current(0))?;
    writer.write(&0u32.to_le_bytes())?;
    Ok(offset)
}

fn encode_size<W: Write + Seek>(mut writer: W, size: u32, offset: u64) -> Result<()> {
    writer.seek(SeekFrom::Start(offset))?;
    writer.write(&size.to_le_bytes())?;
    Ok(())
}

/// Returns (width, height, compressed data size) for the given reader
fn decode_header<R: Read>(mut reader: R) -> Result<(u16, u16, u32)> {
    let mut short_buf = [0u8; 2];
    let mut long_buf = [0u8; 4];

    // Check magic
    reader.read_exact(&mut long_buf)?;
    assert_eq!(&long_buf, MAGIC, "Missing magic number");

    // Read width, height
    reader.read_exact(&mut short_buf)?;
    let width = u16::from_le_bytes(short_buf);

    reader.read_exact(&mut short_buf)?;
    let height = u16::from_le_bytes(short_buf);

    assert_ne!(width, 0);
    assert_ne!(height, 0);

    // Read compressed size
    reader.read_exact(&mut long_buf)?;

    let compressed_data_len = u32::from_le_bytes(long_buf);

    Ok((width, height, compressed_data_len))
}

/// Returns (image data, width, height)
pub fn decode<R: Read>(mut reader: R, channels: ChannelCount) -> Result<(Vec<u8>, u16, u16)> {
    let (width, height, _) = decode_header(&mut reader)?;

    let mut run: u32 = 0; // Run length encoding run length
    let mut px = DEFAULT_PREV_PIXEL; // Previous pixel
    let mut index = [[0; 4]; COLOR_LUT_SIZE];

    let total_pixels = width as usize * height as usize;
    let uncompressed_len = total_pixels * channels as usize; // Uncompressed image data length

    let mut out_buf = Vec::with_capacity(uncompressed_len);

    let mut read_byte = || -> Result<u8> {
        let mut buf = [0u8];
        reader.read_exact(&mut buf)?;
        Ok(buf[0])
    };

    while out_buf.len() < uncompressed_len {
        if run > 0 {
            run -= 1;
        } else {
            let b1 = read_byte()?;

            if (b1 & QOI_MASK_2) == QOI_INDEX {
                px = index[(b1 ^ QOI_INDEX) as usize];
            } else if (b1 & QOI_MASK_3) == QOI_RUN_8 {
                run = (b1 & 0x1f) as u32;
            } else if (b1 & QOI_MASK_3) == QOI_RUN_16 {
                let b2 = read_byte()?;
                run = ((((b1 & 0x1f) as u32) << 8) | (b2 as u32)) + 32;
            } else if (b1 & QOI_MASK_2) == QOI_DIFF_8 {
                px[0] += ((b1 >> 4) & 0x03) - 1;
                px[1] += ((b1 >> 2) & 0x03) - 1;
                px[2] += (b1 & 0x03) - 1;
            } else if (b1 & QOI_MASK_3) == QOI_DIFF_16 {
                let b2 = read_byte()?;
                px[0] += (b1 & 0x1f) - 15;
                px[1] += (b2 >> 4) - 7;
                px[2] += (b2 & 0x0f) - 7;
            } else if (b1 & QOI_MASK_4) == QOI_DIFF_24 {
                let b2 = read_byte()?;
                let b3 = read_byte()?;
                px[0] += (((b1 & 0x0f) << 1) | (b2 >> 7)) - 15;
                px[1] += ((b2 & 0x7c) >> 2) - 15;
                px[2] += (((b2 & 0x03) << 3) | ((b3 & 0xe0) >> 5)) - 15;
                px[3] += (b3 & 0x1f) - 15;
            } else if (b1 & QOI_MASK_4) == QOI_COLOR {
                if b1 & 8 != 0 {
                    px[0] = read_byte()?;
                }
                if b1 & 4 != 0 {
                    px[1] = read_byte()?;
                }
                if b1 & 2 != 0 {
                    px[2] = read_byte()?;
                }
                if b1 & 1 != 0 {
                    px[3] = read_byte()?;
                }
            }

            index[(color_hash(px) % 64) as usize] = px;
        }

        match channels {
            ChannelCount::Rgba => out_buf.extend_from_slice(&px),
            ChannelCount::Rgb => out_buf.extend_from_slice(&px[..3]),
        }
    }

    Ok((out_buf, width, height))
}