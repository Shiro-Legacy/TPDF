#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Native bounds/refusal checks and semantic comparator fault injections for R18."""
import argparse,copy,json,runpy,subprocess,hashlib
from pathlib import Path
from page_glyph_model import generated,bearing_case,pdf
from outline_model import font
from truetype_cmap_model import with_cmap,cmap_table,encode
D=runpy.run_path(str(Path(__file__).with_name('differential-page-glyph.py')))

def main():
    p=argparse.ArgumentParser();p.add_argument('--rust',required=True);p.add_argument('--native',default='/private/tmp/tandem-mupdf-reference/mupdf-page-glyph-oracle');p.add_argument('--report',type=Path,required=True);a=p.parse_args()
    model=next(generated(1));data=model['pdf'];r,_=D['invoke'](a.rust,['--page-glyphs','-'],data);n,_=D['invoke'](a.native,['all'],data);r=r['pages'][0];n=n['pages'][0]
    D['compare_page'](r,n,model);baseline=1;mutations=[]
    def test(label,edit,native=False):
        rr=copy.deepcopy(r);nn=copy.deepcopy(n);edit(nn if native else rr)
        try:D['compare_page'](rr,nn,model)
        except (ValueError,IndexError,KeyError):mutations.append(label);return
        raise AssertionError('comparator accepted mutation: '+label)
    def glyph(field,value):return lambda j:j['glyphs'][0].__setitem__(field,value)
    for field,value in [('source',66),('gid',2),('font',[99,0]),('program',[99,0]),('units_per_em',2048),('origin_shift',0),('metric_lsb',999),('hhea_range',[0,36]),('hmtx_range',[0,12]),('context',99),('code_index',1),('event_index',0),('route','raw_byte'),('lookup_code',66),('command_range',[1,2])]:test(field,glyph(field,value))
    test('selected cmap',lambda j:j['glyphs'][0]['selection'].__setitem__('index',1))
    test('missing glyph attachment',lambda j:j['glyphs'].pop())
    test('reordered glyph attachments',lambda j:j['glyphs'].reverse())
    test('extra unattached command',lambda j:j['commands'].append(['h']))
    test('outline coordinate',lambda j:j['commands'][0].__setitem__(1,j['commands'][0][1]+1))
    test('outline command kind',lambda j:j['commands'][0].__setitem__(0,'l'))
    test('outline command order',lambda j:j['commands'].reverse())
    test('code identity',lambda j:j['trace']['program']['text']['codes'][0].__setitem__('code',66))
    test('unicode',lambda j:j['trace']['program']['text'].__setitem__('text_utf8_hex','00'+j['trace']['program']['text']['text_utf8_hex'][2:]))
    test('initial geometry',lambda j:j['trace']['program']['paths']['initial_ctm'].__setitem__(0,2))
    test('clip ancestry',lambda j:j['trace']['program']['text_contexts'][0].__setitem__('clip',None))
    test('global path/text order',lambda j:j['trace']['program']['events'].reverse())
    test('native source GID',lambda j:next(v for v in j['events'] if v['kind']=='glyph').__setitem__('gid',2),True)
    test('native primary geometry',lambda j:next(v for v in j['events'] if v['kind']=='glyph')['unhinted_commands'][0].__setitem__(1,900),True)
    # The secondary hinted path is intentionally not an equality acceptance condition.
    nn=copy.deepcopy(n);next(v for v in nn['events'] if v['kind']=='glyph')['native_outline']=[['m',900,800],['h']];D['compare_page'](r,nn,model);baseline+=1
    checks=[]
    def native_case(label,data,args=['all'],refusal=None,failed=False):
        result=subprocess.run([a.native,*args],input=data,capture_output=True,timeout=50)
        if failed:assert result.returncode!=0 and not result.stdout,(label,result.returncode,result.stderr)
        else:
            assert result.returncode==0 and not result.stderr,(label,result.returncode,result.stderr)
            out=json.loads(result.stdout);assert not out['repaired'] and len(out['pages'])==1
            if refusal:assert out['pages'][0]['status']=='refused' and refusal in out['pages'][0]['error'],(label,out)
            else:assert out['pages'][0]['status']=='accepted',(label,out)
        checks.append(label)
    native_case('normal trace',data)
    empty,_=font([None,None],seed=1,units=1024);empty=with_cmap(empty,cmap_table([(3,1,encode([(65,65,1-65,None)]))]));empty_pdf=pdf(empty,b'BT /F0 10 Tf <41> Tj ET',{65:250},{65:'A'})
    native_case('mapped nonzero empty glyph accepted',empty_pdf)
    er,_=D['invoke'](a.rust,['--page-glyphs','-'],empty_pdf);en,_=D['invoke'](a.native,['all'],empty_pdf);D['compare_page'](er['pages'][0],en['pages'][0]);assert len(er['pages'][0]['glyphs'])==1 and not er['pages'][0]['commands'];baseline+=1
    native_case('bearing and declared width',bearing_case()['pdf'])
    native_case('input bound',b' '*(16*1024*1024+1),failed=True)
    native_case('page index syntax',data,['-1'],failed=True)
    native_case('page index out of document',data,['2'],failed=True)
    native_case('malformed PDF atomic failure',b'%PDF-1.7\n',failed=True)
    missing=pdf(model['font'],b'BT /F0 10 Tf <45> Tj ET',{69:250},{69:'E'})
    native_case('missing glyph callback is per-page refusal',missing,refusal='glyph/metric metadata')
    wrong=pdf(model['font'],b'/X Do',{},{})
    native_case('unsupported content is per-page refusal',wrong,refusal='operator')
    root=D['ENGINE'].parent
    native_case('mixed Base14 embedded page refuses atomically',(root/'fixtures/p0b/edit-coded.pdf').read_bytes(),refusal='shown font outside')
    report=dict(status='passed',positive_comparator_checks=baseline,rejected_comparator_mutations=len(mutations),mutations=mutations,native_wrapper_checks=len(checks),wrapper_checks=checks,sha256={str(v):hashlib.sha256(Path(v).read_bytes()).hexdigest() for v in [a.rust,a.native,__file__,Path(__file__).with_name('differential-page-glyph.py')]})
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k not in ('mutations','wrapper_checks','sha256')}))
if __name__=='__main__':main()
