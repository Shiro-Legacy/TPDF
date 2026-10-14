#!/usr/bin/env python3
"""Separate readable page trees, render success, edit capability and save policy."""
import hashlib,json,os,pathlib,subprocess,sys,tempfile,time
cli,fixtures,output=map(pathlib.Path,sys.argv[1:]);cli=cli.resolve()
rows=[]
with tempfile.TemporaryDirectory(prefix='tpdf-view-') as scratch:
    for path in sorted(fixtures.rglob('*.pdf')):
        started=time.monotonic();env=dict(os.environ)
        r=subprocess.run([str(cli),'view-info',str(path)],capture_output=True,text=True,timeout=60)
        # The application's encrypted fixtures share one user password (tests/p2-encrypt.test.mjs).
        if r.returncode and r.stderr.startswith('needs-password'):
            env['TPDF_PDF_PASSWORD']='pdf4all-user';r=subprocess.run([str(cli),'view-info',str(path)],capture_output=True,text=True,timeout=60,env=env)
        row={'file':str(path.relative_to(fixtures)),'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'opened':r.returncode==0,'openSeconds':time.monotonic()-started}
        if r.returncode:row['error']=r.stderr.strip()
        else:
            caps=json.loads(r.stdout);row.update(caps)
            for page in row['pages']:
                if not page['canRender']:continue
                target=pathlib.Path(scratch)/'page.ppm';start=time.monotonic()
                try:
                    render=subprocess.run([str(cli),'render',str(path),str(page['page']),'72',str(target)],capture_output=True,text=True,timeout=60,env=env)
                    page['renderedAt72Dpi']=render.returncode==0
                    page['renderSecondsIncludingFreshOpen']=time.monotonic()-start
                    if render.returncode:page['renderFailureAt72Dpi']=render.stderr.strip()
                    else:page['ppmSha256']=hashlib.sha256(target.read_bytes()).hexdigest()
                except subprocess.TimeoutExpired:page['renderedAt72Dpi']=False;page['renderFailureAt72Dpi']='timeout after 60s'
                finally:target.unlink(missing_ok=True)
        rows.append(row)
report={'documents':len(rows),'openedPageTrees':sum(r['opened'] for r in rows),'allPagesRenderedAt72Dpi':sum(r['opened'] and all(p.get('renderedAt72Dpi',False) for p in r['pages']) for r in rows),'documentsWithEditableTargets':sum(r['opened'] and any(p['canEditText'] for p in r['pages']) for r in rows),'rows':rows}
output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k!='rows'}))
