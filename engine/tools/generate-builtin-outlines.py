#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Reproduce OFL canonical unhinted CFF outlines for the pinned M1 membership."""
import argparse,collections,gzip,hashlib,json,re,struct,subprocess
from fractions import Fraction as F
from pathlib import Path
FACES=[('HELVETICA','Helvetica','Helvetica','NimbusSans-Regular','ed4ead49b4d090c80c1d4a8d771879153a41af262d331168dd2635508634cfa1','366720e054067fa49d030f58005c739fc787dcc9727a9327a28767777791d706',12788),('TIMES_ROMAN','Times-Roman','TimesRoman','NimbusRoman-Regular','e476212c416039aa9805f20d945e2b3a522be5de15125cb566d257482379ccd9','eb609a306b81357853ba91c2a3c47f3bba04f3a8cfdeedc733b1aa4af5d03d4c',17838)]

def sha(data):return hashlib.sha256(data).hexdigest()
def cff_index(data,at):
    if at+2>len(data):raise ValueError('CFF INDEX header')
    count=int.from_bytes(data[at:at+2],'big');at+=2
    if not count:return [],at
    size=data[at];at+=1
    if not 1<=size<=4 or count>4096 or at+(count+1)*size>len(data):raise ValueError('CFF INDEX extent')
    offsets=[int.from_bytes(data[p:p+size],'big') for p in range(at,at+(count+1)*size,size)];base=at+(count+1)*size
    if offsets[0]!=1 or any(a>b for a,b in zip(offsets,offsets[1:])) or base+offsets[-1]-1>len(data):raise ValueError('CFF INDEX offsets')
    return [data[base+a-1:base+b-1] for a,b in zip(offsets,offsets[1:])],base+offsets[-1]-1

def cff_dict(data):
    at=0;stack=[];result={}
    while at<len(data):
        b=data[at];at+=1
        if b<=21:
            op=(b,data[at]) if b==12 else (b,)
            if b==12:at+=1
            if op in result:raise ValueError('duplicate CFF DICT operator')
            result[op]=stack;stack=[];continue
        if b==28:value=int.from_bytes(data[at:at+2],'big',signed=True);at+=2
        elif b==29:value=int.from_bytes(data[at:at+4],'big',signed=True);at+=4
        elif b==30:
            text='';done=False
            while not done:
                if at>=len(data):raise ValueError('CFF DICT real')
                x=data[at];at+=1
                for nibble in (x>>4,x&15):
                    if nibble==15:done=True;break
                    if nibble==13:raise ValueError('reserved real nibble')
                    text+=str(nibble) if nibble<10 else {10:'.',11:'E',12:'E-',14:'-'}[nibble]
            value=F(text)
        elif 32<=b<=246:value=b-139
        elif 247<=b<=250:value=(b-247)*256+data[at]+108;at+=1
        elif 251<=b<=254:value=-(b-251)*256-data[at]-108;at+=1
        else:raise ValueError('unsupported CFF DICT number')
        if at>len(data) or len(stack)>=48:raise ValueError('CFF DICT operand extent')
        stack.append(F(value))
    if stack:raise ValueError('CFF DICT trailing operands')
    return result

def matrix_evidence(data):
    if len(data)<4 or data[0:2]!=b'\x01\x00' or not 4<=data[2]<=len(data):raise ValueError('CFF1 header')
    names,at=cff_index(data,data[2]);top,_=cff_index(data,at)
    if len(names)!=1 or len(top)!=1:raise ValueError('single CFF face required')
    fields=cff_dict(top[0]);matrix=fields.get((12,7),[F(1,1000),F(0),F(0),F(1,1000),F(0),F(0)])
    if matrix!=[F(1,1000),0,0,F(1,1000),0,0] or (12,30) in fields or fields.get((12,6),[F(2)])!=[F(2)]:raise ValueError('unsupported CFF matrix/type/CID')
    return dict(name=names[0].decode('ascii'),font_matrix=[str(v) for v in matrix],font_matrix_explicit=(12,7) in fields,charstring_type=2,normalization_units=1000,translation=[0,0],kind='single non-CID CFF1')

def run(oracle,face,names):
    r=subprocess.run([str(oracle),face],input=('\n'.join(names)+'\n').encode('ascii'),capture_output=True,timeout=50)
    if r.returncode or r.stderr or len(r.stdout)>64*1024*1024:raise ValueError('native extraction failure: '+r.stderr.decode(errors='replace')[:1000])
    return json.loads(r.stdout),r.stdout

def validate_commands(commands):
    opened=False;counts=collections.Counter();xs=[];ys=[]
    for c in commands:
        op=c[0]
        if len(c)!={'m':3,'l':3,'c':7,'h':1}.get(op):raise ValueError('canonical command shape')
        if op=='m':
            if opened:raise ValueError('unclosed prior contour')
            opened=True
        elif op=='h':
            if not opened:raise ValueError('orphan close')
            opened=False
        elif not opened:raise ValueError('draw without contour')
        for v in c[1:]:
            if not isinstance(v,int) or abs(v)>1000000:raise ValueError('canonical integer coordinate bound')
        xs.extend(c[1::2]);ys.extend(c[2::2]);counts[op]+=1
    if opened:raise ValueError('unclosed contour')
    return counts,xs,ys

def generate(source,oracle,output,manifest,evidence=None):
    text=['// SPDX-License-Identifier: OFL-1.1','// Derived canonical outline data from pinned URW CFF font resources.','// Copyright (URW)++, Copyright 2014 by (URW)++ Design & Development.','// Copyright 2016 URW++ Design & Development.','// Full SIL Open Font License1.1 and applicable notices: see NOTICE and','// reference/licenses/urw-outlines/OFL.txt. Implementation code remains AGPL.','// Generated by tools/generate-builtin-outlines.py; do not hand-edit.','// Exact AFM/CFF shared names; unhinted actual CFF drawing origin, units1000.','use super::{BuiltinFace, BuiltinOutline, BuiltinOutlineCommand};','']
    rows=[]
    for symbol,face,variant,file,afm_sha,cff_sha,expected_count in FACES:
        afm=source/f'resources/fonts/urw/input/{file}.afm';cff=source/f'resources/fonts/urw/{file}.cff';afm_bytes=afm.read_bytes();cff_bytes=cff.read_bytes()
        if sha(afm_bytes)!=afm_sha or sha(cff_bytes)!=cff_sha:raise ValueError('pinned font source mismatch')
        matrix=matrix_evidence(cff_bytes);metrics={}
        for line in afm_bytes.decode('ascii').splitlines():
            m=re.match(r'^C -?\d+ ; WX (\d+) ; N ([^ ;]+) ;',line)
            if m:
                width,name=m.groups()
                if name in metrics:raise ValueError('duplicate AFM name')
                metrics[name]=int(width)
        if len(metrics)!=855:raise ValueError('AFM size')
        names=sorted(metrics);native,raw=run(oracle,face,names)
        if native['asset_sha256']!=cff_sha or native['asset_bytes']!=len(cff_bytes) or native['units_per_em']!=1000 or native['glyph_count']!=654:raise ValueError('native loaded asset differs')
        if [g['name'] for g in native['glyphs']]!=names:raise ValueError('native query order')
        glyphs=[g for g in native['glyphs'] if g['found']]
        if any(not 1<=len(g['name'].encode('ascii'))<=127 for g in glyphs):raise ValueError('canonical name byte bound')
        if len(glyphs)!=654 or sorted(g['native_gid'] for g in glyphs)!=list(range(654)):raise ValueError('shared membership/gid coverage')
        commands=[];records=[];counts=collections.Counter();xs=[];ys=[]
        for g in glyphs:
            if g['advance']!=metrics[g['name']]:raise ValueError('M1 metric differs')
            stats,gx,gy=validate_commands(g['commands']);counts.update(stats);xs+=gx;ys+=gy
            records.append(dict(name=g['name'],native_gid=g['native_gid'],command_start=len(commands),command_len=len(g['commands']),advance=g['advance'],commands_sha256=sha(json.dumps(g['commands'],separators=(',',':')).encode())))
            commands.extend(g['commands'])
        if len(commands)!=expected_count or native['commands']!=expected_count:raise ValueError('canonical command count')
        text.extend([f'// Source CFF SHA256 {cff_sha}', '#[rustfmt::skip]',f'pub(super) static {symbol}_COMMANDS: [BuiltinOutlineCommand; {len(commands)}] = ['])
        for c in commands:
            if c[0]=='h':value='Close'
            else:
                kind,keys={'m':('MoveTo',['x','y']),'l':('LineTo',['x','y']),'c':('CubicTo',['x1','y1','x2','y2','x','y'])}[c[0]];value=kind+' { '+', '.join(f'{k}: {v}' for k,v in zip(keys,c[1:]))+' }'
            text.append('    BuiltinOutlineCommand::'+value+',')
        text.extend(['];','', '#[rustfmt::skip]',f'pub(super) static {symbol}: [BuiltinOutline; 654] = ['])
        for g in records:
            name=g['name'].replace('\\','\\\\').replace('"','\\"');text.append(f'    BuiltinOutline {{ face: BuiltinFace::{variant}, name: b"{name}", native_gid: {g["native_gid"]}, command_start: {g["command_start"]}, command_len: {g["command_len"]} }},')
        text.extend(['];',''])
        rows.append(dict(face=face,cff_path=str(cff),cff_sha256=cff_sha,cff_bytes=len(cff_bytes),afm_path=str(afm),afm_sha256=afm_sha,font=matrix,native_output_sha256=sha(raw),glyphs=654,commands=len(commands),command_counts=dict(counts),coordinate_extrema=[min(xs),min(ys),max(xs),max(ys)],maximum_name_bytes=max(len(g['name']) for g in records),empty_glyphs=[g['name'] for g in records if not g['command_len']],records=records))
        if evidence:evidence.mkdir(parents=True,exist_ok=True);(evidence/(face+'.json.gz')).write_bytes(gzip.compress(raw,mtime=0))
    data=('\n'.join(text).rstrip()+'\n').encode();output.write_bytes(data)
    sources=[Path(__file__),oracle]
    for name in ['mupdf-builtin-outlines-oracle.c','build-builtin-outlines-reference.sh']:
        p=Path(__file__).with_name(name)
        if p.exists():sources.append(p)
    result=dict(schema=1,license='OFL-1.1 derived font data; AGPL-3.0-or-later implementation and generator',drawing_origin='Unscaled unhinted CFF drawing origin; no glyf/hmtx bearing correction',faces=rows,glyphs=sum(r['glyphs'] for r in rows),commands=sum(r['commands'] for r in rows),output_bytes=len(data),output_sha256=sha(data),generator_evidence_sha256={str(p):sha(p.read_bytes()) for p in sources})
    manifest.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:v for k,v in result.items() if k not in ('faces','generator_evidence_sha256')}))

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('source',type=Path);p.add_argument('oracle',type=Path);p.add_argument('output',type=Path);p.add_argument('manifest',type=Path);p.add_argument('--evidence-dir',type=Path);a=p.parse_args();generate(a.source,a.oracle,a.output,a.manifest,a.evidence_dir)
if __name__=='__main__':main()
