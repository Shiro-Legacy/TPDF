# SPDX-License-Identifier: AGPL-3.0-or-later
"""R22 independent Form model/native/Rust comparison adapters.

Native numeric policies reuse accepted P0/T0/G1 gates. Generated exact dyadic
models must equal binary64 Rust coordinates exactly; native f32 stays bounded.
"""
import copy,hashlib,runpy
from pathlib import Path
from fractions import Fraction as F
from text_position_model import json_numbers
ENGINE=Path(__file__).resolve().parent.parent
OUT=runpy.run_path(str(ENGINE/'tools/differential-page-outlines.py'))
PP=OUT['PP']
def sha(data):return hashlib.sha256(data).hexdigest()
def chain(clip,clips):
    out=[]
    while clip is not None:
        if len(out)>len(clips):raise ValueError('clip cycle')
        out.append(clip);clip=clips[clip]['parent']
    return list(reversed(out))
def color(c):return dict(space=c[0],values=list(c[1]))
def path(p):return PP['normalized_path']([[op,*values] for op,values in p])
def model_semantics(model):
    out=[];current=None
    for event in model['events']:
        kind=event['kind'];ids=chain(current,model['clips'])
        if kind=='restore':
            while current!=event['clip']:
                if current is None:raise ValueError('model invalid restore')
                current=model['clips'][current]['parent'];out.append(dict(kind='pop',clip_ids=chain(current,model['clips'])))
            continue
        if kind=='clip':
            clip=model['clips'][event['clip']]
            if clip['parent']!=current:raise ValueError('model clip parent')
            out.append(dict(kind='clip',path=path(clip['path']),matrix=list(clip['ctm']),rule=clip['rule'],clip_ids=ids));current=event['clip'];continue
        if kind=='text':
            c=model['codes'][event['code']]
            if c['clip']!=current:raise ValueError('model text clip')
            out.append(dict(kind='code',code=c['code'],unicode=list(map(ord,c['unicode'])),matrix=list(c['transform']),advance=c['width']/1000,mode=c['render'],clip_ids=ids,color=color(c['color']) if c['render']==0 else None));continue
        if event['clip']!=current:raise ValueError('model paint clip')
        value=dict(kind=kind,path=path(event['path']),matrix=list(event['ctm']),color=color(event['color']),clip_ids=ids)
        if kind=='fill':value['rule']=event['rule']
        else:
            width,cap,join,miter,(dash,phase)=event['style'];value['style']=dict(width=width,start_cap=cap,dash_cap=cap,end_cap=cap,join=join,miter=miter,phase=phase,dash=list(dash))
        out.append(value)
    return json_numbers(out)
def native_model(n,case):
    if n['status']!='accepted':raise ValueError('native positive refusal')
    model=case['expected'];graph=case['graph'];stats=dict(numeric_nonexact_fields=0,max_absolute_difference=0)
    expected=model_semantics(model);actual=PP['native_semantics'](OUT['native_program'](n));PP['compare'](actual,expected,stats,'model/events')
    PP['compare'](n['initial_ctm'],json_numbers(graph['page']['matrix']),stats,'initial_ctm')
    invocations=n['form_invocations']
    if len(invocations)!=len(model['invocations']):raise ValueError('native invocation count')
    providers={graph['page']['resources']:graph['page'].get('resources_provider',graph['page']['object'][0])}
    for f in graph['forms'].values():
        if f.get('resources') is not None:providers[f['resources']]=f['object'][0]
    for index,(actual,want) in enumerate(zip(invocations,model['invocations'])):
        for key,value in dict(id=index,parent=want['parent'],object=list(want['object']),caller_provider=providers[want['caller_scope']],effective_provider=providers[want['scope']],source_sha256=sha(case['sources'][tuple(want['object'])])).items():
            exact(actual[key],value,'native invocation '+key)
        for key in ('matrix','bbox','ctm'):PP['compare'](actual[key],json_numbers(want[key]),stats,'invocation/'+key)
        lo,hi=actual['event_range']
        if not(0<=lo<hi<=len(n['events'])) or n['events'][lo]['kind']!='clip' or n['events'][hi-1]['kind']!='pop':raise ValueError('native boundary event range')
    if len(n['operation_origins'])!=len(model['origins']):raise ValueError('native source origin count')
    for a,b in zip(n['operation_origins'],model['origins']):
        if a['invocation']!=b['invocation']:raise ValueError('native origin invocation')
        if b['kind']=='boundary':
            if a['phase']!=(1 if b['phase']=='enter' else 2) or a['span']!=list(model['invocations'][b['invocation']]['call']['span']):raise ValueError('native boundary origin')
        elif a['phase'] or a['span']!=list(b['span']):raise ValueError('native operation span')
    nativecodes=[e for e in n['events'] if e['kind']=='glyph']
    if len(nativecodes)!=len(model['codes']):raise ValueError('native glyph count')
    for a,b in zip(nativecodes,model['codes']):
        if a['font']!=list(b['object']):raise ValueError('native source font identity')
    return dict(**stats,invocations=len(invocations),events=len(actual) if isinstance(actual,list) else len(n['events']),codes=len(nativecodes))

def exact(actual,expected,label):
    want=json_numbers(expected)
    def same(a,b):
        if isinstance(b,bool) or b is None:return type(a) is type(b) and a==b
        if isinstance(b,(int,float)) and not isinstance(b,bool):return isinstance(a,(int,float)) and not isinstance(a,bool) and a==b
        if isinstance(b,list):return isinstance(a,list) and len(a)==len(b) and all(same(x,y) for x,y in zip(a,b))
        if isinstance(b,dict):return isinstance(a,dict) and set(a)==set(b) and all(same(a[k],v) for k,v in b.items())
        return type(a) is type(b) and a==b
    if not same(actual,want):raise ValueError(label+': exact rational result differs')
def rust_model(r,case):
    model=case['expected'];program=r['trace']['program'];actual,_=PP['rust_semantics'](OUT['compact'](program));expected=model_semantics(model)
    exact(actual,expected,'Rust/model ordered state and geometry')
    codes=program['text']['codes']
    if len(codes)!=len(model['codes']):raise ValueError('Rust/model codes')
    for c,w in zip(codes,model['codes']):
        if c['operation_index']!=w['operation']:raise ValueError('code operation provenance')
    return dict(model_exact_events=len(expected),model_exact_codes=len(codes))

def model_origin(origin):
    if origin['kind']=='boundary':return dict(kind='boundary',invocation=origin['invocation'],phase='enter' if origin['phase']=='enter' else 'leave')
    result=dict(kind=origin['kind'],span=list(origin['span']))
    if origin['invocation'] is not None:result['invocation']=origin['invocation']
    return result

def provenance(r,n,case=None):
    trace=r['trace'];origins=trace['operation_origins'];native=n['operation_origins'];inv=trace['form_invocations'];streams=trace['form_streams'];scopes=trace['resource_scopes']
    if [v['id'] for v in streams]!=list(range(len(streams))) or [v['id'] for v in scopes]!=list(range(len(scopes))):raise ValueError('source/scope IDs')
    if len(origins)!=len(native) or len(inv)!=len(n['form_invocations']):raise ValueError('invocation/origin counts')
    by_object={tuple(v['object']):v for v in streams}
    if len(by_object)!=len(streams) or set(by_object)!={tuple(v['object']) for v in inv}:raise ValueError('unique retained Form streams')
    enter={};leave={}
    for i,(a,b) in enumerate(zip(origins,native)):
        if b['phase']:
            phase='enter' if b['phase']==1 else 'leave';want=dict(kind='boundary',invocation=b['invocation'],phase=phase)
            (enter if b['phase']==1 else leave)[b['invocation']]=i
        else:
            want=dict(kind='page' if b['invocation'] is None else 'form',span=b['span'])
            if b['invocation'] is not None:want['invocation']=b['invocation']
        exact(a,want,f'operation origin {i}')
        if not b['phase']:
            data=bytes.fromhex(trace['content_hex']) if b['invocation'] is None else bytes.fromhex(by_object[tuple(inv[b['invocation']]['object'])]['content_hex'])
            lo,hi=b['span']
            if not 0<=lo<hi<=len(data) or not data[lo:hi].endswith(b['op'].encode()):raise ValueError('source-local operation bounds/operator')
    for i,(a,b) in enumerate(zip(inv,n['form_invocations'])):
        sid=a['stream_id'];stream=streams[sid]
        if stream['object']!=a['object'] or sha(bytes.fromhex(stream['content_hex']))!=b['source_sha256']:raise ValueError('retained Form source bytes')
        for k in ('id','parent','object','matrix','bbox'):
            exact(a[k],b[k],'invocation '+k)
        for rust_key,native_key in [('caller_scope','caller_provider'),('effective_scope','effective_provider')]:
            if scopes[a[rust_key]]['provider']!=[b[native_key],0]:raise ValueError('invocation scope provider')
        if a['execution_range']!=[enter[i],leave[i]+1]:raise ValueError('Form execution range')
        src=native[enter[i]];want=dict(kind='page' if b['parent'] is None else 'form',span=src['span'])
        if b['parent'] is not None:want['invocation']=b['parent']
        if a['call_origin']!=want:raise ValueError('Form call origin')
    if case:
        exact(origins,[model_origin(v) for v in case['expected']['origins']],'independent source origins')
        if bytes.fromhex(trace['content_hex'])!=case['sources'][(3,0)]:raise ValueError('root source bytes')
        providers={case['graph']['page']['resources']:case['graph']['page'].get('resources_provider',3)}
        for f in case['graph']['forms'].values():
            if f.get('resources') is not None:providers[f['resources']]=f['object'][0]
        bindings=case['expected']['bindings']
        if len(trace['fonts'])!=len(bindings):raise ValueError('scoped font binding count')
        for f,b in zip(trace['fonts'],bindings):
            if f['identity']!={'indirect':list(b['object'])} or bytes.fromhex(f['name_hex']).decode()!=b['name'] or scopes[f['scope_id']]['provider']!=[providers[b['scope']],0]:raise ValueError('scoped font binding identity')
        for c,m in zip(trace['program']['text']['codes'],case['expected']['codes']):
            if c['font_id']!=m['binding']:raise ValueError('selected inherited font ID')
    return dict(invocations=len(inv),retained_streams=len(streams),resource_scopes=len(scopes),origins=len(origins))
