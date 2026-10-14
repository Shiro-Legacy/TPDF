# SPDX-License-Identifier: AGPL-3.0-or-later
"""R22 generated original PDF object graphs, authored independently of execution."""
from copy import deepcopy
from fractions import Fraction as F
from text_position_model import number
from form_review_model import expand,Refusal

def ops(*items):return [dict(op=x[0],args=list(x[1:])) for x in items]
def literal(s):return b'('+s.encode('latin1').replace(b'\\',b'\\\\').replace(b'(',b'\\(').replace(b')',b'\\)')+b')'
def encode_ops(items):
    result=bytearray()
    for item in items:
        op=item['op'];args=item.get('args',[]);start=len(result);parts=[]
        for i,a in enumerate(args):
            if op in ('Tf','Do') and i==0:parts.append(b'/'+a.encode('ascii'))
            elif op=='Tj':parts.append(literal(a))
            elif isinstance(a,(list,tuple)):parts.append(('['+' '.join(map(number,a))+']').encode())
            else:parts.append(number(a).encode())
        result+=b' '.join(parts+[op.encode()]);item['span']=[start,len(result)];result+=b'\n'
    return bytes(result)
def pdf_objects(objects):
    out=bytearray(b'%PDF-1.7\n');offsets={}
    for obj,body in sorted(objects.items()):offsets[obj]=len(out);out+=f'{obj} 0 obj\n'.encode()+body+b'\nendobj\n'
    start=len(out);maximum=max(objects);out+=f'xref\n0 {maximum+1}\n0000000000 65535 f \n'.encode()
    for n in range(1,maximum+1):out+=(f'{offsets[n]:010d} 00000 n \n' if n in offsets else '0000000000 00000 f \n').encode()
    out+=f'trailer\n<< /Size {maximum+1} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n'.encode();return bytes(out)
def stream(data,extra=b''):return b'<< /Length '+str(len(data)).encode()+b' '+extra+b' >>\nstream\n'+data+b'\nendstream'
def encode(document):
    d=deepcopy(document);sources={};objects={1:b'<< /Type /Catalog /Pages 2 0 R >>',2:b'<< /Type /Pages /Count 1 /Kids [3 0 R] >>'}
    def resources(key,inline=False):
        s=d['scopes'][key]
        if s.get('resource_object') and not inline:return f'{s["resource_object"][0]} 0 R'.encode()
        parts=[]
        for cat,entries,table in [('Font',s.get('fonts',{}),d['fonts']),('XObject',s.get('forms',{}),d['forms'])]:
            parts.append('/'+cat+' << '+' '.join('/'+name+' '+str(table[target]['object'][0])+' 0 R' for name,target in entries.items())+' >>')
        return ('<< '+' '.join(parts)+' >>').encode()
    for key,scope in d['scopes'].items():
        if scope.get('resource_object'):
            obj=scope['resource_object'][0];body=resources(key,True)
            if obj in objects and objects[obj]!=body:raise ValueError('shared resource dictionary contents differ')
            objects[obj]=body
    for key,font in d['fonts'].items():
        widths=' '.join(number(font.get('widths',{}).get(str(i),font.get('width',500))) for i in range(32,91))
        objects[font['object'][0]]=f'<< /Type /Font /Subtype /Type1 /BaseFont /{font["face"]} /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 90 /Widths [{widths}] >>'.encode()
    for key,form in d['forms'].items():
        data=encode_ops(form['ops']);sources[tuple(form['object'])]=data
        extra='/Type /XObject /Subtype /Form /BBox ['+' '.join(map(number,form['bbox']))+'] /Matrix ['+' '.join(map(number,form.get('matrix',[1,0,0,1,0,0])))+']'
        extra=extra.encode()
        if form.get('resources') is not None:extra+=b' /Resources '+resources(form['resources'])
        elif form.get('resources_null'):extra+=b' /Resources null'
        extra+=form.get('extra',b'');objects[form['object'][0]]=stream(data,extra)
    data=encode_ops(d['page']['ops']);sources[(3,0)]=data;objects[4]=stream(data)
    page=d['page'];box=page.get('media_box',[0,0,32,32]);extra='/MediaBox ['+' '.join(map(number,box))+']'
    if page.get('crop_box'):extra+=' /CropBox ['+' '.join(map(number,page['crop_box']))+']'
    if page.get('rotate'):extra+=' /Rotate '+str(page['rotate'])
    if page.get('user_unit'):extra+=' /UserUnit '+number(page['user_unit'])
    res=b' /Resources '+resources(page['resources'])
    if page.get('resources_provider')==2:objects[2]=objects[2][:-2]+res+b' >>';res=b''
    objects[3]=b'<< /Type /Page /Parent 2 0 R /Contents 4 0 R'+res+b' '+extra.encode()+b' >>'
    return pdf_objects(objects),d,sources

def base():
    return dict(page=dict(object=[3,0],resources='p',matrix=[1,0,0,-1,0,32],ops=ops(('BT',),('Tf','F1',4),('ET',),('Do','X'),('BT',),('Tm',1,0,0,1,2,24),('Tj','AB'),('ET',))),scopes={'p':dict(fonts={'F1':'helv'},forms={'X':'x'}),'x':dict(fonts={'F1':'times'},forms={})},fonts={'helv':dict(object=[5,0],face='Helvetica',width=500),'times':dict(object=[6,0],face='Times-Roman',width=750)},forms={'x':dict(object=[7,0],resources='x',matrix=[1,0,0,1,4,4],bbox=[0,0,16,16],ops=ops(('re',0,0,8,8),('f',),('BT',),('Tm',1,0,0,1,1,9),('Tj','A'),('Tf','F1',5),('Tj','B'),('ET',)))})
def generated():
    cases=[]
    matrices=[[1,0,0,1,4,4],[1,F(1,2),F(1,4),1,2,3],[0,1,-1,0,20,2],[-1,0,0,1,20,3],[F(1,2),0,0,2,5,1],[1,-F(1,2),F(1,4),1,2,10]]
    for i,m in enumerate(matrices):
        for j,caller in enumerate(([1,0,0,1,0,0],[1,F(1,4),-F(1,2),1,4,2],[F(1,2),0,0,F(1,2),1,2])):
            d=base();d['page']['ops'].insert(0,dict(op='cm',args=caller));d['forms']['x']['matrix']=m
            cases.append((f'affine-{i}-{j}','affine-state',d,True))
    for i in range(8):
        d=base();f=d['forms']['x'];f['ops']=ops(('BT',),('Tm',1,0,0,1,1,2),('Tj','AB'),('ET',),('re',0,0,20,20),('f',))
        if i%2:f['resources']=None
        else:d['scopes']['x']['fonts']={}
        if i in (2,3):f['resources_null']=True;f['resources']=None
        if i>=4:
            d['page']['ops']=ops(('BT',),('Tf','F1',4),('Tc',F(1,4)),('Ts',F(1,2)),('Tz',75),('ET',),('g',0),('Do','X'),('g',1),('Do','X'),('BT',),('Tj','A'),('ET',))
        cases.append((f'inherited-{i}','inherited-state',d,True))
    for i in range(12):
        d=base();d['forms']['y']=dict(object=[8,0],resources=None,matrix=matrices[i%6],bbox=[0,0,8,8],ops=ops(('BT',),('Tf','F1',4),('Tm',1,0,0,1,0,2),('Tj','A'),('ET',)))
        d['scopes']['x']['forms']['Y']='y';d['scopes']['p']['forms']['Y']='y';d['forms']['x']['ops']=ops(('Do','Y'),('g',F(1,2)),('Do','Y'))
        d['page']['ops']=ops(('BT',),('Tf','F1',4),('ET',),('Do','X'),('Do','Y'),('BT',),('Tj','B'),('ET',))
        if i>=6:d['scopes']['y']=deepcopy(d['scopes']['x']);d['forms']['y']['resources']='y'
        cases.append((f'nested-{i}','nested-resource-reuse',d,True))
    for i in range(8):
        d=base();d['forms']['x']['ops']=ops(('q',),('re',1,1,6,6),('W',),('n',),('g',0),('re',0,0,20,20),('f',),('Q',),('RG',0,0,0),('w',1),('m',1,10),('l',12,10),('S',))
        d['forms']['x']['matrix']=matrices[i%6];d['page']['ops']=ops(('q',),('re',0,0,24,24),('W',),('n',),('Do','X'),('Q',),('re',26,26,3,3),('f',))
        cases.append((f'clip-{i}','clip-restore-stroke',d,True))
    for rotation in (0,90,180,270):
        for unit in (1,2):
            d=base();d['page'].update(media_box=[0,0,40,50],crop_box=[4,6,36,42],rotate=rotation,user_unit=unit)
            # Native page geometry independently follows PDF crop/rotation/UserUnit.
            transforms={0:[1,0,0,-1,-4,42],90:[0,1,1,0,-6,-4],180:[-1,0,0,1,36,-6],270:[0,-1,-1,0,42,36]}
            d['page']['matrix']=[v*unit for v in transforms[rotation]]
            d['forms']['x']['matrix']=[1,F(1,2),F(1,4),1,8,12]
            cases.append((f'page-geometry-{rotation}-{unit}','page-crop-rotation-userunit',d,True))
    for variant in range(4):
        d=base()
        if variant<2:d['page']['resources_provider']=2
        if variant%2:
            d['scopes']['p']['fonts']['Alias']='helv';d['page']['ops'].extend(ops(('BT',),('Tf','Alias',4),('Tj','A'),('ET',)))
        if variant>=2:
            d['forms']['y']=deepcopy(d['forms']['x']);d['forms']['y']['object']=[8,0];d['forms']['y']['resources']='y';d['scopes']['y']=deepcopy(d['scopes']['x'])
            d['scopes']['y']['resource_object']=[9,0];d['scopes']['x']['resource_object']=[9,0];d['scopes']['p']['forms']['Y']='y';d['page']['ops'].append(dict(op='Do',args=['Y']))
        cases.append((f'provider-alias-{variant}','provider-alias',d,True))
    # Native outer-resource fallback is deliberately broader than strict Rust scopes.
    for i in range(4):
        d=base();d['scopes']['x']['fonts']={};d['forms']['x']['ops']=ops(('BT',),('Tf','F1',4),('Tj','A'),('ET',))
        if i%2:d['scopes']['x']['forms']={};d['forms']['x']['ops']=ops(('Do','Y'));d['scopes']['p']['forms']['Y']='y';d['forms']['y']=dict(object=[8,0],resources=None,matrix=[1,0,0,1,0,0],bbox=[0,0,4,4],ops=ops(('re',0,0,2,2),('f',)))
        cases.append((f'strict-scope-{i}','strict-scope-native-fallback',d,False))
    for label,tail in [('underflow',ops(('Q',))),('open-save',ops(('q',))),('open-text',ops(('BT',))),('open-path',ops(('m',0,0))),('unsupported-tail',ops(('re',0,0,2,2),('f',),('gs','Bad')))]:
        d=base();d['forms']['x']['ops']=tail;cases.append((label,'structural-refusal',d,False))
    for label,change in [('cycle',None),('singular',[1,0,0,0,0,0]),('reversed-bbox','bbox'),('group','Group'),('oc','OC')]:
        d=base()
        if label=='cycle':d['scopes']['x']['forms']['X']='x';d['forms']['x']['ops']=ops(('Do','X'))
        elif label=='singular':d['forms']['x']['matrix']=change
        elif label=='reversed-bbox':d['forms']['x']['bbox']=[4,0,0,4]
        else:d['forms']['x']['extra']=(' /'+change+' << >>').encode()
        cases.append((label,'structural-refusal',d,False))
    for label,scope,d,accept in cases:
        # Extra metadata refusals are intentionally outside the geometry VM.
        if label=='unsupported-tail':d['forms']['x']['ops'][-1]={'op':'BI','args':[]}
        pdf,graph,sources=encode(d)
        expected=None
        if accept:expected=expand(graph)
        yield dict(label=label,scope=scope,pdf=pdf,graph=graph,sources=sources,expected=expected,accepted=accept)
