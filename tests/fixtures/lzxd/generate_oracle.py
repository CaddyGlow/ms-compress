"""Construct MS-PATCH vectors independently; verify using a libmspack lzxd oracle.

Pass path to a compiled oracle with arguments: input reference output order size.
No external implementation source is copied into this repository.
"""
from pathlib import Path
import hashlib
import json
import subprocess
import sys
import tempfile

ROOT = Path(__file__).parent

class Bits:
    def __init__(self): self.bits = []
    def put(self, value, count): self.bits += [(value >> s) & 1 for s in reversed(range(count))]
    def lens(self, lengths):
        for symbol in range(20): self.put(1 if symbol == 0 else 2 if symbol in (15, 16) else 0, 4)
        for length in lengths: self.put(0 if length == 0 else 3 if length == 1 else 2, 1 if length == 0 else 2)
    def frame(self):
        self.bits += [0] * (-len(self.bits) % 16)
        data = b''.join(sum(bit << (15-i) for i, bit in enumerate(self.bits[n:n+16])).to_bytes(2, 'little') for n in range(0,len(self.bits),16))
        return len(data).to_bytes(2,'little') + data

def patch(blocktype, size, symbol, extra):
    bits=Bits();bits.put(0,1);bits.put(blocktype,3);bits.put(size,24)
    if blocktype==2:
        for _ in range(8): bits.put(3,3)
    main=[0]*528;main[0]=main[symbol]=1
    bits.lens(main[:256]);bits.lens(main[256:]);lens=[0]*249
    if symbol % 8 == 7: lens[0]=lens[248]=1
    bits.lens(lens);bits.put(1,1)
    if symbol % 8 == 7: bits.put(1,1)
    for value, count in extra: bits.put(value,count)
    return bits.frame()

vectors = [
    ('official-abc', bytes.fromhex('14000030300001000000010000000100000061626300'), b'', b'abc'),
    ('reference-verbatim', patch(1,3,305,[(2,2)]), b'ABCDEFGH', b'ABC'),
    ('reference-aligned', patch(2,3,321,[(2,3)]), b'abcdefghijklmnop', b'abc'),
    ('extended-257', patch(1,257,263,[(0,1),(0,8)]), b'Z', b'Z'*257),
    ('extended-513', patch(1,513,263,[(2,2),(0,10)]), b'Z', b'Z'*513),
    ('extended-1537', patch(1,1537,263,[(6,3),(0,12)]), b'Z', b'Z'*1537),
    ('extended-32768', patch(1,32768,263,[(7,3),(32511,15)]), b'Z', b'Z'*32768),
]
records=[]
for name, delta, reference, expected in vectors:
    with tempfile.TemporaryDirectory() as directory:
        directory=Path(directory)
        (directory/'patch').write_bytes(delta);(directory/'reference').write_bytes(reference)
        subprocess.run([sys.argv[1],str(directory/'patch'),str(directory/'reference'),str(directory/'output'),'17',str(len(expected))],check=True)
        actual=(directory/'output').read_bytes()
        assert actual==expected,name
    (ROOT/f'{name}.lzxd').write_bytes(delta)
    records.append({'name':name,'window_order':17,'reference_hex':reference.hex(),'output_length':len(actual),'delta_sha256':hashlib.sha256(delta).hexdigest(),'output_sha256':hashlib.sha256(actual).hexdigest()})
(ROOT/'oracle.json').write_text(json.dumps({'spec_revision':'MS-PATCH 14.1 (2025-08-19)','oracle':'libmspack upstream lzxd.c compiled independently; repository decoder never used to generate expected output','vectors':records},indent=2)+'\n')
