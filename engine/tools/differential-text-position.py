#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""T0 exact-rational and bounded native float32 text-position comparison.

Declared before comparisons: Rust dyadic expectations are exact. Native uses
abs(error) <= 0.002 + 2e-5*max(1,abs(expected)) on bounded <=100-code programs
whose expected coordinates are <=10000. No glyph identity/outline comparison.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from text_position_model import Model, generate, number, json_numbers

NATIVE_ABSOLUTE_TOLERANCE = 0.002
NATIVE_RELATIVE_TOLERANCE = 2e-5


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def cmap(font):
    entries = [f'<{code:02x}><{text.encode("utf-16-be").hex()}>' for code,text in sorted(font['mapping'].items())]
    assert len(entries) <= 100
    return ('begincmap 1 begincodespacerange <00><ff> endcodespacerange ' + str(len(entries)) + ' beginbfchar ' + ' '.join(entries) + ' endbfchar endcmap').encode('ascii')


def font_config(fonts):
    rows = []
    for label,font in fonts.items():
        if 'builtin' in font:
            unicode = f' /ToUnicode <{cmap(font).hex()}>' if font.get('unicode_override') else ''
            rows.append(f'/{label} << /Builtin /{font["builtin"]} /Encoding '+font['encoding_pdf'].decode('ascii')+unicode+' >>')
            continue
        first,last = min(font['widths']),max(font['widths'])
        widths = ' '.join(number(font['widths'][code]) for code in range(first,last+1))
        missing = ' /MissingWidth ' + number(font['missing']) if 'missing' in font else ''
        rows.append(f'/{label} << /FirstChar {first} /LastChar {last} /Widths [{widths}] /ToUnicode <{cmap(font).hex()}>{missing} >>')
    return ('<< ' + ' '.join(rows) + ' >>').encode('ascii')


def native_pdf(spec, content):
    fonts = spec['fonts']
    objects = [b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>']
    resources = ' '.join(f'/{label} {4+i} 0 R' for i,label in enumerate(fonts))
    content_id = 4+len(fonts)
    objects.append(f'<< /Type /Page /Parent 2 0 R /MediaBox [-10000 -10000 10000 10000] /Resources << /Font << {resources} >> >> /Contents {content_id} 0 R >>'.encode())
    for i,font in enumerate(fonts.values()):
        first,last = min(font['widths']),max(font['widths'])
        widths = ' '.join(number(font['widths'][code]) for code in range(first,last+1))
        objects.append(f'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FirstChar {first} /LastChar {last} /Widths [{widths}] /ToUnicode {content_id+1+i} 0 R >>'.encode())
    for raw in [content,*(cmap(font) for font in fonts.values())]:
        objects.append(f'<< /Length {len(raw)} >>\nstream\n'.encode()+raw+b'\nendstream')
    output = bytearray(b'%PDF-1.7\n')
    offsets = []
    for i,obj in enumerate(objects,1):
        offsets.append(len(output))
        output.extend(f'{i} 0 obj\n'.encode()+obj+b'\nendobj\n')
    xref = len(output)
    output.extend(f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode())
    for offset in offsets:
        output.extend(f'{offset:010} 00000 n \n'.encode())
    output.extend(f'trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    return bytes(output)


def compare_native(actual, expected):
    if actual['repaired']:
        raise ValueError('native repaired generated input')
    groups = []
    for item in actual['items']:
        if item['primary']:
            groups.append({'item':item,'unicode':[item['unicode']]})
        else:
            if not groups or item.get('callback') != groups[-1]['item'].get('callback'):
                raise ValueError('orphan native Unicode continuation')
            groups[-1]['unicode'].append(item['unicode'])
    if len(groups) != len(expected['codes']):
        raise ValueError(f'native code count {len(groups)} != {len(expected["codes"])}')
    max_error = 0
    for group,want in zip(groups,expected['codes']):
        got = group['item']
        if got['mode'] != want['mode'] or got['cid'] != want['byte'] or group['unicode'] != list(map(ord,want['text'])):
            raise ValueError('native code/mode/Unicode differs: '+repr((group,want)))
        for a,b in zip(got['page_matrix'],want['rendering']):
            b = float(b)
            assert abs(b) <= 10000
            error = abs(a-b)
            if error > NATIVE_ABSOLUTE_TOLERANCE + NATIVE_RELATIVE_TOLERANCE * max(1,abs(b)):
                raise ValueError(f'native placement differs {a} vs {b}')
            max_error = max(max_error,error)
    return max_error


def compare_rust(actual, expected, spec):
    font_ids = {label:index for index,label in enumerate(sorted(spec['fonts']))}
    codes = []
    for code in expected['codes']:
        array = spec['operations'][code['operation']][0] == 'TJ'
        codes.append({
            'code':code['byte'], 'font_id':font_ids[code['font']], 'text_object':code['object'],
            'operation_index':code['operation'], 'operation_span':code['span'],
            'operand_index':0 if array else code['operand'], 'array_index':code['operand'] if array else None,
            'string_span':code['decoded_range'], 'source_span':code['source_range'], 'utf8_span':code['utf8_range'],
            'width':code['width'], 'metric_origin':code['origin'], 'character_spacing':code['character_spacing'],
            'word_spacing':code['word_spacing'], 'font_size':code['size'], 'horizontal_scale':code['horizontal_scale'],
            'rise':code['rise'], 'rendering_mode':code['mode'], 'before':code['before'], 'after':code['after'],
            'transform':code['rendering'], 'origin':code['page_origin'], 'glyph_displacement':code['glyph_dx'],
            'word_displacement':code['word_dx'],
        })
    adjustments = [{'number':a['value'],'displacement':a['dx'],'before':a['before'],'after':a['after'],'operation_index':a['operation'],'operation_span':a['span'],'array_index':a['element'],'text_object':a['object']} for a in expected['adjustments']]
    ends = [{'text_matrix':a['matrix'],'line_matrix':a['line_matrix'],'operation_index':a['operation'],'operation_span':a['span']} for a in expected['ends']]
    used = []
    for op,args in spec['operations']:
        if op == 'Tf' and args[0] not in used:
            used.append(args[0])
    fonts = [{'id':font_ids[label],'name_hex':label.encode().hex(),'unicode_kind':'to_unicode' if 'builtin' not in spec['fonts'][label] or spec['fonts'][label].get('unicode_override') else 'encoding','metric_kind':'builtin:'+spec['fonts'][label]['builtin'] if 'builtin' in spec['fonts'][label] else 'declared'} for label in used]
    wanted = json_numbers({'text_utf8_hex':expected['utf8_hex'],'source_bytes_hex':expected['source_hex'],'fonts':fonts,'codes':codes,'adjustments':adjustments,'text_objects':ends,'events':expected['events']})
    if actual != wanted:
        def mismatch(a,b,path=''):
            if isinstance(b,dict):
                if set(a)!=set(b):return path+' keys'
                for key in b:
                    result=mismatch(a[key],b[key],path+'.'+key)
                    if result:return result
            elif isinstance(b,list):
                if len(a)!=len(b):return path+' length'
                for index,(x,y) in enumerate(zip(a,b)):
                    result=mismatch(x,y,path+f'[{index}]')
                    if result:return result
            elif a!=b:return path+f': actual {a!r}, expected {b!r}'
            return None
        raise ValueError('exact Rust trace differs '+str(mismatch(actual,wanted)))


def builtin_cases():
    parity = json.loads(Path('reference/builtin-metrics-parity.json').read_text())
    encodings = json.loads(Path('reference/encoding-table-inventory.json').read_text())
    registry = {r['glyph']:chr(r['nativeStrict']) for table in encodings['tables'].values() for r in table if r['glyph'] is not None}
    for index,face in enumerate(parity['faces']):
        advances = {r['name']:r['pdfIntegerWidth'] for r in face['rows'] if r['found'] and r['afmWidth'] is not None}
        selected = {32:'A',65:'fi',66:'space'}
        for override in (False,True):
            mapping = {code:registry[glyph] for code,glyph in selected.items()}
            if override:
                mapping = {32:'Z',65:'two',66:' '}
            font = {'builtin':face['baseFont'],'unicode_override':override,'encoding_pdf':b'<< /BaseEncoding /WinAnsiEncoding /Differences [32 /A 65 /fi /space] >>','widths':{code:advances[glyph] for code,glyph in selected.items()},'mapping':mapping}
            ops = [('Tf',['F0',1000]),('Tc',[1]),('Tw',[7]),('BT',[]),('Tj',[b'A B']),('TJ',[[125,b'BA']]),('ET',[])]
            yield {'case':240+index*2+int(override),'native':False,'builtin':True,'fonts':{'F0':font},'operations':ops}


def missing_width_cases():
    for index,missing in enumerate([None,250]):
        font={'widths':{65:500},'mapping':{32:'Z',65:'fi',66:' '}}
        if missing is not None:
            font['missing']=missing
        operations=[('Tf',['F0',8]),('Tc',[1]),('Tw',[3]),('BT',[]),('Tj',[b'A B']),('ET',[])]
        yield {'case':244+index,'native':False,'fonts':{'F0':font},'operations':operations}


def strict_refusals(rust, config, valid_config):
    # A valid prefix precedes many errors to detect accidental partial publication.
    prefix = b'/F0 8 Tf BT (A) Tj ET '
    malformed = [
        b'BT BT', b'ET', b'BT', b'Q', b'q', b'BT q ET', b'BT Q ET',
        b'BT 1 0 0 1 0 0 cm ET', b'/Missing 8 Tf', b'BT (A) Tj ET',
        b'BT [] TJ ET', b'1 Tr', b'2 Tr', b'4 Tr', b'5 Tr', b'6 Tr', b'7 Tr',
        b'1 0 0 1 0 0 Tm', b'1 2 Td', b'T*', b'/F0 9007199254740993 Tf',
        prefix+b'0 0 m', prefix+b'0 g', prefix+b'BX', prefix+b'/Tag BMC',
        prefix+b'BT /F0 8 Tf [true] TJ ET', prefix+b'BT /F0 8 Tf [[(A)]] TJ ET',
        prefix+b'BT /F0 8 Tf (<bad>) Tj ET', # bytes outside authoritative map
        prefix+b'BT /F0 8 Tf <00> Tj ET', prefix+b'BT /F0 8 Tf <ff> Tj ET',
        prefix+b'BT /F0 8 Tf 1000001 Tc (A) Tj ET',
        prefix+b'BT /F0 1000000 Tf 1000000 Tz (A) Tj ET',
    ]
    bad_configs = [
        b'null', b'[]', b'<< /F0 1 0 R >>', b'<< /F0 << /Builtin /Arial /Encoding /WinAnsiEncoding >> >>',
        b'<< /F0 << /Builtin /Helvetica >> >>',
        b'<< /F0 << /Builtin /Helvetica /Encoding /WinAnsiEncoding /Widths null >> >>',
        b'<< /F0 << /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 65 /Widths [] >> >>',
        b'<< /F0 << /Encoding /WinAnsiEncoding /FirstChar 66 /LastChar 65 /Widths [1] >> >>',
        b'<< /F0 << /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 65 /Widths [1] /Other null >> >>',
        b'<< /F0 << /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 65 /Widths [9007199254740993] >> >>',
    ]
    cases = [(valid_config, raw) for raw in malformed] + [(raw,prefix) for raw in bad_configs]
    override = cmap({'mapping':{65:'A'}}).hex()
    for face in ['Helvetica','Times-Roman']:
        for glyph in ['.notdef','zerosuperior','UnknownBuiltinGlyph']:
            raw = f'<< /F0 << /Builtin /{face} /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /{glyph}] >> /ToUnicode <{override}> >> >>'.encode()
            cases.append((raw,prefix))
    for font_data,raw in cases:
        config.write_bytes(font_data)
        result = subprocess.run([str(rust),'--text='+str(config),'-'],input=raw,capture_output=True,timeout=10)
        if result.returncode <= 0 or result.stdout or not result.stderr or b'panicked' in result.stderr:
            raise ValueError('non-atomic or missing strict text refusal: '+repr((raw,font_data,result.stdout)))
    return len(cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust',type=Path,default=Path('target/debug/pdf-inspect'))
    parser.add_argument('--oracle',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-text-position-oracle'))
    parser.add_argument('--report',type=Path,default=Path('reference/text-position-differential-t0.json'))
    parser.add_argument('--native-only',action='store_true')
    args = parser.parse_args()
    paths = [args.oracle,Path(__file__),Path(__file__).with_name('text_position_model.py'),Path('reference/builtin-metrics-parity.json'),Path('reference/encoding-table-inventory.json')]
    if not args.native_only:
        paths.append(args.rust)
    hashes = {str(p):digest(p) for p in paths}
    rows = []
    with tempfile.TemporaryDirectory(prefix='tandem-text-position-') as temporary:
        config = Path(temporary)/'fonts.pdfsyntax'
        for spec in [*generate(),*builtin_cases(),*missing_width_cases()]:
            raw,expected = Model(spec['fonts']).run(spec['operations'])
            row = {'case':spec['case'],'source_codes':len(expected['codes']),'adjustments':len(expected['adjustments']),'text_objects':len(expected['ends']),'content_sha256':hashlib.sha256(raw).hexdigest()}
            if spec['native']:
                pdf = native_pdf(spec,raw)
                native = subprocess.run([str(args.oracle)],input=pdf,capture_output=True,timeout=20)
                if native.returncode or native.stderr:
                    raise ValueError(f'native case{spec["case"]} refused: '+native.stderr.decode(errors='replace'))
                row['native_max_absolute_error'] = compare_native(json.loads(native.stdout),expected)
                row['native_output_sha256'] = hashlib.sha256(native.stdout).hexdigest()
            if not args.native_only:
                config.write_bytes(font_config(spec['fonts']))
                result = subprocess.run([str(args.rust.resolve()),'--text='+str(config),'-'],input=raw,capture_output=True,timeout=20)
                if result.returncode or result.stderr:
                    raise ValueError(f'Rust case{spec["case"]} refused: '+result.stderr.decode(errors='replace'))
                compare_rust(json.loads(result.stdout),expected,spec)
                row['rust_output_sha256'] = hashlib.sha256(result.stdout).hexdigest()
            rows.append(row)
        refusals = 0 if args.native_only else strict_refusals(args.rust.resolve(),config,font_config(next(generate())['fonts']))
    if hashes != {str(p):digest(p) for p in paths}:
        raise ValueError('binary or harness changed during gate')
    report = {'evidence_sha256':hashes,'seed':2026100313,'programs':len(rows),'native_programs':sum('native_output_sha256' in r for r in rows),'source_codes':sum(r['source_codes'] for r in rows),'native_tolerance':{'absolute':NATIVE_ABSOLUTE_TOLERANCE,'relative':NATIVE_RELATIVE_TOLERANCE,'coordinate_bound':10000,'codes_per_program_bound':100},'rust_tolerance':'Exact equality for independently computed dyadic rational results','builtin_integration_programs':4,'missing_width_provenance_programs':2,'rust_programs':sum('rust_output_sha256' in r for r in rows),'strict_cli_refusals':refusals,'failures':0,'rows':rows}
    args.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:v for k,v in report.items() if isinstance(v,int)}))


if __name__=='__main__':
    main()
