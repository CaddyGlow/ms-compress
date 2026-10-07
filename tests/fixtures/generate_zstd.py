"""Produce deterministic checksum-bearing multiblock fixtures with system libzstd."""
from pathlib import Path
import ctypes,json,hashlib,os
lib=ctypes.CDLL(os.environ.get('ZSTD_LIBRARY','libzstd.so.1'))
lib.ZSTD_versionString.restype=ctypes.c_char_p
lib.ZSTD_createCCtx.restype=ctypes.c_void_p
lib.ZSTD_freeCCtx.argtypes=[ctypes.c_void_p]
lib.ZSTD_CCtx_setParameter.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_int];lib.ZSTD_CCtx_setParameter.restype=ctypes.c_size_t
lib.ZSTD_compressBound.argtypes=[ctypes.c_size_t];lib.ZSTD_compressBound.restype=ctypes.c_size_t
lib.ZSTD_compress2.argtypes=[ctypes.c_void_p,ctypes.c_void_p,ctypes.c_size_t,ctypes.c_void_p,ctypes.c_size_t];lib.ZSTD_compress2.restype=ctypes.c_size_t
lib.ZSTD_isError.argtypes=[ctypes.c_size_t];lib.ZSTD_isError.restype=ctypes.c_uint
lib.ZSTD_getErrorName.argtypes=[ctypes.c_size_t];lib.ZSTD_getErrorName.restype=ctypes.c_char_p
lib.ZSTD_CCtx_loadDictionary.argtypes=[ctypes.c_void_p,ctypes.c_void_p,ctypes.c_size_t];lib.ZSTD_CCtx_loadDictionary.restype=ctypes.c_size_t

def checked(code):
 if lib.ZSTD_isError(code):raise RuntimeError(lib.ZSTD_getErrorName(code))
 return code

def encode(payload,dictionary=b''):
 ctx=lib.ZSTD_createCCtx()
 for key,value in [(100,5),(101,15),(201,1)]:checked(lib.ZSTD_CCtx_setParameter(ctx,key,value))
 if dictionary:checked(lib.ZSTD_CCtx_loadDictionary(ctx,dictionary,len(dictionary)))
 out=ctypes.create_string_buffer(lib.ZSTD_compressBound(len(payload)))
 length=checked(lib.ZSTD_compress2(ctx,out,len(out),payload,len(payload)))
 lib.ZSTD_freeCCtx(ctx)
 return out.raw[:length]

root=Path(__file__).parent
# Repeated source with long-distance transitions spans many 32KiB window blocks.
payload=bytes((i*37+i//251)%256 for i in range(8192))*83+b'end-marker'*133
frame=encode(payload);(root/'zstd-multiblock.bin').write_bytes(frame)
dictionary=bytes((i*19+i//97)%256 for i in range(8192))
dict_payload=dictionary[-6000:]*7+b'dictionary-end'
dict_frame=encode(dict_payload,dictionary);(root/'zstd-dictionary.bin').write_bytes(dict_frame)
(root/'zstd-oracle.json').write_text(json.dumps({'oracle':'libzstd '+lib.ZSTD_versionString().decode(),'multiblock':{'input_bytes':len(payload),'compressed_bytes':len(frame),'frame_sha256':hashlib.sha256(frame).hexdigest(),'output_sha256':hashlib.sha256(payload).hexdigest()},'dictionary':{'input_bytes':len(dict_payload),'compressed_bytes':len(dict_frame),'frame_sha256':hashlib.sha256(dict_frame).hexdigest(),'output_sha256':hashlib.sha256(dict_payload).hexdigest()}},indent=2)+'\n')
