# RTL compression analysis, 2026-10-10

Inspected the native Windows 11 x64 ntdll used by the codec benchmark, with
Microsoft symbol names and IDA/Hex-Rays. The DLL copied from the guest exactly
matches the cached analysis binary:

- File version: 10.0.26100.7920.
- SHA-256: `e758fa414086189b0df0b569750d1e4e164ee1a43598d758536acec159a5f3e5`.
- Image base: `0x180000000`.
- Local analysis database:
  `/data/cache/ru-windbg-extension-plan/gle-real-symbols-1010/ntdll.dll.i64`.

The observations below describe this binary's standard compression engine.
They do not establish behavior for other Windows releases or the maximum engine.
The Rust changes are original safe code; no native instructions or decompiler
output are incorporated into the library.

## Observations

| Routine | Address | Observed behavior |
| --- | --- | --- |
| `RtlCompressBuffer` | `0x1800fac00` | Dispatches by format and passes engine bits to the selected routine. |
| `RtlCompressBufferXpressLzStandard` | `0x1800c9260` | Direct three-byte candidate lookup with bounded auxiliary lookups for longer prefixes. Rejects mismatching initial bytes before extending a match. Advances directly past matches, without inserting every covered byte. Long extension uses unrolled 32-bit loads over 32-byte groups and a bounded tail. |
| `RtlCompressBufferXpressHuffStandard` | `0x1800c99b0` | Uses the same style of bounded direct candidate lookups, token/frequency collection followed by a separate Huffman pass, and nominal 64 KiB block boundaries. It can extend a match beyond the boundary. Covered match positions are not all inserted. |
| `LZNT1FindMatchStandard` | `0x180108200` | A 4,096-bucket table retains two candidates per bucket. Checks the initial three bytes before extending a match. |
| `LZNT1CompressChunk` | `0x1800fbc7c` | Calls the match finder at token starts and advances past matches. No loop hashes every byte inside a match. |
| `XpressBuildHuffmanEncodings` | `0x1800ca1b0` | Sorts active frequencies with two counting/radix passes, merges leaf/internal queues, and retries with reduced weights if code lengths exceed 15. |

The inspected standard XPRESS extension loops use scalar unrolled word loads;
their speed is not evidence of a hidden SIMD implementation. This observation
applies to these routines, not every compression routine in ntdll.

## First implementation changes

Our encoders updated their dictionaries for every covered match byte, even
when the token reached the end of the input (or the end of an LZNT1 chunk).
Those final updates cannot influence a later match. Skip them in all three
encoders. In the reusable Huffman encoder, a new position stamp already
invalidates previous-call chains, so skipping these writes remains safe when
reusing storage.

Our shared matcher also called the long-prefix/SIMD dispatcher for candidates
that did not share three initial bytes. Reject those candidates first. Apply
the same rejection in the Huffman hash-chain parser. Matches shorter than
three were never emitted, so this does not change token selection.

These changes preserve parsing choices and encoded bytes. They retain full
dictionary insertion for non-final matches and retain Huffman's 64-candidate
search bound. The broader RTL policy of skipping all covered positions would
change future candidate selection and must be evaluated as a separate
speed/ratio tradeoff.

## Remaining opportunities

1. Add reusable compression APIs and benchmark retained workspaces separately
   from one-shot calls. Huffman's current one-shot API initializes large
   chain/token/output buffers and then copies the result; these costs are most
   visible for highly compressible inputs. Reduce initialized scratch storage
   while preserving allocation failure handling and the no-unsafe constraint.
2. Evaluate a faster parser with bounded direct lookups and sparse insertion.
   Preserve the existing parser when stronger match search is desired. Measure
   compressed size on independent corpora, not just throughput on repeated data.
3. Combine shared-matcher lookup and insertion to avoid hashing the same literal
   position twice. This can preserve candidate choices with careful ordering.
4. Evaluate radix sorting of Huffman leaves while preserving the documented
   deterministic tie rules and bounded-code fallback behavior.
5. Profile the remaining costs before adding wider SIMD. The library already
   uses feature-detected SIMD for long comparisons; rejected candidates and
   redundant dictionary writes are work that SIMD would not eliminate.

Validation includes all repository-mandated Linux/no_std/embedded/fuzz checks
and native same-buffer cross-decoding through the Windows benchmark. The
follow-up report records measured gains and remaining gaps.
