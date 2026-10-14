#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent path/state model versus strict Rust and bounded native device trace."""
import argparse
import copy
import hashlib
import json
import math
import random
import subprocess
from pathlib import Path


def concatenate(first, second):
    # Independent 3x3 multiplication, acting on column coordinate vectors.
    def matrix(v):
        a,b,c,d,e,f=v
        return [[a,c,e],[b,d,f],[0,0,1]]
    left,right=matrix(second),matrix(first)
    product=[[sum(left[i][k]*right[k][j] for k in range(3)) for j in range(3)] for i in range(3)]
    return [product[0][0],product[1][0],product[0][1],product[1][1],product[0][2],product[1][2]]


def model(operations):
    state={'matrix':[1,0,0,1,0,0],'fill':{'space':'gray','values':[0]},'stroke':{'space':'gray','values':[0]},'style':{'width':1,'start_cap':0,'dash_cap':0,'end_cap':0,'join':0,'miter':10,'phase':0,'dash':[]},'clip_depth':0}
    stack=[];path=[];events=[];pending=None;point=None;start=None
    def close():
        nonlocal point
        if point is None:raise ValueError('close missing point')
        if path[-1][0]!='h':path.append(['h'])
        point=start
    def paint(kind, rule=None):
        event={'kind':kind,'clip_depth':state['clip_depth'],'path':copy.deepcopy(path),'matrix':list(state['matrix'])}
        if kind in ('fill','clip'):event['rule']=rule
        if kind in ('fill','stroke'):event['color']=copy.deepcopy(state[kind])
        if kind=='stroke':event['style']=copy.deepcopy(state['style'])
        events.append(event)
    for op,args in operations:
        if op=='q':stack.append(copy.deepcopy(state))
        elif op=='Q':
            if not stack:raise ValueError('stack underflow')
            saved=stack.pop()
            if (path or pending) and saved['matrix']!=state['matrix']:raise ValueError('mixed CTM')
            for depth in range(state['clip_depth']-1,saved['clip_depth']-1,-1):events.append({'kind':'pop','clip_depth':depth})
            state=saved
        elif op=='cm':
            if path or pending:raise ValueError('mixed CTM')
            state['matrix']=concatenate(args,state['matrix'])
        elif op in ('g','G','rg','RG','k','K'):
            state['fill' if op.islower() else 'stroke']={'space':{1:'gray',3:'rgb',4:'cmyk'}[len(args)],'values':list(args)}
        elif op in ('w','J','j','M'):
            for key in {'w':['width'],'J':['start_cap','dash_cap','end_cap'],'j':['join'],'M':['miter']}[op]:state['style'][key]=args[0]
        elif op=='d':state['style']['dash']=list(args[0]);state['style']['phase']=args[1]
        elif op=='m':
            path.append(['m']+list(args));point=list(args);start=list(args)
        elif op=='l':
            if point is None:raise ValueError('line missing point')
            path.append(['l']+list(args));point=list(args)
        elif op in ('c','v','y'):
            if point is None:raise ValueError('curve missing point')
            values=list(args) if op=='c' else list(point)+list(args) if op=='v' else list(args)+list(args[-2:])
            path.append(['c']+values);point=values[-2:]
        elif op=='h':close()
        elif op=='re':
            x,y,w,h=args
            path.extend([['m',x,y],['l',x+w,y],['l',x+w,y+h],['l',x,y+h],['h']]);point=[x,y];start=[x,y]
        elif op in ('W','W*'):pending='evenodd' if op=='W*' else 'nonzero'
        elif op in ('S','s','f','F','f*','B','B*','b','b*','n'):
            if op in ('s','b','b*'):close()
            if path:
                if op in ('f','F','f*','B','B*','b','b*'):paint('fill','evenodd' if '*' in op else 'nonzero')
                if op in ('S','s','B','B*','b','b*'):paint('stroke')
            if pending:paint('clip',pending);state['clip_depth']+=1
            path=[];point=None;start=None;pending=None
        else:raise ValueError('unknown model op '+op)
    if stack or path or pending:raise ValueError('unbalanced program')
    return {'events':events}


def syntax(operations):
    def value(v):
        if isinstance(v,list):return '['+' '.join(value(x) for x in v)+']'
        return str(v)
    return ('\n'.join(' '.join([*(value(v) for v in args),op]) for op,args in operations)+'\n').encode()


def native_normalize(expected):
    result=copy.deepcopy(expected)
    for event in result['events']:
        if 'path' not in event:continue
        normalized=[];point=None
        for cmd in event['path']:
            if cmd[0]=='m' and normalized and normalized[-1][0]=='m':normalized.pop()
            if cmd[0]=='c' and point==cmd[1:3]==cmd[3:5]==cmd[5:7]:cmd=['l']+cmd[5:7]
            normalized.append(cmd)
            if cmd[0] in ('m','l'):point=cmd[1:3]
            elif cmd[0]=='c':point=cmd[5:7]
        event['path']=normalized
    return result


def compare(actual,wanted,path='',tol=False):
    if isinstance(wanted,(int,float)) and not isinstance(wanted,bool):
        if not isinstance(actual,(int,float)) or not math.isfinite(actual) or not math.isclose(actual,wanted,rel_tol=2e-6 if tol else 1e-13,abs_tol=1e-5 if tol else 1e-13):raise ValueError(f'{path}: {actual!r} != {wanted!r}')
    elif isinstance(wanted,dict):
        if not isinstance(actual,dict) or set(actual)!=set(wanted):raise ValueError(f'{path}: keys {actual!r} != {wanted!r}')
        for key,value in wanted.items():compare(actual[key],value,path+'/'+key,tol)
    elif isinstance(wanted,list):
        if not isinstance(actual,list) or len(actual)!=len(wanted):raise ValueError(f'{path}: lengths {len(actual) if isinstance(actual,list) else "type"} != {len(wanted)}')
        for index,(a,w) in enumerate(zip(actual,wanted)):compare(a,w,path+f'/{index}',tol)
    elif actual!=wanted:raise ValueError(f'{path}: {actual!r} != {wanted!r}')


def run(binary,program,rust=False):
    result=subprocess.run([str(binary)]+(['--path','-'] if rust else []),input=program,capture_output=True,timeout=20)
    if result.returncode:raise ValueError('process refused: '+result.stderr.decode(errors='replace')[:1500])
    return json.loads(result.stdout)


def cases(count,seed):
    examples=[
        [('q',[]),('re',[0,0,20,30]),('W',[]),('B',[]),('Q',[])],
        [('q',[]),('W',[]),('n',[]),('Q',[])],
        [('m',[1,2]),('q',[]),('l',[8,9]),('Q',[]),('S',[])],
        [('q',[]),('re',[1,2,-10,20]),('W',[]),('Q',[]),('B*',[])],
        [('m',[0,0]),('c',[2,3,4,5,6,7]),('v',[8,9,10,11]),('y',[12,13,14,15]),('h',[]),('l',[16,17]),('S',[])],
        [('m',[0,0]),('m',[1,2]),('l',[3,4]),('h',[]),('h',[]),('b*',[])],
        [('m',[0,0]),('l',[0,0]),('S',[])],
        [('re',[0,0,0,0]),('S',[])],
        [('m',[0,0]),('c',[0,0,0,0,0,0]),('S',[])],
        [('q',[]),('re',[0,0,20,30]),('W',[]),('W*',[]),('n',[]),('q',[]),('re',[1,2,10,12]),('W',[]),('n',[]),('Q',[]),('re',[1,1,2,2]),('f',[]),('Q',[])],
    ]
    for index,ops in enumerate(examples):yield f'edge-{index}',ops
    rng=random.Random(seed)
    for index in range(count):
        ops=[]
        for block in range(rng.randrange(1,8)):
            ops.append(('q',[]))
            for _ in range(rng.randrange(1,4)):
                ops.append(('cm',rng.choice([[1,0,0,1,rng.randrange(-20,20),rng.randrange(-20,20)],[2,0,0,0.5,0,0],[0,1,-1,0,0,0],[1,0.5,0.25,1,0,0],[-1,0,0,1,0,0],[0,0,0,1,0,0]])))
            ops.extend([('w',[rng.choice([0,0.5,1,3])]),('J',[rng.randrange(3)]),('j',[rng.randrange(3)]),('M',[rng.choice([1,10,20])]),('d',[rng.choice([[],[1],[2,3],[0,2,4]]),rng.choice([0,1,1000000])])])
            for color_op in rng.choice([('g','G'),('rg','RG'),('k','K')]):ops.append((color_op,[rng.choice([0,0.25,0.5,1]) for _ in range({'g':1,'G':1,'rg':3,'RG':3,'k':4,'K':4}[color_op])]))
            for _ in range(rng.randrange(1,5)):
                if rng.randrange(2):ops.append(('re',[rng.randrange(-20,20),rng.randrange(-20,20),rng.choice([-15,-3,1,10]),rng.choice([-10,-2,2,15])]))
                else:
                    ops.extend([('m',[rng.randrange(-20,0),rng.randrange(-20,0)]),('l',[rng.randrange(1,20),rng.randrange(1,20)])])
                    if rng.randrange(2):ops.append(('c',[23,29,31,37,41,43]))
                if rng.randrange(3)==0:ops.append((rng.choice(['W','W*']),[]))
                ops.append((rng.choice(['S','s','f','F','f*','B','B*','b','b*','n']),[]))
            ops.append(('Q',[]))
        yield f'generated-{index}',ops


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--rust',type=Path,default=Path('target/debug/pdf-inspect'));p.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-path-oracle'));p.add_argument('--cases',type=int,default=300);p.add_argument('--seed',type=int,default=20261003);p.add_argument('--native-only',action='store_true');p.add_argument('--report',type=Path,default=Path('reference/path-differential-p0.json'));a=p.parse_args()
    binaries=[a.native.resolve()]+([] if a.native_only else [a.rust.resolve()]);hashes={str(f):hashlib.sha256(f.read_bytes()).hexdigest() for f in binaries};rows=[];events=0
    for label,ops in cases(a.cases,a.seed):
        program=syntax(ops);wanted=model(ops);row={'case':label,'input_sha256':hashlib.sha256(program).hexdigest()}
        try:
            if not a.native_only:compare(run(a.rust.resolve(),program,True),wanted)
            compare(run(a.native.resolve(),program),native_normalize(wanted),tol=True)
            row['status']='matched';events+=len(wanted['events'])
        except (ValueError,subprocess.TimeoutExpired) as error:row.update(status='failure',error=str(error))
        rows.append(row)
    refusals=[]; differences=[]
    if not a.native_only:
        for program in [b'Q',b'q',b'0 0 m',b'W',b'h',b's',b'b',b'b*',b'1 2 l',b'1 2 3 4 v',b'0 0 m 1 0 0 1 0 0 cm n',b'q 2 0 0 2 0 0 cm 0 0 m Q n',b'W 1 0 0 1 0 0 cm n',b'BT ET',b'/X Do',b'BI xx',b'BX ignored EX',b'2 g',b'9007199254740993 0 m n',b'0 0 m 1 1 l S BT ET']:
            result=subprocess.run([str(a.rust.resolve()),'--path','-'],input=program,capture_output=True,timeout=20)
            refusals.append({'input':program.decode(),'passed':result.returncode!=0 and not result.stdout,'diagnostic':result.stderr.decode(errors='replace')[:1000]})
        empty=b'S f B'
        rust_empty=run(a.rust.resolve(),empty,True);native_empty=run(a.native.resolve(),empty)
        differences.append({'case':'empty-path-paints','rust_events':len(rust_empty['events']),'native_events':len(native_empty['events']),'passed':rust_empty=={'events':[]} and len(native_empty['events'])==4})
    if any(hashlib.sha256(Path(f).read_bytes()).hexdigest()!=digest for f,digest in hashes.items()):raise RuntimeError('binary changed during comparison')
    report={'reference':'Independent Python graphics state/event model and pinned native identity-CTM device trace','seed':a.seed,'native_only':a.native_only,'binary_sha256':hashes,'cases':len(rows),'matched':sum(r['status']=='matched' for r in rows),'events':events,'failures':sum(r['status']!='matched' for r in rows)+sum(not r['passed'] for r in refusals+differences),'strict_refusals':refusals,'native_differences':differences,'results':rows,'limitations':['No rasterization or pixel equivalence claim.','Native repeated moves and all-equal cubic compression normalized only for native comparison; Rust semantic commands remain intact.','Empty-path paint callback omission is an explicit Rust P0 policy difference, not dropped from native traces.','Implicit native processor cleanup pops excluded; explicit Q pops retained.','Native f32 comparison uses absolute1e-5/relative2e-6 tolerance; independent Rust comparison1e-13.']}
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k not in ('results','strict_refusals')},indent=2))
    for row in [r for r in rows if r['status']!='matched'][:10]:print(row)
    raise SystemExit(bool(report['failures']))
if __name__=='__main__':main()
