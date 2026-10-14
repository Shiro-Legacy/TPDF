#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R20 comparator provenance/geometry mutations and bounded native wrapper negatives."""
import argparse,copy,gzip,hashlib,json,runpy,subprocess,sys,tempfile,math
from pathlib import Path
ENGINE=Path(__file__).resolve().parent.parent
A=runpy.run_path(str(ENGINE/'tools/differential-builtin-outlines.py'))
P=runpy.run_path(str(ENGINE/'tools/differential-page-outlines.py'))
from page_outlines_model import generated,pdf,check_commands

def check_dimensions(width,height,page):
    bounds=page['initial_clip'];expected=(math.ceil(bounds[2]-bounds[0]),math.ceil(bounds[3]-bounds[1]))
    if (width,height)!=expected:raise ValueError('Rust extent differs from independent native page bounds')

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--rust',required=True);ap.add_argument('--asset-native',default='/private/tmp/tandem-mupdf-reference/mupdf-builtin-outlines-oracle');ap.add_argument('--page-native',default='/private/tmp/tandem-mupdf-reference/mupdf-page-outlines-oracle');ap.add_argument('--evidence',type=Path,default=Path('/private/tmp/tandem-r20-builtin-evidence'));ap.add_argument('--report',type=Path,required=True);ap.add_argument('--raster-report',type=Path);ap.add_argument('--raster-evidence',type=Path,default=Path('/private/tmp/tandem-r20-raster-evidence')); a=ap.parse_args();mutations=[];wrappers=[];positive=[]
    face=json.loads((ENGINE/'reference/builtin-outlines-manifest.json').read_text())['faces'][0]
    r=json.loads(gzip.decompress((a.evidence/'Helvetica-rust.json.gz').read_bytes()));n=json.loads(gzip.decompress((a.evidence/'Helvetica-native.json.gz').read_bytes()));A['compare'](r,n,face);positive.append('all654assetrecords')
    def rejected(label,call):
        try:call()
        except (ValueError,KeyError,IndexError,TypeError):mutations.append(label)
        else:raise AssertionError('mutation survived: '+label)
    changes=[('native asset SHA','asset_sha256','0'*64),('native asset bytes','asset_bytes',1),('native units','units_per_em',1024),('native glyph count','glyph_count',655)]
    for label,key,value in changes:
        bad=copy.deepcopy(n);bad[key]=value;rejected(label,lambda:A['compare'](r,bad,face))
    for label,which,key,value in [('compiled face','rust','face','Times-Roman'),('native face','native','face','Times-Roman')]:
        rr=copy.deepcopy(r);nn=copy.deepcopy(n);(rr if which=='rust' else nn)[key]=value;rejected(label,lambda:A['compare'](rr,nn,face))
    for label,mutate in [('native missing row',lambda x:x['glyphs'].pop()),('native extra row',lambda x:x['glyphs'].append(copy.deepcopy(x['glyphs'][0]))),('native false membership',lambda x:x['glyphs'][0].update(found=False)),('native reorder',lambda x:x['glyphs'].reverse()),('native GID',lambda x:x['glyphs'][1].update(native_gid=999)),('native advance',lambda x:x['glyphs'][1].update(advance=123))]:
        bad=copy.deepcopy(n);mutate(bad);rejected(label,lambda:A['compare'](r,bad,face))
    nonempty=next(i for i,v in enumerate(r['glyphs']) if v['commands'])
    for key,value in [('name','Bogus'),('native_gid',999),('advance',123),('units_per_em',1024),('asset_sha256','0'*64),('command_range',[0,0]),('commands',[])]:
        bad=copy.deepcopy(r);bad['glyphs'][nonempty][key]=value;rejected('compiled '+key,lambda:A['compare'](bad,n,face))
    model=next(generated(1));data=model['pdf'];rr,_=P['invoke_page'](a.rust,['--page-outlines=1','-'],data,'rust',1);nn,_=P['invoke_page'](a.page_native,['1'],data,'native',1);assets=P['load_assets'](a.evidence);P['compare_page'](rr,nn,assets,model);positive.append('mixedpageoriginandmodel')
    for label,mutate in [('source code',lambda x:x['glyphs'][0].update(source=66)),('Unicode',lambda x:x['trace']['program']['text'].update(text_utf8_hex='41'*len(bytes.fromhex(x['trace']['program']['text']['text_utf8_hex'])))),('page font identity',lambda x:x['glyphs'][0].update(font=[99,0])),('page native GID',lambda x:x['glyphs'][0].update(native_gid=999)),('page name',lambda x:x['glyphs'][0].update(glyph_name='A')),('page asset SHA',lambda x:x['glyphs'][0].update(asset_sha256='0'*64)),('page kind',lambda x:x['glyphs'][0].update(source_kind='embedded_true_type')),('page command missing',lambda x:x['commands'].pop()),('page command coordinate',lambda x:x['commands'][0].__setitem__(1,x['commands'][0][1]+1)),('page glyph order',lambda x:x['glyphs'].reverse()),('page range',lambda x:x['glyphs'][0].update(command_range=[1,2])),('page initial clip',lambda x:x['trace']['program']['paths']['initial_clip']['bounds'].__setitem__(0,100)),('page extra TT provenance',lambda x:x['glyphs'][0].update(program=[0,0]))]:
        bad=copy.deepcopy(rr);mutate(bad);rejected(label,lambda:P['compare_page'](bad,nn,assets,model))
    badn=copy.deepcopy(nn)
    glyph=next(v for v in badn['events'] if isinstance(v,dict) and v['kind']=='glyph');glyph['asset_sha256']='0'*64
    badr=copy.deepcopy(rr);badr['glyphs'][0]['asset_sha256']='0'*64;rejected('colluding Rust/native wrong asset SHA',lambda:P['compare_page'](badr,badn,assets,model))
    for kind in ['rust','native']:
        for label,code,out,err in [('signal',-9,b'',b''),('unexpectedexit',3,b'',b'failure'),('emptydiagnostic',2,b'',b''),('partialoutput',2,b'partial',b'pdf-inspect: failure'),('successdiagnostic',0,b'{}',b'warning')]:
            rejected(kind+' '+label,lambda:P['read_process'](subprocess.CompletedProcess([],code,out,err),kind,1))
    for kind,prefix in [('rust',b'pdf-inspect: '),('native',b'page-outline oracle: ')]:
        v=P['read_process'](subprocess.CompletedProcess([],2,b'',prefix+b'unsupported'),kind,1);assert v['pages'][0]['status']=='refused';positive.append(kind+' explicitatomicrefusal')
    # A declared Rust refusal must not make an unexpected native signal acceptable.
    with tempfile.TemporaryDirectory(prefix='r20-wrapper-') as directory:
        directory=Path(directory);fake_r=directory/'rust';fake_n=directory/'native';report=directory/'result.json'
        fake_r.write_text('#!/bin/sh\necho "pdf-inspect: unsupported" >&2\nexit 2\n');fake_r.chmod(0o755)
        fake_n.write_text('#!/bin/sh\nkill -9 $$\n');fake_n.chmod(0o755)
        run=subprocess.run([sys.executable,str(ENGINE/'tools/differential-page-outlines.py'),'--rust',str(fake_r),'--native',str(fake_n),'--report',str(report),'--generated','1','--skip-corpus'],capture_output=True,timeout=60)
        output=json.loads(report.read_text());row=output['generated_rows'][0]
        assert run.returncode==1 and row['rust_status']=='refused' and row['native_status']=='document_error' and row['status']=='failure',row
        mutations.append('actual driver rejects mixed Rust refusal/native signal')
    rejected('rational coordinate outside derived enclosure',lambda:check_commands([['m',1,0]],[['m',0,0]],[1,0,0,1,0,0],{}))
    def native(label,binary,args,data,valid=False):
        run=subprocess.run([binary,*args],input=data,capture_output=True,timeout=50)
        if valid:assert run.returncode==0 and not run.stderr,(label,run.stderr);json.loads(run.stdout)
        else:assert run.returncode!=0 and not run.stdout,(label,run.returncode,run.stdout[:100])
        wrappers.append(label)
    native('asset valid names',a.asset_native,['Helvetica'],b'A\nspace\n',True)
    for label,args,query in [('unknownface',['Courier'],b'A\n'),('extraargument',['Helvetica','x'],b'A\n'),('emptyname',['Helvetica'],b'\n'),('namebound',['Helvetica'],b'A'*128+b'\n'),('nonASCII',['Helvetica'],b'\xff\n'),('querybound',['Helvetica'],b'A\n'*4097)]:native('asset '+label,a.asset_native,args,query)
    unknown=subprocess.run([a.asset_native,'Helvetica'],input=b'no_such_glyph\n',capture_output=True,timeout=30);assert not unknown.returncode and json.loads(unknown.stdout)['glyphs'][0]['found']==False;wrappers.append('asset absent name explicit no fallback')
    native('page valid CFF',a.page_native,['1'],data,True)
    for label,args,pdfdata in [('badindex',['0'],data),('indexsuffix',['1x'],data),('indexbeyond',['2'],data),('malformed',['1'],b'%PDF-1.7 invalid'),('inputbound',['1'],b' '*(16*1024*1024+1)),('repairsforbidden',['1'],data.replace(b'xref\n',b'xxxx\n',1))]:native('page '+label,a.page_native,args,pdfdata)
    fonts={'F0':dict(face='Helvetica',names={65:'A'},widths={65:250},mapping={65:'Z'})}
    for label,raw in [('unsupported operator suffix',b'BT /F0 8 Tf (A) Tj ET /X Do'),('unfinished text',b'BT /F0 8 Tf (A) Tj')]:
        result=subprocess.run([a.page_native,'1'],input=pdf(raw,fonts),capture_output=True,timeout=50);assert result.returncode==0 and not result.stderr;out=json.loads(result.stdout);assert out['pages'][0]['status']=='refused';wrappers.append('page '+label+' atomic explicit refusal')
    generator=runpy.run_path(str(ENGINE/'tools/generate-builtin-outlines.py'))
    with tempfile.TemporaryDirectory(prefix='r20-repro-') as directory:
        directory=Path(directory);outputs=[]
        for index in range(2):
            out=directory/f'data{index}.rs';manifest=directory/f'manifest{index}.json'
            generator['generate'](Path('/private/tmp/tandem-mupdf-reference/mupdf-1.28.1-source'),Path(a.asset_native),out,manifest)
            outputs.append((out.read_bytes(),manifest.read_bytes()))
        assert outputs[0]==outputs[1]
        assert outputs[0][0]==(ENGINE/'src/builtin_outlines_data.rs').read_bytes()
        assert outputs[0][1]==(ENGINE/'reference/builtin-outlines-manifest.json').read_bytes()
        positive.append('two fresh generator runs exactly reproduce checked-in data and manifest')
    files=[Path(__file__),ENGINE/'tools/differential-builtin-outlines.py',ENGINE/'tools/differential-page-outlines.py',ENGINE/'tools/page_outlines_model.py',Path(a.rust),Path(a.asset_native),Path(a.page_native)]
    baseline_reports=[ENGINE/'reference/page-glyph-differential-g1.json',ENGINE/'reference/page-raster-differential-r1.json',ENGINE/'reference/page-outlines-differential-b0.json']
    files.extend(baseline_reports)
    prior=json.loads(baseline_reports[0].read_text());current=json.loads(baseline_reports[2].read_text())
    prior_keys={(r['file'],r['page']) for r in prior['rows'] if r['status']=='matched'};current_keys={(r['file'],r['page']) for r in current['rows'] if r['status']=='matched'}
    if not prior_keys<=current_keys:raise ValueError('prior embedded outline acceptance disappeared')
    dimension_checks=[]
    if a.raster_report:
        from page_raster_reference import parse_ppm
        raster=json.loads(a.raster_report.read_text());files.append(a.raster_report.resolve())
        prior_raster=json.loads(baseline_reports[1].read_text());old={(r['file'],r['page']) for r in prior_raster['rows'] if r['status']=='native_boundary_diagnostic'};now={(r['file'],r['page']) for r in raster['rows'] if r['status']=='native_boundary_diagnostic'}
        if not old<=now:raise ValueError('prior raster acceptance disappeared')
        for row in raster['rows']:
            if row['status']=='refused':continue
            if row['status']!='native_boundary_diagnostic':raise ValueError('raster gate has failure')
            directory=a.raster_evidence/'corpus'/row['file']/str(row['page'])
            raw=gzip.decompress((directory/'native-trace.json.gz').read_bytes());ppm=gzip.decompress((directory/'rust.ppm.gz').read_bytes())
            if hashlib.sha256(raw).hexdigest()!=row['native_trace_sha256'] or hashlib.sha256(ppm).hexdigest()!=row['rust_ppm_sha256']:raise ValueError('saved raster evidence hash')
            width,height,_=parse_ppm(ppm);native=json.loads(raw)['pages'][0]
            if width!=row['width'] or height!=row['height'] or native['page']!=row['page']:raise ValueError('raster report identity')
            check_dimensions(width,height,native);dimension_checks.append(dict(file=row['file'],page=row['page'],width=width,height=height,native_bounds=native['initial_clip']))
        positive.append('all accepted corpus dimensions independently verified from native page bounds')
    check_dimensions(10,20,dict(initial_clip=[0,0,10,20]))
    rejected('cropped Rust canvas dimensions',lambda:check_dimensions(9,20,dict(initial_clip=[0,0,10,20])))
    rejected('expanded Rust canvas dimensions',lambda:check_dimensions(10,21,dict(initial_clip=[0,0,10,20])))
    report=dict(status='passed',independent_corpus_dimensions=len(dimension_checks),dimension_checks=dimension_checks,positive_checks=len(positive),positive=positive,rejected_mutations=len(mutations),mutations=mutations,native_wrapper_checks=len(wrappers),wrappers=wrappers,evidence_sha256={str(f):hashlib.sha256(f.read_bytes()).hexdigest() for f in files});a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if not isinstance(v,(dict,list))}))
if __name__=='__main__':main()
