#!/usr/bin/env python3
"""Mapped-text byte/span comparison against Python Unicode and native per-code lookup.

This is standalone ToUnicode decoding, not document text extraction.
"""
import argparse
import hashlib
import importlib.util
import json
import random
import subprocess
import tempfile
from pathlib import Path

spec = importlib.util.spec_from_file_location('cmap_reference', Path(__file__).with_name('differential-cmap.py'))
cmap = importlib.util.module_from_spec(spec); spec.loader.exec_module(cmap)


def expected_text(data, spaces, mapping):
    output, records, position = bytearray(), [], 0
    while position < len(data):
        decoded = cmap.decode_expected(data[position:position+4], spaces, mapping)
        if 'error' in decoded: raise ValueError('independent segmentation: ' + decoded['error'])
        end = position + decoded['consumed']
        code = data[position:end]
        scalars = mapping.get(code)
        if scalars is None: raise ValueError('independent missing mapping')
        before = len(output)
        output += ''.join(chr(value) for value in scalars).encode('utf-8')
        records.append({'code': code.hex(), 'source': [position, end], 'utf8': [before, len(output)]})
        position = end
    return {'text_utf8_hex': output.hex(), 'codes': records}


def compare(rust, native, directory, program, data, spaces, mapping, native_parity=True):
    expected = expected_text(data, spaces, mapping)
    raw_file = directory/'text.bin'; raw_file.write_bytes(data)
    actual = subprocess.run([str(rust), f'--cmap-text={raw_file}', '-'], input=program, capture_output=True, timeout=20)
    if actual.returncode: raise ValueError('Rust text refusal: ' + actual.stderr.decode(errors='replace')[:2000])
    result = json.loads(actual.stdout)
    if result != expected: raise ValueError('decoded UTF8 bytes or source/output spans differ from independent expectation')
    query_file = directory/'queries.txt'
    query_file.write_text(''.join('decode '+data[row['source'][0]:row['source'][0]+4].hex()+'\n' for row in expected['codes']))
    status, reference, error, _ = cmap.invoke(native, query_file, program)
    if status: raise ValueError('native map refusal: ' + error)
    if len(reference['queries']) != len(expected['codes']): raise ValueError('native query count mismatch')
    native_text, differences = bytearray(), []
    for record, query in zip(expected['codes'], reference['queries']):
        code = bytes.fromhex(record['code'])
        if query['consumed'] != len(code) or query['width'] != len(code) or query['value'] != int.from_bytes(code,'big') or not query['matched_codespace']:
            raise ValueError('native code boundary mismatch')
        if query['scalars'] != mapping[code]:
            if native_parity: raise ValueError('native Unicode mismatch')
            differences.append({'code':code.hex(),'expected_scalars':mapping[code],'native_scalars':query['scalars']})
        native_text += ''.join(chr(value) for value in query['scalars']).encode('utf-8')
    if native_parity and native_text.hex() != expected['text_utf8_hex']: raise ValueError('native aggregate Unicode mismatch')
    return {'input_bytes':len(data),'codes':len(expected['codes']),'utf8_bytes':len(bytes.fromhex(expected['text_utf8_hex'])),'utf8_sha256':hashlib.sha256(bytes.fromhex(expected['text_utf8_hex'])).hexdigest(),'native_widthless_differences':differences}


def mixed():
    spaces=[(bytes.fromhex(a),bytes.fromhex(b)) for a,b in [('80','82'),('0040','0042'),('010001','010003'),('02000001','02000003')]]
    mapping={bytes.fromhex('80'):[102,102,105],bytes.fromhex('0040'):[65,0x301],bytes.fromhex('010001'):[0x1f600,0],bytes.fromhex('02000001'):[0xfeff,0x10ffff]}
    records=[cmap.hex_string(code)+b' '+cmap.hex_string(cmap.utf16(scalars)) for code,scalars in mapping.items()]
    return cmap.make_program(spaces,[('bfchar',records)]),spaces,mapping


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust',type=Path,default=Path('target/debug/pdf-inspect'))
    parser.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-cmap-oracle'))
    parser.add_argument('--lexer',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-lex-oracle'))
    parser.add_argument('--seed',type=int,default=20261003)
    parser.add_argument('--cases',type=int,default=100)
    parser.add_argument('--inventory',type=Path,default=Path('reference/cmap-inventory.json'))
    parser.add_argument('--corpus-dir',type=Path,default=Path('/private/tmp/tandem-cmap-inventory'))
    parser.add_argument('--report',type=Path,default=Path('reference/text-decode-differential-r7b.json'))
    args=parser.parse_args()
    if not 1<=args.cases<=10000:parser.error('cases must be1..10000')
    rust,native,lexer=args.rust.resolve(),args.native.resolve(),args.lexer.resolve()
    hashes={str(path):hashlib.sha256(path.read_bytes()).hexdigest() for path in (rust,native,lexer)}
    rng=random.Random(args.seed); cases=[]
    for label,program,spaces,mapping in cmap.generate(args.seed,args.cases):
        keys=list(mapping); sequence=keys+[rng.choice(keys) for _ in range(31)];rng.shuffle(sequence)
        cases.append((label,program,b''.join(sequence),spaces,mapping))
    program,spaces,mapping=mixed()
    cases.extend([('mixed-widths',program,b''.join(list(mapping)*37),spaces,mapping),('empty-input',program,b'',spaces,mapping)])
    for row in json.loads(args.inventory.read_text())['cmaps']:
        if row['invalidUtf16Destinations']:continue
        program=(args.corpus_dir/row['savedFile']).read_bytes()
        if hashlib.sha256(program).hexdigest()!=row['sha256']:raise ValueError('fixture CMap changed')
        spaces,mapping=cmap.fixture_mapping(lexer,program)
        cases.append((row['savedFile'],program,b''.join(sorted(mapping)+sorted(mapping,reverse=True)),spaces,mapping))
    results,refusals=[],[]
    with tempfile.TemporaryDirectory(prefix='tandem-text-differential-') as temporary:
        directory=Path(temporary)
        first=cases[0];compare(rust,native,directory,*first[1:])
        for label,program,data,spaces,mapping in cases:
            row={'case':label,'map_sha256':hashlib.sha256(program).hexdigest(),'input_sha256':hashlib.sha256(data).hexdigest()}
            try:row.update(status='matched',**compare(rust,native,directory,program,data,spaces,mapping))
            except (ValueError,subprocess.TimeoutExpired) as error:row.update(status='failure',error=str(error))
            results.append(row)
        spaces=[(b'\x80',b'\xff'),(b'\0\x80',b'\0\xff')];mapping={b'\x80':[65],b'\0\x80':[66]}
        program=cmap.make_program(spaces,[('bfchar',[b'<80><0041>',b'<0080><0042>'])])
        width_collision=compare(rust,native,directory,program,b'\0\x80\x80',spaces,mapping,False)
        program,_,_=mixed()
        for label,data in [('missing',b'\x81'),('invalid',b'\x03'),('incomplete1',b'\x01'),('incomplete2',b'\x01\0'),('late-missing',b'\x80\0\x40\x81'),('late-incomplete',b'\x80\x02\0')]:
            raw_file=directory/'text.bin';raw_file.write_bytes(data)
            result=subprocess.run([str(rust),f'--cmap-text={raw_file}','-'],input=program,capture_output=True,timeout=20)
            refusals.append({'case':label,'passed':result.returncode!=0 and not result.stdout,'diagnostic':result.stderr.decode(errors='replace')[:1000]})
    if any(hashlib.sha256(Path(path).read_bytes()).hexdigest()!=digest for path,digest in hashes.items()):raise RuntimeError('comparison binary changed during run')
    failures=sum(row['status']!='matched' for row in results)+sum(not row['passed'] for row in refusals)
    report={'reference':'Independent Python Unicode/spans plus pinned MuPDF per-code boundaries and scalars','seed':args.seed,'binary_sha256':hashes,'cases':len(results),'matched':sum(row['status']=='matched' for row in results),'codes':sum(row.get('codes',0) for row in results),'utf8_bytes':sum(row.get('utf8_bytes',0) for row in results),'failures':failures,'strict_refusals':refusals,'native_widthless_limitation':width_collision,'results':results,'limitations':['Standalone explicitly mapped byte strings, not PDF document text extraction or font fallback.','Source/output spans cover complete character-code mappings, not grapheme clusters or individual scalar origins.','Native widthless lookup collisions are recorded separately; Rust output follows typed independent mappings.']}
    args.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({key:value for key,value in report.items() if key not in ('results','strict_refusals','native_widthless_limitation')},indent=2))
    for row in [row for row in results if row['status']!='matched'][:10]:print(row)
    raise SystemExit(bool(failures))


if __name__=='__main__':main()
