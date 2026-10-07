use ms_compress::context::FixedStrategyCompressor;
use std::{env, error::Error, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let directory = PathBuf::from(
        env::args_os()
            .nth(1)
            .ok_or("usage: seed-codecs CORPUS_DIRECTORY")?,
    );
    fs::create_dir_all(&directory)?;
    for codec in 1..=3 {
        let mut compressor = FixedStrategyCompressor::new(codec, 32768)?;
        for (name, input) in [
            ("repeat", vec![b'A'; 4096]),
            ("varied", (0..4096).map(|i| (i % 251) as u8).collect()),
        ] {
            let encoded = compressor
                .compress(&input, input.len() * 2 + 4096)?
                .ok_or("seed did not compress")?;
            let mut seed = vec![(codec - 1) as u8];
            seed.extend_from_slice(&(input.len() as u16).to_le_bytes());
            seed.extend_from_slice(&encoded);
            fs::write(directory.join(format!("codec-{codec}-{name}")), seed)?;
        }
    }
    for (selector, name) in [(5, "lznt1"), (6, "xpress-plain")] {
        for size in [1, 25, 280, 4096, 4097, 32768] {
            let input = vec![b'A'; size];
            let encoded = if selector == 5 {
                ms_compress::lznt1::compress(&input)?
            } else {
                ms_compress::xpress_plain::compress(&input)?
            };
            let mut seed = vec![selector];
            seed.extend_from_slice(&(size as u16).to_le_bytes());
            seed.extend_from_slice(&encoded);
            fs::write(directory.join(format!("{name}-{size}")), seed)?;
        }
    }
    for selector in [3, 4] {
        let input = vec![b'A'; 4096];
        let encoded = if selector == 3 {
            ms_compress::lzx_encode::CabinetLzxEncoder::new(15)?
                .compress_frame(&input)?
                .to_vec()
        } else {
            ms_compress::quantum::QuantumEncoder::new(15, 4)?.compress_frame(&input)?
        };
        let mut seed = vec![selector];
        seed.extend_from_slice(&(input.len() as u16).to_le_bytes());
        seed.extend_from_slice(&encoded);
        fs::write(directory.join(format!("cab-{selector}")), seed)?;
    }
    let mut seed = vec![7, 3, 0];
    seed.extend_from_slice(&[0; 256]);
    seed.extend_from_slice(include_bytes!(
        "../../../tests/fixtures/lzxd/official-abc.lzxd"
    ));
    fs::write(directory.join("lzxd-official"), seed)?;
    for (selector, window_bits, name) in [(8, -15, "deflate"), (9, 15, "zlib"), (10, 31, "gzip")] {
        for size in [0, 1, 256, 4096, 32768] {
            let input = vec![b'A'; size];
            let mut buffer = vec![0; size * 2 + 128];
            let config = ms_compress::zlib::DeflateConfig {
                window_bits,
                ..ms_compress::zlib::DeflateConfig::default()
            };
            let (encoded, result) = ms_compress::zlib::compress_slice(&mut buffer, &input, config);
            if result != ms_compress::zlib::ReturnCode::Ok {
                return Err("zlib seed did not compress".into());
            }
            let mut seed = vec![selector];
            seed.extend_from_slice(&(size as u16).to_le_bytes());
            seed.extend_from_slice(encoded);
            fs::write(directory.join(format!("{name}-{size}")), seed)?;
        }
    }
    for size in [0, 1, 256, 4096, 32768] {
        let input = vec![b'A'; size];
        let encoded = ms_compress::lzma::compress(&input, 1)?;
        let mut seed = vec![11];
        seed.extend_from_slice(&(size as u16).to_le_bytes());
        seed.extend_from_slice(&encoded);
        fs::write(directory.join(format!("lzma-{size}")), seed)?;
        let (encoded, dictionary) = ms_compress::lzma::compress_lzma2(&input, 1)?;
        let mut seed = vec![12];
        seed.extend_from_slice(&(size as u16).to_le_bytes());
        seed.extend_from_slice(&dictionary.to_le_bytes());
        seed.extend_from_slice(&encoded);
        fs::write(directory.join(format!("lzma2-{size}")), seed)?;
    }
    let mut independent = Vec::new();
    for byte in b"ABCD" {
        let (block, dictionary) = ms_compress::lzma::compress_lzma2(&vec![*byte; 4096], 1)?;
        assert_eq!(dictionary, 4096);
        independent.extend_from_slice(&block[..block.len() - 1]);
    }
    independent.push(0);
    let mut seed = vec![12];
    seed.extend_from_slice(&16384u16.to_le_bytes());
    seed.extend_from_slice(&4096u32.to_le_bytes());
    seed.extend_from_slice(&independent);
    fs::write(directory.join("lzma2-independent-blocks"), seed)?;
    // Independent expected bytes from upstream's hello_world_quick test.
    let mut seed = vec![9, 13, 0];
    seed.extend_from_slice(&[
        0x78, 0x01, 0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0x08, 0xcf, 0x2f, 0xca, 0x49, 0x51, 0xe4,
        0x02, 0x00, 0x20, 0x91, 0x04, 0x48,
    ]);
    fs::write(directory.join("zlib-upstream-hello-world"), seed)?;
    for selector in 0..13 {
        for size in [0u16, 1, 32768] {
            let mut seed = vec![selector];
            seed.extend_from_slice(&size.to_le_bytes());
            fs::write(directory.join(format!("truncated-{selector}-{size}")), seed)?;
        }
    }
    let roundtrip = directory
        .parent()
        .ok_or("corpus directory has no parent")?
        .join("roundtrip");
    fs::create_dir_all(&roundtrip)?;
    for selector in 0..12 {
        for size in [0, 1, 25, 280, 4096, 32768] {
            let mut seed = vec![selector, 0, 0];
            seed.extend(std::iter::repeat_n(b'A', size));
            fs::write(roundtrip.join(format!("{selector}-{size}")), seed)?;
        }
    }
    for selector in 7..10 {
        for level in 0..10 {
            for strategy in 0..5 {
                let mut seed = vec![selector, level, strategy];
                seed.extend((0..1024).map(|i| (i % 251) as u8));
                fs::write(
                    roundtrip.join(format!("{selector}-level-{level}-strategy-{strategy}")),
                    seed,
                )?;
            }
        }
    }
    for selector in [10, 11] {
        for preset in 0..10 {
            let mut seed = vec![selector, preset, 0];
            seed.extend((0..1024).map(|i| (i % 251) as u8));
            fs::write(roundtrip.join(format!("{selector}-preset-{preset}")), seed)?;
        }
    }
    Ok(())
}
