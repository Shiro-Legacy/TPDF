#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Compare C0 mixed state/event order to independent models and pinned native callbacks."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
from program_model import ProgramModel,generate_programs
from text_position_model import json_numbers

T0=runpy.run_path(str(Path(__file__).with_name('differential-text-position.py')))


def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def chain(clip,clips):
    result=[]
    while clip is not None:
        result.append(clip)
        clip=clips[clip]['clip']
    return list(reversed(result))


def semantic_expected(expected):
    result=[]
    current=None
    for ordered in expected['ordered']:
        if ordered['kind']=='path':
            event=expected['path_events'][ordered['index']]
            if event['kind']=='restore':
                target=event['clip']
                while current!=target:
                    current=expected['clips'][current]['clip']
                    ids=chain(current,expected['clips'])
                    result.append({'kind':'pop','clip_depth':len(ids),'clip_ids':ids})
                continue
            ids=chain(event['clip'],expected['clips'])
            value={k:v for k,v in event.items() if k not in ('span','operation','clip','id')}
            value.update(clip_depth=len(ids),clip_ids=ids)
            result.append(value)
            if event['kind']=='clip':current=event['id']
        else:
            kind,index=expected['text']['events'][ordered['index']]
            if kind!='code':continue
            code=expected['text']['codes'][index]
            context=ordered['context']
            ids=chain(context['clip'],expected['clips'])
            result.append({'kind':'code','mode':code['mode'],'clip_depth':len(ids),'clip_ids':ids,'color':context['fill_color'] if code['mode']==0 else None,'code':code['byte'],'unicode':list(map(ord,code['text'])),'matrix':code['rendering'],'advance':code['width']/1000})
    return json_numbers(result)


def native_semantics(result):
    if result['repaired'] or result['cleanup_pops'] or result['initial_ctm']!=[1,0,0,1,0,0] or result['initial_clip'] is not None:
        raise ValueError('native initial/repair/cleanup policy differs')
    clips={}
    output=[]
    for index,event in enumerate(result['events']):
        ids=[clips[i] for i in event['clip_ids']]
        if len(ids)!=event['clip_depth']:raise ValueError('native clip depth differs from ancestry')
        if event['kind']=='text':
            groups=[]
            for item in event['items']:
                if item['primary']:groups.append({'item':item,'unicode':[item['unicode']]})
                else:
                    if not groups or item['callback']!=groups[-1]['item']['callback']:raise ValueError('orphan text continuation')
                    groups[-1]['unicode'].append(item['unicode'])
            if event['color_observable']!=(event['mode']==0):raise ValueError('native color observation differs')
            for group in groups:
                item=group['item']
                output.append({'kind':'code','mode':event['mode'],'clip_depth':len(ids),'clip_ids':ids,'color':event['color'],'code':item['cid'],'unicode':group['unicode'],'matrix':item['page_matrix'],'advance':item['advance']})
        else:
            value=copy.deepcopy(event)
            value['clip_ids']=ids
            output.append(value)
            if event['kind']=='clip':clips[index]=len(clips)
    return output


def compare_native(actual,expected):
    if len(actual)!=len(expected):raise ValueError(f'mixed callback semantic count {len(actual)} != {len(expected)}')
    max_error=0
    for index,(got,want) in enumerate(zip(actual,expected)):
        if want['kind']!='code':
            if got!=want:raise ValueError(f'native path/state/order differs at {index}: {got!r} != {want!r}')
            continue
        fields=set(want)-{'matrix','advance'}
        if set(got)!=set(want) or any(got[k]!=want[k] for k in fields):raise ValueError(f'native text state/order differs at {index}: {got!r} != {want!r}')
        for a,b in zip([*got['matrix'],got['advance']],[*want['matrix'],want['advance']]):
            assert abs(b)<=10000
            delta=abs(a-b)
            if delta>T0['NATIVE_ABSOLUTE_TOLERANCE']+T0['NATIVE_RELATIVE_TOLERANCE']*max(1,abs(b)):
                raise ValueError(f'native text matrix differs at {index}: {a} != {b}')
            max_error=max(max_error,delta)
    return max_error


def compare_rust(result,expected,spec):
    T0['compare_rust'](result['text'],expected['text'],spec)
    paths=result['paths']
    if paths['initial_ctm']!=[1,0,0,1,0,0] or paths['initial_clip']!={'kind':'unbounded'}:
        raise ValueError('Rust initial graphics state differs')
    expected_paths=[]
    by_operation={}
    for event in expected['path_events']:
        if 'path' in event and event['operation'] not in by_operation:
            by_operation[event['operation']]=len(expected_paths)
            expected_paths.append(event['path'])
    if paths['paths']!=json_numbers(expected_paths):raise ValueError('Rust retained path geometry/identity differs')
    expected_clips=[{'parent':c['clip'],'path':by_operation[c['operation']],'rule':c['rule'],'matrix':c['matrix']} for c in expected['clips']]
    if paths['clips']!=json_numbers(expected_clips):raise ValueError('Rust clip ancestry/geometry differs')
    if paths['dashes']!=[[2,3]]:raise ValueError('Rust dash ownership differs')
    events=[]
    for event in expected['path_events']:
        kind=event['kind']
        if kind=='restore':value={'kind':kind,'clip':event['clip'],'span':event['span']}
        elif kind=='clip':value={'kind':kind,'clip':event['id'],'span':event['span']}
        else:
            value={'kind':kind,'path':by_operation[event['operation']],'matrix':event['matrix'],'clip':event['clip'],'span':event['span'],'color':event['color']}
            if kind=='fill':value['rule']=event['rule']
            else:
                style=event['style']
                value['style']={'width':style['width'],'cap':style['start_cap'],'join':style['join'],'miter':style['miter'],'dash':0 if style['dash'] else None,'phase':style['phase']}
        events.append(value)
    if paths['events']!=json_numbers(events):raise ValueError('Rust raw path event/state differs')
    if len(result['events'])!=len(expected['ordered']):raise ValueError('Rust mixed event count differs')
    for got,want in zip(result['events'],expected['ordered']):
        keys={'operation_index':want['operation'],'span':list(want['span']),'kind':want['kind'],'event':want['index']}
        if any(got[k]!=v for k,v in keys.items()):raise ValueError('Rust global event order/provenance differs')
        if want['kind']=='path':
            if got['context'] is not None:raise ValueError('Rust path event has text context')
        else:
            context=result['text_contexts'][got['context']]
            wanted=want['context']
            wanted={'matrix':wanted['ctm'],'clip':wanted['clip'],'color':wanted['fill_color']}
            if context!=json_numbers(wanted):raise ValueError('Rust text color/clip/CTM snapshot differs')
    expected_ops=len(spec['operations'])
    expected_children=sum(2 if op in {'q','Q','cm'} else 1 for op,_ in spec['operations'])
    if result['usage']['operations']!=expected_ops or result['usage']['child_operations']!=expected_children:
        raise ValueError('Rust original/child operation accounting differs')
    work=expected_ops+expected_children
    for op,args in spec['operations']:
        if op=='TJ':work+=len(args[0])+sum(len(item) for item in args[0] if isinstance(item,bytes))
        elif op in ('Tj',"'"):work+=len(args[0])
        elif op=='"':work+=len(args[2])
        elif op=='d':work+=len(args[0])
    if result['usage']['work_units']!=work:raise ValueError('Rust aggregate execution work accounting differs')


def strict_refusals(binary,config):
    prefix=b'/F0 8 Tf BT (A) Tj ET 0 0 2 3 re f '
    cases=[
        b'Q',b'q',b'BT',b'ET',b'W BT ET',b'W* q Q BT ET',b'0 0 m BT ET',
        b'BT 0 0 m ET',b'BT n ET',b'BT W ET',b'BT q ET',b'BT Q ET',b'BT 1 0 0 1 0 0 cm ET',
        b'0 0 m 2 0 0 2 0 0 cm n',b'q 2 0 0 2 0 0 cm 0 0 m Q n',
        b'/X Do',b'BI binary',b'/Tag BMC',b'BX unknown EX',b'/DeviceRGB cs',b'/State gs',
        b'BT /F0 8 Tf 1 Tr (A) Tj ET',b'BT /F0 8 Tf <00> Tj ET',
        b'/Absent 8 Tf',b'9007199254740993 0 m n',b'2 g',
    ]
    for suffix in cases:
        run=subprocess.run([str(binary),'--program='+str(config),'-'],input=prefix+suffix,capture_output=True,timeout=10)
        if run.returncode<=0 or run.stdout or not run.stderr or b'panicked' in run.stderr:
            raise ValueError('non-atomic or missing C0 refusal: '+repr(suffix))
    return len(cases)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust',type=Path,default=Path('target/debug/pdf-inspect'))
    parser.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-program-oracle'))
    parser.add_argument('--native-only',action='store_true')
    parser.add_argument('--report',type=Path,default=Path('reference/program-differential-c0.json'))
    args=parser.parse_args()
    paths=[args.native,Path(__file__),Path(__file__).with_name('program_model.py'),Path(__file__).with_name('text_position_model.py'),Path(__file__).with_name('differential-text-position.py')]
    if not args.native_only:paths.append(args.rust)
    hashes={str(p):digest(p) for p in paths}
    rows=[]
    with tempfile.TemporaryDirectory(prefix='tandem-program-') as temporary:
        config=Path(temporary)/'fonts.pdfsyntax'
        for spec in generate_programs():
            raw,expected=ProgramModel(spec['fonts']).run(spec['operations'])
            semantic=semantic_expected(expected)
            native=subprocess.run([str(args.native)],input=T0['native_pdf'](spec,raw),capture_output=True,timeout=20)
            if native.returncode or native.stderr:raise ValueError('native refusal: '+native.stderr.decode(errors='replace'))
            error=compare_native(native_semantics(json.loads(native.stdout)),semantic)
            row={'case':spec['case'],'content_sha256':hashlib.sha256(raw).hexdigest(),'ordered_events':len(expected['ordered']),'semantic_events':len(semantic),'source_codes':len(expected['text']['codes']),'native_max_absolute_error':error,'native_output_sha256':hashlib.sha256(native.stdout).hexdigest()}
            if not args.native_only:
                config.write_bytes(T0['font_config'](spec['fonts']))
                rust=subprocess.run([str(args.rust.resolve()),'--program='+str(config),'-'],input=raw,capture_output=True,timeout=20)
                if rust.returncode or rust.stderr:raise ValueError('Rust refusal: '+rust.stderr.decode(errors='replace'))
                compare_rust(json.loads(rust.stdout),expected,spec)
                row['rust_output_sha256']=hashlib.sha256(rust.stdout).hexdigest()
            rows.append(row)
        refusals=0 if args.native_only else strict_refusals(args.rust.resolve(),config)
    if hashes!={str(p):digest(p) for p in paths}:raise ValueError('binary/evidence changed during comparison')
    report={'seed':2026100314,'evidence_sha256':hashes,'programs':len(rows),'native_programs':len(rows),'rust_programs':sum('rust_output_sha256' in r for r in rows),'ordered_events':sum(r['ordered_events'] for r in rows),'semantic_events':sum(r['semantic_events'] for r in rows),'source_codes':sum(r['source_codes'] for r in rows),'native_text_tolerance':{'absolute':T0['NATIVE_ABSOLUTE_TOLERANCE'],'relative':T0['NATIVE_RELATIVE_TOLERANCE']},'path_color_clip_state_tolerance':'exact','strict_cli_refusals':refusals,'failures':0,'rows':rows,'limitations':['Native callback batch boundaries are retained in raw hashes but normalized to primary code records for semantic order comparison.','Mode3 color snapshot is independently tested only; native ignore_text has no observable color.','Native normalized advances share T0 float32 tolerance: an observed750-unit width reports0.75000006, while Rust width and arithmetic expectations stay exact.','No empty-path paint cases in the parity generator; their accepted P0 policy difference remains explicit.']}
    args.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:v for k,v in report.items() if isinstance(v,int)}))


if __name__=='__main__':main()
