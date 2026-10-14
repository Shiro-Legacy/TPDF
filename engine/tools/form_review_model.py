# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent exact state model for the narrow R22 Form execution contract.

No production parser/trace is consumed. The fixture author supplies original PDF
operations and scopes; optional spans are original source-local byte offsets.
Fractions preserve exact input decimals and matrix arithmetic. This is a bounded
reference for state/geometry/provenance, not a PDF parser or pixel renderer.
"""
from fractions import Fraction as F
from copy import deepcopy


class Refusal(ValueError):
    pass


def num(x):
    v = F(str(x)) if not isinstance(x, F) else x
    if max(v.numerator.bit_length(), v.denominator.bit_length()) > 2048:
        raise Refusal('rational size')
    return v


def mat(xs):
    if len(xs) != 6:
        raise Refusal('matrix length')
    return tuple(map(num, xs))


IDENTITY = mat([1, 0, 0, 1, 0, 0])


def mul(a, b):
    """Apply b then a, independently of the engine Affine implementation."""
    aa, ab, ac, ad, ae, af = a
    ba, bb, bc, bd, be, bf = b
    return mat([aa*ba+ac*bb, ab*ba+ad*bb, aa*bc+ac*bd,
                ab*bc+ad*bd, aa*be+ac*bf+ae, ab*be+ad*bf+af])


def rect(args):
    x, y, w, h = map(num, args)
    return [('m', (x, y)), ('l', (x+w, y)), ('l', (x+w, y+h)),
            ('l', (x, y+h)), ('h', ())]


def expand(document, limits=None):
    cap = dict(operations=10000, invocations=1000, depth=32, codes=100000,
               source_commands=100000, clips=10000)
    if limits:
        cap.update(limits)
    used = {key: 0 for key in cap}
    result = dict(invocations=[], origins=[], events=[], clips=[], codes=[], bindings=[], form_streams=[], usage=used)
    page = document['page']
    scopes, forms, fonts = (document[k] for k in ('scopes', 'forms', 'fonts'))
    state = dict(ctm=mat(page.get('matrix', IDENTITY)), clip=None,
                 fill=('gray', (F(0),)), stroke=('gray', (F(0),)), width=F(1),
                 cap=0, join=0, miter=F(10), dash=((), F(0)), font=None, binding=None,
                 size=F(0), char=F(0), word=F(0), scale=F(1), leading=F(0),
                 rise=F(0), render=0)
    stack, active = [], []
    path, pending = [], None
    in_text = False
    tm = lm = IDENTITY

    def charge(key, n=1):
        used[key] += n
        if used[key] > cap[key]:
            raise Refusal('limit ' + key)

    def origin(kind, inv, span, phase=None):
        charge('operations')
        value = dict(kind=kind, invocation=inv, span=tuple(span))
        if phase is not None:
            value['phase'] = phase
        result['origins'].append(value)
        return len(result['origins'])-1

    def event(kind, oi, **kw):
        result['events'].append(dict(kind=kind, operation=oi, **kw))

    def push_clip(commands, oi, rule='nonzero'):
        charge('clips')
        ci = len(result['clips'])
        result['clips'].append(dict(parent=state['clip'], ctm=state['ctm'],
                                    path=deepcopy(commands), rule=rule, operation=oi))
        state['clip'] = ci
        event('clip', oi, clip=ci)

    def flush(oi, paint=None, rule='nonzero'):
        nonlocal path, pending
        if paint:
            event(paint, oi, path=deepcopy(path), ctm=state['ctm'], clip=state['clip'],
                  color=state['fill' if paint == 'fill' else 'stroke'], rule=rule,
                  style=tuple(state[k] for k in ('width', 'cap', 'join', 'miter', 'dash')))
        # PDF painting uses the old clipping path; W takes effect at path end.
        if pending:
            push_clip(path, oi, pending)
        path, pending = [], None

    def run(ops, scope, inv):
        nonlocal state, path, pending, in_text, tm, lm
        floor = len(stack)
        for operation in ops:
            op, args = operation['op'], operation.get('args', [])
            span = operation.get('span', [0, 0])
            if op == 'Do':
                if in_text or path or pending:
                    raise Refusal('Do state')
                charge('invocations')
                if len(active) >= min(cap['depth'], 64):
                    raise Refusal('depth')
                try:
                    key = scopes[scope]['forms'][args[0]]
                    form = forms[key]
                except KeyError as e:
                    raise Refusal('missing Form') from e
                obj = tuple(form['object'])
                if obj in active:
                    raise Refusal('cycle')
                fm = mat(form.get('matrix', IDENTITY))
                if fm[0]*fm[3] == fm[1]*fm[2]:
                    raise Refusal('singular Form')
                bbox = tuple(map(num, form['bbox']))
                x0, y0, x1, y1 = bbox
                if x0 >= x1 or y0 >= y1:
                    raise Refusal('BBox')
                effective = form.get('resources')
                if effective is None:
                    effective = scope
                call = dict(kind='page' if inv is None else 'form', invocation=inv,
                            span=tuple(span))
                child = len(result['invocations'])
                entry = origin('boundary', child, [0, 0], 'enter')
                saved = deepcopy(state)
                state['ctm'] = mul(state['ctm'], fm)
                result['invocations'].append(dict(object=obj, parent=inv, scope=effective,
                    matrix=fm, bbox=bbox, ctm=state['ctm'], caller_scope=scope, call=call, entry=entry))
                if obj not in result['form_streams']:
                    result['form_streams'].append(obj)
                push_clip(rect([x0, y0, x1-x0, y1-y0]), entry)
                active.append(obj)
                used['depth'] = max(used['depth'], len(active))
                run(form['ops'], effective, child)
                active.pop()
                if in_text or path or pending:
                    raise Refusal('Form unfinished state')
                exit_index = origin('boundary', child, [0, 0], 'exit')
                state = saved
                event('restore', exit_index, clip=state['clip'])
                result['invocations'][child]['exit'] = exit_index
                continue
            oi = origin('page' if inv is None else 'form', inv, span)
            if op in ('q', 'Q', 'cm') and in_text:
                raise Refusal('restricted text state')
            if op == 'q':
                stack.append(deepcopy(state))
            elif op == 'Q':
                if len(stack) <= floor:
                    raise Refusal('save floor')
                oldclip = state['clip']
                state = stack.pop()
                if oldclip != state['clip']:
                    event('restore', oi, clip=state['clip'])
            elif op == 'cm':
                state['ctm'] = mul(state['ctm'], mat(args))
            elif op in ('g', 'G', 'rg', 'RG'):
                state['fill' if op.islower() else 'stroke'] = (
                    'gray' if op.lower() == 'g' else 'rgb', tuple(map(num, args)))
            elif op in ('w', 'J', 'j', 'M'):
                state[dict(w='width', J='cap', j='join', M='miter')[op]] = num(args[0])
            elif op == 'd':
                state['dash'] = (tuple(map(num, args[0])), num(args[1]))
            elif op in ('m', 'l', 'c', 'h', 're'):
                if in_text:
                    raise Refusal('path in text')
                additions = rect(args) if op == 're' else [(op, tuple(map(num, args)))]
                charge('source_commands', len(additions))
                path.extend(additions)
            elif op in ('W', 'W*'):
                pending = 'evenodd' if op == 'W*' else 'nonzero'
            elif op in ('f', 'F', 'f*', 'S', 'n'):
                flush(oi, None if op == 'n' else ('stroke' if op == 'S' else 'fill'),
                      'evenodd' if op == 'f*' else 'nonzero')
            elif op == 'BT':
                if in_text or path or pending:
                    raise Refusal('BT state')
                in_text, tm, lm = True, IDENTITY, IDENTITY
            elif op == 'ET':
                if not in_text:
                    raise Refusal('ET state')
                in_text = False
            elif op == 'Tf':
                try:
                    state['font'] = scopes[scope]['fonts'][args[0]]
                    fonts[state['font']]
                except KeyError as e:
                    raise Refusal('missing font') from e
                binding = dict(scope=scope, name=args[0], font=state['font'], object=tuple(fonts[state['font']]['object']))
                if binding not in result['bindings']:
                    result['bindings'].append(binding)
                state['binding'] = result['bindings'].index(binding)
                state['size'] = num(args[1])
            elif op in ('Tc', 'Tw', 'Tz', 'TL', 'Ts', 'Tr'):
                key = dict(Tc='char', Tw='word', Tz='scale', TL='leading', Ts='rise', Tr='render')[op]
                state[key] = num(args[0]) / (100 if op == 'Tz' else 1)
            elif op == 'Tm':
                tm = lm = mat(args)
            elif op in ('Td', 'TD', 'T*'):
                tx, ty = (F(0), -state['leading']) if op == 'T*' else map(num, args)
                if op == 'TD':
                    state['leading'] = -ty
                tm = lm = mul(lm, mat([1, 0, 0, 1, tx, ty]))
            elif op == 'Tj':
                if not in_text or state['font'] is None:
                    raise Refusal('show state')
                font = fonts[state['font']]
                for byte in args[0].encode('latin1'):
                    charge('codes')
                    width = num(font.get('widths', {}).get(str(byte), font.get('width', 0)))
                    scale, size = state['scale'], state['size']
                    trm = mul(state['ctm'], mul(tm, mat([size*scale, 0, 0, size, 0, state['rise']])))
                    code = dict(code=byte, font=state['font'], binding=state['binding'], object=tuple(font['object']),
                        unicode=font.get('unicode', {}).get(str(byte), chr(byte)), width=width,
                        transform=trm, clip=state['clip'], color=state['fill'], render=state['render'],
                        operation=oi, invocation=inv)
                    result['codes'].append(code)
                    event('text', oi, code=len(result['codes'])-1)
                    advance=(width*size/1000 + state['char'] + (state['word'] if byte == 32 else 0))*scale
                    tm=mul(tm, mat([1, 0, 0, 1, advance, 0]))
            else:
                raise Refusal('unsupported model operator ' + op)
        if len(stack) != floor:
            raise Refusal('unbalanced saves')
    run(page['ops'], page['resources'], None)
    if in_text or path or pending:
        raise Refusal('unfinished page')
    return result


def selfcheck():
    def ops(*items):
        return [dict(op=x[0], args=list(x[1:])) for x in items]
    d=dict(page=dict(object=[3,0], resources='p', matrix=[1,0,0,-1,0,100],
                    ops=ops(('cm',2,1,1,3,10,20),('BT',),('Tf','F',10),('ET',),
                            ('Do','X'),('BT',),('Tj','A'),('ET',))),
           scopes={'p':dict(fonts={'F':'a'}, forms={'X':'x'}),
                   'x':dict(fonts={'F':'b'}, forms={})},
           fonts={'a':dict(object=[5,0],width=500),
                  'b':dict(object=[6,0],width=700,unicode={'65':'B'})},
           forms={'x':dict(object=[7,0],resources='x',matrix=[1,2,3,1,4,5],bbox=[1,2,5,7],
                          ops=ops(('BT',),('Tj','A'),('Tf','F',20),('Tj','A'),('ET',)))})
    r=expand(d)
    assert r['invocations'][0]['ctm']==mat([4,-7,7,-6,23,61])
    assert [c['unicode'] for c in r['codes']]==['A','B','A']
    assert [c['width'] for c in r['codes']]==[500,700,500]
    assert [c['clip'] for c in r['codes']]==[0,0,None]
    assert r['codes'][2]['transform']==mat([20,-10,10,-30,10,80])
    for tail in [ops(('Q',)),ops(('q',)),ops(('BT',)),ops(('m',0,0))]:
        bad=deepcopy(d);bad['forms']['x']['ops']=tail
        try: expand(bad)
        except Refusal: pass
        else: raise AssertionError('bad frame accepted')
    for key in ('operations','invocations','depth','codes','clips'):
        try: expand(d,{key:0})
        except Refusal: pass
        else: raise AssertionError('zero budget accepted '+key)
    # The same resource-less stream must use each current caller's scope.
    repeat=deepcopy(d)
    repeat['page']['ops']=ops(('Do','A'),('Do','B'),('Do','A'))
    repeat['scopes']['p']['forms']={'A':'outera','B':'outerb'}
    repeat['scopes']['sa']={'fonts':{'F':'a'},'forms':{'Leaf':'leaf'}}
    repeat['scopes']['sb']={'fonts':{'F':'b'},'forms':{'Leaf':'leaf'}}
    repeat['forms']={
      'outera':dict(object=[7,0],resources='sa',bbox=[0,0,10,10],ops=ops(('Do','Leaf'))),
      'outerb':dict(object=[8,0],resources='sb',bbox=[0,0,10,10],ops=ops(('Do','Leaf'))),
      'leaf':dict(object=[9,0],bbox=[0,0,10,10],ops=ops(('BT',),('Tf','F',10),('Tj','A'),('ET',)))}
    rr=expand(repeat)
    assert [c['unicode'] for c in rr['codes']]==['A','B','A']
    assert [i['scope'] for i in rr['invocations']]==['sa','sa','sb','sb','sa','sa']
    assert [i['parent'] for i in rr['invocations']]==[None,0,None,2,None,4]
    assert len(rr['form_streams'])==3 and len(rr['invocations'])==6
    assert len(rr['bindings'])==2
    assert [c['binding'] for c in rr['codes']]==[0,1,0]
    # Explicit empty scope refuses Tf but cannot erase a selected inherited font.
    empty=deepcopy(d);empty['scopes']['x']={'fonts':{},'forms':{}}
    try: expand(empty)
    except Refusal: pass
    else: raise AssertionError('empty scope merged parent')
    empty['forms']['x']['ops']=ops(('BT',),('Tj','A'),('ET',))
    assert [c['unicode'] for c in expand(empty)['codes']]==['A','A']
    cycle=deepcopy(repeat);cycle['forms']['leaf']['ops']=ops(('Do','Leaf'))
    try: expand(cycle)
    except Refusal as e: assert str(e)=='cycle'
    else: raise AssertionError('active cycle admitted')
    # Exact and one-less limits operate on replay work, not unique cached streams.
    for key in ('operations','invocations','depth','codes','clips','source_commands'):
        count=rr['usage'][key]
        expand(repeat,{key:count})
        if count:
            try: expand(repeat,{key:count-1})
            except Refusal: pass
            else: raise AssertionError('one-less budget admitted '+key)
    assert rr['origins'][0]==dict(kind='boundary',invocation=0,span=(0,0),phase='enter')
    assert rr['origins'][rr['invocations'][1]['exit']]['phase']=='exit'
    # This selfcheck's show follows BT/Tf, so its source origin is child-local.
    assert rr['origins'][rr['codes'][0]['operation']]['invocation']==1
    return {'status':'passed','identities':15,'frame_refusals':4,'zero_budget_refusals':5,
            'scope_cycle_refusals':2,'exact_limit_checks':6,'one_less_limit_checks':5}


if __name__=='__main__':
    import json
    print(json.dumps(selfcheck(),sort_keys=True))
