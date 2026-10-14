#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent bounded mixed path/text model, reusing the exact T0 arithmetic oracle."""
import copy
import random
from fractions import Fraction as F
from text_position_model import Model as TextModel, content, compose, IDENTITY

TEXT_OPS={'BT','ET','Tf','Tc','Tw','Tz','TL','Ts','Tr','Tm','Td','TD','T*','Tj','TJ',"'",'"'}
SHARED={'q','Q','cm'}
STYLE={'g','G','rg','RG','k','K','w','J','j','M','d'}


class ProgramModel(TextModel):
    def __init__(self,fonts):
        super().__init__(fonts)
        self.path_state={'matrix':IDENTITY,'fill':{'space':'gray','values':[0]},'stroke':{'space':'gray','values':[0]},'style':{'width':1,'start_cap':0,'dash_cap':0,'end_cap':0,'join':0,'miter':10,'phase':0,'dash':[]},'clip':None}
        self.path_stack=[]
        self.path=[]
        self.point=self.start=None
        self.pending=None
        self.path_events=[]
        self.clips=[]
        self.contexts=[]
        self.ordered=[]

    def context(self):
        return {'ctm':self.path_state['matrix'],'clip':self.path_state['clip'],'fill_color':copy.deepcopy(self.path_state['fill'])}

    def emit_path(self,kind,rule=None):
        s=self.path_state
        event={'kind':kind,'clip':s['clip'],'path':copy.deepcopy(self.path),'matrix':s['matrix'],'span':self.span,'operation':self.op_index}
        if kind in ('fill','clip'):event['rule']=rule
        if kind in ('fill','stroke'):event['color']=copy.deepcopy(s[kind])
        if kind=='stroke':event['style']=copy.deepcopy(s['style'])
        if kind=='clip':
            event['id']=len(self.clips)
            self.clips.append(event)
            s['clip']=event['id']
        self.ordered.append({'kind':'path','index':len(self.path_events),'operation':self.op_index,'span':self.span})
        self.path_events.append(event)

    def path_push(self,op,args):
        s=self.path_state
        if op=='q':self.path_stack.append(copy.deepcopy(s))
        elif op=='Q':
            previous=self.path_stack.pop()
            if previous['clip']!=s['clip']:
                self.ordered.append({'kind':'path','index':len(self.path_events),'operation':self.op_index,'span':self.span})
                self.path_events.append({'kind':'restore','clip':previous['clip'],'span':self.span,'operation':self.op_index})
            self.path_state=previous
        elif op=='cm':s['matrix']=compose(tuple(map(F,args)),s['matrix'])
        elif op in ('g','G','rg','RG','k','K'):
            s['fill' if op.islower() else 'stroke']={'space':{1:'gray',3:'rgb',4:'cmyk'}[len(args)],'values':list(args)}
        elif op in ('w','J','j','M'):
            for key in {'w':['width'],'J':['start_cap','dash_cap','end_cap'],'j':['join'],'M':['miter']}[op]:s['style'][key]=args[0]
        elif op=='d':s['style']['dash']=list(args[0]);s['style']['phase']=args[1]
        elif op=='m':self.path.append(['m',*args]);self.point=list(args);self.start=list(args)
        elif op=='l':self.path.append(['l',*args]);self.point=list(args)
        elif op=='c':self.path.append(['c',*args]);self.point=list(args[-2:])
        elif op=='h':
            if not self.path or self.path[-1]!=['h']:self.path.append(['h'])
            self.point=self.start
        elif op=='re':
            x,y,w,h=args
            self.path.extend([['m',x,y],['l',x+w,y],['l',x+w,y+h],['l',x,y+h],['h']]);self.point=[x,y];self.start=[x,y]
        elif op in ('W','W*'):self.pending='evenodd' if op=='W*' else 'nonzero'
        elif op in ('S','f','f*','B','B*','n'):
            if self.path:
                if op in ('f','f*','B','B*'):self.emit_path('fill','evenodd' if '*' in op else 'nonzero')
                if op in ('S','B','B*'):self.emit_path('stroke')
            if self.pending:self.emit_path('clip',self.pending)
            self.path=[];self.pending=None;self.point=self.start=None
        else:raise ValueError(op)

    def push(self,op,args):
        if op in SHARED:
            assert self.tm is None
            self.path_push(op,args)
            super().push(op,args)
            assert self.path_state['matrix']==self.state['ctm']
        elif op in TEXT_OPS:
            if op=='BT':assert not self.path and self.pending is None
            old=len(self.events)
            snapshot=self.context()
            super().push(op,args)
            for i in range(old,len(self.events)):
                # One immutable context is sufficient for each operation's new events.
                self.ordered.append({'kind':'text','index':i,'context':snapshot,'operation':self.op_index,'span':self.span})
        else:
            assert op in STYLE or self.tm is None
            self.path_push(op,args)

    def run(self,operations):
        raw,text=super().run(operations)
        assert not self.path_stack and not self.path and self.pending is None
        return raw,{'text':text,'path_events':self.path_events,'clips':self.clips,'ordered':self.ordered}


def generate_programs(count=220,seed=2026100314):
    rng=random.Random(seed)
    mapping={32:'Z',65:'fi',66:' ',67:'\u00a0',68:'\U0001f642'}
    for case in range(count):
        fonts={label:{'mapping':mapping,'widths':{code:F(rng.choice([125,250,500,750,1000])) for code in range(32,69)}} for label in ['F0','F1']}
        # Mapping intentionally only covers shown codes; width declarations still contiguous.
        ctm=rng.choice([(1,0,0,1,3,5),(0,1,-1,0,40,30),(1,F(1,2),F(1,4),1,2,-3),(-1,0,0,1,60,0)])
        palette=rng.choice([(1,0,0),(0,1,0),(0,0,1),(1,1,0)])
        ops=[('Tf',['F0',8]),('Tc',[F(1,2)]),('Tw',[2]),('Tz',[125]),('TL',[5]),('Ts',[F(1,2)]),('rg',list(palette)),('q',[]),('cm',list(ctm)),('w',[rng.choice([F(1,2),1,2])]),('RG',[0,0,1]),('d',[[2,3],1]),('re',[0,0,30,30]),('W*' if case%2 else 'W',[]),('B*' if case%2 else 'B',[]),('BT',[]),('Tm',[1,F(1,2),0,1,3,4]),('Tj',[b'A B']),('rg',[0,1,1]),('w',[3]),('TJ',[[125,b'C',-250,b'D']]),('ET',[]),('q',[]),('re',[2,2,10,10]),('W',[]),('n',[]),('Tf',['F1',12]),('Tc',[1]),('Tw',[3]),('Tz',[50]),('Ts',[-1]),('Tr',[3]),('BT',[]),('Tj',[b'A']),('g',[F(1,2)]),('Tj',[b' B']),('ET',[]),('Q',[]),('BT',[]),("'",[b'A']),('ET',[]),('m',[1,2]),('l',[4,5]),('S',[]),('Q',[]),('BT',[]),('Tj',[b'B']),('ET',[]),('re',[0,0,2,3]),('f',[])]
        if case % 3 == 1:
            ops[6]=('g',[F(1,4)])
        elif case % 3 == 2:
            ops[6]=('k',[0,1,1,0])
        yield {'case':case,'native':True,'fonts':fonts,'operations':ops}


if __name__=='__main__':
    count=events=0
    for spec in generate_programs():
        raw,expected=ProgramModel(spec['fonts']).run(spec['operations'])
        count+=1;events+=len(expected['ordered'])
        assert len(raw)<4096
    print(count,'mixed programs;',events,'ordered events')
