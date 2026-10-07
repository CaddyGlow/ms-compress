"""Audit that offline LZMS cases exercise every match decision and rebuild.

Compiles an instrumented temporary copy of the Rust source. No production
files are modified and no C oracle/runtime is involved. Requires rustc.
"""
from pathlib import Path
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
source = (HERE.parents[1] / "src/lzms.rs").read_text()
labels = ["literal", "lz-explicit", "lz-rep0", "lz-rep1", "lz-rep2",
          "delta-explicit", "delta-rep0", "delta-rep1", "delta-rep2",
          "huffman-rebuild", "x86-translation", "probability-zero", "probability-64"]


def replace_once(old, new):
    global source
    assert source.count(old) == 1, f"Instrumentation anchor changed: {old}"
    source = source.replace(old, new)


replace_once("//! Safe raw-block LZMS decoding with adaptive Huffman and binary range coding.",
             "// Temporary instrumented native decoder copy.")
replace_once("use std::cmp::Reverse;", """use std::cmp::Reverse;
static COUNTS: [std::sync::atomic::AtomicUsize; 13] = [const { std::sync::atomic::AtomicUsize::new(0) }; 13];
fn tick(i: usize) { COUNTS[i].fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
pub fn counts() -> Vec<usize> { COUNTS.iter().map(|v|v.load(std::sync::atomic::Ordering::Relaxed)).collect() }""")
replace_once("output[produced] = literals.symbol(&mut bits)? as u8;",
             "tick(0); output[produced] = literals.symbol(&mut bits)? as u8;")
replace_once("let value = offsets.value(&mut bits, OFFSET_BASES, EXTRA_OFFSET_BITS)?;",
             "tick(1); let value = offsets.value(&mut bits, OFFSET_BASES, EXTRA_OFFSET_BITS)?;")
replace_once("let index = rep_index(&mut range, &mut lz_reps);",
             "let index = rep_index(&mut range, &mut lz_reps); tick(2+index);")
replace_once("let power = powers.symbol(&mut bits)? as u32;",
             "tick(5); let power = powers.symbol(&mut bits)? as u32;")
replace_once("let index = rep_index(&mut range, &mut delta_reps);",
             "let index = rep_index(&mut range, &mut delta_reps); tick(6+index);")
replace_once("if self.remaining == 0 {", "if self.remaining == 0 { tick(9);")
replace_once("if index - last_x86 <= max_offset {", "if index - last_x86 <= max_offset { tick(10);")
replace_once("let prob = entry.zeros.clamp(1, 63);",
             "if entry.zeros == 0 { tick(11); } if entry.zeros == 64 { tick(12); } let prob = entry.zeros.clamp(1, 63);")
runner = r'''mod lzms;
fn take<'a>(input: &mut &'a [u8], count: usize) -> &'a [u8] {
    let (value, rest) = input.split_at(count); *input = rest; value
}
fn u32(input: &mut &[u8]) -> usize {
    u32::from_le_bytes(take(input, 4).try_into().unwrap()) as usize
}
fn main() {
    let data = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    assert_eq!(&data[..4], b"WLM1");
    let mut input = &data[4..]; let count = u32(&mut input);
    for _ in 0..count {
        let n = u16::from_le_bytes(take(&mut input, 2).try_into().unwrap()) as usize;
        take(&mut input, n);
        let size = u32(&mut input); let out = u32(&mut input);
        let success = take(&mut input, 1)[0]; let block = take(&mut input, size);
        let mut output = vec![0; out];
        let status = lzms::decompress_lzms(block, &mut output);
        assert_eq!(status.is_ok(), success == 1);
        if success == 1 { assert_eq!(output, take(&mut input, out)); }
    }
    assert!(input.is_empty());
    for count in lzms::counts() { println!("{count}"); }
}'''
with tempfile.TemporaryDirectory(prefix="wim-lzms-branch-audit-") as directory:
    path = Path(directory)
    (path / "lzms.rs").write_text(source)
    (path / "main.rs").write_text(runner)
    executable = path / "audit"
    subprocess.run(["rustc", "--edition", "2024", "-O", str(path / "main.rs"), "-o", str(executable)], check=True)
    result = subprocess.run([str(executable), str(HERE / "lzms-oracle.wlm")], check=True, text=True, capture_output=True)
counts = [int(value) for value in result.stdout.splitlines()]
assert len(counts) == len(labels)
for label, count in zip(labels, counts):
    assert count > 0, f"Uncovered native LZMS branch: {label}"
    print(f"{label}: {count}")
