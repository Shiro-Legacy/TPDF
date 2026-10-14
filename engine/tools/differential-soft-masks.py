#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R24 original attached-mask proof, generated controls and full R23 pixel replay."""
import argparse,base64,collections,gzip,hashlib,json,subprocess,time,zlib
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
from image_decode_cases import pdf,stream,predictor,runlength,LZW,ppm
from soft_mask_corpus_model import certify,source_alpha
ENGINE=Path(__file__).resolve().parents[1]
sha=lambda b:hashlib.sha256(b).hexdigest()
def require(ok,why):
    if not ok:raise ValueError(why)
def run(binary,args,data,prefix=None):
    r=subprocess.run([str(binary),*map(str,args)],input=data,capture_output=True,timeout=60)
    if r.returncode:
        require(prefix and r.returncode==2 and not r.stdout and r.stderr.startswith(prefix) and r.stderr[len(prefix):].strip(),'unexpected process failure: '+r.stderr.decode(errors='replace'))
        return None,r.stderr.decode()
    require(not r.stderr and r.stdout,'empty/noisy success');return r.stdout,None
def save(dst,name,data):
    p=dst/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(gzip.compress(data,mtime=0));return dict(path=str(p),sha256=sha(data))
def pixels(data):
    import re
    m=re.match(rb'P6\n(\d+) (\d+)\n255\n',data);require(m is not None,'ppm envelope');w,h=map(int,m.groups());b=data[m.end():];require(len(b)==w*h*3,'ppm extent');return w,h,b

# Every file that can change this gate's expectations or native interpretation.
DEPENDENCIES = (
    'reference/image-corpus-differential-r23c.json',
    'reference/image-native-pins.sha256',
    'reference/soft-mask-Helvetica-r24.afm',
    'reference/soft-mask-rust-checks-r24.json',
    'tools/image_decode_cases.py', 'tools/differential-lzw.py',
    'tools/image_page_model.py', 'tools/page_raster_review_model.py',
    'tools/stroke_review_model.py', 'tools/mupdf-image-page-oracle.c',
    'tools/build-image-reference.sh',
)
BINARY_ROLES = {'rust', 'probe', 'decode', 'mask', 'native', 'trace'}

def source_files(root):
    return sorted([root/'Cargo.toml', root/'Cargo.lock',
                   *root.glob('src/*.rs'), *root.glob('tests/*.rs'),
                   *root.glob('examples/*.rs')])

def identity_files(root, binaries):
    require(set(binaries) == BINARY_ROLES, 'exact binary roles')
    paths = [Path(v) for v in binaries.values()]
    require(all(p.is_absolute() for p in paths) and len(set(paths)) == 6,
            'distinct absolute binary paths')
    return sorted(set(source_files(root) + paths +
                      list(root.glob('tools/*soft*')) +
                      [root/name for name in DEPENDENCIES]))

def generated():
    w,h=7,5;base=bytes((i*73+29)%256 for i in range(w*h*3))
    for inv in (False,True):
        raw=bytes([0 if inv else 255])*(w*h);decode=' /Decode [1 0]' if inv else ''
        variants=[('identity',raw,''),('hex',raw.hex().encode()+b'>','/Filter /ASCIIHexDecode'),('a85',base64.a85encode(raw)+b'~>','/Filter /ASCII85Decode'),('runlength',runlength(raw),'/Filter /RunLengthDecode'),('flate',zlib.compress(raw),'/Filter /FlateDecode'),('chain',base64.a85encode(zlib.compress(raw))+b'~>','/Filter [/ASCII85Decode /FlateDecode] /DecodeParms [null null]')]
        for early in (0,1):variants.append((f'lzw-{early}',LZW(raw,early)[0],f'/Filter /LZWDecode /DecodeParms << /EarlyChange {early} >>'))
        for p in (2,10,11,12,13,14,15):
            pred=predictor(raw,w,h,1,p)
            for filt in ('FlateDecode','LZWDecode'):
                encoded=zlib.compress(pred) if filt=='FlateDecode' else LZW(pred,1)[0]
                variants.append((f'{filt}-predictor-{p}',encoded,f'/Filter /{filt} /DecodeParms << /Predictor {p} /Colors 1 /Columns {w} /BitsPerComponent 8 >>'))
        for label,encoded,meta in variants:
            yield case(f'{label}-inv{int(inv)}',base,w,h,encoded,meta+decode,bytes([255])*(w*h),True)
    for inv in (False,True):
        for index in (0,w*h//2,w*h-1):
            raw=bytearray([0 if inv else 255])*(w*h);raw[index]=1 if inv else 254
            expected=bytes(255-v for v in raw) if inv else bytes(raw)
            yield case(f'nonopaque-inv{int(inv)}-{index}',base,w,h,bytes(raw),'/Decode [1 0]' if inv else '',expected,False)
    for label,field in [('matte','/Matte [0]'),('transfer','/TR /Identity'),('interpolate','/Interpolate true'),('nested','/SMask 7 0 R'),('embedded','/SMaskInData 0')]:
        yield case(label,base,w,h,bytes([255])*(w*h),field,bytes([255])*(w*h),False)

def case(label,base,w,h,encoded,meta,alpha,accepted):
    prefix=[b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 32 32] /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R >>',stream(b'q 24 0 0 24 4 4 cm /Im Do Q')]
    base_meta=f'/Type /XObject /Subtype /Image /Width {w} /Height {h} /BitsPerComponent 8 /ColorSpace /DeviceRGB'
    mask=stream(encoded,f'/Type /XObject /Subtype /Image /Width {w} /Height {h} /BitsPerComponent 8 /ColorSpace /DeviceGray {meta}')
    plain=pdf(prefix+[stream(base,base_meta),mask,stream(bytes([255])*(w*h),f'/Subtype /Image /Width {w} /Height {h} /BitsPerComponent 8 /ColorSpace /DeviceGray')])
    masked=pdf(prefix+[stream(base,base_meta+' /SMask 6 0 R'),mask,stream(bytes([255])*(w*h),f'/Subtype /Image /Width {w} /Height {h} /BitsPerComponent 8 /ColorSpace /DeviceGray')])
    return dict(label=label,pdf=masked,unmasked=plain,alpha=alpha,accepted=accepted,expected_base=ppm(base,w,h,3),w=w,h=h)

def generated_row(a,c):
    dst=a.evidence_dir/'generated'/c['label'];evidence={k:save(dst,k+'.gz',c[k]) for k in ('pdf','unmasked','alpha','expected_base')};row=dict(label=c['label'],accepted=c['accepted'],evidence=evidence)
    rust,error=run(a.probe,[5],c['pdf'],b'soft-mask: ')
    require(bool(rust)==c['accepted'],'Rust admission '+c['label']);row['rust_refusal']=error
    if error:evidence['rust_stderr']=save(dst,'rust-stderr.txt.gz',error.encode())
    if rust:
        trace,_=run(a.trace,[1],c['pdf']);page=json.loads(trace)['pages'][0];require(page['status']=='accepted','positive native trace');events=page['events'];require(len(events)==3 and [e['kind'] for e in events]==['opaque_mask_clip','image','pop'],'generated mask callback sequence');clip=events[0];require(clip['base_object']==[5,0] and clip['mask_object']==[6,0] and clip['alpha_hex']==c['alpha'].hex() and clip['matrix']==events[1]['matrix'] and events[1]['clip_ids']==[0] and events[2]['clip_ids']==[],'generated mask callback proof');evidence['native_trace']=save(dst,'native-trace.json.gz',trace)
        value=json.loads(rust);require(value==dict(base_object=5,mask_object=6,width=c['w'],height=c['h'],alpha_hex=c['alpha'].hex()),'Rust provenance/complete alpha');evidence['rust_mask']=save(dst,'rust-mask.json.gz',rust)
        base,_=run(a.decode,[5],c['pdf'],b'image-decode: ');require(base==c['expected_base'],'base bytes');evidence['rust_base']=save(dst,'rust-base.ppm.gz',base)
    native,nerror=run(a.mask,[5],c['pdf'],b'image decode oracle: ')
    if native:
        value=json.loads(native);require(value==dict(base_object=5,mask_object=6,width=c['w'],height=c['h'],alpha_hex=c['alpha'].hex()),'native actual attached alpha');evidence['native_mask']=save(dst,'native-mask.json.gz',native)
    else:require(not c['accepted'],'native positive refused')
    row['native_refusal']=nerror
    if nerror:evidence['native_stderr']=save(dst,'native-stderr.txt.gz',nerror.encode())
    if c['accepted'] or c['label'].startswith('nonopaque'):
        nr,_=run(a.native,[1,72],c['pdf']);nu,_=run(a.native,[1,72],c['unmasked']);evidence['native_page']=save(dst,'native.ppm.gz',nr);evidence['native_unmasked']=save(dst,'native-unmasked.ppm.gz',nu)
        if c['accepted']:
            require(nr==nu,'native opaque/unmasked equality')
            rr,_=run(a.rust,['--page-render=1@72','-'],c['pdf'],b'pdf-inspect: ');ru,_=run(a.rust,['--page-render=1@72','-'],c['unmasked'],b'pdf-inspect: ');require(rr and rr==ru,'Rust opaque/unmasked equality');evidence['rust_page']=save(dst,'rust.ppm.gz',rr);evidence['rust_unmasked']=save(dst,'rust-unmasked.ppm.gz',ru)
            # Independent exact authored geometry/pixels, including all 16 centers.
            # The established independent image model uses known authored texels.
            from fractions import Fraction as F
            from soft_mask_corpus_model import image_samples
            samples=bytearray([255])*(32*32*16*3)
            ev=dict(width=c['w'],height=c['h'],components=3,samples_hex=c['expected_base'].split(b'\n',3)[3].hex())
            for index,color in image_samples(ev,tuple(map(F,[24,0,0,-24,4,28])),32,32):samples[index*3:index*3+3]=color
            from image_page_model import resolve
            model=resolve(samples,128,128);require(pixels(rr)[2]==model,'independent generated pixels');evidence['model']=save(dst,'model.rgb.gz',model)
        else:
            # A one-level alpha decrement can quantize away. A strong control
            # below separately proves native renders nonopaque alpha visibly.
            row['native_differs_from_unmasked']=nr!=nu
    row['status']='passed';return row

def corpus_row(a,prior):
    name,page,dpi=prior['file'],prior['page'],prior['dpi'];data=(ENGINE.parent/'fixtures'/name).read_bytes();dst=a.evidence_dir/'corpus'/name/f'{page}-{dpi}';row=dict(file=name,page=page,dpi=dpi,input_sha256=sha(data),prior_status=prior['status'],evidence={});ev=row['evidence']
    rust,error=run(a.rust,[f'--page-render={page}@{dpi}','-'],data,b'pdf-inspect: ')
    old=prior.get('rust_ppm_sha256')
    if old:
        require(rust and sha(rust)==old,'changed R23 render '+name);row.update(status='preserved_r23',rust_ppm_sha256=sha(rust));ev['rust']=save(dst,'rust.ppm.gz',rust);return row
    trace,_=run(a.trace,[page],data);page_trace=json.loads(trace)['pages'][0];ev['native_trace']=save(dst,'trace.json.gz',trace);row['native_trace_status']=page_trace['status'];row['native_trace_refusal']=page_trace.get('error')
    if not rust:row.update(status='refused',refusal=error);ev['rust_stderr']=save(dst,'rust-stderr.txt.gz',error.encode());return row
    require(page_trace['status']=='accepted','new Rust success with native trace refusal')
    w,h,rgb=pixels(rust);model,boundary,stats=certify(page_trace,w,h,dpi,source_pdf=data);bounds=stats.pop('_channel_bounds');require(len(rgb)==len(bounds),'certificate extent');violations=sum(not lo<=x<=hi for x,(lo,hi) in zip(rgb,bounds));require(not violations,'new corpus certificate violation')
    native,_=run(a.native,[page,dpi*4],data)
    row.update(status='new_opaque_mask_render',rust_ppm_sha256=sha(rust),model_stats=stats,violations=violations,width=w,height=h)
    for key,blob in [('rust',rust),('native4x',native),('model',model),('bounds',(json.dumps(bounds,separators=(',',':'))+'\n').encode())]:ev[key]=save(dst,key+'.gz',blob)
    return row

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--rust',type=Path,required=True);ap.add_argument('--probe',type=Path,required=True);ap.add_argument('--decode',type=Path,required=True);ap.add_argument('--report',type=Path,required=True);ap.add_argument('--evidence-dir',type=Path,required=True);a=ap.parse_args();start=time.monotonic()
    native=Path('/private/tmp/tandem-mupdf-reference');a.mask=native/'mupdf-soft-mask-oracle';a.native=native/'mupdf-image-page-oracle';a.trace=native/'mupdf-soft-mask-trace-oracle';a.evidence_dir=a.evidence_dir.resolve()
    baseline=ENGINE/'reference/image-corpus-differential-r23c.json';prior=json.loads(baseline.read_text());rows=prior['pixel_rows'];require(len(rows)==632 and sum('rust_ppm_sha256' in r for r in rows)==617,'R23 baseline coverage')
    binaries={role:str(getattr(a,role).resolve()) for role in sorted(BINARY_ROLES)}
    identities=identity_files(ENGINE,binaries)
    hashes={str(p):sha(p.read_bytes()) for p in identities}
    cases=list(generated());strong=case('nonopaque-strong',bytes([30,70,120])*35,7,5,bytes([0])*35,'',bytes([0])*35,False);cases.append(strong)
    with ThreadPoolExecutor(max_workers=4) as pool:
        generated_rows=list(pool.map(lambda c:generated_row(a,c),cases))
        corpus=list(pool.map(lambda r:corpus_row(a,r),rows))
        extra=[dict(file=f,page=1,dpi=72,status='supplementary') for f in ['p3/edit-objects.pdf','redact.pdf']]
        corpus.extend(pool.map(lambda r:corpus_row(a,r),extra))
    mask_rows=[]
    for name,base,mask in [('p3/edit-objects.pdf',10,12),('redact.pdf',10,11)]:
        data=(ENGINE.parent/'fixtures'/name).read_bytes();w,h,alpha=source_alpha(data,[mask,0]);r,_=run(a.probe,[base],data,b'soft-mask: ');n,_=run(a.mask,[base],data,b'image decode oracle: ');require(r and n and json.loads(r)==json.loads(n)==dict(base_object=base,mask_object=mask,width=w,height=h,alpha_hex=alpha.hex()),'actual mask proof');require(all(v==255 for v in alpha),'actual opaque proof');dst=a.evidence_dir/'masks'/name;mask_rows.append(dict(file=name,base=base,mask=mask,samples=len(alpha),evidence={k:save(dst,k+'.gz',v) for k,v in [('rust',r),('native',n),('source_alpha',alpha)]}))
    require(next(r for r in generated_rows if r['label']=='nonopaque-strong')['native_differs_from_unmasked'],'strong negative control')
    require(hashes=={str(p):sha(p.read_bytes()) for p in identities},'inputs changed')
    report=dict(status='passed',elapsed_seconds=time.monotonic()-start,identities=hashes,binaries=binaries,generated=generated_rows,corpus=corpus,masks=mask_rows,counts=dict(collections.Counter(r['status'] for r in corpus)))
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ['status','elapsed_seconds','counts']}))
if __name__=='__main__':main()
