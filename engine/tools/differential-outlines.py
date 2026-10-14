#!/usr/bin/env python3
"""Deterministic exact O0 comparison; malformed recovery is never a parity gate."""
import argparse,hashlib,json,pathlib,random,struct,subprocess,tempfile,time
from outline_model import cases
ROOT=pathlib.Path(__file__).resolve().parents[1]
def sha(b):return hashlib.sha256(b).hexdigest()
def run(argv,data):
 p=subprocess.run([str(x) for x in argv],input=data,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=8)
 assert p.returncode==0,(argv,p.returncode,p.stderr[:1000].decode(errors='replace'))
 assert not p.stderr,(argv,p.stderr[:1000])
 return json.loads(p.stdout)
def match(actual,expected,fields):
 assert actual['units_per_em']==expected['units_per_em']
 assert actual['glyph_count']==expected['glyph_count']
 assert len(actual['glyphs'])==len(expected['glyphs'])
 for a,e in zip(actual['glyphs'],expected['glyphs']):
  for key in fields:
   assert a[key]==e[key],(a['gid'],key,a[key],e[key])
def main():
 p=argparse.ArgumentParser();p.add_argument('--rust',default=str(ROOT/'target/debug/pdf-inspect'));p.add_argument('--native',default='/private/tmp/tandem-mupdf-reference/mupdf-outline-oracle');p.add_argument('--report',default=str(ROOT/'reference/outline-differential-o0.json'));p.add_argument('--native-only',action='store_true');args=p.parse_args()
 paths=[pathlib.Path(args.native)]+([] if args.native_only else [pathlib.Path(args.rust)])
 hashes={str(x):sha(x.read_bytes()) for x in paths};start=time.monotonic();glyphs=points=cmds=0;generated=[]
 fields=['gid','kind','range','points','contour_endpoints','commands']
 for i,(data,expected) in enumerate(cases()):
  try:
   native=run([args.native,'all'],data);match(native,expected,fields)
   for a,e in zip(native['glyphs'],expected['glyphs']):
    assert a['bearing_adjustment']==(0 if e['kind']=='empty_loca' else e['bearing_adjustment'])
   if not args.native_only:
    rust=run([args.rust,'--outline=all'],data);match(rust,expected,fields+['instruction_bytes','overlap'])
    for a,e in zip(rust['glyphs'],expected['glyphs']):
     assert a['header_bounds']==e['bounds']
     assert a['control_bounds']==(e['bounds'] if e['points'] else None)
    for t in rust['tables']:
     tag=bytes(t['tag_bytes']).decode();assert [t['offset'],t['length']]==expected['tables'][tag]
     payload=data[t['offset']:t['offset']+t['length']];payload+=bytes((-len(payload))%4)
     assert t['checksum']==sum(struct.unpack('>'+'I'*(len(payload)//4),payload))&0xffffffff
    usage=rust['outline_usage'];gs=expected['glyphs']
    expected_usage={'decode_attempts':len(gs),'glyph_bytes':sum(g['range'][1]-g['range'][0] for g in gs),'contours':sum(len(g['contour_endpoints']) for g in gs),'points':sum(len(g['points']) for g in gs),'commands':sum(len(g['commands']) for g in gs)}
    expected_usage['work']=expected_usage['decode_attempts']+expected_usage['glyph_bytes']+expected_usage['contours']+4*expected_usage['points']+expected_usage['commands']
    for k,v in expected_usage.items():assert usage[k]==v,(k,usage[k],v)
   glyphs+=len(expected['glyphs']);points+=sum(len(g['points']) for g in expected['glyphs']);cmds+=sum(len(g['commands']) for g in expected['glyphs']);generated.append(sha(data))
  except Exception as e:raise AssertionError(('generated',i,str(e))) from e
 fixture_rows=[];inventory=json.loads((ROOT/'reference/embedded-outline-inventory.json').read_text())
 for i,f in enumerate(inventory['fonts']):
  original=(ROOT.parent/'fixtures'/f['file']).read_bytes();assert sha(original)==f['fixtureSha256']
  data=pathlib.Path(f'/private/tmp/outline-fonts/font{i}.ttf').read_bytes();assert sha(data)==f['programs'][0]['sha256']
  native=run([args.native,'all'],data)
  if not args.native_only:match(run([args.rust,'--outline=all'],data),native,fields)
  fixture_rows.append({'file':f['file'],'font':f['font'],'sha256':sha(data),'glyphs':len(native['glyphs']),'simple':sum(g['kind']=='simple' for g in native['glyphs'])})
 refusals=0
 if not args.native_only:
  data,expected=next(cases());rec=expected['tables'];base=bytearray(data)
  mutations=[]
  def patch(at,raw):b=bytearray(base);b[at:at+len(raw)]=raw;mutations.append(bytes(b))
  patch(0,b'OTTO');patch(4,b'\x00\x00');patch(rec['head'][0]+12,b'\0\0\0\0');patch(rec['head'][0]+18,b'\0\x0f');patch(rec['head'][0]+50,b'\0\x02');patch(rec['maxp'][0]+4,b'\0\0')
  # Directory aliases, out-of-bounds range and overlap.
  patch(12+16,b'glyf');patch(20,struct.pack('>I',len(data)+1));patch(24,struct.pack('>I',0xffffffff))
  g=expected['glyphs'][2]['range'][0];patch(g,b'\xff\xff');patch(g+2,struct.pack('>h',2));patch(g+14,b'\x80');patch(g+14,b'\x09\xff')
  # Every proper prefix refuses; no partial JSON even after earlier valid glyphs.
  mutations.extend(data[:n] for n in sorted(set(range(0,len(data),max(1,len(data)//80)))|{len(data)-1}))
  for bad in mutations:
   r=subprocess.run([args.rust,'--outline=all'],input=bad,capture_output=True,timeout=8)
   assert r.returncode!=0 and not r.stdout,(len(bad),r.returncode,r.stdout[:100]);assert b'panicked' not in r.stderr
   refusals+=1
  with tempfile.TemporaryDirectory() as td:
   query=pathlib.Path(td)/'queries'
   for raw in (b'-1',b'+1',b'1 2',b'0\n999999',b'4294967296',b'\xff',b'1.0'):
    query.write_bytes(raw);r=subprocess.run([args.rust,f'--outline={query}'],input=data,capture_output=True,timeout=8);assert r.returncode!=0 and not r.stdout;refusals+=1
   query.write_bytes(b'2\n2\n0\n3');a=run([args.rust,f'--outline={query}'],data);assert [g['gid'] for g in a['glyphs']]==[2,2,0,3]
 assert hashes=={str(x):sha(x.read_bytes()) for x in paths},'binary changed during gate'
 report={'stage':'O0','seed':0x0150A11,'generated_fonts':len(generated),'generated_glyphs':glyphs,'generated_points':points,'generated_commands':cmds,'generated_sha256_aggregate':sha(''.join(generated).encode()),'fixtures':fixture_rows,'fixture_glyphs':sum(f['glyphs'] for f in fixture_rows),'strict_atomic_refusals':refusals,'binary_sha256':hashes,'native_only':args.native_only,'comparison':'Exact integers and booleans; independent cyclic implicit-point expansion; no tolerance. hmtx bearings vary independently of glyph xMin. No malformed recovery parity.','seconds':round(time.monotonic()-start,3)}
 pathlib.Path(args.report).write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
if __name__=='__main__':main()
