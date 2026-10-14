#!/usr/bin/env python3
"""Test-only pinned-source instrumentation, without a custom unsafe allocator.
Copies dependencies into a temporary directory and adds capacity diagnostics.
Does not modify Cargo.lock, engine code, or the user's registry. Report measures
requested Vec capacities; process RSS/allocator overhead and OOM are not proved.
"""
import argparse, hashlib, json, os, pathlib, re, shutil, subprocess, tempfile
p=argparse.ArgumentParser();p.add_argument('--output',required=True);a=p.parse_args()
engine=pathlib.Path(__file__).resolve().parent.parent
cargo=shutil.which('cargo') or str(pathlib.Path.home()/'.cargo/bin/cargo')
meta=json.loads(subprocess.check_output([cargo,'metadata','--offline','--features','raster','--format-version','1'],cwd=engine))
packages={p['name']:p for p in meta['packages']}
root=pathlib.Path(tempfile.mkdtemp(prefix='tandem-raster-allocation-',dir='/private/tmp'))
def patch(package,file,before,after):
 path=root/package/file;source=path.read_text();assert source.count(before)==1,(file,before);path.write_text(source.replace(before,after))
for name in ['tiny-skia','tiny-skia-path']:
 assert packages[name]['version']=='0.12.0';shutil.copytree(pathlib.Path(packages[name]['manifest_path']).parent,root/name)
patch('tiny-skia','src/scan/path.rs','    start_y <<= shift_edges_up;', '    std::eprintln!("AUDIT edge {} {} {}", edges.len(), edges.capacity(), core::mem::size_of::<Edge>());\n    start_y <<= shift_edges_up;')
patch('tiny-skia-path','src/path_builder.rs','        Some(Path {','        std::eprintln!("AUDIT path {} {} {} {}", self.verbs.capacity(), core::mem::size_of::<PathVerb>(), self.points.capacity(), core::mem::size_of::<Point>());\n        Some(Path {')
(root/'src').mkdir()
(root/'Cargo.toml').write_text('[package]\nname="raster-allocation-audit"\nversion="0.0.0"\nedition="2024"\n[dependencies]\ntandem-pdf-engine={path='+json.dumps(str(engine))+',features=["raster"]}\n[patch.crates-io]\ntiny-skia={path="tiny-skia"}\ntiny-skia-path={path="tiny-skia-path"}\n')
(root/'src/main.rs').write_text('''#![forbid(unsafe_code)]
use tandem_pdf_engine::{content::ContentLimits,path::*,raster::*,geometry::Affine};
fn main(){
 let mut inputs=vec!["1 1 4000 1000 re f".to_string(),"1 1 m 3000 1 l 3000 500 l 1000 500 l 1000 900 l 1 900 l f".to_string()];
 let mut many=String::new();for i in 0..200{many.push_str(&format!("{} 1 1 1000 re ",i*2+1));}many.push('f');inputs.push(many);
 let mut clips=String::new();for _ in 0..4{clips.push_str("1 1 100 100 re W n ");}clips.push_str("0 0 128 128 re f");inputs.push(clips);
 for (i,source) in inputs.iter().enumerate(){eprintln!("AUDIT case {i}");let program=interpret_path_content(source.as_bytes(),PathInitialState::default(),ContentLimits::default(),PathLimits::default()).unwrap();let target=RasterTarget{width:if i==3{128}else{4096},height:if i==3{128}else{1024},page_to_device:Affine::from_pdf([1.,0.,0.,1.,0.,0.]).unwrap()};let target=if i<3{RasterTarget{width:4096,height:1024,..target}}else{target};let plan=prepare_orthogonal_raster(&program,target,RasterLimits{max_work:u64::MAX,..Default::default()}).unwrap();let image=plan.render().unwrap();eprintln!("AUDIT done {} {}",image.pixels().len(),plan.usage().planned_storage_bytes);}
}
''')
run=subprocess.run([cargo,'run','--offline','--quiet','--manifest-path',str(root/'Cargo.toml')],cwd=engine,capture_output=True,text=True,timeout=180)
if run.returncode:raise RuntimeError(run.stderr)
records=[];case=None
for line in run.stderr.splitlines():
 if line.startswith('AUDIT case '):case=int(line.split()[-1])
 elif line.startswith('AUDIT edge '):
  n,cap,size=map(int,line.split()[2:]);records.append({'case':case,'kind':'edge','length':n,'capacity':cap,'elementBytes':size,'capacityBytes':cap*size,'conservativeSortScratchBytes':n*size})
 elif line.startswith('AUDIT path '):
  vc,vs,pc,ps=map(int,line.split()[2:]);records.append({'case':case,'kind':'path','verbCapacity':vc,'pointCapacity':pc,'capacityBytes':vc*vs+pc*ps})
assert records
maximum=max((r['capacityBytes']+r.get('conservativeSortScratchBytes',0)) for r in records)
assert maximum<1024*1024
report={'tool':'safe source-capacity instrumentation of isolated dependency copies','compiler':subprocess.check_output([str(pathlib.Path(cargo).parent/'rustc'),'--version'],text=True).strip(),'cases':4,'records':records,'maximumSingleReportedAllocationPlusSortAllowance':maximum,'adapterBackendAllowanceBytes':1024*1024,'limitations':['Reports requested Vec capacities, not allocator metadata or exact process peak.','Sort scratch conservatively charged as a full edge slice; backend sort implementation is in Rust std.','Infallible backend allocations remain subject to process OOM.','No engine or original registry sources were modified.'],'temporaryDirectory':str(root)}
pathlib.Path(a.output).write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({'cases':4,'records':len(records),'maximumBytes':maximum,'output':a.output}))
