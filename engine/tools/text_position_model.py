#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent exact-rational text-state model for the T0 review gate.

No engine parser, matrix implementation, or output is used to derive expectations.
Matrices use column-vector coordinates. compose(A,B) means apply A, then B.
"""
from fractions import Fraction as F
import copy
import random

IDENTITY = tuple(map(F, (1, 0, 0, 1, 0, 0)))


def compose(first, second):
    a,b,c,d,e,f = first
    g,h,i,j,k,l = second
    return (g*a+i*b, h*a+j*b, g*c+i*d, h*c+j*d, g*e+i*f+k, h*e+j*f+l)


def translation(x, y):
    return (F(1), F(0), F(0), F(1), F(x), F(y))


def number(value):
    value = F(value)
    # All generated denominators contain only factors two and five.
    n, d = value.numerator, value.denominator
    sign = '-' if n < 0 else ''
    n = abs(n)
    whole, rem = divmod(n, d)
    if not rem:
        return sign + str(whole)
    digits = ''
    for _ in range(32):
        rem *= 10
        q, rem = divmod(rem, d)
        digits += str(q)
        if not rem:
            return sign + str(whole) + '.' + digits
    raise ValueError('nonterminating generated decimal')


def operand(value):
    if isinstance(value, bytes):
        return '<' + value.hex() + '>'
    if isinstance(value, str):
        return '/' + value
    if isinstance(value, list):
        return '[' + ' '.join(map(operand, value)) + ']'
    return number(value)


def content(operations):
    output = bytearray()
    spans = []
    for operator, args in operations:
        start = len(output)
        output.extend((' '.join([*(operand(v) for v in args), operator])).encode('ascii'))
        spans.append((start, len(output)))
        output.append(10)
    return bytes(output), spans


def json_numbers(value):
    if isinstance(value, F):
        return float(value)
    if isinstance(value, (list, tuple)):
        return [json_numbers(v) for v in value]
    if isinstance(value, dict):
        return {k: json_numbers(v) for k,v in value.items()}
    return value


class Model:
    def __init__(self, fonts):
        self.fonts = fonts
        self.state = {'ctm': IDENTITY, 'font': None, 'size': F(0), 'tc': F(0), 'tw': F(0), 'h': F(1), 'leading': F(0), 'rise': F(0), 'mode': 0}
        self.stack = []
        self.tm = self.tlm = None
        self.object = -1
        self.codes, self.adjustments, self.ends, self.events = [], [], [], []
        self.utf8 = bytearray()
        self.source = bytearray()
        self.op_index = 0
        self.span = None

    def move_line(self, x, y):
        self.tlm = compose(translation(x,y), self.tlm)
        self.tm = self.tlm

    def show(self, source, operand_index):
        s = self.state
        font = self.fonts[s['font']]
        for index, byte in enumerate(source):
            before = self.tm
            width = F(font['widths'].get(byte, font.get('missing', 0)))
            origin = 'widths' if byte in font['widths'] else 'declared_missing' if 'missing' in font else 'default_missing'
            if 'builtin' in font:
                origin = 'builtin:' + font['builtin']
            text = font['mapping'][byte]
            text_range = (len(self.utf8), len(self.utf8) + len(text.encode('utf-8')))
            self.utf8.extend(text.encode('utf-8'))
            source_range = (len(self.source), len(self.source)+1)
            self.source.append(byte)
            em = (s['size']*s['h'], F(0), F(0), s['size'], F(0), s['rise'])
            rendering = compose(compose(em,before),s['ctm'])
            glyph_dx = (width/F(1000)*s['size'] + s['tc'])*s['h']
            word_dx = s['tw']*s['h'] if byte == 32 else F(0)
            self.tm = compose(translation(glyph_dx,0),self.tm)
            self.tm = compose(translation(word_dx,0),self.tm)
            displacement = (s['ctm'][0]*(self.tm[4]-before[4])+s['ctm'][2]*(self.tm[5]-before[5]), s['ctm'][1]*(self.tm[4]-before[4])+s['ctm'][3]*(self.tm[5]-before[5]))
            row = {'byte':byte, 'font':s['font'], 'object':self.object, 'operation':self.op_index, 'span':self.span, 'operand':operand_index, 'decoded_range':(index,index+1), 'source_range':source_range, 'utf8_range':text_range, 'text':text, 'width':width, 'origin':origin, 'before':before, 'after':self.tm, 'rendering':rendering, 'page_origin':rendering[4:], 'page_displacement':displacement, 'glyph_dx':glyph_dx, 'word_dx':word_dx, 'mode':s['mode'], 'size':s['size'], 'horizontal_scale':s['h'], 'rise':s['rise'], 'character_spacing':s['tc'], 'word_spacing':s['tw'] if byte==32 else F(0)}
            self.events.append(('code',len(self.codes)))
            self.codes.append(row)

    def push(self, op, args):
        s = self.state
        if op == 'q': self.stack.append(copy.copy(s))
        elif op == 'Q': self.state = self.stack.pop()
        elif op == 'cm': s['ctm'] = compose(tuple(map(F,args)),s['ctm'])
        elif op == 'BT':
            self.tm = self.tlm = IDENTITY
            self.object += 1
        elif op == 'ET':
            self.events.append(('end',len(self.ends)))
            self.ends.append({'object':self.object,'operation':self.op_index,'span':self.span,'matrix':self.tm,'line_matrix':self.tlm})
            self.tm = self.tlm = None
        elif op == 'Tf': s['font'],s['size'] = args[0],F(args[1])
        elif op in ('Tc','Tw','TL','Ts','Tr'):
            s[{'Tc':'tc','Tw':'tw','TL':'leading','Ts':'rise','Tr':'mode'}[op]] = F(args[0]) if op!='Tr' else int(args[0])
        elif op == 'Tz': s['h'] = F(args[0])/100
        elif op == 'Tm': self.tm = self.tlm = tuple(map(F,args))
        elif op in ('Td','TD'):
            if op=='TD': s['leading'] = -F(args[1])
            self.move_line(*args)
        elif op == 'T*': self.move_line(0,-s['leading'])
        elif op == 'Tj': self.show(args[0],0)
        elif op == "'":
            self.move_line(0,-s['leading'])
            self.show(args[0],0)
        elif op == '"':
            s['tw'],s['tc'] = F(args[0]),F(args[1])
            self.move_line(0,-s['leading'])
            self.show(args[2],2)
        elif op == 'TJ':
            for index,item in enumerate(args[0]):
                if isinstance(item,bytes): self.show(item,index)
                else:
                    before = self.tm
                    dx = -F(item)/1000*s['size']*s['h']
                    self.tm = compose(translation(dx,0),self.tm)
                    self.events.append(('adjustment',len(self.adjustments)))
                    self.adjustments.append({'object':self.object,'operation':self.op_index,'span':self.span,'element':index,'value':F(item),'dx':dx,'before':before,'after':self.tm})
        else: raise ValueError(op)

    def run(self, operations):
        raw,spans = content(operations)
        for self.op_index,((op,args),self.span) in enumerate(zip(operations,spans)):
            self.push(op,args)
        assert not self.stack and self.tm is None
        return raw, {'codes':self.codes,'adjustments':self.adjustments,'ends':self.ends,'events':self.events,'source_hex':self.source.hex(),'utf8_hex':self.utf8.hex()}


def generate(seed=2026100313, count=240):
    rng = random.Random(seed)
    for case in range(count):
        native = case < 160
        widths = {code: F(rng.choice([0,125,250,375,500,625,750,875,1000])) for code in range(32,91)}
        if not native:
            widths[65] = rng.choice([F(-125,2),F(125,4),F(0)])
        # Deliberately use source-code word spacing, not Unicode-space spacing.
        mapping = {code:chr(code) for code in range(32,91)}
        mapping.update({32:'Z',65:'fi',66:' ',67:'\u00a0',68:'\U0001f642',69:'e\u0301'})
        fonts = {'F0':{'widths':widths,'mapping':mapping}, 'F1':{'widths':{code:F(500) for code in range(32,91)},'mapping':mapping}}
        size = rng.choice([F(8),F(12),F(25,2)]) if native else rng.choice([F(-8),F(0),F(9,2)])
        scale = rng.choice([50,100,125,150]) if native else rng.choice([-100,0,25])
        ctm = rng.choice([(1,0,0,1,3,5),(0,1,-1,0,80,3),(1,F(1,2),F(1,4),1,3,-2),(-1,0,0,1,80,0)])
        tm = rng.choice([(1,0,0,1,10,20),(0,1,-1,0,30,20),(1,F(1,2),F(1,4),1,2,3)])
        ops = [('q',[]),('cm',list(ctm)),('Tf',['F0',size]),('Tc',[rng.choice([F(0),F(1,2),F(-1,4)])]),('Tw',[rng.choice([F(0),F(3,2),F(-1)])]),('Tz',[scale]),('TL',[F(7,2)]),('Ts',[rng.choice([F(0),F(1,2),F(-2)])]),('Tr',[0 if case%2 else 3]),('BT',[]),('Tm',list(tm)),('Tj',[b'A B']),('TJ',[[b'C',125,-250,b'DE',b'',F(375)]]),('Td',[F(3,2),-2]),("'",[b'A']),('TD',[2,-5]),('T*',[]),('"',[F(5,2),F(-1,2),b' B']),('Tf',['F1',size]),('Tj',[b'A']),('TJ',[[250,-125]]),('ET',[]),('BT',[]),('Tj',[b'A']),('ET',[]),('Q',[]),('Tf',['F0',8]),('BT',[]),('TJ',[[]]),('Tj',[b'']),('Tj',[b'B']),('ET',[])]
        yield {'case':case,'native':native,'fonts':fonts,'operations':ops}


if __name__ == '__main__':
    programs = codes = 0
    for spec in generate():
        data, trace = Model(spec['fonts']).run(spec['operations'])
        programs += 1
        codes += len(trace['codes'])
        assert len(trace['codes']) < 100 and len(data) < 4096
    print(f'{programs} independent rational programs, {codes} expected source-code placements')
